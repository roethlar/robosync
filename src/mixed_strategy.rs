//! Mixed-mode strategy that uses different copy methods for different file types
//!
//! This module implements an intelligent mixed strategy that:
//! - Uses parallel batch operations for small files
//! - Uses delta transfer for large modified files
//! - Uses platform APIs for medium files
//! - Can batch small files to native tools for efficiency

use anyhow::Result;
use rayon::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::error::RoboSyncError;
use crate::error_logger::ErrorLogger;
use crate::file_list::FileOperation;
use crate::formatted_display::{self, WorkerStats};
use crate::logging::SyncLogger;
use crate::operation_utils::{
    prepare_destination_path, update_progress, update_progress_with_file,
};
use crate::options::SyncOptions;
use crate::platform_api::PlatformCopier;
use crate::progress::SyncProgress;
use crate::sync_stats::SyncStats;

/// Default size thresholds for categorizing files - optimized for performance
const DEFAULT_SMALL_FILE_THRESHOLD: u64 = 256 * 1024; // 256KB - increased for better batching
const DEFAULT_MEDIUM_FILE_THRESHOLD: u64 = 10 * 1024 * 1024; // 10MB
const DEFAULT_LARGE_FILE_THRESHOLD: u64 = 100 * 1024 * 1024; // 100MB - files above this use delta

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
    logger: Option<Arc<Mutex<SyncLogger>>>,
}

impl MixedStrategyExecutor {
    pub fn new(total_files: u64, total_bytes: u64) -> Self {
        Self {
            progress: Arc::new(SyncProgress::new_silent(total_files, total_bytes)),
            logger: None,
        }
    }

    pub fn new_with_no_progress() -> Self {
        Self {
            progress: Arc::new(SyncProgress::new_noop()),
            logger: None,
        }
    }

    pub fn with_logger(mut self, logger: Arc<Mutex<SyncLogger>>) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Execute mixed strategy synchronization
    pub fn execute(
        &self,
        operations: Vec<FileOperation>,
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        use indicatif::{ProgressBar, ProgressStyle};
        use std::collections::HashMap;
        use std::sync::mpsc;
        use std::thread;

        // Create logger with optional log file
        let logger = Arc::new(Mutex::new(SyncLogger::new(
            options.log_file.as_deref(),
            options.show_eta,
        )?));

        {
            let logger_guard = logger.lock().unwrap();
            logger_guard.log("Starting mixed strategy synchronization...");
            logger_guard.log(&format!("Source: {}", source_root.display()));
            logger_guard.log(&format!("Destination: {}", dest_root.display()));
        }

        // Update self with the logger for worker threads
        let executor_with_logger = self.clone().with_logger(Arc::clone(&logger));

        // Create error logger for automatic error reporting
        let error_logger = ErrorLogger::new(options.clone(), source_root, dest_root);
        let _error_handle = error_logger.get_handle();

        // Show spinner during categorization for user feedback
        let spinner = if options.show_progress {
            let s = ProgressBar::new_spinner();
            s.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner} {msg}")
                    .expect("Failed to set spinner template"),
            );
            s.set_message("Analyzing files for optimal strategy...");
            s.enable_steady_tick(std::time::Duration::from_millis(100));
            Some(s)
        } else {
            None
        };

        // Categorize files by size and type
        let categorized = self.categorize_operations(operations, options);
        let operation_count = categorized.total_operations();

        if let Some(spinner) = spinner {
            spinner.finish_and_clear();
        }

        if operation_count == 0 {
            if options.show_progress {
                println!("\n  ℹ️  No operations to perform - all files are up to date!");
            }
            return Ok(SyncStats::default());
        }

        // Calculate file and size statistics
        let (_file_stats, _size_stats) = self.calculate_stats(&categorized, source_root);

        // Strategy already printed by parent
        // File analysis already printed by parent

        // Print pending operations
        if options.show_progress {
            if options.verbose >= 1 {
                // Verbose mode: show detailed breakdown
                let detailed_stats = executor_with_logger
                    .calculate_detailed_pending_stats(&categorized, source_root);
                formatted_display::print_pending_operations_detailed(
                    &detailed_stats,
                    options.verbose,
                );
            } else {
                // Normal mode: show simple summary
                let pending_stats =
                    executor_with_logger.calculate_pending_stats(&categorized, source_root);
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
            }
        }

        // Process directories first (must be done before files)
        executor_with_logger.create_directories(
            &categorized.directories,
            source_root,
            dest_root,
        )?;

        // Clear any previous progress output before starting main progress bar
        if options.show_progress {
            println!(); // Add blank line for separation
        }

        // Create progress bar - always track progress internally
        let pb = if !options.show_progress {
            // Create a hidden progress bar that still tracks position
            let pb = indicatif::ProgressBar::new(operation_count);
            pb.set_draw_target(indicatif::ProgressDrawTarget::hidden());
            pb
        } else {
            let pb = formatted_display::create_progress_bar(operation_count);
            pb.set_message("Starting...");
            pb
        };

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
            let executor = executor_with_logger.clone();
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
            let executor = executor_with_logger.clone();
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
            let executor = executor_with_logger.clone();
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
                let result = std::panic::catch_unwind(|| {
                    executor.process_large_files(
                        &large_files,
                        &source_root,
                        &dest_root,
                        &options,
                        Some(&pb_clone),
                        st,
                    )
                });

                let stats = match result {
                    Ok(stats_result) => stats_result,
                    Err(panic) => {
                        eprintln!("[ERROR] Large worker thread panicked: {:?}", panic);
                        Err(anyhow::anyhow!("Large worker thread panicked"))
                    }
                };

                if let Err(e) = tx.send(("large", stats, worker_start.elapsed())) {
                    eprintln!("[ERROR] Failed to send large worker results: {:?}", e);
                }
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
            let executor = executor_with_logger.clone();
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
            let executor = executor_with_logger.clone();
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
                let result = std::panic::catch_unwind(|| {
                    executor.process_deletes(&deletes, &options, Some(&pb_clone), st)
                });

                let stats = match result {
                    Ok(stats_result) => stats_result,
                    Err(panic) => {
                        eprintln!("[ERROR] Delete worker thread panicked: {:?}", panic);
                        Err(anyhow::anyhow!("Delete worker thread panicked"))
                    }
                };

                if let Err(e) = tx.send(("delete", stats, worker_start.elapsed())) {
                    eprintln!("[ERROR] Failed to send delete worker results: {:?}", e);
                }
            });
            handles.push(handle);
        }

        // Start a status logger thread that shows single-line status updates
        let logger_handle = if !options.show_progress {
            Some({
                let pb_status = pb.clone();
                let total_ops = operation_count;
                let start_time = std::time::Instant::now();
                let progress_tracker = Arc::clone(&self.progress);
                thread::spawn(move || {
                    let mut last_position = 0u64;
                    let mut stall_count = 0;
                    let mut last_rate = 0.0;
                    let spinner_chars = vec!['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                    let mut spinner_idx = 0;

                    loop {
                        thread::sleep(std::time::Duration::from_millis(100));
                        let current_position = pb_status.position();
                        let elapsed = start_time.elapsed();

                        // Calculate rate
                        let elapsed_secs = elapsed.as_secs_f64();
                        let rate = if elapsed_secs > 0.0 && current_position > 0 {
                            current_position as f64 / elapsed_secs
                        } else {
                            0.0
                        };

                        // Calculate throughput
                        let bytes_transferred = progress_tracker.get_bytes_transferred();
                        let throughput = if elapsed_secs > 0.0 {
                            (bytes_transferred as f64 / elapsed_secs) as u64
                        } else {
                            0
                        };

                        // Check for stalls
                        if current_position == last_position {
                            stall_count += 1;
                        } else {
                            stall_count = 0;
                            last_rate = rate;
                        }

                        // Update spinner
                        spinner_idx = (spinner_idx + 1) % spinner_chars.len();

                        // Print status line
                        let status = if stall_count > 50 {
                            // 5 seconds
                            format!(
                                " {} Syncing: {}/{} files | {}/s | ⚠️  Stalled for {}s",
                                spinner_chars[spinner_idx],
                                current_position,
                                total_ops,
                                indicatif::HumanBytes(throughput),
                                stall_count / 10
                            )
                        } else {
                            format!(
                                " {} Syncing: {}/{} files | {}/s",
                                spinner_chars[spinner_idx], 
                                current_position, 
                                total_ops, 
                                indicatif::HumanBytes(throughput)
                            )
                        };

                        // Clear line and print status
                        print!("\r{:80}\r{}", "", status);
                        let _ = std::io::stdout().flush();

                        last_position = current_position;

                        // Check if we're done
                        if current_position >= total_ops || pb_status.is_finished() {
                            print!("\r{:80}\r", ""); // Clear the line
                            let _ = std::io::stdout().flush();
                            break;
                        }
                    }
                })
            })
        } else {
            None
        };

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
                    // Don't print to stderr - it breaks the progress bar
                    // Record structured error for log file
                    let robosync_err = RoboSyncError::operation_failed(
                        format!("{}_worker", worker_type),
                        e.to_string(),
                    );
                    total_stats.add_structured_error(robosync_err, "worker_thread");
                }
            }
        }

        // Wait for all threads to complete
        for handle in handles {
            let _ = handle.join();
        }

        // Finish progress bar (only if progress is enabled)
        if options.show_progress {
            pb.finish_and_clear();
        } else {
            // Even for hidden progress bars, we need to finish them
            pb.finish();
        }

        // Wait for logger thread to finish
        if let Some(handle) = logger_handle {
            let _ = handle.join();
        }

        // Print worker performance (if progress enabled)
        if options.show_progress {
            formatted_display::print_worker_performance(worker_stats);
        }

        // Finish the internal progress tracker to prevent any final output
        self.progress.finish();

        // Final summary
        let elapsed = start_time.elapsed();
        let throughput = if elapsed.as_secs() > 0 {
            total_stats.bytes_transferred() / elapsed.as_secs()
        } else {
            total_stats.bytes_transferred()
        };

        // Always print summary statistics
        if options.show_progress {
            // With progress bar, show fancy completion message
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
        }

        // Log final statistics to file only (not console)
        {
            let logger = logger.lock().unwrap();
            logger.log_to_file_only(&format!("Files copied: {}", total_stats.files_copied()));
            logger.log_to_file_only(&format!("Files deleted: {}", total_stats.files_deleted()));
            logger.log_to_file_only(&format!(
                "Bytes transferred: {}",
                total_stats.bytes_transferred()
            ));
            let reflinks = total_stats.reflinks_succeeded();
            let reflink_fallbacks = total_stats.reflinks_failed_fallback();
            if reflinks > 0 || reflink_fallbacks > 0 {
                logger.log_to_file_only(&format!("Reflinks succeeded: {}", reflinks));
                logger.log_to_file_only(&format!("Reflinks fallback: {}", reflink_fallbacks));
            }
            logger.log_to_file_only(&format!("Errors: {}", total_stats.errors()));

            // Log error details
            let error_details = total_stats.get_error_details();
            logger.log_to_file_only(&format!("Error details count: {}", error_details.len()));
            for error_detail in error_details {
                logger.log_to_file_only(&format!(
                    "ERROR: {} - {} - {}",
                    error_detail.path.display(),
                    error_detail.operation,
                    error_detail.message
                ));
            }
        }

        // Finalize error reporting before printing summary
        let error_report_path =
            if let Ok(report_path) = error_logger.finalize_with_stats(&total_stats) {
                report_path
            } else {
                None
            };

        // Get metadata warning count
        let warning_count = crate::metadata::get_and_reset_metadata_warning_count();

        // Print error/warning summary if there were any
        if total_stats.errors() > 0 || warning_count > 0 {
            eprintln!();
            if total_stats.errors() > 0 {
                eprintln!(
                    "⚠️  {} errors occurred during synchronization",
                    total_stats.errors()
                );
                if let Some(ref path) = error_report_path {
                    eprintln!("📄 Error details saved to: {}", path.display());
                }
            }
            if warning_count > 0 {
                eprintln!(
                    "⚠️  {warning_count} metadata warnings (permissions/ownership/timestamps)"
                );
                eprintln!("These are non-fatal - files were copied successfully.");
                if warning_count > 10 {
                    eprintln!("Consider using --copy DAT to skip metadata preservation.");
                }
            }
        }

        // Close the logger
        logger.lock().unwrap().close();

        Ok(total_stats)
    }

    /// Categorize operations by file size and type
    fn categorize_operations(
        &self,
        operations: Vec<FileOperation>,
        options: &SyncOptions,
    ) -> CategorizedOps {
        let mut categorized = CategorizedOps::default();

        // Use configured thresholds or defaults
        let small_threshold = options
            .small_file_threshold
            .unwrap_or(DEFAULT_SMALL_FILE_THRESHOLD);
        let medium_threshold = options
            .medium_file_threshold
            .unwrap_or(DEFAULT_MEDIUM_FILE_THRESHOLD);
        let large_threshold = options
            .large_file_threshold
            .unwrap_or(DEFAULT_LARGE_FILE_THRESHOLD);

        for op in operations {
            match &op {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    // Get file info from the operation
                    match std::fs::metadata(path) {
                        Ok(metadata) => {
                            let size = metadata.len();

                            if metadata.is_dir() {
                                categorized.directories.push(op);
                            } else if size <= small_threshold {
                                categorized.small_files.push(op);
                            } else if size <= medium_threshold {
                                categorized.medium_files.push(op);
                            } else if size <= large_threshold {
                                categorized.large_files.push(op);
                            } else {
                                // Very large files (>100MB) benefit from delta transfer
                                categorized.delta_files.push(op);
                            }
                        }
                        Err(_) => {
                            // If we can't read metadata, treat as small file
                            // The actual error will be captured when we try to copy it
                            categorized.small_files.push(op);
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
        use crate::metadata::{copy_file_with_metadata_and_reflink, CopyFlags};
        use crate::reflink::ReflinkOptions;

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
        let active_files = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
            std::path::PathBuf,
            std::time::Instant,
        >::new()));
        let active_files_clone = active_files.clone();

        // Spawn a monitoring thread to detect stalled files
        let monitor_handle = std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                let active = active_files_clone.lock().unwrap();
                for (path, start_time) in active.iter() {
                    let elapsed = start_time.elapsed();
                    if elapsed > std::time::Duration::from_secs(10) {
                        eprintln!(
                            "[WARNING] File has been processing for {}s: {}",
                            elapsed.as_secs(),
                            path.display()
                        );
                    }
                }
                // Also report if we have active files
                if !active.is_empty() {}
                if active.is_empty() {
                    break;
                }
            }
        });

        let chunk_stats: Vec<_> = pool.install(|| {
            files
                .par_chunks(SMALL_FILE_BATCH_SIZE)
                .map(|chunk| {
                    let chunk_stats = SyncStats::default();

                    for op in chunk {
                        // Track active file
                        let file_path = match &op {
                            FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                                Some(path.clone())
                            }
                            FileOperation::CreateSymlink { path, .. }
                            | FileOperation::UpdateSymlink { path, .. } => Some(path.clone()),
                            _ => None,
                        };

                        if let Some(path) = &file_path {
                            active_files
                                .lock()
                                .unwrap()
                                .insert(path.clone(), std::time::Instant::now());
                        }

                        match op {
                            FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                                // Use utility function for path resolution and parent directory creation
                                let dest =
                                    match prepare_destination_path(path, source_root, dest_root) {
                                        Ok(dest) => dest,
                                        Err(e) => {
                                            let robosync_err = RoboSyncError::operation_failed(
                                                "prepare_path",
                                                e.to_string(),
                                            );
                                            chunk_stats.add_structured_error(
                                                robosync_err,
                                                "prepare_destination",
                                            );
                                            continue;
                                        }
                                    };

                                // Copy the file
                                let reflink_options = ReflinkOptions {
                                    mode: options.reflink,
                                };
                                match copy_file_with_metadata_and_reflink(path, &dest, &copy_flags, &reflink_options, Some(&chunk_stats)) {
                                    Ok(bytes) => {
                                        chunk_stats.add_bytes_transferred(bytes);
                                        chunk_stats.increment_files_copied();

                                        // Use utility function for progress tracking with file info
                                        update_progress_with_file(
                                            &self.progress,
                                            progress_bar,
                                            bytes,
                                            start_time,
                                            path,
                                        );

                                        // Show on console only with -vv mode
                                        if options.verbose >= 2 {
                                            eprintln!(
                                                "  ✓ Copied: {} → {} ({} bytes)",
                                                path.display(),
                                                dest.display(),
                                                bytes
                                            );
                                        }

                                        // Log to file with -v or higher
                                        if options.verbose >= 1 {
                                            if let Some(ref logger) = self.logger {
                                                if let Ok(logger) = logger.lock() {
                                                    logger.log_file_operation(
                                                        "Copied",
                                                        &format!(
                                                            "{} → {} ({} bytes)",
                                                            path.display(),
                                                            dest.display(),
                                                            bytes
                                                        ),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let error_str = e.to_string();

                                        // Log error to file
                                        if let Some(ref logger) = self.logger {
                                            if let Ok(logger) = logger.lock() {
                                                logger.log_error(&format!(
                                                    "Failed to copy {} → {}: {}",
                                                    path.display(),
                                                    dest.display(),
                                                    error_str
                                                ));
                                            }
                                        }

                                        // Convert anyhow::Error to RoboSyncError
                                        let robosync_err = match e.downcast::<std::io::Error>() {
                                            Ok(io_err) => {
                                                RoboSyncError::io_error(io_err, Some(path.clone()))
                                            }
                                            Err(e) => RoboSyncError::sync_failed(
                                                e.to_string(),
                                                Some(path.clone()),
                                                Some(dest.clone()),
                                            ),
                                        };
                                        chunk_stats.add_structured_error(robosync_err, "copy_file");

                                        // Still increment progress bar even for failed files
                                        if let Some(pb) = progress_bar {
                                            pb.inc(1);
                                        }
                                    }
                                }
                            }
                            FileOperation::CreateSymlink { path, target }
                            | FileOperation::UpdateSymlink { path, target } => {
                                // Use utility function for path resolution and parent directory creation
                                let dest =
                                    match prepare_destination_path(path, source_root, dest_root) {
                                        Ok(dest) => dest,
                                        Err(e) => {
                                            let robosync_err = RoboSyncError::operation_failed(
                                                "prepare_path",
                                                e.to_string(),
                                            );
                                            chunk_stats.add_structured_error(
                                                robosync_err,
                                                "prepare_destination",
                                            );
                                            continue;
                                        }
                                    };

                                // Remove existing symlink if updating
                                if matches!(op, FileOperation::UpdateSymlink { .. }) {
                                    let _ = std::fs::remove_file(&dest);
                                }

                                // Create symlink
                                #[cfg(unix)]
                                match std::os::unix::fs::symlink(target, &dest) {
                                    Ok(_) => {
                                        chunk_stats.increment_files_copied();
                                        // Use utility function for progress tracking (symlinks have 0 bytes)
                                        update_progress(
                                            &self.progress,
                                            progress_bar,
                                            0,
                                            start_time,
                                        );
                                    }
                                    Err(e) => {
                                        // Log error to file
                                        if let Some(ref logger) = self.logger {
                                            if let Ok(logger) = logger.lock() {
                                                logger.log_error(&format!(
                                                    "Failed to create symlink {} → {}: {}",
                                                    dest.display(),
                                                    target.display(),
                                                    e
                                                ));
                                            }
                                        }

                                        // Convert to RoboSyncError
                                        let robosync_err =
                                            RoboSyncError::io_error(e, Some(dest.clone()));
                                        chunk_stats
                                            .add_structured_error(robosync_err, "create_symlink");
                                    }
                                }

                                #[cfg(windows)]
                                match crate::windows_symlinks::create_symlink(&dest, target) {
                                    Ok(_) => {
                                        chunk_stats.increment_files_copied();
                                        // Use utility function for progress tracking (symlinks have 0 bytes)
                                        update_progress(
                                            &self.progress,
                                            progress_bar,
                                            0,
                                            start_time,
                                        );
                                    }
                                    Err(e) => {
                                        // Log error to file
                                        if let Some(ref logger) = self.logger {
                                            if let Ok(logger) = logger.lock() {
                                                logger.log_error(&format!(
                                                    "Failed to create symlink {} → {}: {}",
                                                    dest.display(),
                                                    target.display(),
                                                    e
                                                ));
                                            }
                                        }

                                        // Convert to RoboSyncError
                                        let robosync_err = RoboSyncError::operation_failed(
                                            "create_symlink",
                                            e.to_string(),
                                        );
                                        chunk_stats
                                            .add_structured_error(robosync_err, "create_symlink");
                                    }
                                }
                            }
                            _ => {}
                        }

                        // Remove from active tracking
                        if let Some(path) = file_path {
                            active_files.lock().unwrap().remove(&path);
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

        // Clean up monitor thread
        drop(active_files);
        let _ = monitor_handle.join();

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
        _start_time: std::time::Instant,
    ) -> Result<SyncStats> {
        let copier = PlatformCopier::new();

        // Convert operations to file pairs
        let file_pairs: Vec<(PathBuf, PathBuf)> = files
            .iter()
            .filter_map(|op| match op {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    // Use utility function for path resolution
                    let dest = crate::operation_utils::resolve_destination_path(
                        path,
                        source_root,
                        dest_root,
                    );
                    Some((path.clone(), dest))
                }
                _ => None,
            })
            .collect();

        // Copy files and update progress
        let stats = copier.copy_files(&file_pairs)?;

        // Update progress for all files attempted (not just successful copies)
        let total_files = file_pairs.len() as u64;
        for _ in 0..total_files {
            if let Some(pb) = progress_bar {
                pb.inc(1);
            }
        }

        // Update internal progress tracking
        for _ in 0..stats.files_copied() {
            self.progress.add_file();
        }
        self.progress.add_bytes(stats.bytes_transferred());

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
                    let dest = crate::operation_utils::resolve_destination_path(
                        path,
                        source_root,
                        dest_root,
                    );

                    match self.delta_copy_file(path, &dest, options) {
                        Ok(bytes) => {
                            stats.add_bytes_transferred(bytes);
                            stats.increment_files_copied();

                            // Show on console only with -vv mode
                            if options.verbose >= 2 {
                                eprintln!(
                                    "  ✓ Delta transfer complete: {} → {} ({} transferred)",
                                    path.display(),
                                    dest.display(),
                                    humanize_bytes(bytes)
                                );
                            }

                            // Log to file with -v or higher
                            if options.verbose >= 1 {
                                if let Some(ref logger) = self.logger {
                                    if let Ok(logger) = logger.lock() {
                                        logger.log_file_operation(
                                            "Delta transfer complete",
                                            &format!(
                                                "{} → {} ({} bytes)",
                                                path.display(),
                                                dest.display(),
                                                bytes
                                            ),
                                        );
                                    }
                                }
                            }

                            // Use utility function for progress tracking
                            update_progress(&self.progress, progress_bar, bytes, start_time);
                        }
                        Err(e) => {
                            let error_str = e.to_string();

                            // Log error to file
                            if let Some(ref logger) = self.logger {
                                if let Ok(logger) = logger.lock() {
                                    logger.log_error(&format!(
                                        "Delta transfer failed {} → {}: {}",
                                        path.display(),
                                        dest.display(),
                                        error_str
                                    ));
                                }
                            }

                            // Convert anyhow::Error to RoboSyncError
                            let robosync_err = match e.downcast::<std::io::Error>() {
                                Ok(io_err) => RoboSyncError::io_error(io_err, Some(path.clone())),
                                Err(e) => RoboSyncError::delta_failed(e.to_string(), path.clone()),
                            };
                            stats.add_structured_error(robosync_err, "delta_transfer");
                        }
                    }
                }
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    // Use platform API for new large files or updates without checksum
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest = dest_root.join(relative);

                    // Ensure parent directory exists
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    let copier = PlatformCopier::new();
                    match copier.copy_file(path, &dest) {
                        Ok(bytes) => {
                            stats.add_bytes_transferred(bytes);
                            stats.increment_files_copied();

                            self.progress.add_file();
                            self.progress.add_bytes(bytes);

                            // Show on console only with -vv mode
                            if options.verbose >= 2 {
                                eprintln!(
                                    "  ✓ Copied: {} → {} ({} bytes)",
                                    path.display(),
                                    dest.display(),
                                    bytes
                                );
                            }

                            // Log to file with -v or higher
                            if options.verbose >= 1 {
                                if let Some(ref logger) = self.logger {
                                    if let Ok(logger) = logger.lock() {
                                        logger.log_file_operation(
                                            "Copied",
                                            &format!(
                                                "{} → {} ({} bytes)",
                                                path.display(),
                                                dest.display(),
                                                bytes
                                            ),
                                        );
                                    }
                                }
                            }

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
                            let error_str = e.to_string();

                            // Log error to file
                            if let Some(ref logger) = self.logger {
                                if let Ok(logger) = logger.lock() {
                                    logger.log_error(&format!(
                                        "Delta transfer failed {} → {}: {}",
                                        path.display(),
                                        dest.display(),
                                        error_str
                                    ));
                                }
                            }

                            // Convert anyhow::Error to RoboSyncError
                            let robosync_err = match e.downcast::<std::io::Error>() {
                                Ok(io_err) => RoboSyncError::io_error(io_err, Some(path.clone())),
                                Err(e) => RoboSyncError::delta_failed(e.to_string(), path.clone()),
                            };
                            stats.add_structured_error(robosync_err, "delta_transfer");
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

                    // Ensure parent directory exists
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

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
                            let error_str = e.to_string();

                            // Log error to file
                            if let Some(ref logger) = self.logger {
                                if let Ok(logger) = logger.lock() {
                                    logger.log_error(&format!(
                                        "Delta transfer failed {} → {}: {}",
                                        path.display(),
                                        dest.display(),
                                        error_str
                                    ));
                                }
                            }

                            // Convert anyhow::Error to RoboSyncError
                            let robosync_err = match e.downcast::<std::io::Error>() {
                                Ok(io_err) => RoboSyncError::io_error(io_err, Some(path.clone())),
                                Err(e) => RoboSyncError::delta_failed(e.to_string(), path.clone()),
                            };
                            stats.add_structured_error(robosync_err, "delta_transfer");
                        }
                    }
                }
                FileOperation::Update { path, .. } => {
                    // Use delta transfer for updates
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest = dest_root.join(relative);

                    // Log start of delta transfer to console only with -vv
                    if options.verbose >= 2 {
                        eprintln!(
                            "  ⟳ Starting delta transfer: {} → {} (file size: {})",
                            path.display(),
                            dest.display(),
                            humanize_bytes(std::fs::metadata(path).map(|m| m.len()).unwrap_or(0))
                        );
                    }

                    // Log to file with -v or higher
                    if options.verbose >= 1 {
                        if let Some(ref logger) = self.logger {
                            if let Ok(logger) = logger.lock() {
                                logger.log_file_operation(
                                    "Delta transfer starting",
                                    &format!("{} → {}", path.display(), dest.display()),
                                );
                            }
                        }
                    }

                    match self.delta_copy_file(path, &dest, options) {
                        Ok(bytes) => {
                            stats.add_bytes_transferred(bytes);
                            stats.increment_files_copied();

                            // Log completion to console only with -vv
                            if options.verbose >= 2 {
                                eprintln!(
                                    "  ✓ Delta transfer complete: {} → {} ({} transferred)",
                                    path.display(),
                                    dest.display(),
                                    humanize_bytes(bytes)
                                );
                            }

                            // Log to file with -v or higher
                            if options.verbose >= 1 {
                                if let Some(ref logger) = self.logger {
                                    if let Ok(logger) = logger.lock() {
                                        logger.log_file_operation(
                                            "Delta transfer complete",
                                            &format!(
                                                "{} → {} ({} bytes)",
                                                path.display(),
                                                dest.display(),
                                                bytes
                                            ),
                                        );
                                    }
                                }
                            }

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
                            let error_str = e.to_string();

                            // Log error to file
                            if let Some(ref logger) = self.logger {
                                if let Ok(logger) = logger.lock() {
                                    logger.log_error(&format!(
                                        "Delta transfer failed {} → {}: {}",
                                        path.display(),
                                        dest.display(),
                                        error_str
                                    ));
                                }
                            }

                            // Convert anyhow::Error to RoboSyncError
                            let robosync_err = match e.downcast::<std::io::Error>() {
                                Ok(io_err) => RoboSyncError::io_error(io_err, Some(path.clone())),
                                Err(e) => RoboSyncError::delta_failed(e.to_string(), path.clone()),
                            };
                            stats.add_structured_error(robosync_err, "delta_transfer");
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
        use crate::metadata::{copy_file_with_metadata_and_reflink, CopyFlags};
        use crate::reflink::ReflinkOptions;
        use crate::streaming_delta::StreamingDelta;
        use std::fs;

        // Create parent directory if needed
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // If destination doesn't exist, do a regular copy
        if !dest.exists() {
            let copy_flags = CopyFlags::from_string(&options.copy_flags);
            let reflink_options = ReflinkOptions {
                mode: options.reflink,
            };
            return copy_file_with_metadata_and_reflink(source, dest, &copy_flags, &reflink_options, None);
        }

        // Use streaming delta for all file sizes
        let streaming_delta = StreamingDelta::new(DELTA_BLOCK_SIZE);

        // Log checksum generation
        if options.verbose >= 2 {
            eprintln!("    - Generating checksums for destination file...");
        }

        // Generate checksums for destination file
        let checksums = streaming_delta.generate_checksums_streaming(dest)?;

        if options.verbose >= 2 {
            eprintln!("    - Generated {} block checksums", checksums.len());
        }

        // Create temp file for reconstruction
        let temp_path = dest.with_extension("robosync_tmp");

        // Log delta application
        if options.verbose >= 2 {
            eprintln!("    - Applying delta transfer...");
        }

        // Apply delta transfer using streaming
        let transferred_bytes =
            streaming_delta.apply_delta_streaming(source, dest, &temp_path, &checksums)?;

        if options.verbose >= 2 {
            eprintln!(
                "    - Delta transfer complete, {} bytes transferred",
                transferred_bytes
            );
        }

        // Move temp file to destination
        fs::rename(&temp_path, dest)?;

        // Copy metadata - the file content is already updated, just need metadata
        let copy_flags = CopyFlags::from_string(&options.copy_flags);
        let _ = crate::metadata_utils::apply_metadata_after_delta(source, dest, &copy_flags);

        // Return the amount of data actually transferred
        Ok(transferred_bytes)
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

        // Merge error details
        for error_detail in other.get_error_details() {
            base.add_error(
                error_detail.path,
                &error_detail.operation,
                &error_detail.message,
            );
        }

        // Merge structured errors
        for structured_error in other.get_structured_errors() {
            base.add_structured_error(structured_error.error, structured_error.context);
        }

        base
    }

    /// Process delete operations
    fn process_deletes(
        &self,
        deletes: &[FileOperation],
        _options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
        _start_time: std::time::Instant,
    ) -> Result<SyncStats> {
        use crate::sync_stats::StructuredError;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::{Arc, Mutex};

        // Thread-safe counters for parallel processing
        let files_deleted = Arc::new(AtomicU64::new(0));
        let structured_errors = Arc::new(Mutex::new(Vec::<StructuredError>::new()));

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
                                    // Don't print to stderr with progress bar active
                                    // Record structured error for log file
                                    let robosync_err =
                                        RoboSyncError::io_error(e, Some(path.clone()));
                                    if let Ok(mut errors) = structured_errors.lock() {
                                        errors.push(StructuredError {
                                            error: robosync_err,
                                            context: "remove_dir_all".to_string(),
                                        });
                                    }
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
                                    // Don't print to stderr with progress bar active
                                    // Record structured error for log file
                                    let robosync_err =
                                        RoboSyncError::io_error(e, Some(path.clone()));
                                    if let Ok(mut errors) = structured_errors.lock() {
                                        errors.push(StructuredError {
                                            error: robosync_err,
                                            context: "remove_file".to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Record all errors during delete operations
                        let robosync_err = RoboSyncError::io_error(e, Some(path.clone()));
                        if let Ok(mut errors) = structured_errors.lock() {
                            errors.push(StructuredError {
                                error: robosync_err,
                                context: "symlink_metadata".to_string(),
                            });
                        }
                    }
                }
            }
        });

        // Create final stats from atomic counters
        let stats = SyncStats::default();

        // Add the counts using the proper methods
        let deleted_count = files_deleted.load(Ordering::Relaxed);

        // Use loop to add multiple counts since increment methods only add 1
        for _ in 0..deleted_count {
            stats.increment_files_deleted();
        }

        // Add structured errors to stats
        if let Ok(mut errors) = structured_errors.lock() {
            for error in errors.drain(..) {
                stats.add_structured_error(error.error, &error.context);
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
pub struct PendingStats {
    pub files_create: u64,
    pub files_update: u64,
    pub files_delete: u64,
    pub dirs_create: u64,
    pub size_create: u64,
    pub size_update: u64,
    pub size_delete: u64,
}

/// Detailed breakdown by size category
pub struct SizeBreakdown {
    pub small_count: u64,
    pub small_size: u64,
    pub medium_count: u64,
    pub medium_size: u64,
    pub large_count: u64,
    pub large_size: u64,
    pub delta_count: u64,
    pub delta_size: u64,
    pub delta_actual: u64, // Estimated actual transfer size for delta files
}

/// Detailed pending stats with size breakdowns
pub struct DetailedPendingStats {
    pub basic: PendingStats,
    pub create_breakdown: SizeBreakdown,
    pub update_breakdown: SizeBreakdown,
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

    /// Calculate detailed pending stats with size breakdowns
    fn calculate_detailed_pending_stats(
        &self,
        categorized: &CategorizedOps,
        _source_root: &Path,
    ) -> DetailedPendingStats {
        let mut stats = DetailedPendingStats {
            basic: PendingStats {
                files_create: 0,
                files_update: 0,
                files_delete: 0,
                dirs_create: 0,
                size_create: 0,
                size_update: 0,
                size_delete: 0,
            },
            create_breakdown: SizeBreakdown {
                small_count: 0,
                small_size: 0,
                medium_count: 0,
                medium_size: 0,
                large_count: 0,
                large_size: 0,
                delta_count: 0,
                delta_size: 0,
                delta_actual: 0,
            },
            update_breakdown: SizeBreakdown {
                small_count: 0,
                small_size: 0,
                medium_count: 0,
                medium_size: 0,
                large_count: 0,
                large_size: 0,
                delta_count: 0,
                delta_size: 0,
                delta_actual: 0,
            },
        };

        // Process small files
        for op in &categorized.small_files {
            match op {
                FileOperation::Create { path } => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        if !metadata.is_dir() {
                            let size = metadata.len();
                            stats.basic.files_create += 1;
                            stats.basic.size_create += size;
                            stats.create_breakdown.small_count += 1;
                            stats.create_breakdown.small_size += size;
                        }
                    }
                }
                FileOperation::Update { path, .. } => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        if !metadata.is_dir() {
                            let size = metadata.len();
                            stats.basic.files_update += 1;
                            stats.basic.size_update += size;
                            stats.update_breakdown.small_count += 1;
                            stats.update_breakdown.small_size += size;
                        }
                    }
                }
                _ => {}
            }
        }

        // Process medium files
        for op in &categorized.medium_files {
            match op {
                FileOperation::Create { path } => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        if !metadata.is_dir() {
                            let size = metadata.len();
                            stats.basic.files_create += 1;
                            stats.basic.size_create += size;
                            stats.create_breakdown.medium_count += 1;
                            stats.create_breakdown.medium_size += size;
                        }
                    }
                }
                FileOperation::Update { path, .. } => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        if !metadata.is_dir() {
                            let size = metadata.len();
                            stats.basic.files_update += 1;
                            stats.basic.size_update += size;
                            stats.update_breakdown.medium_count += 1;
                            stats.update_breakdown.medium_size += size;
                        }
                    }
                }
                _ => {}
            }
        }

        // Process large files
        for op in &categorized.large_files {
            match op {
                FileOperation::Create { path } => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        if !metadata.is_dir() {
                            let size = metadata.len();
                            stats.basic.files_create += 1;
                            stats.basic.size_create += size;
                            stats.create_breakdown.large_count += 1;
                            stats.create_breakdown.large_size += size;
                        }
                    }
                }
                FileOperation::Update { path, .. } => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        if !metadata.is_dir() {
                            let size = metadata.len();
                            stats.basic.files_update += 1;
                            stats.basic.size_update += size;
                            stats.update_breakdown.large_count += 1;
                            stats.update_breakdown.large_size += size;
                        }
                    }
                }
                _ => {}
            }
        }

        // Process delta files (with estimated actual transfer)
        for op in &categorized.delta_files {
            match op {
                FileOperation::Create { path } => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        if !metadata.is_dir() {
                            let size = metadata.len();
                            stats.basic.files_create += 1;
                            stats.basic.size_create += size;
                            stats.create_breakdown.delta_count += 1;
                            stats.create_breakdown.delta_size += size;
                            stats.create_breakdown.delta_actual += size; // Full transfer for new files
                        }
                    }
                }
                FileOperation::Update { path, .. } => {
                    if let Ok(metadata) = std::fs::metadata(path) {
                        if !metadata.is_dir() {
                            let size = metadata.len();
                            stats.basic.files_update += 1;
                            stats.basic.size_update += size;
                            stats.update_breakdown.delta_count += 1;
                            stats.update_breakdown.delta_size += size;
                            // Estimate 10% change for delta files (conservative)
                            stats.update_breakdown.delta_actual += size / 10;
                        }
                    }
                }
                _ => {}
            }
        }

        // Count directory creates
        for op in &categorized.directories {
            if let FileOperation::CreateDirectory { .. } = op {
                stats.basic.dirs_create += 1;
            }
        }

        // Count deletes
        stats.basic.files_delete = categorized.deletes.len() as u64;

        stats
    }
}
