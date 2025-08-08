//! True Adaptive Execution - Act → Analyze Concurrently → Adapt Dynamically
//! 
//! This module implements the EXPERTS.md vision correctly:
//! - NO upfront analysis or categorization
//! - Walk files and dispatch IMMEDIATELY based on size
//! - Self-managing components (Dam, Pool, Slicer)
//! - Dynamic threshold adaptation based on observed performance
//! - Zero collection phase - pure streaming

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use anyhow::Result;
use walkdir::WalkDir;

use crate::sync_stats::SyncStats;
use crate::options::SyncOptions;
use crate::thread_pool;

// ========== DYNAMIC THRESHOLDS (Adapt Dynamically) ==========
// These adapt based on observed performance - the system learns!

/// Dynamic flush thresholds for SmallFileDam
pub struct DynamicThresholds {
    // Time threshold: How often to flush (milliseconds)
    pub flush_time_ms: AtomicU64,
    // Count threshold: How many files trigger flush
    pub flush_count: AtomicUsize,
    // Size threshold: Total bytes that trigger flush
    pub flush_size_bytes: AtomicU64,
    // Pool batch size: Files per task
    pub pool_batch_size: AtomicUsize,
    // Throughput tracking for adaptation
    last_throughput_mbps: AtomicU64,
    last_adaptation_time: Mutex<Instant>,
}

impl DynamicThresholds {
    pub fn new_for_high_end_hardware() -> Self {
        Self {
            // Start aggressive for high-end hardware
            flush_time_ms: AtomicU64::new(50),        // 50ms timer
            flush_count: AtomicUsize::new(1000),      // 1000 files
            flush_size_bytes: AtomicU64::new(100_000_000), // 100MB
            pool_batch_size: AtomicUsize::new(32),    // 32 files per batch
            last_throughput_mbps: AtomicU64::new(0),
            last_adaptation_time: Mutex::new(Instant::now()),
        }
    }
    
    /// Adapt thresholds based on observed performance
    pub fn adapt(&self, current_throughput_mbps: f64, queue_depth: usize, is_network: bool) {
        let mut last_time = self.last_adaptation_time.lock().unwrap();
        
        // Only adapt every second to avoid thrashing
        if last_time.elapsed() < Duration::from_secs(1) {
            return;
        }
        *last_time = Instant::now();
        
        let last_throughput = self.last_throughput_mbps.load(Ordering::Relaxed) as f64;
        self.last_throughput_mbps.store(current_throughput_mbps as u64, Ordering::Relaxed);
        
        // If throughput is improving, be more aggressive
        if current_throughput_mbps > last_throughput * 1.1 {
            // Increase batch sizes
            let new_count = self.flush_count.load(Ordering::Relaxed).saturating_mul(120) / 100;
            self.flush_count.store(new_count.min(5000), Ordering::Relaxed);
            
            let new_size = self.flush_size_bytes.load(Ordering::Relaxed).saturating_mul(120) / 100;
            self.flush_size_bytes.store(new_size.min(500_000_000), Ordering::Relaxed);
            
            let new_batch = self.pool_batch_size.load(Ordering::Relaxed).saturating_mul(120) / 100;
            self.pool_batch_size.store(new_batch.min(64), Ordering::Relaxed);
        }
        // If throughput is degrading, back off
        else if current_throughput_mbps < last_throughput * 0.9 {
            // Decrease batch sizes
            let new_count = self.flush_count.load(Ordering::Relaxed).saturating_mul(80) / 100;
            self.flush_count.store(new_count.max(100), Ordering::Relaxed);
            
            let new_size = self.flush_size_bytes.load(Ordering::Relaxed).saturating_mul(80) / 100;
            self.flush_size_bytes.store(new_size.max(10_000_000), Ordering::Relaxed);
            
            let new_batch = self.pool_batch_size.load(Ordering::Relaxed).saturating_mul(80) / 100;
            self.pool_batch_size.store(new_batch.max(16), Ordering::Relaxed);
        }
        
        // Adjust for network vs local
        if is_network && queue_depth > 100 {
            // Network with deep queue - flush faster
            self.flush_time_ms.store(25, Ordering::Relaxed);
        } else if !is_network {
            // Local filesystem - can batch more
            self.flush_time_ms.store(100, Ordering::Relaxed);
        }
    }
}

// ========== SELF-MANAGING SMALL FILE DAM ==========

pub struct AdaptiveSmallFileDam {
    buffer: Arc<Mutex<Vec<FileEntry>>>,
    thresholds: Arc<DynamicThresholds>,
    flush_handle: Option<thread::JoinHandle<()>>,
    dest_root: PathBuf,
    stats: Arc<Mutex<SyncStats>>,
    is_network: bool,
}

#[derive(Clone)]
pub struct FileEntry {
    pub src_path: PathBuf,
    pub dst_path: PathBuf,
    pub size: u64,
}

impl AdaptiveSmallFileDam {
    pub fn new(
        dest_root: PathBuf,
        stats: Arc<Mutex<SyncStats>>,
        thresholds: Arc<DynamicThresholds>,
        is_network: bool,
    ) -> Self {
        let buffer = Arc::new(Mutex::new(Vec::<FileEntry>::new()));
        
        // Clone for timer thread
        let buffer_clone = Arc::clone(&buffer);
        let thresholds_clone = Arc::clone(&thresholds);
        let dest_clone = dest_root.clone();
        let stats_clone = Arc::clone(&stats);
        
        // Start autonomous flush timer thread (self-managing!)
        let flush_handle: thread::JoinHandle<()> = thread::spawn(move || {
            let mut last_flush = Instant::now();
            
            loop {
                let flush_time_ms = thresholds_clone.flush_time_ms.load(Ordering::Relaxed);
                thread::sleep(Duration::from_millis(flush_time_ms));
                
                let should_flush = {
                    let buffer = buffer_clone.lock().unwrap();
                    let elapsed = last_flush.elapsed();
                    let flush_count = thresholds_clone.flush_count.load(Ordering::Relaxed);
                    let flush_size = thresholds_clone.flush_size_bytes.load(Ordering::Relaxed);
                    
                    // Dynamic thresholds!
                    elapsed >= Duration::from_millis(flush_time_ms) ||
                    buffer.len() >= flush_count ||
                    buffer.iter().map(|f| f.size).sum::<u64>() >= flush_size
                };
                
                if should_flush {
                    Self::flush_internal(&buffer_clone, &dest_clone, &stats_clone);
                    last_flush = Instant::now();
                }
            }
        });
        
        Self {
            buffer,
            thresholds,
            flush_handle: Some(flush_handle),
            dest_root,
            stats,
            is_network,
        }
    }
    
    /// Add file to dam - that's it! Dam decides when to flush
    pub fn add_file(&self, file: FileEntry) {
        let mut buffer = self.buffer.lock().unwrap();
        buffer.push(file);
        // No analysis here - just add and let timer handle it
    }
    
    fn flush_internal(
        buffer: &Arc<Mutex<Vec<FileEntry>>>,
        _dest_root: &Path,
        stats: &Arc<Mutex<SyncStats>>,
    ) {
        let files_to_process = {
            let mut buffer = buffer.lock().unwrap();
            if buffer.is_empty() {
                return;
            }
            std::mem::take(&mut *buffer)
        };
        
        // Process via tar streaming for batches, direct for small
        if files_to_process.len() >= 15 {
            Self::process_tar_batch(files_to_process, stats);
        } else {
            Self::process_individual(files_to_process, stats);
        }
    }
    
    fn process_tar_batch(files: Vec<FileEntry>, stats: &Arc<Mutex<SyncStats>>) {
        // TODO: Implement in-memory tar streaming
        // For now, just copy individually using thread pool
        Self::process_individual(files, stats);
    }
    
    fn process_individual(files: Vec<FileEntry>, stats: &Arc<Mutex<SyncStats>>) {
        thread_pool::GLOBAL_THREAD_POOL.scope(|s| {
            for file in files {
                let stats_clone = Arc::clone(stats);
                s.spawn(move |_| {
                    // Ensure destination directory exists
                    if let Some(parent) = file.dst_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    
                    // Copy file
                    if std::fs::copy(&file.src_path, &file.dst_path).is_ok() {
                        let stats = stats_clone.lock().unwrap();
                        stats.increment_files_copied();
                        stats.add_bytes_transferred(file.size);
                    }
                });
            }
        });
    }
    
    pub fn final_flush(&self) {
        Self::flush_internal(&self.buffer, &self.dest_root, &self.stats);
    }
}

// ========== SELF-MANAGING MEDIUM FILE POOL ==========

pub struct AdaptiveMediumFilePool {
    batch_queue: Arc<Mutex<Vec<FileEntry>>>,
    thresholds: Arc<DynamicThresholds>,
    stats: Arc<Mutex<SyncStats>>,
}

impl AdaptiveMediumFilePool {
    pub fn new(stats: Arc<Mutex<SyncStats>>, thresholds: Arc<DynamicThresholds>) -> Self {
        Self {
            batch_queue: Arc::new(Mutex::new(Vec::new())),
            thresholds,
            stats,
        }
    }
    
    pub fn process_file(&self, file: FileEntry) {
        let mut queue = self.batch_queue.lock().unwrap();
        queue.push(file);
        
        // Check if we have enough for a batch (dynamic!)
        let batch_size = self.thresholds.pool_batch_size.load(Ordering::Relaxed);
        if queue.len() >= batch_size {
            let batch: Vec<_> = queue.drain(..batch_size).collect();
            drop(queue); // Release lock before dispatching
            self.dispatch_batch(batch);
        }
    }
    
    fn dispatch_batch(&self, batch: Vec<FileEntry>) {
        let stats_clone = Arc::clone(&self.stats);
        
        // Single task processes entire batch (reduces synchronization)
        thread_pool::GLOBAL_THREAD_POOL.spawn(move || {
            for file in batch {
                if let Some(parent) = file.dst_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                
                if std::fs::copy(&file.src_path, &file.dst_path).is_ok() {
                    let stats = stats_clone.lock().unwrap();
                    stats.increment_files_copied();
                    stats.add_bytes_transferred(file.size);
                }
            }
        });
    }
    
    pub fn flush_remaining(&self) {
        let batch = {
            let mut queue = self.batch_queue.lock().unwrap();
            std::mem::take(&mut *queue)
        };
        if !batch.is_empty() {
            self.dispatch_batch(batch);
        }
    }
}

// ========== LARGE FILE SLICER ==========

pub struct AdaptiveLargeFileSlicer {
    stats: Arc<Mutex<SyncStats>>,
}

impl AdaptiveLargeFileSlicer {
    pub fn new(stats: Arc<Mutex<SyncStats>>) -> Self {
        Self { stats }
    }
    
    pub fn process_file(&self, file: FileEntry) {
        let stats_clone = Arc::clone(&self.stats);
        
        // Process large file immediately with dedicated resources
        thread_pool::GLOBAL_THREAD_POOL.spawn(move || {
            if let Some(parent) = file.dst_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            
            // TODO: Implement chunked parallel copy for very large files
            // For now, simple copy
            if std::fs::copy(&file.src_path, &file.dst_path).is_ok() {
                let stats = stats_clone.lock().unwrap();
                stats.increment_files_copied();
                stats.add_bytes_transferred(file.size);
            }
        });
    }
}

// ========== PURE STREAMING DISPATCHER ==========

pub struct StreamingDispatcher {
    dam: Arc<AdaptiveSmallFileDam>,
    pool: Arc<AdaptiveMediumFilePool>,
    slicer: Arc<AdaptiveLargeFileSlicer>,
    thresholds: Arc<DynamicThresholds>,
}

impl StreamingDispatcher {
    pub fn new(dest_root: PathBuf, stats: Arc<Mutex<SyncStats>>, is_network: bool) -> Self {
        let thresholds = Arc::new(DynamicThresholds::new_for_high_end_hardware());
        
        Self {
            dam: Arc::new(AdaptiveSmallFileDam::new(
                dest_root,
                Arc::clone(&stats),
                Arc::clone(&thresholds),
                is_network,
            )),
            pool: Arc::new(AdaptiveMediumFilePool::new(Arc::clone(&stats), Arc::clone(&thresholds))),
            slicer: Arc::new(AdaptiveLargeFileSlicer::new(Arc::clone(&stats))),
            thresholds,
        }
    }
    
    /// Pure dispatch - no analysis, just size-based routing
    /// Components self-manage their workloads
    pub fn dispatch_file(&self, file: FileEntry) {
        // Act immediately based on size - that's the strategy!
        match file.size {
            s if s < 1_000_000 => self.dam.add_file(file),
            s if s < 100_000_000 => self.pool.process_file(file),
            _ => self.slicer.process_file(file),
        }
    }
    
    pub fn adapt_thresholds(&self, throughput_mbps: f64, queue_depth: usize, is_network: bool) {
        self.thresholds.adapt(throughput_mbps, queue_depth, is_network);
    }
    
    pub fn finish(&self) {
        self.dam.final_flush();
        self.pool.flush_remaining();
    }
}

// ========== TRUE ADAPTIVE SYNC - THE EXPERTS.MD VISION ==========

/// Execute adaptive sync with true streaming dispatch
/// Walk → Size check → Immediate dispatch → Self-managing components
pub fn execute_adaptive_sync(
    source: &Path,
    dest: &Path,
    options: &SyncOptions,
) -> Result<SyncStats> {
    let start_time = Instant::now();
    let stats = Arc::new(Mutex::new(SyncStats::default()));
    
    // Detect if destination is network
    let is_network = false; // TODO: Detect network filesystem properly
    
    let dispatcher = StreamingDispatcher::new(dest.to_path_buf(), Arc::clone(&stats), is_network);
    
    // Track metrics for adaptation
    let mut files_seen = 0u64;
    let mut last_adapt_time = Instant::now();
    
    // Walk and dispatch immediately - NO COLLECTION PHASE!
    for entry in WalkDir::new(source)
        .follow_links(false) // TODO: Get from options
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        
        let src_path = entry.path();
        let relative = src_path.strip_prefix(source)?;
        let dst_path = dest.join(relative);
        
        // Get metadata ONCE during walk
        let metadata = entry.metadata()?;
        
        // Create FileEntry
        let file_entry = FileEntry {
            src_path: src_path.to_path_buf(),
            dst_path,
            size: metadata.len(),
        };
        
        // IMMEDIATE dispatch based on size - no analysis!
        dispatcher.dispatch_file(file_entry);
        
        files_seen += 1;
        
        // Adapt thresholds periodically (Analyze Concurrently → Adapt Dynamically)
        if last_adapt_time.elapsed() > Duration::from_secs(1) {
            let current_stats = stats.lock().unwrap();
            let elapsed = start_time.elapsed().as_secs_f64();
            let bytes_transferred = current_stats.bytes_transferred();
            let throughput_mbps = if elapsed > 0.0 {
                (bytes_transferred as f64 / elapsed) / 1_048_576.0
            } else {
                0.0
            };
            let files_copied = current_stats.files_copied();
            drop(current_stats);
            
            // Queue depth approximation
            let queue_depth = (files_seen - files_copied) as usize;
            
            dispatcher.adapt_thresholds(throughput_mbps, queue_depth, is_network);
            last_adapt_time = Instant::now();
        }
    }
    
    // Final flush
    dispatcher.finish();
    
    // Brief wait for final operations
    thread::sleep(Duration::from_millis(100));
    
    // Return stats
    let final_stats = stats.lock().unwrap().clone();
    Ok(final_stats)
}