//! Multithreaded synchronization implementation

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::algorithm::{BlockChecksum, DeltaAlgorithm, Match};
use crate::compression::{CompressionType, decompress_data};
use crate::fast_file_list::{FastEnumConfig, FastFileListGenerator, compare_file_lists_fast};
use crate::file_list::{
    FileInfo, FileOperation, compare_file_lists_with_roots,
    compare_file_lists_with_roots_and_progress, generate_file_list_with_options,
    generate_file_list_with_options_and_progress,
};
// Pattern export functionality moved to separate shimmer project
use crate::color_output::ConditionalColor;
#[cfg(target_os = "linux")]
use crate::file_list::generate_file_list_parallel;
#[cfg(target_os = "linux")]
use crate::linux_fast_copy::IO_URING_BATCH_SIZE;
use crate::logging::SyncLogger;
use crate::metadata::{CopyFlags, copy_file_with_metadata_with_warnings};
use crate::mixed_strategy::MixedStrategyExecutor;
use crate::native_tools::NativeToolExecutor;
use crate::options::SyncOptions;
use crate::platform_api::PlatformCopier;
use crate::progress::SyncProgress;
use crate::strategy::{CopyStrategy, FileStats, StrategySelector};
use crate::sync_stats::SyncStats;
use crossterm::style::Color;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// Format number with thousands separator
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

/// Format bytes to human readable string
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

/// Configuration for multithreaded synchronization
#[derive(Debug, Clone)]
pub struct ParallelSyncConfig {
    /// Number of worker threads for file processing
    pub worker_threads: usize,
    /// Number of I/O threads for reading/writing
    pub io_threads: usize,
    /// Block size for delta algorithm
    pub block_size: usize,
    /// Maximum number of files to process in parallel
    pub max_parallel_files: usize,
}

impl Default for ParallelSyncConfig {
    fn default() -> Self {
        let num_cpus = std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1);
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

    /// Helper function to create progress style with fallback
    fn create_progress_style(template: &str, tick_chars: &str) -> ProgressStyle {
        ProgressStyle::default_spinner()
            .template(template)
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_chars(tick_chars)
    }

    /// Ask user for confirmation before proceeding with operations
    fn confirm_operations(&self, operations: &[FileOperation]) -> Result<bool> {
        use std::io::{self, Write};

        // Count operation types for summary
        let mut new_files = 0;
        let mut new_dirs = 0;
        let mut updates = 0;
        let mut deletions = 0;
        let mut symlinks = 0;

        for op in operations {
            match op {
                FileOperation::Create { path } => {
                    if std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false) {
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

        // Show summary
        println!("\n     Pending Operation Summary:");
        if new_files > 0 {
            println!("       New Files: {new_files}");
        }
        if new_dirs > 0 {
            println!("       New Directories: {new_dirs}");
        }
        if updates > 0 {
            println!("       Updates: {updates}");
        }
        if deletions > 0 {
            println!("       Deletions: {deletions}");
        }
        if symlinks > 0 {
            println!("       Symlinks: {symlinks}");
        }
        println!();

        // Ask for confirmation
        print!("     Continue? Y/n: ");
        if let Err(e) = io::stdout().flush() {
            return Err(anyhow::anyhow!("Failed to flush stdout: {}", e));
        }
        let mut input = String::new();
        if let Err(e) = io::stdin().read_line(&mut input) {
            return Err(anyhow::anyhow!("Failed to read user input: {}", e));
        }
        let input = input.trim().to_lowercase();

        Ok(input == "y" || input == "yes" || input.is_empty())
    }

    /// Execute mixed mode directly without file analysis
    fn execute_mixed_mode_direct(
        &self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        // Go straight to mixed mode execution
        // Show spinners during scanning, they'll be cleared before mixed strategy progress bar
        let operations =
            self.collect_operations_with_progress(&source, &destination, &options, true)?;

        // Check if confirmation is needed
        if options.confirm && !operations.is_empty()
            && !self.confirm_operations(&operations)? {
                println!("     Operation cancelled by user.");
                return Ok(SyncStats::default());
            }

        // We need file count for the executor, but we can get it from operations
        let total_files = operations.len() as u64;
        let total_bytes = operations
            .iter()
            .map(|op| match op {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
                }
                _ => 0,
            })
            .sum();

        let executor = if options.no_progress {
            MixedStrategyExecutor::new_with_no_progress()
        } else {
            MixedStrategyExecutor::new(total_files, total_bytes)
        };
        executor.execute(operations, &source, &destination, &options)
    }

    /// Synchronize using intelligent strategy selection
    pub fn synchronize_smart(
        &self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        // Mixed mode is the default - only do analysis if specific strategy is forced
        if let Some(ref forced) = options.forced_strategy {
            if forced != "mixed" {
                // User specified a specific strategy - go through analysis
                return self.synchronize_with_analysis(source, destination, options);
            }
        }

        // Default behavior: go straight to mixed mode (fastest path)
        self.execute_mixed_mode_direct(source, destination, options)
    }

    /// Synchronize with strategy analysis (for forced strategies other than mixed)
    fn synchronize_with_analysis(
        &self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        // Create a spinner for file analysis (only for diagnostic modes)
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(Self::create_progress_style(
            "  {spinner} RoboSync Diagnostic Mode: Analyzing files for strategy selection...",
            "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏",
        ));
        spinner.enable_steady_tick(std::time::Duration::from_millis(80));

        // Analyze the files to determine the best strategy (diagnostic modes only)
        let source_files = if source.is_dir() {
            #[cfg(target_os = "linux")]
            let files = if options.linux_optimized {
                generate_file_list_parallel(&source, &options)?
            } else {
                generate_file_list_with_options(&source, &options)?
            };
            #[cfg(not(target_os = "linux"))]
            let files = generate_file_list_with_options(&source, &options)?;
            files
        } else {
            // Single file
            vec![FileInfo {
                path: source.clone(),
                size: fs::metadata(&source)?.len(),
                modified: fs::metadata(&source)?.modified()?,
                is_directory: false,
                is_symlink: false,
                symlink_target: None,
                checksum: None,
            }]
        };

        // Calculate statistics
        let file_stats = FileStats::from_operations(&source_files);

        // Stop the spinner
        spinner.finish_with_message("     ✅ Analysis complete".to_string());

        println!(
            "\n     {}",
            "File Analysis Complete:".color_bold_if(Color::Cyan)
        );
        println!();
        println!(
            "     {}  {:>8} {}   ({:>8} {}, {:>7} {}, {:>3} {})",
            "Files:".color_if(Color::White),
            format_number(file_stats.total_files as u64).color_bold_if(Color::White),
            "total".color_if(Color::White),
            format_number(file_stats.small_files as u64).color_if(Color::Green),
            "small".color_if(Color::Green),
            format_number(file_stats.medium_files as u64).color_if(Color::Yellow),
            "medium".color_if(Color::Yellow),
            format_number(file_stats.large_files as u64).color_if(Color::Red),
            "large".color_if(Color::Red)
        );
        let avg_size_str = if file_stats.total_files > 0 {
            humanize_bytes(file_stats.avg_size)
        } else {
            "0 B".to_string()
        };
        println!(
            "     {}   {:>8} {}   ({} {})",
            "Size:".color_if(Color::White),
            humanize_bytes(file_stats.total_size).color_bold_if(Color::White),
            "total".color_if(Color::White),
            "average:".color_if(Color::DarkGrey),
            avg_size_str.as_str().color_if(Color::White)
        );

        // Choose strategy using intelligent heuristics
        let strategy = if let Some(ref forced) = options.forced_strategy {
            // Check if user forced a specific strategy
            let selector = StrategySelector::new();
            match forced.as_str() {
                "rsync" => {
                    println!("     Using forced strategy: rsync");
                    CopyStrategy::NativeRsync {
                        extra_args: selector.build_rsync_args(&options),
                    }
                }
                "robocopy" => {
                    println!("     Using forced strategy: robocopy");
                    CopyStrategy::NativeRobocopy {
                        extra_args: selector.build_robocopy_args(&options),
                    }
                }
                "platform" => {
                    println!("     Using forced strategy: platform API");
                    selector.platform_api_strategy()
                }
                "delta" => {
                    println!("     Using forced strategy: delta transfer");
                    CopyStrategy::DeltaTransfer {
                        block_size: selector.optimal_block_size(file_stats.avg_size),
                    }
                }
                "parallel" => {
                    println!("     Using forced strategy: parallel");
                    CopyStrategy::ParallelCustom {
                        threads: selector.optimal_thread_count(false),
                    }
                }
                #[cfg(target_os = "linux")]
                "io_uring" => {
                    println!("     Using forced strategy: io_uring");
                    CopyStrategy::IoUringBatch { batch_size: 256 }
                }
                "mixed" => {
                    println!("     Using forced strategy: mixed mode");
                    CopyStrategy::MixedMode
                }
                "concurrent" => {
                    println!("     Using forced strategy: mixed mode");
                    CopyStrategy::MixedMode
                }
                _ => {
                    eprintln!("Unknown strategy '{forced}', using automatic selection");
                    selector.choose_strategy(&file_stats, &source, &destination, &options)
                }
            }
        } else {
            let selector = StrategySelector::new();
            let chosen = selector.choose_strategy(&file_stats, &source, &destination, &options);
            println!(
                "     {} {}",
                "Automatically selected strategy:".color_if(Color::White),
                selector
                    .describe_strategy(&chosen)
                    .color_bold_if(Color::Cyan)
            );
            chosen
        };

        // Clone strategy for later use in pattern recording
        let _strategy_for_recording = strategy.clone();

        // Create unified progress manager (but not for mixed modes which have their own)
        let use_mixed_mode = matches!(strategy, CopyStrategy::MixedMode);
        let progress_manager = if options.no_progress || use_mixed_mode {
            None
        } else {
            Some(Arc::new(SyncProgress::new(
                file_stats.total_files as u64,
                file_stats.total_size,
            )))
        };

        // Execute based on strategy
        let result = match strategy {
            CopyStrategy::NativeRsync { extra_args } => {
                println!("Delegating to rsync for optimal small file performance...");

                #[cfg(unix)]
                {
                    let executor = NativeToolExecutor::new(options.dry_run);
                    executor.run_rsync(&source, &destination, extra_args, progress_manager.clone())
                }
                #[cfg(not(unix))]
                {
                    let _ = extra_args; // Unused on non-Unix
                    // Fall back to our implementation on non-Unix
                    self.synchronize_with_options(source, destination, options)
                }
            }

            CopyStrategy::NativeRobocopy { extra_args } => {
                println!("Delegating to robocopy for optimal Windows performance...");
                let executor = NativeToolExecutor::new(options.dry_run);

                #[cfg(target_os = "windows")]
                {
                    executor.run_robocopy(
                        &source,
                        &destination,
                        extra_args,
                        progress_manager.clone(),
                    )
                }
                #[cfg(not(target_os = "windows"))]
                {
                    // Fall back to our implementation on non-Windows
                    self.synchronize_with_options(source, destination, options)
                }
            }

            CopyStrategy::PlatformApi { method: _ } => {
                println!("Using platform-specific APIs for optimal performance...");
                let mut copier = PlatformCopier::new();

                // Add progress tracking if available
                if let Some(ref pm) = progress_manager {
                    copier = copier.with_progress(Arc::clone(pm).create_tracker());
                }

                // Collect file operations
                let operations = self.collect_operations(&source, &destination, &options)?;
                let file_pairs: Vec<(PathBuf, PathBuf)> = operations
                    .into_iter()
                    .filter_map(|op| match op {
                        FileOperation::Create { path } | FileOperation::Update { path, .. } => self
                            .map_source_to_dest(&path, &source, &destination)
                            .ok()
                            .map(|dest| (path, dest)),
                        _ => None,
                    })
                    .collect();

                copier.copy_files(&file_pairs)
            }

            CopyStrategy::DeltaTransfer { block_size } => {
                println!(
                    "Using delta transfer algorithm with {}KB blocks...",
                    block_size / 1024
                );
                // Use our existing implementation with delta transfer enabled
                let mut opts = options.clone();
                opts.checksum = true; // Force checksum mode for delta
                self.synchronize_with_options(source, destination, opts)
            }

            CopyStrategy::ParallelCustom { threads } => {
                println!("Using parallel transfer with {threads} threads...");
                // Use our existing parallel implementation
                let mut config = self.config.clone();
                config.worker_threads = threads;
                let syncer = ParallelSyncer::new(config);
                syncer.synchronize_with_options(source, destination, options)
            }

            #[cfg(target_os = "linux")]
            CopyStrategy::IoUringBatch { batch_size } => {
                println!(
                    "Using io_uring batch mode with batch size {batch_size}..."
                );
                // Use our Linux optimized path
                let mut opts = options.clone();
                opts.linux_optimized = true;
                self.synchronize_with_options(source, destination, opts)
            }

            CopyStrategy::MixedMode => {
                // Using mixed mode strategy

                // Collect all operations
                let operations = self.collect_operations(&source, &destination, &options)?;

                // Execute mixed strategy with simple progress
                let executor = if options.no_progress {
                    MixedStrategyExecutor::new_with_no_progress()
                } else {
                    MixedStrategyExecutor::new(
                        file_stats.total_files as u64,
                        file_stats.total_size,
                    )
                };
                executor.execute(operations, &source, &destination, &options)
            }
        };

        // Finish progress tracking
        if let Some(pm) = progress_manager {
            pm.finish();
        }

        // Pattern learning functionality moved to separate shimmer project

        result
    }

    /// Helper to collect file operations
    fn collect_operations(
        &self,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
    ) -> Result<Vec<FileOperation>> {
        self.collect_operations_with_progress(source, destination, options, true)
    }

    /// Helper to collect file operations with optional progress indicators
    fn collect_operations_with_progress(
        &self,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
        show_progress: bool,
    ) -> Result<Vec<FileOperation>> {
        // Starting fast file enumeration

        // Create fast enumeration configuration
        let enum_config = FastEnumConfig {
            scan_threads: self.config.worker_threads * 2, // More threads for I/O bound scanning
            batch_size: 5000,                             // Larger batch for better performance
            pre_scan: true,
            progress_interval: 2000, // Update every 2000 files
        };

        let generator = FastFileListGenerator::new(enum_config);

        // Create MultiProgress to manage multiple spinners properly
        let multi_progress = if show_progress && !options.no_progress {
            Some(MultiProgress::new())
        } else {
            None
        };

        // Show progress during source enumeration
        let source_files = if let Some(ref mp) = multi_progress {
            let source_pb = mp.add(ProgressBar::new_spinner());
            source_pb.set_style(Self::create_progress_style(
                "   {spinner:.green} Scanning source directory...",
                "⠁⠂⠄⡀⢀⠠⠐⠈",
            ));
            source_pb.enable_steady_tick(std::time::Duration::from_millis(100));

            // Enumerating source files
            let files = generator.generate_file_list(source, options)?;
            source_pb.finish_with_message(format!("     ✅ Found {} source files", files.len()));

            // Print newline to separate spinners
            let _ = mp.println("");

            files
        } else {
            generator.generate_file_list(source, options)?
        };

        let dest_files = if destination.exists() {
            if let Some(ref mp) = multi_progress {
                // Show progress during destination enumeration
                let dest_pb = mp.add(ProgressBar::new_spinner());
                dest_pb.set_style(Self::create_progress_style(
                    "   {spinner:.yellow} Scanning destination directory...",
                    "⠁⠂⠄⡀⢀⠠⠐⠈",
                ));
                dest_pb.enable_steady_tick(std::time::Duration::from_millis(100));

                // Enumerating destination files
                let files = generator.generate_file_list(destination, options)?;
                dest_pb.finish_with_message(format!(
                    "     ✅ Found {} destination files",
                    files.len()
                ));
                files
            } else {
                generator.generate_file_list(destination, options)?
            }
        } else {
            if let Some(ref mp) = multi_progress {
                let _ = mp.println("     📁 Destination directory doesn't exist - will be created");
            } else if show_progress {
                println!("     📁 Destination directory doesn't exist - will be created");
            }
            Vec::new()
        };

        // Show progress during comparison
        let operations = if let Some(ref mp) = multi_progress {
            let compare_pb = mp.add(ProgressBar::new_spinner());
            compare_pb.set_style(Self::create_progress_style(
                "   {spinner:.blue} Analyzing differences...",
                "⠁⠂⠄⡀⢀⠠⠐⠈",
            ));
            compare_pb.enable_steady_tick(std::time::Duration::from_millis(100));

            // Comparing file lists
            let ops = compare_file_lists_fast(
                &source_files,
                &dest_files,
                source,
                destination,
                options,
                None, // No progress for now since this is fast
            );

            compare_pb.finish_with_message(format!("     ✅ Generated {} operations", ops.len()));
            ops
        } else {
            compare_file_lists_fast(
                &source_files,
                &dest_files,
                source,
                destination,
                options,
                None,
            )
        };

        // Clear MultiProgress to ensure clean output
        if let Some(_mp) = multi_progress {
            // Don't clear, just ensure all spinners are finished
            // mp.clear().ok();
        }

        // Generated operations
        Ok(operations)
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
            let is_network = destination
                .to_str()
                .map(|s| s.starts_with("\\\\"))
                .unwrap_or(false);
            let is_mapped_drive = destination
                .to_str()
                .and_then(|s| {
                    let first_char = s.chars().next()?;
                    let second_char = s.chars().nth(1)?;
                    Some(first_char.is_ascii_alphabetic() && second_char == ':')
                })
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
                        println!("Warning: Could not write test file: {e}");
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
        let start_time = Instant::now();
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
        logger.update_progress(1, stats.bytes_transferred());

        // Use formatted display for completion
        let elapsed = start_time.elapsed();
        let throughput = if elapsed.as_secs() > 0 {
            stats.bytes_transferred() / elapsed.as_secs()
        } else {
            stats.bytes_transferred()
        };

        println!(
            "\n     ✅ Completed in {:.1}s: {} files, {} transferred ({}/s)",
            elapsed.as_secs_f32(),
            format_number(stats.files_copied()),
            humanize_bytes(stats.bytes_transferred()),
            humanize_bytes(throughput)
        );

        Ok(stats)
    }

    /// Synchronize directories using parallel processing
    fn sync_directories(
        &self,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        let start_time = Instant::now();

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
            source_pb.set_style(Self::create_progress_style(
                "{spinner:.green} Scanning source: {pos} files found...",
                "⠁⠂⠄⡀⢀⠠⠐⠈",
            ));
            source_pb.enable_steady_tick(std::time::Duration::from_millis(100));

            let source_files = generate_file_list_with_options_and_progress(
                source,
                options,
                Some(|count| {
                    source_pb.set_position(count as u64);
                }),
            )
            .context("Failed to generate source file list")?;

            source_pb.finish_with_message(format!(
                "     ✅ Found {} items in source",
                source_files.len()
            ));
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
                dest_pb.set_style(Self::create_progress_style(
                    "{spinner:.green} Scanning destination: {pos} files found...",
                    "⠁⠂⠄⡀⢀⠠⠐⠈",
                ));
                dest_pb.enable_steady_tick(std::time::Duration::from_millis(100));

                let files = generate_file_list_with_options_and_progress(
                    destination,
                    options,
                    Some(|count| {
                        dest_pb.set_position(count as u64);
                    }),
                )
                .context("Failed to generate destination file list")?;

                dest_pb.finish_with_message(format!(
                    "     ✅ Found {} items in destination",
                    files.len()
                ));
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
                let _ = mp.println("Destination does not exist, will create");
            } else {
                logger.log("Destination does not exist, will create");
            }
            Vec::new()
        };

        // Analysis phase with progress indication
        let mut operations = if !options.no_progress {
            // Create a spinner to show analysis activity
            let pb = ProgressBar::new_spinner();
            pb.set_style(Self::create_progress_style(
                "{spinner:.green} Analyzing changes... {pos} files processed",
                "⠁⠂⠄⡀⢀⠠⠐⠈",
            ));
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
            pb.finish_with_message("     ✅ Analysis complete".to_string());
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
                pb.set_style(Self::create_progress_style(
                    "{spinner:.green} Finding files to purge...",
                    "⠁⠂⠄⡀⢀⠠⠐⠈",
                ));
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
                    "     ✅ Purge analysis complete - {purge_count} files to remove"
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
            if let Err(e) = io::stdout().flush() {
                return Err(anyhow::anyhow!("Failed to flush stdout: {}", e));
            }
            let mut input = String::new();
            if let Err(e) = io::stdin().read_line(&mut input) {
                return Err(anyhow::anyhow!("Failed to read user input: {}", e));
            }
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
                        .unwrap_or_else(|_| ProgressStyle::default_bar())
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
        let effective_threads = if destination
            .to_str()
            .map(|s| s.starts_with("\\\\") || s.contains(":"))
            .unwrap_or(false)
        {
            self.config.worker_threads * 2 // Double threads for network operations
        } else {
            self.config.worker_threads
        };

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(effective_threads)
            .build()
            .context("Failed to create thread pool")?;

        println!(
            "DEBUG: Using {effective_threads} threads for parallel operations"
        );

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
                self.execute_operation(
                    operation,
                    source,
                    destination,
                    &stats,
                    options,
                    &mut logger,
                )?;
                logger.update_progress(1, 0);
                if let Some(ref progress) = progress {
                    if let Ok(mut p) = progress.lock() {
                        p.update_file_complete(0);
                    }
                }
            }
        }

        // Batch small files for efficient processing
        println!(
            "DEBUG: Starting file categorization of {} operations",
            file_ops.len()
        );
        let (small_files, large_files): (Vec<_>, Vec<_>) = file_ops
            .into_iter()
            .partition(|op| self.is_small_file_operation(op, &source_files));
        println!(
            "DEBUG: Categorized {} small files, {} large files",
            small_files.len(),
            large_files.len()
        );

        // Log file processing start
        if options.verbose >= 1 {
            logger.log(&format!(
                "Processing {} small files and {} large files...",
                small_files.len(),
                large_files.len()
            ));
        }

        // Additional debug info for network drives
        if destination
            .to_str()
            .map(|s| s.starts_with("\\\\") || s.contains(":"))
            .unwrap_or(false)
        {
            logger.log("Note: Destination appears to be a network location. Operations may be slower than local disk.");
            logger.log(&format!(
                "Using {} threads with batch size of 100 for small files",
                self.config.worker_threads
            ));
        }

        // Process files in parallel - note: logger is not thread-safe for parallel updates
        // We'll collect stats and update at the end of each operation
        let logger_arc = Arc::new(Mutex::new(logger));
        let progress_arc = progress.clone();

        // Process small files in batches
        if !small_files.is_empty() {
            let small_files_count = small_files.len();
            if options.verbose >= 1 {
                if let Ok(logger) = logger_arc.lock() {
                    logger.log(&format!(
                        "Starting parallel processing of {small_files_count} small files"
                    ));
                }
            }

            // Pre-create all necessary directories to avoid redundant checks
            println!(
                "DEBUG: Starting directory pre-creation for {small_files_count} small files"
            );
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
            println!(
                "DEBUG: Creating {} unique directories",
                dirs_to_create.len()
            );
            for dir in dirs_to_create {
                let _ = fs::create_dir_all(dir);
            }
            println!("DEBUG: Directory creation complete, starting parallel file processing");

            // Use io_uring batch copy on Linux when optimized mode is enabled
            #[cfg(target_os = "linux")]
            if options.linux_optimized {
                println!("DEBUG: Linux optimized mode enabled, checking for batch copy");
                // Import io_uring directly here to avoid module issues
                use io_uring::{IoUring, opcode, types};
                use std::os::unix::io::AsRawFd;

                // Collect source-destination pairs for batch processing
                let mut file_pairs = Vec::new();
                for operation in &small_files {
                    match operation {
                        FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                            if let Ok(dest_path) =
                                self.map_source_to_dest(path, source, destination)
                            {
                                // Check if it's a regular file (not symlink)
                                if let Ok(metadata) = fs::symlink_metadata(path) {
                                    if metadata.is_file() && !metadata.file_type().is_symlink() {
                                        file_pairs.push((path.clone(), dest_path));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                if !file_pairs.is_empty() {
                    println!(
                        "DEBUG: Using optimized batch copy for {} files",
                        file_pairs.len()
                    );

                    // Use memory-mapped files for now until io_uring is fully working
                    const SMALL_FILE_THRESHOLD: usize = 64 * 1024;

                    let mut total_bytes_copied = 0u64;

                    // Process files in batches
                    for batch in file_pairs.chunks(IO_URING_BATCH_SIZE) {
                        let mut ring = IoUring::builder()
                            .setup_sqpoll(1000) // Use kernel polling thread
                            .build(IO_URING_BATCH_SIZE as u32)?;

                        let mut file_handles = Vec::new();
                        let mut buffers: Vec<Vec<u8>> = Vec::new();

                        // Pre-allocate buffers
                        for _ in 0..batch.len() {
                            buffers.push(vec![0u8; SMALL_FILE_THRESHOLD]);
                        }

                        // Open all files and submit read operations
                        for (idx, (src, dst)) in batch.iter().enumerate() {
                            // Open source file
                            let src_file = match fs::File::open(src) {
                                Ok(f) => f,
                                Err(e) => {
                                    eprintln!("Failed to open source {src:?}: {e}");
                                    continue;
                                }
                            };

                            let metadata = match src_file.metadata() {
                                Ok(m) => m,
                                Err(e) => {
                                    eprintln!("Failed to get metadata for {src:?}: {e}");
                                    continue;
                                }
                            };

                            let file_size = metadata.len();
                            if file_size > SMALL_FILE_THRESHOLD as u64 {
                                // Fall back to regular copy for large files
                                match fs::copy(src, dst) {
                                    Ok(bytes) => total_bytes_copied += bytes,
                                    Err(e) => {
                                        eprintln!("Failed to copy large file {src:?}: {e}")
                                    }
                                }
                                continue;
                            }

                            // Open destination file
                            let dst_file = match fs::File::create(dst) {
                                Ok(f) => f,
                                Err(e) => {
                                    eprintln!("Failed to create destination {dst:?}: {e}");
                                    continue;
                                }
                            };

                            let src_fd = src_file.as_raw_fd();
                            let dst_fd = dst_file.as_raw_fd();

                            // Get a buffer for this file
                            let buffer_ptr = buffers[idx].as_mut_ptr();

                            // Submit read operation
                            let read_op =
                                opcode::Read::new(types::Fd(src_fd), buffer_ptr, file_size as u32)
                                    .offset(0)
                                    .build()
                                    .user_data(idx as u64 * 2); // Even numbers for reads

                            unsafe {
                                ring.submission()
                                    .push(&read_op)
                                    .map_err(|e| anyhow::anyhow!("Failed to submit read: {}", e))?;
                            }

                            use std::os::unix::fs::MetadataExt;
                            file_handles.push((
                                src_file,
                                dst_file,
                                file_size,
                                dst_fd,
                                buffer_ptr,
                                metadata.mode(),
                            ));
                        }

                        if file_handles.is_empty() {
                            continue;
                        }

                        // Submit the batch
                        ring.submit_and_wait(file_handles.len())
                            .map_err(|e| anyhow::anyhow!("Failed to submit batch: {}", e))?;

                        // Process completions and submit writes
                        let mut completed_reads = Vec::new();
                        for _ in 0..file_handles.len() {
                            let cqe: io_uring::cqueue::Entry =
                                ring.completion().next().expect("completion queue entry");
                            let user_data = cqe.user_data();
                            let idx = (user_data / 2) as usize;

                            if cqe.result() < 0 {
                                eprintln!("Read failed for file {}: {}", idx, cqe.result());
                                continue;
                            }

                            completed_reads.push((idx, cqe.result() as u32));
                        }

                        // Count writes to submit
                        let num_writes = completed_reads.len();

                        // Submit write operations for successful reads
                        for (idx, bytes_read) in completed_reads {
                            if let Some((_, _, _, dst_fd, buffer_ptr, _)) = file_handles.get(idx) {
                                let write_op =
                                    opcode::Write::new(types::Fd(*dst_fd), *buffer_ptr, bytes_read)
                                        .offset(0)
                                        .build()
                                        .user_data(idx as u64 * 2 + 1); // Odd numbers for writes

                                unsafe {
                                    ring.submission().push(&write_op).map_err(|e| {
                                        anyhow::anyhow!("Failed to submit write: {}", e)
                                    })?;
                                }
                            }
                        }

                        // Submit the writes
                        ring.submit_and_wait(num_writes)
                            .map_err(|e| anyhow::anyhow!("Failed to submit writes: {}", e))?;

                        // Process write completions
                        for _ in 0..num_writes {
                            let cqe: io_uring::cqueue::Entry =
                                ring.completion().next().expect("completion queue entry");
                            let idx = ((cqe.user_data() - 1) / 2) as usize;

                            if cqe.result() < 0 {
                                eprintln!("Write failed for file {}: {}", idx, cqe.result());
                                continue;
                            }

                            total_bytes_copied += cqe.result() as u64;

                            // Set permissions on destination file
                            if let Some((_, dst_file, _, _, _, mode)) = file_handles.get(idx) {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = dst_file
                                    .set_permissions(std::fs::Permissions::from_mode(*mode));
                            }
                        }
                    }

                    stats.add_bytes_transferred(total_bytes_copied);

                    // Update progress for all files at once
                    if let Ok(mut log) = logger_arc.lock() {
                        log.update_progress(file_pairs.len() as u64, total_bytes_copied);
                    }
                }

                // Handle any remaining special files (symlinks, etc) the standard way
                let special_files: Vec<_> = small_files
                    .into_iter()
                    .filter(|op| match op {
                        FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                            if let Ok(metadata) = fs::symlink_metadata(path) {
                                !metadata.is_file() || metadata.file_type().is_symlink()
                            } else {
                                true
                            }
                        }
                        _ => true,
                    })
                    .collect();

                if !special_files.is_empty() {
                    let verbose = options.verbose;
                    pool.install(|| {
                        use rayon::prelude::*;
                        special_files
                            .into_par_iter()
                            .try_for_each(|operation| -> Result<()> {
                                match operation {
                                    FileOperation::Create { path }
                                    | FileOperation::Update { path, .. } => {
                                        let dest_path =
                                            self.map_source_to_dest(&path, source, destination)?;

                                        if let Ok(metadata) = fs::symlink_metadata(&path) {
                                            let file_type = metadata.file_type();

                                            // Handle symlinks specially
                                            if file_type.is_symlink() {
                                                if verbose >= 1 {
                                                    println!(
                                                        "Handling symlink: {}",
                                                        path.display()
                                                    );
                                                }

                                                // Copy the symlink itself, not what it points to
                                                if let Ok(_target) = fs::read_link(&path) {
                                                    // Remove destination if it exists
                                                    let _ = fs::remove_file(&dest_path);

                                                    #[cfg(unix)]
                                                    {
                                                        use std::os::unix::fs::symlink;
                                                        symlink(&_target, &dest_path)?;
                                                    }
                                                    #[cfg(windows)]
                                                    {
                                                        // On Windows, just skip symlinks for now
                                                        if verbose >= 1 {
                                                            println!(
                                                                "Skipping symlink on Windows: {}",
                                                                path.display()
                                                            );
                                                        }
                                                    }
                                                }
                                                return Ok(());
                                            }
                                        }

                                        // Just copy any other special files normally
                                        if verbose >= 2 {
                                            println!(
                                                "Copying special file: {} -> {}",
                                                path.display(),
                                                dest_path.display()
                                            );
                                        }

                                        let bytes_copied = fs::copy(&path, &dest_path)?;
                                        stats.add_bytes_transferred(bytes_copied);
                                    }
                                    _ => {}
                                }
                                Ok(())
                            })
                    })?;
                }
            } else {
                // Non-Linux or non-optimized path: use standard parallel processing
                let verbose = options.verbose;
                let file_counter = std::sync::atomic::AtomicU64::new(0);

                pool.install(|| {
                    use rayon::prelude::*;
                    small_files
                        .into_par_iter()
                        .try_for_each(|operation| -> Result<()> {
                            match operation {
                                FileOperation::Create { path }
                                | FileOperation::Update { path, .. } => {
                                    let current_file = file_counter
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    let start_time = std::time::Instant::now();
                                    if current_file % 100 == 0 || current_file > 3560 {
                                        println!(
                                            "Processing file #{}: {}",
                                            current_file,
                                            path.display()
                                        );
                                    }
                                    // Use symlink_metadata to check without following symlinks
                                    if let Ok(metadata) = fs::symlink_metadata(&path) {
                                        let file_type = metadata.file_type();

                                        // Handle symlinks specially
                                        if file_type.is_symlink() {
                                            if verbose >= 1 {
                                                println!("Handling symlink: {}", path.display());
                                            }
                                            let dest_path = self.map_source_to_dest(
                                                &path,
                                                source,
                                                destination,
                                            )?;

                                            // Copy the symlink itself, not what it points to
                                            if let Ok(_target) = fs::read_link(&path) {
                                                // Remove destination if it exists
                                                let _ = fs::remove_file(&dest_path);

                                                #[cfg(unix)]
                                                {
                                                    use std::os::unix::fs::symlink;
                                                    symlink(&_target, &dest_path)?;
                                                }
                                                #[cfg(windows)]
                                                {
                                                    // On Windows, just skip symlinks for now
                                                    if verbose >= 1 {
                                                        println!(
                                                            "Skipping symlink on Windows: {}",
                                                            path.display()
                                                        );
                                                    }
                                                }
                                            }
                                            return Ok(());
                                        }

                                        // Skip other special files
                                        if !file_type.is_file() && !file_type.is_dir() {
                                            if verbose >= 1 {
                                                println!(
                                                    "Skipping special file: {}",
                                                    path.display()
                                                );
                                            }
                                            return Ok(());
                                        }
                                    }

                                    // Regular file copy
                                    let dest_path =
                                        self.map_source_to_dest(&path, source, destination)?;

                                    // Debug: Log files being processed in verbose mode
                                    if verbose >= 2 {
                                        println!(
                                            "Copying: {} -> {}",
                                            path.display(),
                                            dest_path.display()
                                        );
                                    }

                                    let bytes_copied = fs::copy(&path, &dest_path)?;
                                    stats.add_bytes_transferred(bytes_copied);

                                    // Log slow files
                                    let elapsed = start_time.elapsed();
                                    if elapsed.as_secs() > 1 {
                                        println!(
                                            "SLOW FILE #{}: {} took {:.2}s ({} bytes)",
                                            current_file,
                                            path.display(),
                                            elapsed.as_secs_f64(),
                                            bytes_copied
                                        );
                                    }

                                    // Temporarily disable progress updates for small files to avoid mutex contention
                                    // TODO: Implement lock-free progress tracking
                                }
                                _ => {}
                            }
                            Ok(())
                        })
                })?;
            }

            #[cfg(not(target_os = "linux"))]
            {
                // Non-Linux path: use standard parallel processing
                let verbose = options.verbose;
                let file_counter = std::sync::atomic::AtomicU64::new(0);

                pool.install(|| {
                    use rayon::prelude::*;
                    small_files
                        .into_par_iter()
                        .try_for_each(|operation| -> Result<()> {
                            match operation {
                                FileOperation::Create { path }
                                | FileOperation::Update { path, .. } => {
                                    let current_file = file_counter
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    let start_time = std::time::Instant::now();
                                    if current_file % 100 == 0 || current_file > 3560 {
                                        println!(
                                            "Processing file #{}: {}",
                                            current_file,
                                            path.display()
                                        );
                                    }
                                    // Use symlink_metadata to check without following symlinks
                                    if let Ok(metadata) = fs::symlink_metadata(&path) {
                                        let file_type = metadata.file_type();

                                        // Handle symlinks specially
                                        if file_type.is_symlink() {
                                            if verbose >= 1 {
                                                println!("Handling symlink: {}", path.display());
                                            }
                                            let dest_path = self.map_source_to_dest(
                                                &path,
                                                source,
                                                destination,
                                            )?;

                                            // Copy the symlink itself, not what it points to
                                            if let Ok(_target) = fs::read_link(&path) {
                                                // Remove destination if it exists
                                                let _ = fs::remove_file(&dest_path);

                                                #[cfg(unix)]
                                                {
                                                    use std::os::unix::fs::symlink;
                                                    symlink(&_target, &dest_path)?;
                                                }
                                                #[cfg(windows)]
                                                {
                                                    // On Windows, just skip symlinks for now
                                                    if verbose >= 1 {
                                                        println!(
                                                            "Skipping symlink on Windows: {}",
                                                            path.display()
                                                        );
                                                    }
                                                }
                                            }
                                            return Ok(());
                                        }

                                        // Skip other special files
                                        if !file_type.is_file() && !file_type.is_dir() {
                                            if verbose >= 1 {
                                                println!(
                                                    "Skipping special file: {}",
                                                    path.display()
                                                );
                                            }
                                            return Ok(());
                                        }
                                    }

                                    // Regular file copy
                                    let dest_path =
                                        self.map_source_to_dest(&path, source, destination)?;

                                    // Debug: Log files being processed in verbose mode
                                    if verbose >= 2 {
                                        println!(
                                            "Copying: {} -> {}",
                                            path.display(),
                                            dest_path.display()
                                        );
                                    }

                                    let bytes_copied = fs::copy(&path, &dest_path)?;
                                    stats.add_bytes_transferred(bytes_copied);

                                    // Log slow files
                                    let elapsed = start_time.elapsed();
                                    if elapsed.as_secs() > 1 {
                                        println!(
                                            "SLOW FILE #{}: {} took {:.2}s ({} bytes)",
                                            current_file,
                                            path.display(),
                                            elapsed.as_secs_f64(),
                                            bytes_copied
                                        );
                                    }

                                    // Temporarily disable progress updates for small files to avoid mutex contention
                                    // TODO: Implement lock-free progress tracking
                                }
                                _ => {}
                            }
                            Ok(())
                        })
                })?;
            }

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
                        let _progress_ref = progress_arc.clone();
                        let _file_stats = self.execute_operation_parallel(
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
            .map_err(|e| anyhow::anyhow!("Failed to lock logger mutex: {:?}", e))?;

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

        let final_stats = Arc::try_unwrap(stats).unwrap_or_else(|arc| (*arc).clone());

        // Use formatted display for completion
        let elapsed = start_time.elapsed();
        let throughput = if elapsed.as_secs() > 0 {
            final_stats.bytes_transferred() / elapsed.as_secs()
        } else {
            final_stats.bytes_transferred()
        };

        if final_stats.files_deleted() > 0 {
            println!(
                "\n     ✅ Completed in {:.1}s: {} files copied, {} deleted, {} transferred ({}/s)",
                elapsed.as_secs_f32(),
                format_number(final_stats.files_copied()),
                format_number(final_stats.files_deleted()),
                humanize_bytes(final_stats.bytes_transferred()),
                humanize_bytes(throughput)
            );
        } else {
            println!(
                "\n     ✅ Completed in {:.1}s: {} files copied, {} transferred ({}/s)",
                elapsed.as_secs_f32(),
                format_number(final_stats.files_copied()),
                humanize_bytes(final_stats.bytes_transferred()),
                humanize_bytes(throughput)
            );
        }

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
                let file_metadata = fs::metadata(&path).with_context(|| {
                    format!("Failed to read source file metadata: {}", path.display())
                })?;
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
                    fs::create_dir_all(parent).with_context(|| {
                        format!("Failed to create parent directory: {}", parent.display())
                    })?;
                }

                // Parse copy flags and copy file with metadata
                let copy_flags = CopyFlags::from_string(&options.copy_flags);
                let bytes_copied = copy_file_with_metadata_with_warnings(
                    &path,
                    &dest_path,
                    &copy_flags,
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
                // TODO: Re-enable delta sync when performance is optimized
                // if use_delta {
                //     let file_stats = self.sync_file_pair(&path, &dest_path, options)?;
                //     stats.add_bytes_transferred(file_stats.bytes_transferred());
                //     Ok(file_stats)
                // } else {
                    // Parse copy flags and copy file with metadata
                    let copy_flags = CopyFlags::from_string(&options.copy_flags);
                    let bytes_copied = copy_file_with_metadata_with_warnings(
                        &path,
                        &dest_path,
                        &copy_flags,
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
                let bytes_copied = fs::copy(&path, &dest_path).with_context(|| {
                    format!(
                        "Failed to copy: {} -> {}",
                        path.display(),
                        dest_path.display()
                    )
                })?;

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
                let bytes_copied = fs::copy(&path, &dest_path).with_context(|| {
                    format!(
                        "Failed to update: {} -> {}",
                        path.display(),
                        dest_path.display()
                    )
                })?;

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
            std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(1)
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
