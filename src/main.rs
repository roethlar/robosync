use anyhow::Result;
use clap::{Arg, Command};
use std::path::{Path, PathBuf};

use robosync::compression::CompressionConfig;
use robosync::options::SyncOptions;
use robosync::parallel_sync::{ParallelSyncConfig, ParallelSyncer};
use robosync::sync;

/// Get the maximum safe thread count based on OS file handle limits
fn get_max_thread_count() -> usize {
    #[cfg(target_os = "macos")]
    {
        // macOS has more restrictive limits, typically 256-1024 file descriptors
        // We use 64 as a safe default that works reliably
        64
    }
    #[cfg(target_os = "windows")]
    {
        // Windows can handle more threads, especially for file copying
        // Modern Windows systems have much higher handle limits
        256
    }
    #[cfg(target_os = "linux")]
    {
        // Linux can handle more, check the actual limit
        use std::process::Command;

        // Try to get the soft limit for file descriptors
        if let Ok(output) = Command::new("sh").arg("-c").arg("ulimit -n").output() {
            if let Ok(limit_str) = String::from_utf8(output.stdout) {
                if let Ok(limit) = limit_str.trim().parse::<usize>() {
                    // Use 1/4 of the file descriptor limit as a safe maximum
                    // This leaves room for other file operations
                    return (limit / 4).clamp(64, 512);
                }
            }
        }
        // Default to 256 if we can't determine the limit
        256
    }
    #[cfg(target_os = "freebsd")]
    {
        // FreeBSD typically has good file descriptor limits
        // Check the actual limit similar to Linux
        use std::process::Command;

        if let Ok(output) = Command::new("sh").arg("-c").arg("ulimit -n").output() {
            if let Ok(limit_str) = String::from_utf8(output.stdout) {
                if let Ok(limit) = limit_str.trim().parse::<usize>() {
                    return (limit / 4).clamp(64, 256);
                }
            }
        }
        128
    }
    #[cfg(target_os = "openbsd")]
    {
        // OpenBSD has more conservative limits by default
        64
    }
    #[cfg(target_os = "netbsd")]
    {
        // NetBSD similar to FreeBSD
        128
    }
    #[cfg(target_os = "dragonfly")]
    {
        // DragonFly BSD
        128
    }
    #[cfg(target_os = "solaris")]
    {
        // Solaris/illumos typically have good limits
        256
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        target_os = "solaris"
    )))]
    {
        // Conservative default for other systems
        64
    }
}

fn main() -> Result<()> {
    let matches = Command::new("RoboSync")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Fast, parallel file synchronization with delta-transfer algorithm")
        .arg(
            Arg::new("source")
                .help("Source directory or file")
                .required_unless_present_any(["shimmer-status", "pattern-stats", "test-shimmer-model"])
                .value_parser(clap::value_parser!(PathBuf))
        )
        .arg(
            Arg::new("destination")
                .help("Destination directory or file")
                .required_unless_present_any(["shimmer-status", "pattern-stats", "test-shimmer-model", "export-patterns"])
                .value_parser(clap::value_parser!(PathBuf))
        )

        // Core copy options (RoboCopy style)
        .arg(
            Arg::new("subdirs")
                .short('s')
                .short_alias('S')
                .help("Copy subdirectories, but not empty ones")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("empty-dirs")
                .short('e')
                .short_alias('E')
                .help("Copy subdirectories, including empty ones")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("mirror")
                .long("mir")
                .help("Mirror a directory tree (equivalent to -e plus --purge)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("purge")
                .long("purge")
                .help("Delete dest files/dirs that no longer exist in source")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("list-only")
                .short('l')
                .short_alias('L')
                .help("List only - don't copy, timestamp or delete any files")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("move-files")
                .long("mov")
                .help("Move files (delete source after successful copy). WARNING: If sync is interrupted and restarted, already moved files will be lost!")
                .action(clap::ArgAction::SetTrue)
        )

        // File filtering options
        .arg(
            Arg::new("exclude-files")
                .long("xf")
                .value_name("PATTERN")
                .help("Exclude files matching given patterns")
                .action(clap::ArgAction::Append)
        )
        .arg(
            Arg::new("exclude-dirs")
                .long("xd")
                .value_name("PATTERN")
                .help("Exclude directories matching given patterns")
                .action(clap::ArgAction::Append)
        )
        .arg(
            Arg::new("min-size")
                .long("min")
                .value_name("SIZE")
                .help("Minimum file size - exclude files smaller than SIZE bytes")
                .value_parser(clap::value_parser!(u64))
        )
        .arg(
            Arg::new("max-size")
                .long("max")
                .value_name("SIZE")
                .help("Maximum file size - exclude files bigger than SIZE bytes")
                .value_parser(clap::value_parser!(u64))
        )

        // Copy flags
        .arg(
            Arg::new("copy-flags")
                .long("copy")
                .value_name("FLAGS")
                .help("What to copy: D=Data, A=Attributes, T=Timestamps, S=Security, O=Owner, U=aUditing (default: DAT)")
                .default_value("DAT")
        )
        .arg(
            Arg::new("copy-all")
                .long("copyall")
                .help("Copy all file info including security/ownership (equivalent to --copy DATSOU)")
                .action(clap::ArgAction::SetTrue)
        )

        // Logging and verbosity
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .alias("Verbose")
                .alias("VERBOSE")
                .help("Produce verbose output (-v shows operations preview, -vv shows all operations)")
                .action(clap::ArgAction::Count)
        )
        .arg(
            Arg::new("confirm")
                .long("confirm")
                .help("Prompt for confirmation before executing operations")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("no-progress")
                .long("np")
                .help("No progress - don't display percentage copied")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("log-file")
                .long("log")
                .value_name("FILE")
                .help("Output status to log file (overwrite existing)")
        )
        .arg(
            Arg::new("eta")
                .long("eta")
                .help("Show estimated time of arrival and progress updates")
                .action(clap::ArgAction::SetTrue)
        )

        // Retry options
        .arg(
            Arg::new("retry-count")
                .short('r')
                .short_alias('R')
                .long("retry")
                .value_name("NUM")
                .help("Number of retries on failed copies (default: 0)")
                .default_value("0")
                .value_parser(clap::value_parser!(u32))
        )
        .arg(
            Arg::new("retry-wait")
                .short('w')
                .short_alias('W')
                .long("wait")
                .value_name("SECONDS")
                .help("Wait time between retries in seconds (default: 30)")
                .default_value("30")
                .value_parser(clap::value_parser!(u32))
        )

        // Performance options
        .arg(
            Arg::new("threads")
                .long("mt")
                .value_name("NUM")
                .help("Do multi-threaded copies with NUM threads (default: CPU cores)")
                .value_parser(clap::value_parser!(usize))
        )
        .arg(
            Arg::new("block-size")
                .short('b')
                .short_alias('B')
                .long("block-size")
                .alias("blocksize")
                .value_name("SIZE")
                .help("Block size for delta algorithm in bytes (default: 1024). Smaller blocks find more matches but use more CPU/memory. Larger blocks are faster but may transfer more data.")
                .value_parser(clap::value_parser!(usize))
        )
        .arg(
            Arg::new("sequential")
                .long("sequential")
                .help("Force sequential processing (disables parallelism)")
                .action(clap::ArgAction::SetTrue)
        )

        // Legacy rsync options
        .arg(
            Arg::new("archive")
                .short('a')
                .short_alias('A')
                .long("archive")
                .help("Archive mode - preserve everything (permissions, timestamps, ownership)")
                .action(clap::ArgAction::SetTrue)
        )
        // Note: -r is already used for retry-count, using long form only
        .arg(
            Arg::new("recursive")
                .long("recursive")
                .help("Recurse into directories")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("compress")
                .short('z')
                .long("compress")
                .help("Compress file data during transfer")
                .action(clap::ArgAction::SetTrue)
        );
        
        #[cfg(target_os = "linux")]
        let matches = matches.arg(
            Arg::new("linux-optimized")
                .long("linux-optimized")
                .help("Enable Linux-specific optimizations for small files")
                .action(clap::ArgAction::SetTrue)
        );
        
        #[cfg(not(target_os = "linux"))]
        let matches = matches;
        
        let matches = matches.arg(
            Arg::new("smart")
                .long("smart")
                .help("Use intelligent strategy selection to choose the optimal copy method")
                .action(clap::ArgAction::SetTrue)
        );
        
        let matches = matches.arg(
            Arg::new("strategy")
                .long("strategy")
                .value_name("METHOD")
                .help("Force a specific copy strategy: rsync, robocopy, platform, delta, parallel, io_uring, mixed, concurrent")
                .value_parser(["rsync", "robocopy", "platform", "delta", "parallel", "io_uring", "mixed", "concurrent"])
        );
        
        let matches = matches.arg(
            Arg::new("dry-run")
                .short('n')
                .short_alias('N')
                .long("dry-run")
                .alias("dryrun")
                .help("Show what would be done without actually doing it")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("checksum")
                .short('c')
                .long("checksum")
                .alias("Checksum")
                .alias("CHECKSUM")
                .help("Skip based on checksum, not mod-time & size")
                .action(clap::ArgAction::SetTrue)
        )
        
        // Shimmer AI integration options
        .arg(
            Arg::new("export-patterns")
                .long("export-patterns")
                .help("Export file patterns for Shimmer AI training")
                .value_name("DIR")
                .value_parser(clap::value_parser!(PathBuf))
        )
        .arg(
            Arg::new("shimmer-model")
                .long("shimmer-model")
                .help("Use Shimmer AI model for strategy selection")
                .value_name("MODEL_PATH")
                .value_parser(clap::value_parser!(PathBuf))
        )
        .arg(
            Arg::new("test-shimmer-model")
                .long("test-shimmer-model")
                .help("Test Shimmer model predictions")
                .value_name("TEST_DATA")
                .value_parser(clap::value_parser!(PathBuf))
        )
        .arg(
            Arg::new("shimmer-status")
                .long("shimmer-status")
                .help("Show Shimmer integration status")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("pattern-stats")
                .long("pattern-stats")
                .help("Show pattern collection statistics")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("privacy-filter")
                .long("privacy-filter")
                .help("Enable privacy filtering for pattern export")
                .action(clap::ArgAction::SetTrue)
        )
        .get_matches();

    // Handle Shimmer integration commands first
    if matches.get_flag("shimmer-status") {
        return handle_shimmer_status();
    }
    
    if matches.get_flag("pattern-stats") {
        return handle_pattern_stats();
    }
    
    if let Some(export_dir) = matches.get_one::<PathBuf>("export-patterns") {
        let source = matches.get_one::<PathBuf>("source")
            .ok_or_else(|| anyhow::anyhow!("Source path required for pattern export"))?;
        let privacy_filter = matches.get_flag("privacy-filter");
        return handle_pattern_export(source, export_dir, privacy_filter);
    }
    
    if let Some(test_data) = matches.get_one::<PathBuf>("test-shimmer-model") {
        return handle_shimmer_test(test_data);
    }

    // For regular sync operations, source and destination are required
    let source: PathBuf = matches.get_one::<PathBuf>("source")
        .ok_or_else(|| anyhow::anyhow!("Source path required"))?
        .clone();
    let destination: PathBuf = matches.get_one::<PathBuf>("destination")
        .ok_or_else(|| anyhow::anyhow!("Destination path required"))?
        .clone();

    // Parse options
    let compress = matches.get_flag("compress");
    let sequential = matches.get_flag("sequential");
    let parallel = !sequential;
    let verbose = matches.get_count("verbose");
    let confirm = matches.get_flag("confirm");
    let dry_run = matches.get_flag("dry-run") || matches.get_flag("list-only");
    let no_progress = matches.get_flag("no-progress");
    let move_files = matches.get_flag("move-files");
    let checksum = matches.get_flag("checksum");
    let smart_mode = matches.get_flag("smart");
    let forced_strategy = matches.get_one::<String>("strategy").cloned();
    let has_forced_strategy = forced_strategy.is_some();
    let shimmer_model_path = matches.get_one::<PathBuf>("shimmer-model").cloned();
    #[cfg(target_os = "linux")]
    let linux_optimized = matches.get_flag("linux-optimized");
    #[cfg(not(target_os = "linux"))]
    let linux_optimized = false;

    // Copy options
    let subdirs = matches.get_flag("subdirs");
    let empty_dirs = matches.get_flag("empty-dirs");
    let mirror = matches.get_flag("mirror");
    let purge = matches.get_flag("purge") || mirror;
    let archive = matches.get_flag("archive");
    // Mirror mode implies empty directories (/E), which implies subdirectories (/S)
    // For directory operations, recursive is always enabled by default
    let recursive = source.is_dir()
        || matches.get_flag("recursive")
        || archive
        || subdirs
        || empty_dirs
        || mirror;

    // File filtering
    let exclude_files: Vec<String> = matches
        .get_many::<String>("exclude-files")
        .unwrap_or_default()
        .cloned()
        .collect();
    let exclude_dirs: Vec<String> = matches
        .get_many::<String>("exclude-dirs")
        .unwrap_or_default()
        .cloned()
        .collect();
    let min_size = matches.get_one::<u64>("min-size").copied();
    let max_size = matches.get_one::<u64>("max-size").copied();

    // Copy flags
    let copy_flags = matches.get_one::<String>("copy-flags").unwrap();
    let copy_all = matches.get_flag("copy-all");

    // Performance
    let num_cpus = std::thread::available_parallelism().unwrap().get();
    let threads = matches
        .get_one::<usize>("threads")
        .copied()
        .unwrap_or(num_cpus);
    let block_size = matches
        .get_one::<usize>("block-size")
        .copied()
        .unwrap_or(1024);

    // Validate thread count based on OS limits
    let max_threads = get_max_thread_count();
    if threads > max_threads {
        eprintln!(
            "Error: Maximum thread count is {max_threads} to avoid system file handle limits."
        );
        eprintln!("Requested: {threads}, Maximum allowed: {max_threads}");
        std::process::exit(1);
    }

    // Logging
    let log_file = matches.get_one::<String>("log-file");
    let show_eta = matches.get_flag("eta");

    // Retry options
    let retry_count = matches.get_one::<u32>("retry-count").copied().unwrap_or(0);
    let retry_wait = matches.get_one::<u32>("retry-wait").copied().unwrap_or(30);

    println!(
        "RoboSync v{}: Fast parallel file synchronization",
        env!("CARGO_PKG_VERSION")
    );
    println!("Source: {}", source.display());
    println!("Destination: {}", destination.display());

    if dry_run {
        println!("Mode: DRY RUN (list only)");
    } else if smart_mode {
        println!("Mode: SMART (intelligent strategy selection)");
    } else {
        println!("Mode: {}", if parallel { "parallel" } else { "sequential" });
    }

    if parallel && !dry_run {
        println!("Threads: {threads} (max allowed: {max_threads})");
        println!("Block size: {block_size} bytes");
    }

    // Show active options
    let mut options = Vec::new();
    if recursive {
        options.push("recursive");
    }
    if purge {
        options.push("purge");
    }
    if mirror {
        options.push("mirror");
    }
    let verbose_str = format!("verbose={verbose}");
    if verbose > 0 {
        options.push(&verbose_str);
    }
    if confirm {
        options.push("confirm");
    }
    if compress {
        options.push("compress");
    }
    if move_files {
        options.push("move-files");
    }
    if !exclude_files.is_empty() {
        options.push("exclude-files");
    }
    if !exclude_dirs.is_empty() {
        options.push("exclude-dirs");
    }
    if min_size.is_some() {
        options.push("min-size");
    }
    if max_size.is_some() {
        options.push("max-size");
    }

    if !options.is_empty() {
        println!("Options: {}", options.join(", "));
    }

    if copy_all || archive {
        println!("Copy flags: DATSOU (all)");
    } else {
        println!("Copy flags: {copy_flags}");
    }

    if retry_count > 0 {
        println!("Retry: {retry_count} retries, {retry_wait} seconds wait");
    }

    // Warn about dangerous combinations
    if move_files && mirror {
        eprintln!("\nWARNING: Using --mov with --mir is dangerous!");
        eprintln!("If the sync is interrupted, source files already moved will be lost.");
        eprintln!("Consider using --mir without --mov for safety.\n");
    }

    // Create sync options struct
    let sync_options = SyncOptions {
        recursive,
        purge,
        mirror,
        dry_run,
        verbose,
        confirm,
        no_progress,
        move_files,
        exclude_files,
        exclude_dirs,
        min_size,
        max_size,
        copy_flags: if copy_all || archive {
            "DATSOU".to_string()
        } else {
            copy_flags.clone()
        },
        log_file: log_file.cloned(),
        compress,
        compression_config: if compress {
            CompressionConfig::balanced()
        } else {
            CompressionConfig::default()
        },
        show_eta,
        retry_count,
        retry_wait,
        checksum,
        #[cfg(target_os = "linux")]
        linux_optimized,
        forced_strategy,
        shimmer_model_path,
    };

    if smart_mode || has_forced_strategy {
        // Use intelligent strategy selection or forced strategy
        let config = ParallelSyncConfig {
            worker_threads: threads,
            io_threads: threads,
            block_size,
            max_parallel_files: threads * 2,
        };

        let syncer = ParallelSyncer::new(config);
        let _stats = syncer.synchronize_smart(source, destination, sync_options)?;
    } else if parallel && !dry_run {
        // Use new parallel synchronization engine
        let config = ParallelSyncConfig {
            worker_threads: threads,
            io_threads: threads, // Same as worker threads, like RoboCopy
            block_size,
            max_parallel_files: threads * 2,
        };

        let syncer = ParallelSyncer::new(config);
        let _stats = syncer.synchronize_with_options(source, destination, sync_options)?;
    } else {
        // Fall back to sequential synchronization or dry run
        sync::synchronize_with_options(source, destination, threads, sync_options)?;
    }

    Ok(())
}

// Shimmer integration handlers
fn handle_shimmer_status() -> Result<()> {
    use robosync::shimmer_integration::PatternExporter;
    use robosync::shared_paths;
    use std::fs;
    
    println!("=== Shimmer Integration Status ===\n");
    
    // Check if shared directory exists
    let shared_dir = Path::new(shared_paths::SHARED_BASE);
    if shared_dir.exists() {
        println!("✓ Shared directory exists");
        
        // Check for pattern exports
        let patterns_dir = shared_paths::patterns_dir();
        if patterns_dir.exists() {
            let pattern_count = fs::read_dir(&patterns_dir)?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "json"))
                .count();
            println!("✓ Pattern exports: {} files", pattern_count);
        } else {
            println!("✗ No pattern exports found");
        }
        
        // Check for models
        let models_dir = shared_paths::models_dir();
        if models_dir.exists() {
            let model_files: Vec<_> = fs::read_dir(&models_dir)?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "json"))
                .collect();
            
            if model_files.is_empty() {
                println!("✗ No Shimmer models found");
            } else {
                println!("✓ Shimmer models found:");
                for entry in model_files {
                    println!("  - {}", entry.file_name().to_string_lossy());
                }
            }
        } else {
            println!("✗ Models directory not found");
        }
    } else {
        println!("✗ Shared directory not found - run pattern export first");
    }
    
    // Check project sync status
    let sync_file = shared_paths::project_sync_file();
    if sync_file.exists() {
        println!("\n✓ Project sync file found");
        // Could parse and display more details here
    } else {
        println!("\n✗ Project sync file not found");
    }
    
    println!("\nTo export patterns: robosync <source> --export-patterns {}", shared_paths::patterns_dir().display());
    println!("To use Shimmer model: robosync <source> <dest> --shimmer-model {}", shared_paths::default_shimmer_model().display());
    
    Ok(())
}

fn handle_pattern_stats() -> Result<()> {
    use robosync::shared_paths;
    use std::fs;
    
    println!("=== Pattern Collection Statistics ===\n");
    
    let patterns_dir = shared_paths::patterns_dir();
    if !patterns_dir.exists() {
        println!("No patterns collected yet.");
        println!("Run: robosync <source> --export-patterns {}", shared_paths::patterns_dir().display());
        return Ok(());
    }
    
    let mut total_patterns = 0;
    let mut file_count = 0;
    let mut strategy_counts = std::collections::HashMap::new();
    
    for entry in fs::read_dir(patterns_dir)? {
        let entry = entry?;
        if entry.path().extension().map_or(false, |ext| ext == "json") {
            file_count += 1;
            
            // Parse the file to get statistics
            if let Ok(content) = fs::read_to_string(entry.path()) {
                if let Ok(export) = serde_json::from_str::<robosync::shimmer_integration::PatternExport>(&content) {
                    total_patterns += export.patterns.len();
                    
                    for pattern in &export.patterns {
                        *strategy_counts.entry(pattern.strategy.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    
    println!("Export files: {}", file_count);
    println!("Total patterns: {}", total_patterns);
    
    if !strategy_counts.is_empty() {
        println!("\nStrategy distribution:");
        for (strategy, count) in strategy_counts {
            println!("  {}: {} ({:.1}%)", 
                strategy, 
                count, 
                (count as f64 / total_patterns as f64) * 100.0
            );
        }
    }
    
    Ok(())
}

fn handle_pattern_export(source: &Path, export_dir: &Path, privacy_filter: bool) -> Result<()> {
    use robosync::shimmer_integration::PatternExporter;
    use robosync::file_list::generate_file_list_with_options;
    use robosync::strategy::{FileStats, StrategySelector};
    use robosync::options::SyncOptions;
    
    println!("=== Pattern Export for Shimmer Training ===\n");
    println!("Source: {}", source.display());
    println!("Export directory: {}", export_dir.display());
    if privacy_filter {
        println!("Privacy filter: ENABLED");
    }
    
    // Create exporter
    let mut exporter = PatternExporter::new(export_dir.to_path_buf())?;
    
    // Analyze source directory
    let options = SyncOptions::default();
    let files = generate_file_list_with_options(source, &options)?;
    
    // Calculate file statistics
    let mut stats = FileStats::default();
    stats.total_files = files.len();
    
    for file in &files {
        stats.total_size += file.size;
        if file.size <= 256 * 1024 {
            stats.small_files += 1;
        } else if file.size <= 10 * 1024 * 1024 {
            stats.medium_files += 1;
        } else {
            stats.large_files += 1;
        }
    }
    
    stats.avg_size = if stats.total_files > 0 {
        stats.total_size / stats.total_files as u64
    } else {
        0
    };
    
    // Create pattern
    let is_network = robosync::strategy::is_network_path(source);
    let pattern = robosync::shimmer_integration::FilePattern::from_stats(&stats, is_network);
    
    // Select strategy (for recording purposes)
    let selector = StrategySelector::new();
    let strategy = selector.choose_strategy(&stats, source, source, &options);
    
    // Record the pattern
    let dummy_stats = robosync::sync_stats::SyncStats::default();
    let duration = std::time::Duration::from_secs(0);
    exporter.record_sync(pattern, &strategy, &dummy_stats, duration)?;
    
    // Export immediately
    let export_path = exporter.export()?;
    
    println!("\n✓ Pattern exported successfully");
    println!("Export file: {}", export_path.display());
    println!("Total files analyzed: {}", stats.total_files);
    println!("Strategy selected: {:?}", strategy);
    
    // Update project sync status
    update_project_sync_status("pattern_export", &export_path)?;
    
    Ok(())
}

fn handle_shimmer_test(test_data: &Path) -> Result<()> {
    use robosync::shimmer_strategy_bridge::create_shimmer_strategy_selector;
    
    println!("=== Testing Shimmer Model ===\n");
    
    let selector = create_shimmer_strategy_selector()?;
    
    // Load test data (simplified for now)
    println!("Test data: {}", test_data.display());
    println!("Model loaded successfully");
    
    // Would implement actual test logic here
    println!("\nTest functionality not yet implemented.");
    println!("This will compare Shimmer predictions against heuristic strategies.");
    
    Ok(())
}

fn update_project_sync_status(action: &str, data: &Path) -> Result<()> {
    use robosync::shared_paths;
    use std::fs;
    use chrono::Utc;
    
    let sync_file = shared_paths::project_sync_file();
    
    // Ensure directory exists
    if let Some(parent) = sync_file.parent() {
        fs::create_dir_all(parent)?;
    }
    
    // Read existing or create new
    let mut sync_data = if sync_file.exists() {
        let content = fs::read_to_string(&sync_file)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({
            "last_sync": Utc::now().to_rfc3339(),
            "projects": {},
            "messages": [],
            "pending_tasks": []
        })
    };
    
    // Update based on action
    match action {
        "pattern_export" => {
            sync_data["projects"]["robosync"]["last_pattern_export"] = serde_json::Value::String(Utc::now().to_rfc3339());
            sync_data["projects"]["robosync"]["patterns_collected"] = serde_json::Value::Number(
                sync_data["projects"]["robosync"]["patterns_collected"].as_u64().unwrap_or(0).wrapping_add(1).into()
            );
            
            // Add message
            if let Some(messages) = sync_data["messages"].as_array_mut() {
                messages.push(serde_json::json!({
                    "timestamp": Utc::now().to_rfc3339(),
                    "type": "pattern_export",
                    "from": "robosync",
                    "data": {
                        "export_path": data.to_string_lossy()
                    }
                }));
            }
        }
        _ => {}
    }
    
    // Write back
    fs::write(sync_file, serde_json::to_string_pretty(&sync_data)?)?;
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_max_thread_count_returns_valid_range() {
        let max_threads = get_max_thread_count();

        // Should return a reasonable number
        assert!(
            max_threads >= 16,
            "Max threads should be at least 16, got {}",
            max_threads
        );
        assert!(
            max_threads <= 512,
            "Max threads should be at most 512, got {}",
            max_threads
        );

        // Should be a power-friendly number for efficiency
        assert!(
            max_threads % 8 == 0 || max_threads == 64 || max_threads == 128 || max_threads == 256,
            "Max threads {} should be divisible by 8 or a common power",
            max_threads
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_get_max_thread_count_linux() {
        let max_threads = get_max_thread_count();

        // Linux should return between 64 and 512
        assert!(max_threads >= 64);
        assert!(max_threads <= 512);

        // Try to verify against actual ulimit
        if let Ok(output) = std::process::Command::new("sh")
            .arg("-c")
            .arg("ulimit -n")
            .output()
        {
            if let Ok(limit_str) = String::from_utf8(output.stdout) {
                if let Ok(limit) = limit_str.trim().parse::<usize>() {
                    let expected = (limit / 4).clamp(64, 512);
                    assert_eq!(max_threads, expected);
                }
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_get_max_thread_count_macos() {
        let max_threads = get_max_thread_count();
        assert_eq!(max_threads, 64);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_get_max_thread_count_windows() {
        let max_threads = get_max_thread_count();
        assert_eq!(max_threads, 128);
    }

    #[test]
    #[cfg(target_os = "freebsd")]
    fn test_get_max_thread_count_freebsd() {
        let max_threads = get_max_thread_count();

        // FreeBSD should return between 64 and 256
        assert!(max_threads >= 64);
        assert!(max_threads <= 256);
    }

    #[test]
    fn test_thread_count_consistency() {
        // Call multiple times to ensure it returns consistent results
        let first_call = get_max_thread_count();
        let second_call = get_max_thread_count();
        let third_call = get_max_thread_count();

        assert_eq!(first_call, second_call);
        assert_eq!(second_call, third_call);
    }
}
