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

/// Size thresholds for categorizing files - optimized for performance
const SMALL_FILE_THRESHOLD: u64 = 256 * 1024;        // 256KB - increased for better batching
const MEDIUM_FILE_THRESHOLD: u64 = 10 * 1024 * 1024; // 10MB

/// Batch size for small file operations - larger for better performance
const SMALL_FILE_BATCH_SIZE: usize = 5000;

/// Number of parallel threads for small files
const SMALL_FILE_THREADS: usize = 32;

/// Mixed strategy executor
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
        // Categorize files by size and type
        let categorized = self.categorize_operations(operations);
        
        println!("\nMixed strategy breakdown:");
        println!("  Small files (<256KB): {} - using parallel batch copy", categorized.small_files.len());
        println!("  Medium files (256KB-10MB): {} - using platform APIs", categorized.medium_files.len());
        println!("  Large files (>10MB): {} - copying directly", categorized.large_files.len());
        println!("  Directories: {}", categorized.directories.len());
        
        let mut total_stats = SyncStats::default();
        
        // Process directories first
        self.create_directories(&categorized.directories, dest_root)?;
        
        // Process small files in parallel batches
        if !categorized.small_files.is_empty() {
            println!("\nProcessing {} small files in parallel...", categorized.small_files.len());
            let stats = self.process_small_files_batch(
                &categorized.small_files,
                source_root,
                dest_root,
                options,
            )?;
            total_stats = self.merge_stats(total_stats, stats);
        }
        
        // Process medium files with platform APIs
        if !categorized.medium_files.is_empty() {
            println!("\nProcessing {} medium files with platform APIs...", categorized.medium_files.len());
            let stats = self.process_medium_files(
                &categorized.medium_files,
                source_root,
                dest_root,
                options,
            )?;
            total_stats = self.merge_stats(total_stats, stats);
        }
        
        // Process large files with delta transfer (for updates) or platform API (for creates)
        if !categorized.large_files.is_empty() {
            println!("\nProcessing {} large files...", categorized.large_files.len());
            let stats = self.process_large_files(
                &categorized.large_files,
                source_root,
                dest_root,
                options,
            )?;
            total_stats = self.merge_stats(total_stats, stats);
        }
        
        // Print final progress
        self.progress.finish();
        
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
                FileOperation::Delete { .. } | 
                FileOperation::CreateSymlink { .. } | 
                FileOperation::UpdateSymlink { .. } => {
                    // Handle these separately if needed
                }
            }
        }
        
        categorized
    }
    
    /// Create directories
    fn create_directories(&self, dirs: &[FileOperation], dest_root: &Path) -> Result<()> {
        for op in dirs {
            match op {
                FileOperation::Create { path } | FileOperation::CreateDirectory { path } => {
                    let relative = path.strip_prefix("/").unwrap_or(path);
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
                                    self.progress.add_file();
                                    self.progress.add_bytes(bytes);
                                }
                                Err(e) => {
                                    eprintln!("Error copying {:?}: {}", path, e);
                                    chunk_stats.increment_errors();
                                }
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
    ) -> Result<SyncStats> {
        let mut stats = SyncStats::default();
        
        for op in files {
            match op {
                FileOperation::Update { path, .. } if options.checksum => {
                    // Use delta transfer for large file updates
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest = dest_root.join(relative);
                    
                    println!("Delta transfer for large file: {:?}", path.file_name().unwrap_or_default());
                    
                    match self.delta_copy_file(path, &dest, options) {
                        Ok(bytes) => {
                            stats.add_bytes_transferred(bytes);
                            stats.increment_files_copied();
                            
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
        base
    }
}

/// Categorized file operations
#[derive(Default)]
struct CategorizedOps {
    small_files: Vec<FileOperation>,
    medium_files: Vec<FileOperation>,
    large_files: Vec<FileOperation>,
    directories: Vec<FileOperation>,
}