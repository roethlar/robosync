//! Mixed-mode strategy that uses different copy methods for different file types
//!
//! This module implements an intelligent mixed strategy that:
//! - Uses parallel batch operations for small files
//! - Uses delta transfer for large modified files
//! - Uses platform APIs for medium files
//! - Can batch small files to native tools for efficiency

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use rayon::prelude::*;

use crate::file_list::{FileInfo, FileOperation};
use crate::fast_file_list::{FastFileListGenerator, FastEnumConfig};
use crate::options::SyncOptions;
use crate::parallel_sync::ParallelSyncer;
use crate::platform_api::PlatformCopier;
use crate::sync_stats::SyncStats;
use crate::progress::SyncProgress;
use crate::checksum::ChecksumType;
use crate::formatted_display::{self, WorkerStats};

/// Size thresholds for categorizing files - optimized for performance
const SMALL_FILE_THRESHOLD: u64 = 256 * 1024;        // 256KB - increased for better batching
const MEDIUM_FILE_THRESHOLD: u64 = 10 * 1024 * 1024; // 10MB

/// Batch size for small file operations - larger for better performance
const SMALL_FILE_BATCH_SIZE: usize = 5000;

/// Number of parallel threads for small files
const SMALL_FILE_THREADS: usize = 32;

/// Format bytes into human readable string
fn humanize_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let exponent = (bytes as f64).log(1024.0).floor() as usize;
    let exponent = exponent.min(UNITS.len() - 1);
    let value = bytes as f64 / 1024_f64.powi(exponent as i32);
    if exponent == 0 {
        format!("{} {}", bytes, UNITS[exponent])
    } else {
        format!("{:.1} {}", value, UNITS[exponent])
    }
}

/// Format number with thousands separators
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    
    for (i, &ch) in chars.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    
    result.chars().rev().collect()
}

/// Mixed strategy executor
#[derive(Clone)]
pub struct MixedStrategyExecutor {
    progress: Arc<SyncProgress>,
}

impl MixedStrategyExecutor {
    pub fn new(total_files: u64, total_bytes: u64) -> Self {
        Self { 
            progress: Arc::new(SyncProgress::new_silent(total_files, total_bytes))
        }
    }
    
    /// Execute mixed strategy synchronization
    pub fn execute(
        &self,
        operations: Vec<FileOperation>,
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        use std::sync::mpsc;
        use std::thread;
        use std::collections::HashMap;
        
        // Categorize files by size and type
        let categorized = self.categorize_operations(operations);
        let operation_count = categorized.total_operations();
        
        if operation_count == 0 {
            println!("\n  ℹ️  No operations to perform - all files are up to date!");
            return Ok(SyncStats::default());
        }
        
        // Calculate file and size statistics
        let (file_stats, size_stats) = self.calculate_stats(&categorized, source_root);
        
        // Print strategy selection
        println!("  Automatically selected strategy: Mixed mode");
        println!("  ───────────────────────────────────────────────────────────────────────────────");
        
        // Print file analysis
        formatted_display::print_file_analysis(
            file_stats.total,
            file_stats.small,
            file_stats.medium,
            file_stats.large,
            size_stats.total,
            size_stats.small,
            size_stats.medium,
            size_stats.large,
        );
        
        // Print pending operations  
        let pending_stats = self.calculate_pending_stats(&categorized, source_root);
        formatted_display::print_pending_operations(
            pending_stats.files_create,
            pending_stats.files_update,
            pending_stats.files_delete,
            0, // files_skip - we don't skip in this implementation
            pending_stats.dirs_create,
            0, // dirs_update
            0, // dirs_delete
            0, // dirs_skip
            pending_stats.size_create,
            pending_stats.size_update,
            pending_stats.size_delete,
            0, // size_skip
        );
        
        // Process directories first (must be done before files)
        self.create_directories(&categorized.directories, source_root, dest_root)?;
        
        // Create progress bar
        let pb = formatted_display::create_progress_bar(operation_count);
        pb.set_message(format!("{} | 0 B/s", humanize_bytes(0)));
        
        // Setup for parallel execution
        let (tx, rx) = mpsc::channel();
        let mut handles = vec![];
        let mut worker_info = HashMap::new();
        let start_time = std::time::Instant::now();
        
        // Process small files in parallel
        if !categorized.small_files.is_empty() {
            let tx = tx.clone();
            let small_files = categorized.small_files.clone();
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            let executor = self.clone();
            let pb_clone = pb.clone();
            let worker_start = std::time::Instant::now();
            
            worker_info.insert("small", ("Small (<256KB)".to_string(), categorized.small_files.len() as u64, worker_start));
            
            let handle = thread::spawn(move || {
                let stats = executor.process_small_files_batch(
                    &small_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                );
                tx.send(("small", stats, worker_start.elapsed())).unwrap();
            });
            handles.push(handle);
        }
        
        // Process medium files in parallel
        if !categorized.medium_files.is_empty() {
            let tx = tx.clone();
            let medium_files = categorized.medium_files.clone();
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            let executor = self.clone();
            let pb_clone = pb.clone();
            let worker_start = std::time::Instant::now();
            
            worker_info.insert("medium", ("Medium (256KB-10MB)".to_string(), categorized.medium_files.len() as u64, worker_start));
            
            let handle = thread::spawn(move || {
                let stats = executor.process_medium_files(
                    &medium_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                );
                tx.send(("medium", stats, worker_start.elapsed())).unwrap();
            });
            handles.push(handle);
        }
        
        // Process large files in parallel
        if !categorized.large_files.is_empty() {
            let tx = tx.clone();
            let large_files = categorized.large_files.clone();
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            let executor = self.clone();
            let pb_clone = pb.clone();
            let worker_start = std::time::Instant::now();
            
            worker_info.insert("large", ("Large (>10MB)".to_string(), categorized.large_files.len() as u64, worker_start));
            
            let handle = thread::spawn(move || {
                let stats = executor.process_large_files(
                    &large_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                );
                tx.send(("large", stats, worker_start.elapsed())).unwrap();
            });
            handles.push(handle);
        }
        
        // Process deletes in parallel (after all creates/updates start)
        if !categorized.deletes.is_empty() {
            let tx = tx.clone();
            let deletes = categorized.deletes.clone();
            let options = options.clone();
            let executor = self.clone();
            let pb_clone = pb.clone();
            let worker_start = std::time::Instant::now();
            
            worker_info.insert("delete", ("Delete operations".to_string(), categorized.deletes.len() as u64, worker_start));
            
            let handle = thread::spawn(move || {
                let stats = executor.process_deletes(
                    &deletes,
                    &options,
                    Some(&pb_clone),
                );
                tx.send(("delete", stats, worker_start.elapsed())).unwrap();
            });
            handles.push(handle);
        }
        
        // Drop the original sender so rx.recv() will end when all threads finish
        drop(tx);
        
        // Collect results from all threads
        let mut total_stats = SyncStats::default();
        let mut worker_stats = Vec::new();
        
        for (worker_type, result, duration) in rx {
            match result {
                Ok(stats) => {
                    if let Some((name, count, _)) = worker_info.get(worker_type) {
                        let bytes = stats.bytes_transferred();
                        let throughput = if duration.as_secs() > 0 {
                            bytes / duration.as_secs()
                        } else {
                            bytes
                        };
                        
                        worker_stats.push(WorkerStats {
                            name: name.clone(),
                            files: if worker_type == "delete" { stats.files_deleted() } else { stats.files_copied() },
                            bytes,
                            duration_secs: duration.as_secs_f32(),
                            throughput,
                        });
                    }
                    total_stats = self.merge_stats(total_stats, stats);
                }
                Err(e) => {
                    eprintln!("Worker {} failed: {}", worker_type, e);
                    total_stats.increment_errors();
                }
            }
        }
        
        // Wait for all threads to complete
        for handle in handles {
            let _ = handle.join();
        }
        
        // Finish progress bar
        pb.finish_and_clear();
        
        println!("\n  ───────────────────────────────────────────────────────────────────────────────");
        
        // Print worker performance
        formatted_display::print_worker_performance(worker_stats);
        
        // Final summary
        let elapsed = start_time.elapsed();
        let throughput = if elapsed.as_secs() > 0 {
            total_stats.bytes_transferred() / elapsed.as_secs()
        } else {
            total_stats.bytes_transferred()
        };
        
        if total_stats.files_deleted() > 0 {
            println!("\n  ✅ Completed in {:.1}s: {} files copied, {} files deleted, {} transferred ({}/s)",
                elapsed.as_secs_f32(),
                format_number(total_stats.files_copied()),
                format_number(total_stats.files_deleted()),
                humanize_bytes(total_stats.bytes_transferred()),
                humanize_bytes(throughput)
            );
        } else {
            println!("\n  ✅ Completed in {:.1}s: {} files, {} transferred ({}/s)",
                elapsed.as_secs_f32(),
                format_number(total_stats.files_copied()),
                humanize_bytes(total_stats.bytes_transferred()),
                humanize_bytes(throughput)
            );
        }
        
        Ok(total_stats)
    }
    
    /// Categorize operations by file size and type
    fn categorize_operations(&self, operations: Vec<FileOperation>) -> CategorizedOps {
        let mut categorized = CategorizedOps::default();
        
        for op in operations {
            match &op {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    // Get file info from the operation
                    if let Ok(metadata) = std::fs::metadata(path) {
                        let size = metadata.len();
                        
                        if metadata.is_dir() {
                            categorized.directories.push(op);
                        } else if size <= SMALL_FILE_THRESHOLD {
                            categorized.small_files.push(op);
                        } else if size <= MEDIUM_FILE_THRESHOLD {
                            categorized.medium_files.push(op);
                        } else {
                            categorized.large_files.push(op);
                        }
                    }
                }
                FileOperation::CreateDirectory { .. } => {
                    categorized.directories.push(op);
                }
                FileOperation::Delete { .. } => {
                    categorized.deletes.push(op);
                }
                FileOperation::CreateSymlink { .. } | 
                FileOperation::UpdateSymlink { .. } => {
                    // Symlinks are small operations, add to small files
                    categorized.small_files.push(op);
                }
            }
        }
        
        categorized
    }
    
    /// Create directories
    fn create_directories(&self, dirs: &[FileOperation], source_root: &Path, dest_root: &Path) -> Result<()> {
        for op in dirs {
            match op {
                FileOperation::CreateDirectory { path } => {
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest = dest_root.join(relative);
                    std::fs::create_dir_all(&dest)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
    
    /// Process small files using parallel batch operations
    fn process_small_files_batch(
        &self,
        files: &[FileOperation],
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
    ) -> Result<SyncStats> {
        use crate::metadata::{copy_file_with_metadata, CopyFlags};
        
        let mut stats = SyncStats::default();
        let copy_flags = CopyFlags::from_string(&options.copy_flags);
        
        // Configure thread pool for optimal performance
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(SMALL_FILE_THREADS)
            .build()
            .unwrap();
        
        // Process in parallel using rayon with optimized settings
        let chunk_stats: Vec<_> = pool.install(|| {
            files
                .par_chunks(SMALL_FILE_BATCH_SIZE)
                .map(|chunk| {
                let mut chunk_stats = SyncStats::default();
                
                for op in chunk {
                    match op {
                        FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                            let relative = path.strip_prefix(source_root).unwrap_or(path);
                            let dest = dest_root.join(relative);
                            
                            // Create parent directory if needed
                            if let Some(parent) = dest.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            
                            // Copy the file
                            match copy_file_with_metadata(path, &dest, &copy_flags) {
                                Ok(bytes) => {
                                    chunk_stats.add_bytes_transferred(bytes);
                                    chunk_stats.increment_files_copied();
                                    
                                    // Update progress
                                    if let Some(pb) = progress_bar {
                                        pb.inc(1);
                                        pb.set_message(format!("{} | {}/s", 
                                            humanize_bytes(chunk_stats.bytes_transferred()),
                                            humanize_bytes(chunk_stats.bytes_transferred())
                                        ));
                                    }
                                    self.progress.add_file();
                                    self.progress.add_bytes(bytes);
                                }
                                Err(e) => {
                                    eprintln!("Error copying {:?}: {}", path, e);
                                    chunk_stats.increment_errors();
                                }
                            }
                        }
                        FileOperation::CreateSymlink { path, target } | 
                        FileOperation::UpdateSymlink { path, target } => {
                            let relative = path.strip_prefix(source_root).unwrap_or(path);
                            let dest = dest_root.join(relative);
                            
                            // Create parent directory if needed
                            if let Some(parent) = dest.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            
                            // Remove existing symlink if updating
                            if matches!(op, FileOperation::UpdateSymlink { .. }) {
                                let _ = std::fs::remove_file(&dest);
                            }
                            
                            // Create symlink
                            #[cfg(unix)]
                            match std::os::unix::fs::symlink(target, &dest) {
                                Ok(_) => {
                                    chunk_stats.increment_files_copied();
                                    if let Some(pb) = progress_bar {
                                        pb.inc(1);
                                    }
                                    self.progress.add_file();
                                }
                                Err(_e) => {
                                    chunk_stats.increment_errors();
                                }
                            }
                            
                            #[cfg(windows)]
                            {
                                // Windows symlink creation not implemented
                                chunk_stats.increment_errors();
                            }
                        }
                        _ => {}
                    }
                }
                
                chunk_stats
            })
            .collect()
        });
        
        // Merge all chunk statistics
        for chunk_stat in chunk_stats {
            stats = self.merge_stats(stats, chunk_stat);
        }
        
        Ok(stats)
    }
    
    /// Process medium files using platform APIs
    fn process_medium_files(
        &self,
        files: &[FileOperation],
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
    ) -> Result<SyncStats> {
        let copier = PlatformCopier::new();
        
        // Convert operations to file pairs
        let file_pairs: Vec<(PathBuf, PathBuf)> = files
            .iter()
            .filter_map(|op| match op {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest = dest_root.join(relative);
                    Some((path.clone(), dest))
                }
                _ => None,
            })
            .collect();
        
        // Copy files and update progress
        let stats = copier.copy_files(&file_pairs)?;
        
        // Update progress
        if let Some(pb) = progress_bar {
            pb.inc(stats.files_copied());
            pb.set_message(format!("{} | {}/s", 
                humanize_bytes(stats.bytes_transferred()),
                humanize_bytes(stats.bytes_transferred())
            ));
        }
        self.progress.add_bytes(stats.bytes_transferred());
        for _ in 0..stats.files_copied() {
            self.progress.add_file();
        }
        
        Ok(stats)
    }
    
    /// Process large files with appropriate strategy
    fn process_large_files(
        &self,
        files: &[FileOperation],
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
    ) -> Result<SyncStats> {
        let mut stats = SyncStats::default();
        
        for op in files {
            match op {
                FileOperation::Update { path, .. } if options.checksum => {
                    // Use delta transfer for large file updates
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest = dest_root.join(relative);
                    
                    match self.delta_copy_file(path, &dest, options) {
                        Ok(bytes) => {
                            stats.add_bytes_transferred(bytes);
                            stats.increment_files_copied();
                            
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                                pb.set_message(format!("{} | {}/s", 
                                    humanize_bytes(stats.bytes_transferred()),
                                    humanize_bytes(stats.bytes_transferred())
                                ));
                            }
                            self.progress.add_file();
                            self.progress.add_bytes(bytes);
                        }
                        Err(e) => {
                            eprintln!("Delta transfer failed for {:?}: {}", path, e);
                            stats.increment_errors();
                        }
                    }
                }
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    // Use platform API for new large files or updates without checksum
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest = dest_root.join(relative);
                    
                    let copier = PlatformCopier::new();
                    match copier.copy_file(path, &dest) {
                        Ok(bytes) => {
                            stats.add_bytes_transferred(bytes);
                            stats.increment_files_copied();
                            
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                                pb.set_message(format!("{} | {}/s", 
                                    humanize_bytes(stats.bytes_transferred()),
                                    humanize_bytes(stats.bytes_transferred())
                                ));
                            }
                            self.progress.add_file();
                            self.progress.add_bytes(bytes);
                        }
                        Err(e) => {
                            eprintln!("Copy failed for {:?}: {}", path, e);
                            stats.increment_errors();
                        }
                    }
                }
                _ => {}
            }
        }
        
        Ok(stats)
    }
    
    /// Perform delta copy for a single file
    fn delta_copy_file(&self, source: &Path, dest: &Path, options: &SyncOptions) -> Result<u64> {
        use crate::algorithm::DeltaAlgorithm;
        use crate::metadata::{copy_file_with_metadata, CopyFlags};
        
        // Create parent directory if needed
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        // For now, just use regular copy with metadata
        // TODO: Implement actual delta transfer
        let copy_flags = CopyFlags::from_string(&options.copy_flags);
        copy_file_with_metadata(source, dest, &copy_flags)
    }
    
    /// Merge two SyncStats
    fn merge_stats(&self, mut base: SyncStats, other: SyncStats) -> SyncStats {
        base.add_bytes_transferred(other.bytes_transferred());
        for _ in 0..other.files_copied() {
            base.increment_files_copied();
        }
        for _ in 0..other.errors() {
            base.increment_errors();
        }
        for _ in 0..other.files_deleted() {
            base.increment_files_deleted();
        }
        base
    }
    
    /// Process delete operations
    fn process_deletes(
        &self,
        deletes: &[FileOperation],
        _options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
    ) -> Result<SyncStats> {
        let mut stats = SyncStats::default();
        
        for op in deletes {
            match op {
                FileOperation::Delete { path } => {
                    // Use symlink_metadata to check type without following symlinks
                    match std::fs::symlink_metadata(path) {
                        Ok(metadata) => {
                            if metadata.is_dir() {
                                match std::fs::remove_dir_all(path) {
                                    Ok(_) => {
                                        stats.increment_files_deleted();
                                        if let Some(pb) = progress_bar {
                                            pb.inc(1);
                                        }
                                        self.progress.add_file();
                                    }
                                    Err(_e) => {
                                        stats.increment_errors();
                                    }
                                }
                            } else {
                                match std::fs::remove_file(path) {
                                    Ok(_) => {
                                        stats.increment_files_deleted();
                                        if let Some(pb) = progress_bar {
                                            pb.inc(1);
                                        }
                                        self.progress.add_file();
                                    }
                                    Err(_e) => {
                                        stats.increment_errors();
                                    }
                                }
                            }
                        }
                        Err(_e) => {
                            stats.increment_errors();
                        }
                    }
                }
                _ => {}
            }
        }
        
        Ok(stats)
    }
}

/// Categorized file operations
#[derive(Default, Clone)]
struct CategorizedOps {
    small_files: Vec<FileOperation>,
    medium_files: Vec<FileOperation>,
    large_files: Vec<FileOperation>,
    directories: Vec<FileOperation>,
    deletes: Vec<FileOperation>,
}

impl CategorizedOps {
    fn total_operations(&self) -> u64 {
        (self.small_files.len() + self.medium_files.len() + 
         self.large_files.len() + self.deletes.len()) as u64
    }
}

/// File statistics by category
struct FileStats {
    total: u64,
    small: u64,
    medium: u64,
    large: u64,
}

/// Size statistics by category
struct SizeStats {
    total: u64,
    small: u64,
    medium: u64,
    large: u64,
}

/// Pending operation statistics
struct PendingStats {
    files_create: u64,
    files_update: u64,
    files_delete: u64,
    dirs_create: u64,
    size_create: u64,
    size_update: u64,
    size_delete: u64,
}

impl MixedStrategyExecutor {
    /// Calculate file and size statistics
    fn calculate_stats(&self, categorized: &CategorizedOps, source_root: &Path) -> (FileStats, SizeStats) {
        let mut file_stats = FileStats {
            total: 0,
            small: 0,
            medium: 0,
            large: 0,
        };
        
        let mut size_stats = SizeStats {
            total: 0,
            small: 0,
            medium: 0,
            large: 0,
        };
        
        // Count and sum small files
        for op in &categorized.small_files {
            if let FileOperation::Create { path } | FileOperation::Update { path, .. } = op {
                if let Ok(metadata) = std::fs::metadata(path) {
                    if !metadata.is_dir() {
                        file_stats.small += 1;
                        file_stats.total += 1;
                        let size = metadata.len();
                        size_stats.small += size;
                        size_stats.total += size;
                    }
                }
            }
        }
        
        // Count and sum medium files
        for op in &categorized.medium_files {
            if let FileOperation::Create { path } | FileOperation::Update { path, .. } = op {
                if let Ok(metadata) = std::fs::metadata(path) {
                    if !metadata.is_dir() {
                        file_stats.medium += 1;
                        file_stats.total += 1;
                        let size = metadata.len();
                        size_stats.medium += size;
                        size_stats.total += size;
                    }
                }
            }
        }
        
        // Count and sum large files
        for op in &categorized.large_files {
            if let FileOperation::Create { path } | FileOperation::Update { path, .. } = op {
                if let Ok(metadata) = std::fs::metadata(path) {
                    if !metadata.is_dir() {
                        file_stats.large += 1;
                        file_stats.total += 1;
                        let size = metadata.len();
                        size_stats.large += size;
                        size_stats.total += size;
                    }
                }
            }
        }
        
        (file_stats, size_stats)
    }
    
    /// Calculate pending operation statistics
    fn calculate_pending_stats(&self, categorized: &CategorizedOps, source_root: &Path) -> PendingStats {
        let mut stats = PendingStats {
            files_create: 0,
            files_update: 0,
            files_delete: 0,
            dirs_create: 0,
            size_create: 0,
            size_update: 0,
            size_delete: 0,
        };
        
        // Process all file operations
        let all_file_ops = categorized.small_files.iter()
            .chain(categorized.medium_files.iter())
            .chain(categorized.large_files.iter());
            
        for op in all_file_ops {
            match op {
                FileOperation::Create { path } => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        if !metadata.is_dir() {
                            stats.files_create += 1;
                            stats.size_create += metadata.len();
                        }
                    }
                }
                FileOperation::Update { path, .. } => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        if !metadata.is_dir() {
                            stats.files_update += 1;
                            stats.size_update += metadata.len();
                        }
                    }
                }
                _ => {}
            }
        }
        
        // Count directory creates
        for op in &categorized.directories {
            if let FileOperation::CreateDirectory { .. } = op {
                stats.dirs_create += 1;
            }
        }
        
        // Count deletes
        stats.files_delete = categorized.deletes.len() as u64;
        // We don't track delete sizes in the current implementation
        
        stats
    }
}