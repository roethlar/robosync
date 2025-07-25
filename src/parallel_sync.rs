//! Multithreaded synchronization implementation

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::algorithm::{BlockChecksum, DeltaAlgorithm, Match};
use crate::compression::{decompress_data, CompressionType};
use crate::file_list::{
    compare_file_lists_with_roots, compare_file_lists_with_roots_and_progress,
    generate_file_list_with_options, generate_file_list_with_options_and_progress, FileInfo,
    FileOperation,
};
#[cfg(target_os = "linux")]
use crate::file_list::generate_file_list_parallel;
use crate::logging::SyncLogger;
use crate::metadata::{copy_file_with_metadata, copy_file_with_metadata_with_warnings, CopyFlags};
use crate::options::SyncOptions;
use crate::progress::SyncProgress;
use crate::retry::{with_retry, RetryConfig};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// Configuration for multithreaded synchronization
#[derive(Debug, Clone)]
pub struct ParallelSyncConfig {
    /// Number of worker threads for file processing
    pub worker_threads: usize,
    /// Number of I/O threads for reading/writing
    #[allow(dead_code)]
    pub io_threads: usize,
    /// Block size for delta algorithm
    pub block_size: usize,
    /// Maximum number of files to process in parallel
    #[allow(dead_code)]
    pub max_parallel_files: usize,
}

impl Default for ParallelSyncConfig {
    fn default() -> Self {
        let num_cpus = std::thread::available_parallelism().unwrap().get();
        Self {
            worker_threads: num_cpus,
            io_threads: std::cmp::min(4, num_cpus),
            block_size: 1024,
            max_parallel_files: num_cpus * 2,
        }
    }
}

/// Multithreaded file synchronization engine
pub struct ParallelSyncer {
    config: ParallelSyncConfig,
}

impl ParallelSyncer {
    pub fn new(config: ParallelSyncConfig) -> Self {
        Self { config }
    }

    /// Synchronize files using multiple threads with options
    pub fn synchronize_with_options(
        &self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        let _start_time = Instant::now();

        println!("Starting parallel synchronization...");
        println!("  Source: {}", source.display());
        println!("  Destination: {}", destination.display());
        println!("  Threads: {}", self.config.worker_threads);

        // Create destination parent directory if needed, but don't create destination itself for file-to-file sync
        if source.is_dir() && !destination.exists() {
            fs::create_dir_all(&destination).with_context(|| {
                format!(
                    "Failed to create destination directory: {}",
                    destination.display()
                )
            })?;
            println!("Created destination directory: {}", destination.display());
        }

        // Pre-warm network connection for UNC paths or mapped network drives to avoid initial delay
        #[cfg(windows)]
        {
            use std::time::Duration;
            
            // Check if destination might be a network location
            let is_network = destination.to_str().map(|s| s.starts_with("\\\\")).unwrap_or(false);
            let is_mapped_drive = destination.to_str()
                .and_then(|s| s.chars().next())
                .map(|c| c.is_ascii_alphabetic() && destination.to_str().unwrap().chars().nth(1) == Some(':'))
                .unwrap_or(false);
            
            if is_network || is_mapped_drive {
                println!("Testing network connection to destination...");
                
                // Try to create a test file to establish connection and verify write access
                let test_file = destination.join(".robosync_test");
                match std::fs::write(&test_file, b"test") {
                    Ok(_) => {
                        let _ = std::fs::remove_file(&test_file);
                        println!("Network connection established successfully.");
                    }
                    Err(e) => {
                        // If we can't write, at least try to read to establish connection
                        println!("Warning: Could not write test file: {}", e);
                        println!("Attempting to establish read connection...");
                        
                        // Try with timeout
                        let start = std::time::Instant::now();
                        let timeout = Duration::from_secs(30);
                        
                        while start.elapsed() < timeout {
                            if fs::metadata(&destination).is_ok() {
                                println!("Read connection established.");
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        
                        if start.elapsed() >= timeout {
                            eprintln!("Warning: Network connection is slow or unresponsive");
                        }
                    }
                }
            }
        }

        if source.is_file() {
            // Single file sync
            self.sync_single_file(&source, &destination, &options)
        } else if source.is_dir() {
            // Directory sync
            self.sync_directories(&source, &destination, &options)
        } else {
            Err(anyhow::anyhow!("Invalid source: {}", source.display()))
        }
    }

    /// Synchronize a single file
    fn sync_single_file(
        &self,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        let mut logger = SyncLogger::new(options.log_file.as_deref(), options.show_eta)?;
        logger.initialize_progress(1, std::fs::metadata(source)?.len());

        let dest_path = if destination.exists() && destination.is_dir() {
            let file_name = source
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Source file has no name"))?;
            destination.join(file_name)
        } else {
            destination.to_path_buf()
        };

        let stats = self.sync_file_pair(source, &dest_path, options)?;
        logger.update_progress(1, stats.get_bytes_transferred());
        logger.log_summary(&stats);

        Ok(stats)
    }

    /// Synchronize directories using parallel processing
    fn sync_directories(
        &self,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        // Create logger and multi-progress for this sync operation
        let mut logger = SyncLogger::new(options.log_file.as_deref(), options.show_eta)?;

        // Create MultiProgress for analysis phase - always use for scanning progress
        let multi_progress = if options.no_progress {
            None
        } else {
            // In indicatif 0.18, MultiProgress automatically handles rendering
            Some(Arc::new(MultiProgress::new()))
        };

        // Scan source directory with progress
        let source_files = if let Some(ref mp) = multi_progress {
            let source_pb = mp.add(ProgressBar::new_spinner());
            source_pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} Scanning source: {pos} files found...")
                    .unwrap(),
            );
            source_pb.enable_steady_tick(std::time::Duration::from_millis(100));

            let source_files = generate_file_list_with_options_and_progress(
                source,
                options,
                Some(|count| {
                    source_pb.set_position(count as u64);
                }),
            )
            .context("Failed to generate source file list")?;

            source_pb.finish_with_message(format!("Found {} items in source", source_files.len()));
            source_files
        } else {
            logger.log("Scanning source directory...");
            #[cfg(target_os = "linux")]
            let files = if options.linux_optimized {
                logger.log("Using Linux-optimized parallel scanning...");
                generate_file_list_parallel(source, options)
                    .context("Failed to generate source file list")?
            } else {
                generate_file_list_with_options(source, options)
                    .context("Failed to generate source file list")?
            };
            #[cfg(not(target_os = "linux"))]
            let files = generate_file_list_with_options(source, options)
                .context("Failed to generate source file list")?;
            logger.log(&format!("Found {} items in source", files.len()));
            files
        };

        // Scan destination directory with progress
        let dest_files = if destination.exists() {
            let dest_files = if let Some(ref mp) = multi_progress {
                let dest_pb = mp.add(ProgressBar::new_spinner());
                dest_pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{spinner:.green} Scanning destination: {pos} files found...")
                        .unwrap(),
                );
                dest_pb.enable_steady_tick(std::time::Duration::from_millis(100));

                let files = generate_file_list_with_options_and_progress(
                    destination,
                    options,
                    Some(|count| {
                        dest_pb.set_position(count as u64);
                    }),
                )
                .context("Failed to generate destination file list")?;

                dest_pb.finish_with_message(format!("Found {} items in destination", files.len()));
                files
            } else {
                logger.log("Scanning destination directory...");
                #[cfg(target_os = "linux")]
                let files = if options.linux_optimized {
                    logger.log("Using Linux-optimized parallel scanning...");
                    generate_file_list_parallel(destination, options)
                        .context("Failed to generate destination file list")?
                } else {
                    generate_file_list_with_options(destination, options)
                        .context("Failed to generate destination file list")?
                };
                #[cfg(not(target_os = "linux"))]
                let files = generate_file_list_with_options(destination, options)
                    .context("Failed to generate destination file list")?;
                logger.log(&format!("Found {} items in destination", files.len()));
                files
            };

            // Filter out the destination root directory to avoid deleting it
            let mut files = dest_files;
            files.retain(|f| f.path != *destination);
            files
        } else {
            if let Some(ref mp) = multi_progress {
                mp.println("Destination does not exist, will create")
                    .unwrap();
            } else {
                logger.log("Destination does not exist, will create");
            }
            Vec::new()
        };

        // Analysis phase with progress indication
        let mut operations = if !options.no_progress {
            // Create a spinner to show analysis activity
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} Analyzing changes... {pos} files processed")
                    .unwrap(),
            );
            pb.enable_steady_tick(std::time::Duration::from_millis(100));

            let operations = compare_file_lists_with_roots_and_progress(
                &source_files,
                &dest_files,
                source,
                destination,
                options,
                Some(|count| {
                    pb.set_position(count as u64);
                }),
            );
            pb.finish_with_message("Analysis complete");
            operations
        } else {
            logger.log("Analyzing changes...");
            let operations = compare_file_lists_with_roots(
                &source_files,
                &dest_files,
                source,
                destination,
                options,
            );
            logger.log("Analysis complete");
            operations
        };

        // Add purge operations if mirror or purge mode is enabled
        if options.purge || options.mirror {
            if !options.no_progress {
                let pb = if let Some(ref mp) = multi_progress {
                    mp.add(ProgressBar::new_spinner())
                } else {
                    ProgressBar::new_spinner()
                };
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{spinner:.green} Finding files to purge...")
                        .unwrap(),
                );
                pb.enable_steady_tick(std::time::Duration::from_millis(100));

                let purge_ops = self.find_purge_operations_with_progress(
                    &source_files,
                    &dest_files,
                    source,
                    destination,
                    |_count| {
                        // Don't update position since it completes too fast to see
                    },
                )?;
                let purge_count = purge_ops.len();
                operations.extend(purge_ops);

                pb.finish_with_message(format!(
                    "Purge analysis complete - {purge_count} files to remove"
                ));
            } else {
                logger.log("Finding files to purge...");
                let purge_ops =
                    self.find_purge_operations(&source_files, &dest_files, source, destination)?;
                let purge_count = purge_ops.len();
                operations.extend(purge_ops);
                logger.log(&format!(
                    "Purge analysis complete - {purge_count} files to remove"
                ));
            }
        }

        if operations.is_empty() {
            logger.log("No changes needed.");
            return Ok(SyncStats::default());
        }

        // Create a HashMap for O(1) source file lookups instead of O(n) linear search
        let source_file_map: std::collections::HashMap<&PathBuf, &FileInfo> =
            source_files.iter().map(|f| (&f.path, f)).collect();

        // Count operations and calculate total bytes for operations that will transfer data
        let total_files = operations.len() as u64;
        let total_bytes: u64 = operations
            .iter()
            .filter_map(|op| match op {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    source_file_map
                        .get(path)
                        .filter(|f| !f.is_directory)
                        .map(|f| f.size)
                }
                _ => None,
            })
            .sum();

        // Initialize progress tracking in logger
        logger.initialize_progress(total_files, total_bytes);

        logger.log(&format!(
            "Processing {} operations, {} create operations, {} delete operations",
            operations.len(),
            operations
                .iter()
                .filter(|op| matches!(
                    op,
                    FileOperation::Create { .. }
                        | FileOperation::Update { .. }
                        | FileOperation::CreateSymlink { .. }
                        | FileOperation::UpdateSymlink { .. }
                ))
                .count(),
            operations
                .iter()
                .filter(|op| matches!(op, FileOperation::Delete { .. }))
                .count()
        ));

        // Show file list only in verbose mode (but not when using --confirm, as it shows summary instead)
        if options.verbose >= 1 && !options.confirm {
            // Use MultiProgress's println if available, otherwise use logger
            if let Some(ref mp) = multi_progress {
                let _ = mp.println("\nFile operations to be performed:");
                for operation in &operations {
                    match operation {
                        FileOperation::Create { path } => {
                            if let Some(file_info) = source_files.iter().find(|f| f.path == *path) {
                                if file_info.is_directory {
                                    let _ = mp.println(format!(
                                        "    New Dir                      {}",
                                        path.display()
                                    ));
                                } else {
                                    let _ = mp.println(format!(
                                        "    New File        {:>12}  {}",
                                        file_info.size,
                                        path.display()
                                    ));
                                }
                            }
                        }
                        FileOperation::Update { path, use_delta } => {
                            if let Some(file_info) = source_files.iter().find(|f| f.path == *path) {
                                let method = if *use_delta { "Delta" } else { "Newer" };
                                let _ = mp.println(format!(
                                    "    {}           {:>12}  {}",
                                    method,
                                    file_info.size,
                                    path.display()
                                ));
                            }
                        }
                        FileOperation::Delete { path } => {
                            if path.is_file() {
                                let file_size =
                                    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                                let _ = mp.println(format!(
                                    "    *EXTRA File     {:>12}  {}",
                                    file_size,
                                    path.display()
                                ));
                            } else {
                                let _ = mp.println(format!(
                                    "    *EXTRA Dir                   {}",
                                    path.display()
                                ));
                            }
                        }
                        FileOperation::CreateDirectory { path } => {
                            let _ = mp.println(format!(
                                "    New Dir                      {}",
                                path.display()
                            ));
                        }
                        FileOperation::CreateSymlink { path, target } => {
                            let _ = mp.println(format!(
                                "    New Symlink                  {} -> {}",
                                path.display(),
                                target.display()
                            ));
                        }
                        FileOperation::UpdateSymlink { path, target } => {
                            let _ = mp.println(format!(
                                "    Update Symlink               {} -> {}",
                                path.display(),
                                target.display()
                            ));
                        }
                    }
                }
                let _ = mp.println("");
            } else {
                logger.log("\nFile operations to be performed:");
                for operation in &operations {
                    match operation {
                        FileOperation::Create { path } => {
                            if let Some(file_info) = source_files.iter().find(|f| f.path == *path) {
                                if file_info.is_directory {
                                    logger.log(&format!(
                                        "    New Dir                      {}",
                                        path.display()
                                    ));
                                } else {
                                    logger.log(&format!(
                                        "    New File        {:>12}  {}",
                                        file_info.size,
                                        path.display()
                                    ));
                                }
                            }
                        }
                        FileOperation::Update { path, use_delta } => {
                            if let Some(file_info) = source_files.iter().find(|f| f.path == *path) {
                                let method = if *use_delta { "Delta" } else { "Newer" };
                                logger.log(&format!(
                                    "    {}           {:>12}  {}",
                                    method,
                                    file_info.size,
                                    path.display()
                                ));
                            }
                        }
                        FileOperation::Delete { path } => {
                            if path.is_file() {
                                let file_size =
                                    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                                logger.log(&format!(
                                    "    *EXTRA File     {:>12}  {}",
                                    file_size,
                                    path.display()
                                ));
                            } else {
                                logger.log(&format!(
                                    "    *EXTRA Dir                   {}",
                                    path.display()
                                ));
                            }
                        }
                        FileOperation::CreateDirectory { path } => {
                            logger.log(&format!(
                                "    New Dir                      {}",
                                path.display()
                            ));
                        }
                        FileOperation::CreateSymlink { path, target } => {
                            logger.log(&format!(
                                "    New Symlink                  {} -> {}",
                                path.display(),
                                target.display()
                            ));
                        }
                        FileOperation::UpdateSymlink { path, target } => {
                            logger.log(&format!(
                                "    Update Symlink               {} -> {}",
                                path.display(),
                                target.display()
                            ));
                        }
                    }
                }
                logger.log("");
            }
        }

        // Clear MultiProgress before confirmation prompt to ensure clean output
        if let Some(ref mp) = multi_progress {
            mp.clear().ok();
        }

        // Ask for confirmation if requested
        if options.confirm && !operations.is_empty() {
            use std::io::{self, Write};

            // Show progress while counting operations for summary
            logger.log("Preparing operation summary...");

            // Count operation types for summary
            let mut new_files = 0;
            let mut new_dirs = 0;
            let mut updates = 0;
            let mut deletions = 0;
            let mut symlinks = 0;

            for op in &operations {
                match op {
                    FileOperation::Create { path } => {
                        if source_file_map.get(path).is_some_and(|f| f.is_directory) {
                            new_dirs += 1;
                        } else {
                            new_files += 1;
                        }
                    }
                    FileOperation::CreateDirectory { .. } => new_dirs += 1,
                    FileOperation::Update { .. } => updates += 1,
                    FileOperation::Delete { .. } => deletions += 1,
                    FileOperation::CreateSymlink { .. } | FileOperation::UpdateSymlink { .. } => {
                        symlinks += 1
                    }
                }
            }

            // For confirmation, always use regular output to avoid MultiProgress clearing
            logger.log("\nPending Operation Summary:");
            if new_files > 0 {
                logger.log(&format!("  New Files: {new_files}"));
            }
            if new_dirs > 0 {
                logger.log(&format!("  New Directories: {new_dirs}"));
            }
            if updates > 0 {
                logger.log(&format!("  Updates: {updates}"));
            }
            if deletions > 0 {
                logger.log(&format!("  Deletions: {deletions}"));
            }
            if symlinks > 0 {
                logger.log(&format!("  Symlinks: {symlinks}"));
            }
            logger.log("");

            // Ask for confirmation
            print!("Continue? Y/n: ");
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();
            let input = input.trim().to_lowercase();

            if input != "y" && input != "yes" && !input.is_empty() {
                logger.log("Operation cancelled by user.");
                return Ok(SyncStats::default());
            }
        }

        // Create progress tracking - disable for -vv mode
        let progress = if options.no_progress || options.verbose >= 2 {
            None
        } else {
            // Create progress bar that works with MultiProgress for verbose mode compatibility
            let copy_pb = if let Some(ref mp) = multi_progress {
                let pb = mp.add(ProgressBar::new(total_files));
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("{spinner:.green} [{elapsed_precise}] [{bar:50.cyan/blue}] {pos}/{len} files ({per_sec}, {eta}) {msg}")
                        .unwrap()
                        .progress_chars("#>-"),
                );
                pb.enable_steady_tick(std::time::Duration::from_millis(100));
                Some(pb)
            } else {
                None
            };
            Some(Arc::new(Mutex::new(SyncProgress::new_with_progress_bar(
                total_files,
                total_bytes,
                copy_pb,
            ))))
        };

        // Remove duplicate progress tracking - use logger's progress system only
        let stats = Arc::new(SyncStats::new());

        // Set up Rayon thread pool for parallel processing
        // For network drives, use more threads to hide latency
        let effective_threads = if destination.to_str().map(|s| s.starts_with("\\\\") || s.contains(":")).unwrap_or(false) {
            self.config.worker_threads * 2  // Double threads for network operations
        } else {
            self.config.worker_threads
        };
        
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(effective_threads)
            .build()
            .context("Failed to create thread pool")?;
            
        println!("DEBUG: Using {} threads for parallel operations", effective_threads);

        // Separate operations by type for optimal ordering
        let (dir_ops, file_ops): (Vec<_>, Vec<_>) = operations
            .into_iter()
            .partition(|op| matches!(op, FileOperation::CreateDirectory { .. }));

        // Separate delete operations to run last
        let (file_ops, delete_ops): (Vec<_>, Vec<_>) = file_ops
            .into_iter()
            .partition(|op| !matches!(op, FileOperation::Delete { .. }));

        // Create directories first (sequentially to avoid race conditions)
        if !dir_ops.is_empty() {
            println!("DEBUG: Creating {} directories...", dir_ops.len());
            if options.verbose >= 1 {
                logger.log(&format!("Creating {} directories...", dir_ops.len()));
            }
            for operation in dir_ops {
                self.execute_operation(operation, source, destination, &stats, options, &mut logger)?;
                logger.update_progress(1, 0);
                if let Some(ref progress) = progress {
                    if let Ok(mut p) = progress.lock() {
                        p.update_file_complete(0);
                    }
                }
            }
        }

        // Batch small files for efficient processing
        println!("DEBUG: Starting file categorization of {} operations", file_ops.len());
        let (small_files, large_files): (Vec<_>, Vec<_>) = file_ops
            .into_iter()
            .partition(|op| self.is_small_file_operation(op, &source_files));
        println!("DEBUG: Categorized {} small files, {} large files", small_files.len(), large_files.len());

        // Log file processing start
        if options.verbose >= 1 {
            logger.log(&format!("Processing {} small files and {} large files...", 
                small_files.len(), large_files.len()));
        }
        
        // Additional debug info for network drives
        if destination.to_str().map(|s| s.starts_with("\\\\") || s.contains(":")).unwrap_or(false) {
            logger.log(&format!("Note: Destination appears to be a network location. Operations may be slower than local disk."));
            logger.log(&format!("Using {} threads with batch size of 100 for small files", self.config.worker_threads));
        }

        // Process files in parallel - note: logger is not thread-safe for parallel updates
        // We'll collect stats and update at the end of each operation
        let logger_arc = Arc::new(Mutex::new(logger));
        let progress_arc = progress.clone();

        // Process small files in batches
        if !small_files.is_empty() {
            let small_files_count = small_files.len();
            if options.verbose >= 1 {
                logger_arc.lock().unwrap().log(&format!("Starting parallel processing of {} small files", small_files_count));
            }
            
            // Pre-create all necessary directories to avoid redundant checks
            println!("DEBUG: Starting directory pre-creation for {} small files", small_files_count);
            let mut dirs_to_create = std::collections::HashSet::new();
            for operation in &small_files {
                match operation {
                    FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                        if let Ok(dest_path) = self.map_source_to_dest(path, source, destination) {
                            if let Some(parent) = dest_path.parent() {
                                dirs_to_create.insert(parent.to_path_buf());
                            }
                        }
                    }
                    _ => {}
                }
            }
            
            // Create all directories at once
            println!("DEBUG: Creating {} unique directories", dirs_to_create.len());
            for dir in dirs_to_create {
                let _ = fs::create_dir_all(dir);
            }
            println!("DEBUG: Directory creation complete, starting parallel file processing");
            
            // For now, use standard parallel copy on all platforms
            // TODO: Integrate Linux-specific optimizations when module import issues are resolved
            let verbose = options.verbose;
            
            // Add atomic counter to track which file causes hang
            let file_counter = std::sync::atomic::AtomicU64::new(0);
            
            pool.install(|| {
                use rayon::prelude::*;
                small_files
                    .into_par_iter()
                    .try_for_each(|operation| -> Result<()> {
                        match operation {
                            FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                                let current_file = file_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let start_time = std::time::Instant::now();
                                if current_file % 100 == 0 || current_file > 3560 {
                                    println!("Processing file #{}: {}", current_file, path.display());
                                }
                                // Use symlink_metadata to check without following symlinks
                                if let Ok(metadata) = fs::symlink_metadata(&path) {
                                    let file_type = metadata.file_type();
                                    
                                    // Handle symlinks specially
                                    if file_type.is_symlink() {
                                        if verbose >= 1 {
                                            println!("Handling symlink: {}", path.display());
                                        }
                                        let dest_path = self.map_source_to_dest(&path, source, destination)?;
                                        
                                        // Copy the symlink itself, not what it points to
                                        if let Ok(target) = fs::read_link(&path) {
                                            // Remove destination if it exists
                                            let _ = fs::remove_file(&dest_path);
                                            
                                            #[cfg(unix)]
                                            {
                                                use std::os::unix::fs::symlink;
                                                symlink(&target, &dest_path)?;
                                            }
                                            #[cfg(windows)]
                                            {
                                                // On Windows, just skip symlinks for now
                                                if verbose >= 1 {
                                                    println!("Skipping symlink on Windows: {}", path.display());
                                                }
                                            }
                                        }
                                        return Ok(());
                                    }
                                    
                                    // Skip other special files
                                    if !file_type.is_file() && !file_type.is_dir() {
                                        if verbose >= 1 {
                                            println!("Skipping special file: {}", path.display());
                                        }
                                        return Ok(());
                                    }
                                }
                                
                                let dest_path = self.map_source_to_dest(&path, source, destination)?;
                                
                                // Debug: Log files being processed in verbose mode
                                if verbose >= 2 {
                                    println!("Copying: {} -> {}", path.display(), dest_path.display());
                                }
                                
                                let bytes_copied = fs::copy(&path, &dest_path)?;
                                stats.add_bytes_transferred(bytes_copied);
                                
                                // Log slow files
                                let elapsed = start_time.elapsed();
                                if elapsed.as_secs() > 1 {
                                    println!("SLOW FILE #{}: {} took {:.2}s ({} bytes)", 
                                        current_file, path.display(), elapsed.as_secs_f64(), bytes_copied);
                                }
                                
                                // Temporarily disable progress updates for small files to avoid mutex contention
                                // TODO: Implement lock-free progress tracking
                            }
                            _ => {}
                        }
                        Ok(())
                    })
            })?;
            
            // Update progress for all small files at once
            if let Ok(mut log) = logger_arc.lock() {
                log.update_progress(small_files_count as u64, 0);
            }
        }

        // Process large files individually in parallel
        if !large_files.is_empty() {
            pool.install(|| {
                use rayon::prelude::*;
                large_files
                    .par_iter()
                    .try_for_each(|operation| -> Result<()> {
                        // Clone logger reference for thread safety
                        let logger_ref = Arc::clone(&logger_arc);
                        let progress_ref = progress_arc.clone();
                        let file_stats = self.execute_operation_parallel(
                            operation.clone(),
                            source,
                            destination,
                            &stats,
                            options,
                            logger_ref,
                        )?;

                        // Skip progress updates during parallel processing

                        Ok(())
                    })
            })?;
        }

        // Recover logger from Arc
        let mut logger = Arc::try_unwrap(logger_arc)
            .map_err(|_| anyhow::anyhow!("Failed to recover logger"))?
            .into_inner()
            .unwrap();

        // Process delete operations last (sequentially to avoid issues)
        for operation in delete_ops {
            self.execute_operation(operation, source, destination, &stats, options, &mut logger)?;
            logger.update_progress(1, 0);
            if let Some(ref progress) = progress {
                if let Ok(mut p) = progress.lock() {
                    p.update_file_complete(0);
                }
            }
        }

        if let Some(ref progress) = progress {
            if let Ok(p) = progress.lock() {
                p.finish();
            }
        }

        // Clear the MultiProgress to ensure clean output
        if let Some(ref mp) = multi_progress {
            mp.clear().ok();
        }

        let final_stats = Arc::try_unwrap(stats).unwrap();
        logger.log_summary(&final_stats);

        Ok(final_stats)
    }

    /// Execute a single file operation with logging
    fn execute_operation(
        &self,
        operation: FileOperation,
        source_root: &Path,
        dest_root: &Path,
        stats: &Arc<SyncStats>,
        options: &SyncOptions,
        logger: &mut SyncLogger,
    ) -> Result<SyncStats> {
        match operation {
            FileOperation::CreateDirectory { path } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;

                if options.verbose >= 2 {
                    logger.log(&format!(
                        "    Creating Dir                 {}",
                        dest_path.display()
                    ));
                }

                fs::create_dir_all(&dest_path).with_context(|| {
                    format!("Failed to create directory: {}", dest_path.display())
                })?;
                Ok(SyncStats::default())
            }
            FileOperation::Create { path } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;
                
                // Get file info before operations for better error reporting
                let file_metadata = fs::metadata(&path)
                    .with_context(|| format!("Failed to read source file metadata: {}", path.display()))?;
                let file_size = file_metadata.len();
                
                if options.verbose >= 1 {
                    logger.log(&format!(
                        "    Copying File    {:>12}  {} -> {}",
                        file_size,
                        path.display(),
                        dest_path.display()
                    ));
                }
                
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("Failed to create parent directory: {}", parent.display()))?;
                }

                // Parse copy flags and copy file with metadata
                let copy_flags = CopyFlags::from_string(&options.copy_flags);
                let bytes_copied = self.copy_file_with_retry_with_warnings(
                    &path,
                    &dest_path,
                    &copy_flags,
                    options,
                    &stats.warnings,
                )?;

                // If move mode is enabled, delete source file after successful copy
                if options.move_files && !options.dry_run {
                    fs::remove_file(&path).with_context(|| {
                        format!(
                            "Failed to delete source file after move: {}",
                            path.display()
                        )
                    })?;

                    if options.verbose >= 2 {
                        let message = format!(
                            "    Moved File      {:>12}  {} -> {}",
                            file_size,
                            path.display(),
                            dest_path.display()
                        );
                        logger.log(&message);
                    }
                }

                stats.add_bytes_transferred(bytes_copied);
                let stats = SyncStats::default();
                stats.add_bytes_transferred(bytes_copied);
                Ok(stats)
            }
            FileOperation::Update { path, use_delta } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;
                let file_size = fs::metadata(&path)?.len();

                if options.verbose >= 2 {
                    let message = if use_delta {
                        format!(
                            "    Updating (Delta) {:>12}  {}",
                            file_size,
                            dest_path.display()
                        )
                    } else {
                        format!(
                            "    Updating (Full)  {:>12}  {}",
                            file_size,
                            dest_path.display()
                        )
                    };
                    logger.log(&message);
                }

                // Always skip delta for now - just do a full copy for maximum performance
                if false && use_delta {
                    let file_stats = self.sync_file_pair(&path, &dest_path, options)?;
                    stats.add_bytes_transferred(file_stats.get_bytes_transferred());
                    Ok(file_stats)
                } else {
                    // Parse copy flags and copy file with metadata
                    let copy_flags = CopyFlags::from_string(&options.copy_flags);
                    let bytes_copied = self.copy_file_with_retry_with_warnings(
                        &path,
                        &dest_path,
                        &copy_flags,
                        options,
                        &stats.warnings,
                    )?;

                    // If move mode is enabled, delete source file after successful copy
                    if options.move_files && !options.dry_run {
                        fs::remove_file(&path).with_context(|| {
                            format!(
                                "Failed to delete source file after move: {}",
                                path.display()
                            )
                        })?;

                        if options.verbose >= 2 {
                            let message = format!(
                                "    Moved File      {:>12}  {} -> {}",
                                file_size,
                                path.display(),
                                dest_path.display()
                            );
                            logger.log(&message);
                        }
                    }

                    stats.add_bytes_transferred(bytes_copied);
                    let stats = SyncStats::default();
                    stats.add_bytes_transferred(bytes_copied);
                    Ok(stats)
                }
            }
            FileOperation::Delete { path } => {
                // Use symlink_metadata to check type without following symlinks
                let metadata = fs::symlink_metadata(&path)
                    .with_context(|| format!("Failed to get metadata for: {}", path.display()))?;

                if options.verbose >= 2 {
                    if metadata.is_file() {
                        let file_size = metadata.len();
                        logger.log(&format!(
                            "    Deleting File   {:>12}  {}",
                            file_size,
                            path.display()
                        ));
                    } else {
                        logger.log(&format!(
                            "    Deleting Dir                 {}",
                            path.display()
                        ));
                    }
                }

                if metadata.is_symlink() || metadata.is_file() {
                    fs::remove_file(&path)
                        .with_context(|| format!("Failed to delete: {}", path.display()))?;
                } else if metadata.is_dir() {
                    fs::remove_dir_all(&path).with_context(|| {
                        format!("Failed to delete directory: {}", path.display())
                    })?;
                }
                Ok(SyncStats::default())
            }
            FileOperation::CreateSymlink { path, target } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)?;
                }

                if options.verbose >= 2 {
                    let message = format!(
                        "    New Symlink                  {} -> {}",
                        dest_path.display(),
                        target.display()
                    );
                    logger.log(&message);
                }

                self.create_symlink(&target, &dest_path)?;
                Ok(SyncStats::default())
            }
            FileOperation::UpdateSymlink { path, target } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;

                if options.verbose >= 2 {
                    let message = format!(
                        "    Update Symlink               {} -> {}",
                        dest_path.display(),
                        target.display()
                    );
                    logger.log(&message);
                }

                // Remove existing symlink
                fs::remove_file(&dest_path).with_context(|| {
                    format!("Failed to remove existing symlink: {}", dest_path.display())
                })?;

                // Create new symlink
                self.create_symlink(&target, &dest_path)?;
                Ok(SyncStats::default())
            }
        }
    }

    /// Execute a single file operation in parallel context (thread-safe logging)
    fn execute_operation_parallel(
        &self,
        operation: FileOperation,
        source_root: &Path,
        dest_root: &Path,
        stats: &Arc<SyncStats>,
        options: &SyncOptions,
        logger: Arc<Mutex<SyncLogger>>,
    ) -> Result<SyncStats> {
        match operation {
            FileOperation::CreateDirectory { path } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;

                if options.verbose >= 2 {
                    if let Ok(log) = logger.lock() {
                        log.log(&format!(
                            "    Creating Dir                 {}",
                            dest_path.display()
                        ));
                    }
                }

                fs::create_dir_all(&dest_path).with_context(|| {
                    format!("Failed to create directory: {}", dest_path.display())
                })?;

                // Skip logger updates in parallel mode

                Ok(SyncStats::default())
            }
            FileOperation::Create { path } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;
                
                // Create parent directory if needed
                if let Some(parent) = dest_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }

                // SUPER SIMPLE COPY - just use fs::copy directly
                let bytes_copied = fs::copy(&path, &dest_path)
                    .with_context(|| format!("Failed to copy: {} -> {}", path.display(), dest_path.display()))?;

                // If move mode is enabled, delete source file after successful copy
                if options.move_files && !options.dry_run {
                    fs::remove_file(&path).with_context(|| {
                        format!(
                            "Failed to delete source file after move: {}",
                            path.display()
                        )
                    })?;

                    // Verbose logging for moves is suppressed during execution to avoid interfering with progress bars
                }

                stats.add_bytes_transferred(bytes_copied);

                // Skip logger update here - will be done in batch

                let stats = SyncStats::default();
                stats.add_bytes_transferred(bytes_copied);
                Ok(stats)
            }
            FileOperation::Update { path, use_delta } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;
                let file_size = fs::metadata(&path)?.len();

                if options.verbose >= 2 {
                    if let Ok(log) = logger.lock() {
                        if use_delta {
                            log.log(&format!(
                                "    Updating (Delta) {:>12}  {}",
                                file_size,
                                dest_path.display()
                            ));
                        } else {
                            log.log(&format!(
                                "    Updating (Full)  {:>12}  {}",
                                file_size,
                                dest_path.display()
                            ));
                        }
                    }
                }

                // SUPER SIMPLE COPY for updates too
                let bytes_copied = fs::copy(&path, &dest_path)
                    .with_context(|| format!("Failed to update: {} -> {}", path.display(), dest_path.display()))?;

                // If move mode is enabled, delete source file after successful copy
                if options.move_files && !options.dry_run {
                    fs::remove_file(&path).with_context(|| {
                        format!(
                            "Failed to delete source file after move: {}",
                            path.display()
                        )
                    })?;
                }

                stats.add_bytes_transferred(bytes_copied);

                // Skip logger update here - will be done in batch
                let file_stats = SyncStats::default();
                file_stats.add_bytes_transferred(bytes_copied);
                Ok(file_stats)
            }
            FileOperation::Delete { path } => {
                // Use symlink_metadata to check type without following symlinks
                let metadata = fs::symlink_metadata(&path)
                    .with_context(|| format!("Failed to get metadata for: {}", path.display()))?;

                if options.verbose >= 2 {
                    if let Ok(log) = logger.lock() {
                        if metadata.is_file() {
                            let file_size = metadata.len();
                            log.log(&format!(
                                "    Deleting File   {:>12}  {}",
                                file_size,
                                path.display()
                            ));
                        } else {
                            log.log(&format!(
                                "    Deleting Dir                 {}",
                                path.display()
                            ));
                        }
                    }
                }

                if metadata.is_symlink() || metadata.is_file() {
                    fs::remove_file(&path)
                        .with_context(|| format!("Failed to delete: {}", path.display()))?;
                } else if metadata.is_dir() {
                    fs::remove_dir_all(&path).with_context(|| {
                        format!("Failed to delete directory: {}", path.display())
                    })?;
                }

                // Update logger progress
                if let Ok(mut log) = logger.lock() {
                    log.update_progress(1, 0);
                }

                Ok(SyncStats::default())
            }
            FileOperation::CreateSymlink { path, target } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)?;
                }

                if options.verbose >= 2 {
                    if let Ok(log) = logger.lock() {
                        log.log(&format!(
                            "    New Symlink                  {} -> {}",
                            dest_path.display(),
                            target.display()
                        ));
                    }
                }

                self.create_symlink(&target, &dest_path)?;

                // Update logger progress
                if let Ok(mut log) = logger.lock() {
                    log.update_progress(1, 0);
                }

                Ok(SyncStats::default())
            }
            FileOperation::UpdateSymlink { path, target } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;

                if options.verbose >= 2 {
                    if let Ok(log) = logger.lock() {
                        log.log(&format!(
                            "    Update Symlink               {} -> {}",
                            dest_path.display(),
                            target.display()
                        ));
                    }
                }

                // Remove existing symlink
                fs::remove_file(&dest_path).with_context(|| {
                    format!("Failed to remove existing symlink: {}", dest_path.display())
                })?;

                // Create new symlink
                self.create_symlink(&target, &dest_path)?;

                // Update logger progress
                if let Ok(mut log) = logger.lock() {
                    log.update_progress(1, 0);
                }

                Ok(SyncStats::default())
            }
        }
    }

    /// Synchronize a single file pair using parallel block processing
    fn sync_file_pair(
        &self,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        let file_size = fs::metadata(source)?.len();

        // For large files (>10MB), use streaming copy instead of loading into memory
        const STREAMING_THRESHOLD: u64 = 10 * 1024 * 1024; // 10MB

        if !destination.exists() {
            // New file, use optimized copy strategy based on file size
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create parent directory: {}", parent.display())
                })?;
            }

            if file_size > STREAMING_THRESHOLD {
                // Use streaming copy for large files
                self.streaming_copy(source, destination)?;
            } else {
                // Use memory copy for small files (faster for small files)
                let source_data = fs::read(source)
                    .with_context(|| format!("Failed to read source file: {}", source.display()))?;
                fs::write(destination, &source_data).with_context(|| {
                    format!(
                        "Failed to write destination file: {}",
                        destination.display()
                    )
                })?;
            }

            // Apply metadata based on copy flags
            let copy_flags = CopyFlags::from_string(&options.copy_flags);
            let stats = SyncStats::new();

            // Check for auditing flag and collect warning
            if copy_flags.auditing {
                stats.add_warning(
                    "Warning: Auditing info copying (U flag) not supported on this platform"
                        .to_string(),
                );
            }

            if copy_flags.timestamps
                || copy_flags.security
                || copy_flags.attributes
                || copy_flags.owner
            {
                let source_metadata = fs::metadata(source).with_context(|| {
                    format!("Failed to read source metadata: {}", source.display())
                })?;

                if copy_flags.timestamps {
                    crate::metadata::copy_timestamps(source, destination, &source_metadata)?;
                }
                if copy_flags.security {
                    crate::metadata::copy_permissions(source, destination, &source_metadata)?;
                }
                if copy_flags.attributes {
                    crate::metadata::copy_attributes(source, destination, &source_metadata)?;
                }
                #[cfg(unix)]
                if copy_flags.owner {
                    crate::metadata::copy_ownership(source, destination, &source_metadata)?;
                }
                // Auditing warning handled separately to avoid interrupting progress
            }

            // If move mode is enabled, delete source file after successful copy
            if options.move_files && !options.dry_run {
                fs::remove_file(source).with_context(|| {
                    format!(
                        "Failed to delete source file after move: {}",
                        source.display()
                    )
                })?;
            }

            stats.add_bytes_transferred(file_size);
            return Ok(stats);
        }

        // Existing file, use parallel delta algorithm with streaming for large files
        if file_size > STREAMING_THRESHOLD {
            // For large files, use streaming delta algorithm (to be implemented)
            // For now, fall back to direct copy for large files to avoid memory issues
            self.streaming_copy(source, destination)?;

            // Apply metadata from source to destination
            let copy_flags = CopyFlags::from_string(&options.copy_flags);
            let stats = SyncStats::new();

            // Check for auditing flag and collect warning
            if copy_flags.auditing {
                stats.add_warning(
                    "Warning: Auditing info copying (U flag) not supported on this platform"
                        .to_string(),
                );
            }

            if copy_flags.timestamps
                || copy_flags.security
                || copy_flags.attributes
                || copy_flags.owner
            {
                let source_metadata = fs::metadata(source).with_context(|| {
                    format!("Failed to read source metadata: {}", source.display())
                })?;

                if copy_flags.timestamps {
                    crate::metadata::copy_timestamps(source, destination, &source_metadata)?;
                }
                if copy_flags.security {
                    crate::metadata::copy_permissions(source, destination, &source_metadata)?;
                }
                if copy_flags.attributes {
                    crate::metadata::copy_attributes(source, destination, &source_metadata)?;
                }
                #[cfg(unix)]
                if copy_flags.owner {
                    crate::metadata::copy_ownership(source, destination, &source_metadata)?;
                }
            }

            if options.move_files && !options.dry_run {
                fs::remove_file(source).with_context(|| {
                    format!(
                        "Failed to delete source file after delta move: {}",
                        source.display()
                    )
                })?;
            }

            stats.add_bytes_transferred(file_size);
            return Ok(stats);
        }

        // Small files: use traditional delta algorithm
        let source_data = fs::read(source)
            .with_context(|| format!("Failed to read source file: {}", source.display()))?;
        let dest_data = fs::read(destination).with_context(|| {
            format!("Failed to read destination file: {}", destination.display())
        })?;

        let mut algorithm = DeltaAlgorithm::new(self.config.block_size);
        if options.compress {
            algorithm = algorithm.with_compression(options.compression_config);
        }

        // Generate checksums in parallel
        let checksums = self.parallel_generate_checksums(&algorithm, &dest_data)?;

        // Find matches
        let matches = algorithm
            .find_matches(&source_data, &checksums)
            .context("Failed to find matches")?;

        // Apply delta to reconstruct file
        let compression_type = if options.compress {
            options.compression_config.algorithm
        } else {
            CompressionType::None
        };
        let new_data = self.apply_delta(&dest_data, &matches, compression_type)?;

        // Write updated file
        fs::write(destination, &new_data)
            .with_context(|| format!("Failed to write updated file: {}", destination.display()))?;

        // Apply metadata from source to destination
        let copy_flags = CopyFlags::from_string(&options.copy_flags);

        // Check for auditing flag and collect warning
        if copy_flags.auditing {
            // This warning will be collected by the parent stats object
        }

        if copy_flags.timestamps || copy_flags.security || copy_flags.attributes || copy_flags.owner
        {
            // Apply metadata (but not data since we already wrote the delta-reconstructed data)
            let source_metadata = fs::metadata(source)
                .with_context(|| format!("Failed to read source metadata: {}", source.display()))?;

            if copy_flags.timestamps {
                crate::metadata::copy_timestamps(source, destination, &source_metadata)?;
            }
            if copy_flags.security {
                crate::metadata::copy_permissions(source, destination, &source_metadata)?;
            }
            if copy_flags.attributes {
                crate::metadata::copy_attributes(source, destination, &source_metadata)?;
            }
            #[cfg(unix)]
            if copy_flags.owner {
                crate::metadata::copy_ownership(source, destination, &source_metadata)?;
            }
        }

        // Calculate transfer statistics
        let literal_bytes: u64 = matches
            .iter()
            .filter_map(|m| match m {
                Match::Literal { data, .. } => Some(data.len() as u64),
                _ => None,
            })
            .sum();

        // If move mode is enabled, delete source file after successful delta sync
        if options.move_files && !options.dry_run {
            fs::remove_file(source).with_context(|| {
                format!(
                    "Failed to delete source file after delta move: {}",
                    source.display()
                )
            })?;
        }

        let stats = SyncStats::new();
        stats.add_bytes_transferred(literal_bytes);

        // Check for auditing flag and collect warning for delta transfer
        let copy_flags = CopyFlags::from_string(&options.copy_flags);
        if copy_flags.auditing {
            stats.add_warning(
                "Warning: Auditing info copying (U flag) not supported on this platform"
                    .to_string(),
            );
        }

        Ok(stats)
    }

    /// Generate checksums in parallel using Rayon
    fn parallel_generate_checksums(
        &self,
        algorithm: &DeltaAlgorithm,
        data: &[u8],
    ) -> Result<Vec<BlockChecksum>> {
        use rayon::prelude::*;
        let block_size = self.config.block_size;

        // Split data into chunks for parallel processing
        let chunks: Vec<(usize, &[u8])> = data.chunks(block_size).enumerate().collect();

        // Process chunks in parallel
        let checksums: Result<Vec<_>, _> = chunks
            .par_iter()
            .map(|(index, block)| {
                algorithm
                    .generate_checksums(block)
                    .map(|mut block_checksums| {
                        // Adjust offsets for the chunk position
                        for checksum in &mut block_checksums {
                            checksum.offset = (index * block_size) as u64;
                        }
                        block_checksums
                    })
            })
            .collect();

        // Flatten results
        Ok(checksums?.into_iter().flatten().collect())
    }

    /// Apply delta matches to reconstruct a file
    fn apply_delta(
        &self,
        dest_data: &[u8],
        matches: &[Match],
        compression_type: CompressionType,
    ) -> Result<Vec<u8>> {
        let mut result = Vec::new();

        for match_item in matches {
            match match_item {
                Match::Literal {
                    data,
                    is_compressed,
                    ..
                } => {
                    if *is_compressed {
                        // Decompress the literal data
                        let decompressed = decompress_data(data, compression_type)?;
                        result.extend_from_slice(&decompressed);
                    } else {
                        result.extend_from_slice(data);
                    }
                }
                Match::Block {
                    target_offset,
                    length,
                    ..
                } => {
                    let start = *target_offset as usize;
                    let end = start + length;
                    if end <= dest_data.len() {
                        result.extend_from_slice(&dest_data[start..end]);
                    } else {
                        return Err(anyhow::anyhow!(
                            "Block match extends beyond destination data"
                        ));
                    }
                }
            }
        }

        Ok(result)
    }

    /// Map a source path to the corresponding destination path
    fn map_source_to_dest(
        &self,
        source_file: &Path,
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<PathBuf> {
        let relative = source_file.strip_prefix(source_root).with_context(|| {
            format!(
                "File {} is not under source root {}",
                source_file.display(),
                source_root.display()
            )
        })?;
        Ok(dest_root.join(relative))
    }

    /// Create a symlink at the destination pointing to the target
    fn create_symlink(&self, target: &Path, destination: &Path) -> Result<()> {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, destination).with_context(|| {
            format!(
                "Failed to create symlink: {} -> {}",
                destination.display(),
                target.display()
            )
        })?;

        #[cfg(windows)]
        {
            // On Windows, we need to determine if the target is a directory or file
            // For relative paths, we need to resolve them relative to the symlink location
            let target_path = if target.is_absolute() {
                target.to_path_buf()
            } else if let Some(parent) = destination.parent() {
                parent.join(target)
            } else {
                target.to_path_buf()
            };

            if target_path.is_dir() {
                std::os::windows::fs::symlink_dir(target, destination).with_context(|| {
                    format!(
                        "Failed to create directory symlink: {} -> {}",
                        destination.display(),
                        target.display()
                    )
                })?;
            } else {
                std::os::windows::fs::symlink_file(target, destination).with_context(|| {
                    format!(
                        "Failed to create file symlink: {} -> {}",
                        destination.display(),
                        target.display()
                    )
                })?;
            }
        }

        Ok(())
    }

    /// Find files/directories in destination that should be purged (deleted)
    fn find_purge_operations_with_progress<F>(
        &self,
        source_files: &[FileInfo],
        dest_files: &[FileInfo],
        source_root: &Path,
        dest_root: &Path,
        progress_callback: F,
    ) -> Result<Vec<FileOperation>>
    where
        F: Fn(usize) + Send + Sync,
    {
        use rayon::prelude::*;
        use std::collections::HashSet;

        // Create a set of all source file paths (relative to source root) in parallel
        // Pre-allocate capacity to avoid rehashing
        let mut source_paths = HashSet::with_capacity(source_files.len());
        source_paths.extend(
            source_files
                .par_iter()
                .filter_map(|f| f.path.strip_prefix(source_root).ok())
                .map(|p| p.to_path_buf())
                .collect::<Vec<_>>(),
        );

        // Process destination files in parallel and collect operations with metadata
        let operations_with_metadata: Vec<_> = dest_files
            .par_iter()
            .filter_map(|dest_file| {
                dest_file
                    .path
                    .strip_prefix(dest_root)
                    .ok()
                    .and_then(|relative_path| {
                        if !source_paths.contains(relative_path) {
                            let operation = FileOperation::Delete {
                                path: dest_file.path.clone(),
                            };
                            let is_directory = dest_file.is_directory;
                            let depth = if is_directory {
                                dest_file.path.components().count()
                            } else {
                                0
                            };
                            Some((operation, is_directory, depth))
                        } else {
                            None
                        }
                    })
            })
            .collect();

        // Update progress to show sorting phase
        progress_callback(dest_files.len() + 1);

        // Separate and sort operations
        let mut file_ops = Vec::new();
        let mut dir_ops = Vec::new();

        for (op, is_dir, depth) in operations_with_metadata {
            if is_dir {
                dir_ops.push((op, depth));
            } else {
                file_ops.push(op);
            }
        }

        // Sort directories by depth (deepest first)
        dir_ops.sort_by(|a, b| b.1.cmp(&a.1));

        // Combine operations in correct order
        let mut purge_ops: Vec<FileOperation> = file_ops;
        purge_ops.extend(dir_ops.into_iter().map(|(op, _)| op));

        Ok(purge_ops)
    }

    fn find_purge_operations(
        &self,
        source_files: &[FileInfo],
        dest_files: &[FileInfo],
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<Vec<FileOperation>> {
        use rayon::prelude::*;
        use std::collections::HashSet;

        // Create a set of all source file paths (relative to source root) in parallel
        // Pre-allocate capacity to avoid rehashing
        let mut source_paths = HashSet::with_capacity(source_files.len());
        source_paths.extend(
            source_files
                .par_iter()
                .filter_map(|f| f.path.strip_prefix(source_root).ok())
                .map(|p| p.to_path_buf())
                .collect::<Vec<_>>(),
        );

        // Process destination files and collect operations with metadata
        let operations_with_metadata: Vec<_> = dest_files
            .par_iter()
            .filter_map(|dest_file| {
                dest_file
                    .path
                    .strip_prefix(dest_root)
                    .ok()
                    .and_then(|relative_path| {
                        if !source_paths.contains(relative_path) {
                            let operation = FileOperation::Delete {
                                path: dest_file.path.clone(),
                            };
                            let is_directory = dest_file.is_directory;
                            let depth = if is_directory {
                                dest_file.path.components().count()
                            } else {
                                0
                            };
                            Some((operation, is_directory, depth))
                        } else {
                            None
                        }
                    })
            })
            .collect();

        // Separate and sort operations
        let mut file_ops = Vec::new();
        let mut dir_ops = Vec::new();

        for (op, is_dir, depth) in operations_with_metadata {
            if is_dir {
                dir_ops.push((op, depth));
            } else {
                file_ops.push(op);
            }
        }

        // Sort directories by depth (deepest first)
        dir_ops.sort_by(|a, b| b.1.cmp(&a.1));

        // Combine operations in correct order
        let mut purge_ops: Vec<FileOperation> = file_ops;
        purge_ops.extend(dir_ops.into_iter().map(|(op, _)| op));

        Ok(purge_ops)
    }

    /// Check if a file operation involves a small file (for batching optimization)
    fn is_small_file_operation(
        &self,
        operation: &FileOperation,
        source_files: &[FileInfo],
    ) -> bool {
        const SMALL_FILE_THRESHOLD: u64 = 1024 * 1024; // 1MB

        match operation {
            FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                // Look up file size in source_files
                source_files
                    .iter()
                    .find(|f| f.path == *path)
                    .map(|f| f.size < SMALL_FILE_THRESHOLD)
                    .unwrap_or(true) // Default to small if not found
            }
            // Non-file operations are considered "small" for batching purposes
            _ => true,
        }
    }

    /// Streaming copy for large files to reduce memory usage
    fn streaming_copy(&self, source: &Path, destination: &Path) -> Result<u64> {
        use std::fs::File;
        use std::io::{BufReader, BufWriter, Read, Write};

        const BUFFER_SIZE: usize = 4 * 1024 * 1024; // 4MB buffer for better network performance

        let source_file = File::open(source)
            .with_context(|| format!("Failed to open source file: {}", source.display()))?;
        let dest_file = File::create(destination).with_context(|| {
            format!(
                "Failed to create destination file: {}",
                destination.display()
            )
        })?;

        let mut reader = BufReader::with_capacity(BUFFER_SIZE, source_file);
        let mut writer = BufWriter::with_capacity(BUFFER_SIZE, dest_file);

        let mut buffer = vec![0u8; BUFFER_SIZE];
        let mut total_bytes = 0u64;

        loop {
            let bytes_read = reader.read(&mut buffer).with_context(|| {
                format!("Failed to read from source file: {}", source.display())
            })?;

            if bytes_read == 0 {
                break;
            }

            writer.write_all(&buffer[..bytes_read]).with_context(|| {
                format!(
                    "Failed to write to destination file: {}",
                    destination.display()
                )
            })?;

            total_bytes += bytes_read as u64;
        }

        writer.flush().with_context(|| {
            format!(
                "Failed to flush destination file: {}",
                destination.display()
            )
        })?;

        Ok(total_bytes)
    }

    /// Copy file with metadata, using retry logic if configured
    fn copy_file_with_retry(
        &self,
        source: &Path,
        dest: &Path,
        copy_flags: &CopyFlags,
        options: &SyncOptions,
    ) -> Result<u64> {
        self.copy_file_with_retry_internal(source, dest, copy_flags, options, None)
    }

    /// Copy file with metadata, using retry logic if configured, with warnings collector
    fn copy_file_with_retry_with_warnings(
        &self,
        source: &Path,
        dest: &Path,
        copy_flags: &CopyFlags,
        options: &SyncOptions,
        warnings: &Arc<Mutex<Vec<String>>>,
    ) -> Result<u64> {
        self.copy_file_with_retry_internal(source, dest, copy_flags, options, Some(warnings))
    }

    /// Internal implementation for copy_file_with_retry
    fn copy_file_with_retry_internal(
        &self,
        source: &Path,
        dest: &Path,
        copy_flags: &CopyFlags,
        options: &SyncOptions,
        warnings: Option<&Arc<Mutex<Vec<String>>>>,
    ) -> Result<u64> {
        let retry_config = RetryConfig::new(options.retry_count, options.retry_wait);

        if retry_config.should_retry() {
            // Use retry logic
            let description = format!("Copy {}", source.display());

            // We need to handle the logger mutex carefully
            let result = with_retry(
                || {
                    if let Some(warnings) = warnings {
                        copy_file_with_metadata_with_warnings(source, dest, copy_flags, warnings)
                    } else {
                        copy_file_with_metadata(source, dest, copy_flags)
                    }
                },
                &retry_config,
                &description,
                None, // We'll log retries separately
            );

            result.with_context(|| {
                format!(
                    "Failed to copy file after {} retries: {} -> {}",
                    retry_config.max_retries,
                    source.display(),
                    dest.display()
                )
            })
        } else {
            // No retry
            if let Some(warnings) = warnings {
                copy_file_with_metadata_with_warnings(source, dest, copy_flags, warnings)
            } else {
                copy_file_with_metadata(source, dest, copy_flags)
            }
            .with_context(|| {
                format!(
                    "Failed to copy file: {} -> {}",
                    source.display(),
                    dest.display()
                )
            })
        }
    }
}

/// Statistics for synchronization performance
#[derive(Debug, Default)]
pub struct SyncStats {
    #[allow(dead_code)]
    pub files_processed: AtomicU64,
    pub bytes_transferred: AtomicU64,
    #[allow(dead_code)]
    pub blocks_matched: AtomicU64,
    #[allow(dead_code)]
    pub elapsed_time: std::time::Duration,
    pub warnings: Arc<Mutex<Vec<String>>>,
}

impl SyncStats {
    pub fn new() -> Self {
        Self {
            files_processed: AtomicU64::new(0),
            bytes_transferred: AtomicU64::new(0),
            blocks_matched: AtomicU64::new(0),
            elapsed_time: std::time::Duration::from_secs(0),
            warnings: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn add_warning(&self, warning: String) {
        if let Ok(mut warnings) = self.warnings.lock() {
            warnings.push(warning);
        }
    }

    pub fn add_bytes_transferred(&self, bytes: u64) {
        self.bytes_transferred.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn get_bytes_transferred(&self) -> u64 {
        self.bytes_transferred.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parallel_sync_config() {
        let config = ParallelSyncConfig::default();
        assert!(config.worker_threads > 0);
        assert!(config.io_threads > 0);
        assert!(config.block_size > 0);
    }

    #[test]
    fn test_parallel_syncer_creation() {
        let config = ParallelSyncConfig::default();
        let syncer = ParallelSyncer::new(config);
        assert_eq!(
            syncer.config.worker_threads,
            std::thread::available_parallelism().unwrap().get()
        );
    }

    #[test]
    fn test_map_source_to_dest() -> Result<()> {
        let config = ParallelSyncConfig::default();
        let syncer = ParallelSyncer::new(config);

        let source_root = Path::new("/source");
        let dest_root = Path::new("/dest");
        let source_file = Path::new("/source/subdir/file.txt");

        let result = syncer.map_source_to_dest(source_file, source_root, dest_root)?;
        assert_eq!(result, Path::new("/dest/subdir/file.txt"));

        Ok(())
    }
}
