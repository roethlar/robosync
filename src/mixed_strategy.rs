//! Mixed-mode strategy that uses different copy methods for different file types
//!
//! This module implements an intelligent mixed strategy that:
//! - Uses parallel batch operations for small files
//! - Uses delta transfer for large modified files
//! - Uses platform APIs for medium files
//! - Can batch small files to native tools for efficiency

use anyhow::Result;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::file_list::FileOperation;
use crate::formatted_display::{self, WorkerStats};
use crate::options::SyncOptions;
use crate::platform_api::PlatformCopier;
use crate::progress::SyncProgress;
use crate::sync_stats::SyncStats;

/// Size thresholds for categorizing files - optimized for performance
const SMALL_FILE_THRESHOLD: u64 = 256 * 1024; // 256KB - increased for better batching
const MEDIUM_FILE_THRESHOLD: u64 = 10 * 1024 * 1024; // 10MB
const LARGE_FILE_THRESHOLD: u64 = 100 * 1024 * 1024; // 100MB - files above this use delta

/// Delta transfer is most effective for very large files
const DELTA_BLOCK_SIZE: usize = 64 * 1024; // 64KB blocks for large files

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
            progress: Arc::new(SyncProgress::new_silent(total_files, total_bytes)),
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
        use std::collections::HashMap;
        use std::sync::mpsc;
        use std::thread;
        use indicatif::{ProgressBar, ProgressStyle};

        // Show spinner during categorization for user feedback
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner} {msg}")
                .expect("Failed to set spinner template")
        );
        spinner.set_message("Analyzing files for optimal strategy...");
        spinner.enable_steady_tick(std::time::Duration::from_millis(100));

        // Categorize files by size and type
        let categorized = self.categorize_operations(operations);
        let operation_count = categorized.total_operations();
        
        spinner.finish_and_clear();

        if operation_count == 0 {
            println!("\n  ℹ️  No operations to perform - all files are up to date!");
            return Ok(SyncStats::default());
        }

        // Calculate file and size statistics
        let (_file_stats, _size_stats) = self.calculate_stats(&categorized, source_root);

        // Strategy already printed by parent
        // File analysis already printed by parent

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

        // Clear any previous progress output before starting main progress bar
        println!(); // Add blank line for separation

        // Create progress bar
        let pb = formatted_display::create_progress_bar(operation_count);
        pb.set_message("Starting...");

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

            worker_info.insert(
                "small",
                (
                    "Small".to_string(),
                    categorized.small_files.len() as u64,
                    worker_start,
                ),
            );

            let st = start_time;
            let handle = thread::spawn(move || {
                let stats = executor.process_small_files_batch(
                    &small_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                    st,
                );
                let _ = tx.send(("small", stats, worker_start.elapsed()));
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

            worker_info.insert(
                "medium",
                (
                    "Medium".to_string(),
                    categorized.medium_files.len() as u64,
                    worker_start,
                ),
            );

            let st = start_time;
            let handle = thread::spawn(move || {
                let stats = executor.process_medium_files(
                    &medium_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                    st,
                );
                let _ = tx.send(("medium", stats, worker_start.elapsed()));
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

            worker_info.insert(
                "large",
                (
                    "Large".to_string(),
                    categorized.large_files.len() as u64,
                    worker_start,
                ),
            );

            let st = start_time;
            let handle = thread::spawn(move || {
                let stats = executor.process_large_files(
                    &large_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                    st,
                );
                let _ = tx.send(("large", stats, worker_start.elapsed()));
            });
            handles.push(handle);
        }

        // Process delta files (very large files)
        if !categorized.delta_files.is_empty() {
            let tx = tx.clone();
            let delta_files = categorized.delta_files.clone();
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            let executor = self.clone();
            let pb_clone = pb.clone();
            let worker_start = std::time::Instant::now();

            worker_info.insert(
                "delta",
                (
                    "Delta transfer".to_string(),
                    categorized.delta_files.len() as u64,
                    worker_start,
                ),
            );

            let st = start_time;
            let handle = thread::spawn(move || {
                let stats = executor.process_delta_files(
                    &delta_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                    st,
                );
                let _ = tx.send(("delta", stats, worker_start.elapsed()));
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

            worker_info.insert(
                "delete",
                (
                    "Delete operations".to_string(),
                    categorized.deletes.len() as u64,
                    worker_start,
                ),
            );

            let st = start_time;
            let handle = thread::spawn(move || {
                let stats = executor.process_deletes(&deletes, &options, Some(&pb_clone), st);
                let _ = tx.send(("delete", stats, worker_start.elapsed()));
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
                    if let Some((name, _count, _)) = worker_info.get(worker_type) {
                        let bytes = stats.bytes_transferred();
                        let throughput = if duration.as_secs() > 0 {
                            bytes / duration.as_secs()
                        } else {
                            bytes
                        };

                        worker_stats.push(WorkerStats {
                            name: name.clone(),
                            files: if worker_type == "delete" {
                                stats.files_deleted()
                            } else {
                                stats.files_copied()
                            },
                            bytes,
                            duration_secs: duration.as_secs_f32(),
                            throughput,
                        });
                    }
                    total_stats = self.merge_stats(total_stats, stats);
                }
                Err(e) => {
                    eprintln!("Worker {worker_type} failed: {e}");
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
            println!(
                "\n     ✅ Completed in {:.1}s: {} files copied, {} deleted, {} transferred ({}/s)",
                elapsed.as_secs_f32(),
                format_number(total_stats.files_copied()),
                format_number(total_stats.files_deleted()),
                humanize_bytes(total_stats.bytes_transferred()),
                humanize_bytes(throughput)
            );
        } else {
            println!(
                "\n     ✅ Completed in {:.1}s: {} files copied, {} transferred ({}/s)",
                elapsed.as_secs_f32(),
                format_number(total_stats.files_copied()),
                humanize_bytes(total_stats.bytes_transferred()),
                humanize_bytes(throughput)
            );
        }

        // Get metadata warning count
        let warning_count = crate::metadata::get_and_reset_metadata_warning_count();

        // Print error/warning summary if there were any
        if total_stats.errors() > 0 || warning_count > 0 {
            eprintln!();
            if total_stats.errors() > 0 {
                eprintln!(
                    "     ⚠️  {} errors occurred during synchronization",
                    total_stats.errors()
                );
            }
            if warning_count > 0 {
                eprintln!(
                    "     ⚠️  {warning_count} metadata warnings (permissions/ownership/timestamps)"
                );
                eprintln!("        These are non-fatal - files were copied successfully.");
                if warning_count > 10 {
                    eprintln!("        Consider using --copy DAT to skip metadata preservation.");
                }
            }
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
                        } else if size <= LARGE_FILE_THRESHOLD {
                            categorized.large_files.push(op);
                        } else {
                            // Very large files (>100MB) benefit from delta transfer
                            categorized.delta_files.push(op);
                        }
                    }
                }
                FileOperation::CreateDirectory { .. } => {
                    categorized.directories.push(op);
                }
                FileOperation::Delete { .. } => {
                    categorized.deletes.push(op);
                }
                FileOperation::CreateSymlink { .. } | FileOperation::UpdateSymlink { .. } => {
                    // Symlinks are small operations, add to small files
                    categorized.small_files.push(op);
                }
            }
        }

        categorized
    }

    /// Create directories
    fn create_directories(
        &self,
        dirs: &[FileOperation],
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<()> {
        for op in dirs {
            if let FileOperation::CreateDirectory { path } = op {
                let relative = path.strip_prefix(source_root).unwrap_or(path);
                let dest = dest_root.join(relative);
                std::fs::create_dir_all(&dest)?;
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
        start_time: std::time::Instant,
    ) -> Result<SyncStats> {
        use crate::metadata::{CopyFlags, copy_file_with_metadata};

        let mut stats = SyncStats::default();
        let copy_flags = CopyFlags::from_string(&options.copy_flags);

        // Configure thread pool for optimal performance
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(SMALL_FILE_THREADS)
            .build()
            .unwrap_or_else(|_| {
                // Fall back to global thread pool if custom build fails
                rayon::ThreadPoolBuilder::new()
                    .build()
                    .expect("Failed to create thread pool")
            });

        // Process in parallel using rayon with optimized settings
        let chunk_stats: Vec<_> = pool.install(|| {
            files
                .par_chunks(SMALL_FILE_BATCH_SIZE)
                .map(|chunk| {
                let chunk_stats = SyncStats::default();

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
                                    
                                    if let Some(pb) = progress_bar {
                                        pb.inc(1);
                                        let elapsed = start_time.elapsed().as_secs_f64();
                                        if elapsed > 0.0 {
                                            let total_bytes = self.progress.get_bytes_transferred();
                                            let throughput = (total_bytes as f64 / elapsed) as u64;
                                            pb.set_message(format!("{}/s", humanize_bytes(throughput)));
                                        }
                                    }
                                }
                                Err(_e) => {
                                    // Error will be logged by copy_file_with_metadata
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
                                Err(e) => {
                                    if options.verbose > 0 {
                                        eprintln!("Error creating symlink {}: {}", path.display(), e);
                                    }
                                    chunk_stats.increment_errors();
                                }
                            }

                            #[cfg(windows)]
                            {
                                let _ = target; // Unused on Windows
                                // Windows symlink creation not implemented
                                if options.verbose > 0 {
                                    eprintln!("Error: Symlink creation not implemented on Windows for {}", path.display());
                                }
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
        _options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
        start_time: std::time::Instant,
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
        
        if let Some(pb) = progress_bar {
            pb.inc(stats.files_copied());
            let elapsed = start_time.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                let total_bytes = self.progress.get_bytes_transferred();
                let throughput = (total_bytes as f64 / elapsed) as u64;
                pb.set_message(format!("{}/s", humanize_bytes(throughput)));
            }
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
        start_time: std::time::Instant,
    ) -> Result<SyncStats> {
        let stats = SyncStats::default();

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

                            self.progress.add_file();
                            self.progress.add_bytes(bytes);
                            
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                                let elapsed = start_time.elapsed().as_secs_f64();
                                if elapsed > 0.0 {
                                    let total_bytes = self.progress.get_bytes_transferred();
                                    let throughput = (total_bytes as f64 / elapsed) as u64;
                                    pb.set_message(format!("{}/s", humanize_bytes(throughput)));
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Delta transfer failed for {path:?}: {e}");
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
                            
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                                let elapsed = start_time.elapsed().as_secs_f64();
                                if elapsed > 0.0 {
                                    let total_bytes = self.progress.get_bytes_transferred();
                                    let throughput = (total_bytes as f64 / elapsed) as u64;
                                    pb.set_message(format!("{}/s", humanize_bytes(throughput)));
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Copy failed for {path:?}: {e}");
                            stats.increment_errors();
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(stats)
    }

    /// Process very large files using delta transfer
    fn process_delta_files(
        &self,
        files: &[FileOperation],
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
        start_time: std::time::Instant,
    ) -> Result<SyncStats> {
        let stats = SyncStats::default();

        for op in files {
            match op {
                FileOperation::Create { path } => {
                    // New files can't use delta, use platform copy
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest = dest_root.join(relative);
                    
                    let copier = PlatformCopier::new();
                    match copier.copy_file(path, &dest) {
                        Ok(bytes) => {
                            stats.add_bytes_transferred(bytes);
                            stats.increment_files_copied();
                            
                            self.progress.add_file();
                            self.progress.add_bytes(bytes);
                            
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                                let elapsed = start_time.elapsed().as_secs_f64();
                                if elapsed > 0.0 {
                                    let total_bytes = self.progress.get_bytes_transferred();
                                    let throughput = (total_bytes as f64 / elapsed) as u64;
                                    pb.set_message(format!("{}/s", humanize_bytes(throughput)));
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Copy failed for {path:?}: {e}");
                            stats.increment_errors();
                        }
                    }
                }
                FileOperation::Update { path, .. } => {
                    // Use delta transfer for updates
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest = dest_root.join(relative);
                    
                    match self.delta_copy_file(path, &dest, options) {
                        Ok(bytes) => {
                            stats.add_bytes_transferred(bytes);
                            stats.increment_files_copied();
                            
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                                pb.set_message(format!(
                                    "{} transferred (delta)",
                                    humanize_bytes(stats.bytes_transferred())
                                ));
                            }
                            self.progress.add_file();
                            self.progress.add_bytes(bytes);
                        }
                        Err(e) => {
                            eprintln!("Delta transfer failed for {path:?}: {e}");
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
        use crate::algorithm::{DeltaAlgorithm, Match};
        use crate::metadata::{CopyFlags, copy_file_with_metadata};
        use std::fs::{File, OpenOptions};
        use std::io::{Read, Write};

        // Create parent directory if needed
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // If destination doesn't exist, do a regular copy
        if !dest.exists() {
            let copy_flags = CopyFlags::from_string(&options.copy_flags);
            return copy_file_with_metadata(source, dest, &copy_flags);
        }

        // For very large files, we should avoid reading everything into memory
        // but for now, let's implement a working version
        
        // Read destination file for checksums
        let mut dest_data = Vec::new();
        File::open(dest)?.read_to_end(&mut dest_data)?;
        
        // Create delta algorithm with larger block size for big files
        let delta = DeltaAlgorithm::new(DELTA_BLOCK_SIZE);
        
        // Generate checksums for destination
        let checksums = delta.generate_checksums(&dest_data)?;
        
        // Read source file
        let mut source_data = Vec::new();
        File::open(source)?.read_to_end(&mut source_data)?;
        
        // Find matching blocks
        let matches = delta.find_matches(&source_data, &checksums)?;
        
        // Calculate how much literal data we're transferring
        let mut delta_bytes = 0u64;
        let mut last_end = 0;
        
        for m in &matches {
            match m {
                Match::Block { source_offset, length, .. } => {
                    // Count literal data before this block
                    if *source_offset as usize > last_end {
                        delta_bytes += (*source_offset as usize - last_end) as u64;
                    }
                    last_end = *source_offset as usize + length;
                }
                Match::Literal { data, .. } => {
                    delta_bytes += data.len() as u64;
                }
            }
        }
        
        // Count any trailing literal data
        if last_end < source_data.len() {
            delta_bytes += (source_data.len() - last_end) as u64;
        }
        
        // If delta transfer would send >70% of the file, just copy it
        let source_size = source_data.len() as u64;
        if delta_bytes > (source_size * 7 / 10) {
            let copy_flags = CopyFlags::from_string(&options.copy_flags);
            return copy_file_with_metadata(source, dest, &copy_flags);
        }
        
        // Create temp file for reconstruction
        let temp_path = dest.with_extension("robosync_tmp");
        {
            let mut temp_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&temp_path)?;
                
            // Reconstruct file using matches
            let mut source_pos = 0;
            for m in &matches {
                match m {
                    Match::Block { source_offset, target_offset, length } => {
                        // Write any literal data before this block
                        if *source_offset as usize > source_pos {
                            temp_file.write_all(&source_data[source_pos..*source_offset as usize])?;
                        }
                        // Write the matching block from destination
                        temp_file.write_all(&dest_data[*target_offset as usize..(*target_offset as usize + length)])?;
                        source_pos = *source_offset as usize + length;
                    }
                    Match::Literal { data, .. } => {
                        temp_file.write_all(data)?;
                    }
                }
            }
            // Write any remaining data
            if source_pos < source_data.len() {
                temp_file.write_all(&source_data[source_pos..])?;
            }
        }
        
        // Move temp file to destination
        std::fs::rename(&temp_path, dest)?;
        
        // Copy metadata - the file content is already updated, just need metadata
        let copy_flags = CopyFlags::from_string(&options.copy_flags);
        if copy_flags.timestamps {
            if let Ok(metadata) = std::fs::metadata(source) {
                let atime = filetime::FileTime::from_last_access_time(&metadata);
                let mtime = filetime::FileTime::from_last_modification_time(&metadata);
                let _ = filetime::set_file_times(dest, atime, mtime);
            }
        }
        if copy_flags.attributes || copy_flags.security {
            if let Ok(metadata) = std::fs::metadata(source) {
                let _ = crate::metadata::copy_permissions(source, dest, &metadata);
            }
        }
        
        // Return the amount of data actually transferred (delta size, not full file)
        Ok(delta_bytes)
    }

    /// Merge two SyncStats
    fn merge_stats(&self, base: SyncStats, other: SyncStats) -> SyncStats {
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
        options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
        _start_time: std::time::Instant,
    ) -> Result<SyncStats> {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};

        // Thread-safe counters for parallel processing
        let files_deleted = Arc::new(AtomicU64::new(0));
        let errors = Arc::new(AtomicU64::new(0));

        // Process deletes in parallel using rayon
        deletes.par_iter().for_each(|op| {
            if let FileOperation::Delete { path } = op {
                // Use symlink_metadata to check type without following symlinks
                match std::fs::symlink_metadata(path) {
                    Ok(metadata) => {
                        if metadata.is_dir() {
                            match std::fs::remove_dir_all(path) {
                                Ok(_) => {
                                    files_deleted.fetch_add(1, Ordering::Relaxed);
                                    if let Some(pb) = progress_bar {
                                        pb.inc(1);
                                    }
                                    self.progress.add_file();
                                }
                                Err(e) => {
                                    if options.verbose > 0 {
                                        eprintln!(
                                            "Error deleting directory {}: {}",
                                            path.display(),
                                            e
                                        );
                                    }
                                    errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        } else {
                            match std::fs::remove_file(path) {
                                Ok(_) => {
                                    files_deleted.fetch_add(1, Ordering::Relaxed);
                                    if let Some(pb) = progress_bar {
                                        pb.inc(1);
                                    }
                                    self.progress.add_file();
                                }
                                Err(e) => {
                                    if options.verbose > 0 {
                                        eprintln!(
                                            "Error deleting file {}: {}",
                                            path.display(),
                                            e
                                        );
                                    }
                                    errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if options.verbose > 0 {
                            eprintln!(
                                "Error accessing file for deletion {}: {}",
                                path.display(),
                                e
                            );
                        }
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });

        // Create final stats from atomic counters
        let stats = SyncStats::default();

        // Add the counts using the proper methods
        let deleted_count = files_deleted.load(Ordering::Relaxed);
        let error_count = errors.load(Ordering::Relaxed);

        // Use loop to add multiple counts since increment methods only add 1
        for _ in 0..deleted_count {
            stats.increment_files_deleted();
        }
        for _ in 0..error_count {
            stats.increment_errors();
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
    delta_files: Vec<FileOperation>, // Very large files that benefit from delta
    directories: Vec<FileOperation>,
    deletes: Vec<FileOperation>,
}

impl CategorizedOps {
    fn total_operations(&self) -> u64 {
        (self.small_files.len()
            + self.medium_files.len()
            + self.large_files.len()
            + self.delta_files.len()
            + self.deletes.len()) as u64
    }
}

/// File statistics by category
struct FileStats {
    total: u64,
    small: u64,
    medium: u64,
    large: u64,
    delta: u64,
}

/// Size statistics by category
struct SizeStats {
    total: u64,
    small: u64,
    medium: u64,
    large: u64,
    delta: u64,
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
    fn calculate_stats(
        &self,
        categorized: &CategorizedOps,
        _source_root: &Path,
    ) -> (FileStats, SizeStats) {
        let mut file_stats = FileStats {
            total: 0,
            small: 0,
            medium: 0,
            large: 0,
            delta: 0,
        };

        let mut size_stats = SizeStats {
            total: 0,
            small: 0,
            medium: 0,
            large: 0,
            delta: 0,
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

        // Count and sum delta files (very large files)
        for op in &categorized.delta_files {
            if let FileOperation::Create { path } | FileOperation::Update { path, .. } = op {
                if let Ok(metadata) = std::fs::metadata(path) {
                    if !metadata.is_dir() {
                        file_stats.delta += 1;
                        file_stats.total += 1;
                        let size = metadata.len();
                        size_stats.delta += size;
                        size_stats.total += size;
                    }
                }
            }
        }

        (file_stats, size_stats)
    }

    /// Calculate pending operation statistics
    fn calculate_pending_stats(
        &self,
        categorized: &CategorizedOps,
        _source_root: &Path,
    ) -> PendingStats {
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
        let all_file_ops = categorized
            .small_files
            .iter()
            .chain(categorized.medium_files.iter())
            .chain(categorized.large_files.iter())
            .chain(categorized.delta_files.iter());

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
