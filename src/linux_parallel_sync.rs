//! Linux-optimized parallel sync for small files

use anyhow::Result;
use std::path::PathBuf;
use rayon::prelude::*;

use crate::file_list::FileOperation;
use crate::options::SyncOptions;
use crate::sync_stats::SyncStats;

#[cfg(target_os = "linux")]
use crate::linux_fast_copy::batch_copy_files;

/// Linux-optimized synchronizer for thousands of small files
pub struct LinuxParallelSyncer {
    worker_threads: usize,
}

impl LinuxParallelSyncer {
    pub fn new(threads: usize) -> Self {
        Self {
            worker_threads: threads,
        }
    }
    
    /// Synchronize using Linux-specific optimizations
    #[cfg(target_os = "linux")]
    pub fn synchronize_optimized(
        &self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        use crate::file_list::generate_file_list_parallel;
        use crate::file_list::compare_file_lists_with_roots;
        
        println!("Linux-optimized sync mode");
        println!("Using {} threads", self.worker_threads);
        
        // Use parallel directory scanning
        println!("Scanning source directory (parallel)...");
        let source_files = generate_file_list_parallel(&source, &options)?;
        println!("Found {} source files", source_files.len());
        
        // Scan destination if it exists
        let dest_files = if destination.exists() {
            println!("Scanning destination directory (parallel)...");
            let files = generate_file_list_parallel(&destination, &options)?;
            println!("Found {} destination files", files.len());
            files
        } else {
            println!("Destination does not exist, will create");
            Vec::new()
        };
        
        // Compare file lists
        println!("Analyzing changes...");
        let operations = compare_file_lists_with_roots(
            &source_files,
            &dest_files,
            &source,
            &destination,
            &options,
        );
        
        if operations.is_empty() {
            println!("No changes needed.");
            return Ok(SyncStats::default());
        }
        
        println!("Processing {} operations", operations.len());
        
        // Convert operations to source/dest pairs
        let copy_operations: Vec<(PathBuf, PathBuf)> = operations
            .into_par_iter()
            .filter_map(|op| {
                match op {
                    FileOperation::Create { path } | 
                    FileOperation::Update { path, .. } => {
                        let dest_path = destination.join(
                            path.strip_prefix(&source).ok()?
                        );
                        Some((path, dest_path))
                    }
                    _ => None,
                }
            })
            .collect();
        
        // Use batched copy for optimal performance
        println!("Starting optimized batch copy of {} files...", copy_operations.len());
        let stats = batch_copy_files(copy_operations)?;
        
        println!("\nCompleted in {:?}", stats.elapsed);
        println!("Files copied: {}/{}", stats.files_copied, stats.total_files);
        println!("Speed: {:.0} files/second", stats.files_per_second());
        println!("Throughput: {:.2} MB/s", stats.throughput_mb_per_sec());
        
        // Convert to SyncStats
        let sync_stats = SyncStats::new();
        sync_stats.add_bytes_transferred(stats.bytes_copied);
        
        Ok(sync_stats)
    }
    
    /// Fall back to standard sync on non-Linux platforms
    #[cfg(not(target_os = "linux"))]
    pub fn synchronize_optimized(
        &self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        use crate::parallel_sync::{ParallelSyncer, ParallelSyncConfig};
        
        println!("Using standard parallel sync (non-Linux platform)");
        
        let config = ParallelSyncConfig {
            worker_threads: self.worker_threads,
            io_threads: self.worker_threads,
            block_size: 1024,
            max_parallel_files: self.worker_threads * 2,
        };
        
        let syncer = ParallelSyncer::new(config);
        syncer.synchronize_with_options(source, destination, options)
    }
}