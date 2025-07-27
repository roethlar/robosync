//! Concurrent mixed strategy for intelligent workload distribution
//! 
//! This strategy processes different file types concurrently rather than sequentially,
//! maximizing hardware utilization and reducing overall sync time.

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::task::JoinSet;
use rayon::prelude::*;

use crate::file_list::FileOperation;
use crate::options::SyncOptions;
use crate::sync_stats::SyncStats;
use crate::simple_progress::SimpleProgress;

// File size thresholds for strategy selection
const SMALL_FILE_THRESHOLD: u64 = 256 * 1024;    // 256KB
const MEDIUM_FILE_THRESHOLD: u64 = 10 * 1024 * 1024; // 10MB

// Concurrency settings
const SMALL_FILE_BATCH_SIZE: usize = 64;
const MEDIUM_FILE_THREADS: usize = 8;
const LARGE_FILE_THREADS: usize = 4;

#[derive(Debug, Default)]
struct CategorizedOps {
    small_files: Vec<FileOperation>,
    medium_files: Vec<FileOperation>,
    large_files: Vec<FileOperation>,
    directories: Vec<FileOperation>,
}

pub struct ConcurrentMixedStrategy {
    progress: Arc<SimpleProgress>,
    runtime: tokio::runtime::Runtime,
}

impl ConcurrentMixedStrategy {
    pub fn new(total_files: u64, total_bytes: u64) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(16) // Async I/O threads
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");
            
        Self { 
            progress: SimpleProgress::new(total_files, total_bytes),
            runtime,
        }
    }
    
    /// Execute concurrent mixed strategy synchronization
    pub fn execute(
        &self,
        operations: Vec<FileOperation>,
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        // Categorize files by size and type
        let categorized = self.categorize_operations(operations);
        
        println!("\nConcurrent mixed strategy breakdown:");
        println!("  Small files (<256KB): {} - parallel batch processing", categorized.small_files.len());
        println!("  Medium files (256KB-10MB): {} - concurrent platform APIs", categorized.medium_files.len());
        println!("  Large files (>10MB): {} - priority async processing", categorized.large_files.len());
        println!("  Directories: {}", categorized.directories.len());
        
        // Create directories first (fast operation)
        self.create_directories(&categorized.directories, dest_root)?;
        
        // Execute all strategies concurrently
        self.runtime.block_on(async {
            self.execute_concurrent_strategies(categorized, source_root, dest_root, options).await
        })
    }
    
    /// Execute all file processing strategies concurrently
    async fn execute_concurrent_strategies(
        &self,
        categorized: CategorizedOps,
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        let mut join_set = JoinSet::new();
        
        // Spawn large file worker (highest priority - start immediately)
        if !categorized.large_files.is_empty() {
            let large_files = categorized.large_files;
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            
            join_set.spawn(async move {
                println!("🚀 Starting {} large files immediately...", large_files.len());
                Self::process_large_files_async(large_files, &source_root, &dest_root, &options).await
            });
        }
        
        // Spawn medium file worker (concurrent with others)
        if !categorized.medium_files.is_empty() {
            let medium_files = categorized.medium_files;
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            
            join_set.spawn(async move {
                println!("⚡ Processing {} medium files concurrently...", medium_files.len());
                Self::process_medium_files_async(medium_files, &source_root, &dest_root, &options).await
            });
        }
        
        // Spawn small file batch worker (high throughput parallel processing)
        if !categorized.small_files.is_empty() {
            let small_files = categorized.small_files;
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            
            join_set.spawn(async move {
                println!("🔥 Batch processing {} small files in parallel...", small_files.len());
                Self::process_small_files_async(small_files, &source_root, &dest_root, &options).await
            });
        }
        
        // Collect results from all workers
        let total_stats = SyncStats::new();
        let mut completed_workers = 0;
        
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(stats)) => {
                    // Add stats from this worker to total
                    for _ in 0..stats.files_copied() {
                        total_stats.increment_files_copied();
                    }
                    total_stats.add_bytes_transferred(stats.bytes_transferred());
                    completed_workers += 1;
                    
                    println!("✅ Worker {} completed", completed_workers);
                }
                Ok(Err(e)) => {
                    eprintln!("❌ Worker failed: {}", e);
                    return Err(e);
                }
                Err(e) => {
                    eprintln!("❌ Join error: {}", e);
                    return Err(anyhow::anyhow!("Task join failed: {}", e));
                }
            }
        }
        
        println!("🎉 All {} workers completed successfully!", completed_workers);
        Ok(total_stats)
    }
    
    /// Process large files asynchronously with priority
    async fn process_large_files_async(
        files: Vec<FileOperation>,
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        let stats = SyncStats::new();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(LARGE_FILE_THREADS));
        let mut tasks = JoinSet::new();
        
        for op in files {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            
            let verbose = options.verbose > 0;
            
            tasks.spawn(async move {
                let _permit = permit; // Hold permit until task completes
                Self::process_single_large_file(op, &source_root, &dest_root, verbose).await
            });
        }
        
        // Collect results
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok((files_copied, bytes_copied))) => {
                    for _ in 0..files_copied {
                        stats.increment_files_copied();
                    }
                    stats.add_bytes_transferred(bytes_copied);
                }
                Ok(Err(e)) => {
                    eprintln!("Large file processing error: {}", e);
                    stats.increment_errors();
                }
                Err(e) => {
                    eprintln!("Large file task join error: {}", e);
                    stats.increment_errors();
                }
            }
        }
        
        Ok(stats)
    }
    
    /// Process medium files asynchronously
    async fn process_medium_files_async(
        files: Vec<FileOperation>,
        source_root: &Path,
        dest_root: &Path,
        _options: &SyncOptions,
    ) -> Result<SyncStats> {
        let stats = SyncStats::new();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(MEDIUM_FILE_THREADS));
        let mut tasks = JoinSet::new();
        
        for op in files {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            
            tasks.spawn(async move {
                let _permit = permit;
                Self::process_single_medium_file(op, &source_root, &dest_root).await
            });
        }
        
        // Collect results
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok((files_copied, bytes_copied))) => {
                    for _ in 0..files_copied {
                        stats.increment_files_copied();
                    }
                    stats.add_bytes_transferred(bytes_copied);
                }
                Ok(Err(e)) => {
                    eprintln!("Medium file processing error: {}", e);
                    stats.increment_errors();
                }
                Err(e) => {
                    eprintln!("Medium file task join error: {}", e);
                    stats.increment_errors();
                }
            }
        }
        
        Ok(stats)
    }
    
    /// Process small files in parallel batches (CPU-bound, use rayon)
    async fn process_small_files_async(
        files: Vec<FileOperation>,
        source_root: &Path,
        dest_root: &Path,
        _options: &SyncOptions,
    ) -> Result<SyncStats> {
        // Use tokio::task::spawn_blocking for CPU-bound work
        let source_root = source_root.to_path_buf();
        let dest_root = dest_root.to_path_buf();
        
        tokio::task::spawn_blocking(move || {
            Self::process_small_files_parallel(files, &source_root, &dest_root)
        }).await.unwrap()
    }
    
    /// CPU-intensive parallel processing of small files using rayon
    fn process_small_files_parallel(
        files: Vec<FileOperation>,
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<SyncStats> {
        let stats = SyncStats::new();
        
        let results: Vec<_> = files
            .par_chunks(SMALL_FILE_BATCH_SIZE)
            .map(|chunk| {
                let mut files_copied = 0u64;
                let mut bytes_copied = 0u64;
                let mut errors = 0u64;
                
                for op in chunk {
                    match Self::process_single_small_file(op, source_root, dest_root) {
                        Ok((f, b)) => {
                            files_copied += f;
                            bytes_copied += b;
                        }
                        Err(_) => {
                            errors += 1;
                        }
                    }
                }
                
                (files_copied, bytes_copied, errors)
            })
            .collect();
        
        for (files_copied, bytes_copied, errors) in results {
            for _ in 0..files_copied {
                stats.increment_files_copied();
            }
            stats.add_bytes_transferred(bytes_copied);
            for _ in 0..errors {
                stats.increment_errors();
            }
        }
        
        Ok(stats)
    }
    
    /// Process a single large file (use async I/O)
    async fn process_single_large_file(
        op: FileOperation,
        source_root: &Path,
        dest_root: &Path,
        verbose: bool,
    ) -> Result<(u64, u64)> {
        match op {
            FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                let relative = path.strip_prefix(source_root).unwrap_or(&path);
                let dest = dest_root.join(relative);
                
                // Create parent directory if needed
                if let Some(parent) = dest.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                
                // For large files, use async I/O
                let bytes_copied = tokio::fs::copy(&path, &dest).await?;
                
                if verbose {
                    println!("  ✓ Large file: {} ({} MB)", 
                        relative.display(), 
                        bytes_copied / (1024 * 1024)
                    );
                }
                
                Ok((1, bytes_copied))
            }
            _ => Ok((0, 0))
        }
    }
    
    /// Process a single medium file with async I/O
    async fn process_single_medium_file(
        op: FileOperation,
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<(u64, u64)> {
        match op {
            FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                let relative = path.strip_prefix(source_root).unwrap_or(&path);
                let dest = dest_root.join(relative);
                
                // Create parent directory if needed
                if let Some(parent) = dest.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                
                // Use async I/O for medium files
                let bytes_copied = tokio::fs::copy(&path, &dest).await?;
                Ok((1, bytes_copied))
            }
            _ => Ok((0, 0))
        }
    }
    
    /// Process a single small file (synchronous, used in parallel batches)
    fn process_single_small_file(
        op: &FileOperation,
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<(u64, u64)> {
        match op {
            FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                let relative = path.strip_prefix(source_root).unwrap_or(path);
                let dest = dest_root.join(relative);
                
                // Create parent directory if needed
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                
                // Use synchronous I/O for small files (more efficient in batches)
                let bytes_copied = std::fs::copy(path, &dest)?;
                Ok((1, bytes_copied))
            }
            _ => Ok((0, 0))
        }
    }
    
    /// Categorize operations by file size and type
    fn categorize_operations(&self, operations: Vec<FileOperation>) -> CategorizedOps {
        let mut categorized = CategorizedOps::default();
        
        for op in operations {
            match &op {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
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
                FileOperation::Delete { .. } => {
                    // Handle deletes separately if needed
                }
                FileOperation::CreateDirectory { .. } => {
                    categorized.directories.push(op);
                }
                FileOperation::CreateSymlink { .. } | FileOperation::UpdateSymlink { .. } => {
                    // Handle symlinks as small files for now
                    categorized.small_files.push(op);
                }
            }
        }
        
        categorized
    }
    
    /// Create directories
    fn create_directories(&self, directories: &[FileOperation], dest_root: &Path) -> Result<()> {
        for op in directories {
            match op {
                FileOperation::Create { path } | FileOperation::CreateDirectory { path } => {
                    let relative = if path.is_absolute() {
                        path.strip_prefix("/").unwrap_or(path)
                    } else {
                        path
                    };
                    let dest = dest_root.join(relative);
                    std::fs::create_dir_all(dest)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}