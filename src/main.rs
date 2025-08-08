use anyhow::Result;
use clap::{Arg, Command};
use crossterm::style::Color;
use robosync::color_output::ConditionalColor;
use std::path::PathBuf;

use robosync::compression::CompressionConfig;
use robosync::options::{load_config, SymlinkBehavior, SyncOptions};
use robosync::parallel_sync::{ParallelSyncConfig, ParallelSyncer};
use robosync::reflink::ReflinkMode;
use robosync::sync;
use robosync::worker_pool;

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

/// Build the command-line interface
fn build_cli() -> Command {
    Command::new("RoboSync")
        .version(env!("CARGO_PKG_VERSION"))
        .about("High-performance file synchronization tool with intelligent strategy selection")
        .arg(
            Arg::new("source")
                .help("Source path (file or directory to copy from)")
                .value_parser(clap::value_parser!(PathBuf))
        )
        .arg(
            Arg::new("destination")
                .help("Destination path (where to copy files to)")
                .value_parser(clap::value_parser!(PathBuf))
        )

        // Core copy options (RoboCopy style)
        .arg(
            Arg::new("subdirs")
                .short('s')
                .short_alias('S')
                .help("Copy subdirectories recursively (excluding empty ones)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("empty-dirs")
                .short('e')
                .short_alias('E')
                .help("Copy subdirectories recursively (including empty ones)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("mirror")
                .long("mir")
                .alias("mirror")
                .help("Mirror mode: make destination exactly like source (deletes extra files)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("purge")
                .long("purge")
                .help("Delete destination files/directories that don't exist in source")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("list-only")
                .short('l')
                .short_alias('L')
                .help("List mode: show what would be done without making changes")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("move-files")
                .long("mov")
                .help("Move files: delete from source after successful copy (CAUTION: interruption may cause data loss)")
                .action(clap::ArgAction::SetTrue)
        )

        // File filtering options
        .arg(
            Arg::new("exclude-files")
                .long("xf")
                .alias("exclude")
                .value_name("PATTERN")
                .help("Exclude files matching pattern (can be used multiple times)")
                .action(clap::ArgAction::Append)
        )
        .arg(
            Arg::new("exclude-dirs")
                .long("xd")
                .value_name("PATTERN")
                .help("Exclude directories matching pattern (can be used multiple times)")
                .action(clap::ArgAction::Append)
        )
        .arg(
            Arg::new("min-size")
                .long("min")
                .value_name("SIZE")
                .help("Skip files smaller than SIZE bytes")
                .value_parser(clap::value_parser!(u64))
        )
        .arg(
            Arg::new("max-size")
                .long("max")
                .value_name("SIZE")
                .help("Skip files larger than SIZE bytes")
                .value_parser(clap::value_parser!(u64))
        )

        // Copy flags
        .arg(
            Arg::new("copy-flags")
                .long("copy")
                .value_name("FLAGS")
                .help("Specify what to copy: D=Data, A=Attributes, T=Timestamps, S=Security, O=Owner, U=Auditing")
                .default_value("DAT")
        )
        .arg(
            Arg::new("copy-all")
                .long("copyall")
                .help("Copy all metadata: data, attributes, timestamps, security, ownership")
                .action(clap::ArgAction::SetTrue)
        )

        // Logging and verbosity
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .alias("Verbose")
                .alias("VERBOSE")
                .help("Verbose output (-v: show preview and progress, -vv: show all file operations)")
                .action(clap::ArgAction::Count)
        )
        .arg(
            Arg::new("confirm")
                .long("confirm")
                .help("Ask for confirmation before starting sync")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("no-progress")
                .long("no-progress")
                .help("Disable the progress spinner")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("no-report-errors")
                .long("no-report-errors")
                .help("Don't create error report file on failures")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("log-file")
                .long("log")
                .value_name("FILE")
                .help("Save operation log to FILE (overwrites existing)")
        )
        .arg(
            Arg::new("eta")
                .long("eta")
                .help("Show ETA and detailed progress information")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("debug")
                .long("debug")
                .help("Enable debug output for troubleshooting")
                .action(clap::ArgAction::SetTrue)
        )

        // Retry options
        .arg(
            Arg::new("retry-count")
                .long("retry")
                .value_name("NUM")
                .help("Retry failed operations N times")
                .default_value("0")
                .value_parser(clap::value_parser!(u32))
        )
        .arg(
            Arg::new("retry-wait")
                .short('w')
                .short_alias('W')
                .long("wait")
                .value_name("SECONDS")
                .help("Wait N seconds between retries")
                .default_value("30")
                .value_parser(clap::value_parser!(u32))
        )

        // Performance options
        .arg(
            Arg::new("threads")
                .long("mt")
                .value_name("NUM")
                .help("Use N threads for parallel operations (default: CPU count)")
                .value_parser(clap::value_parser!(usize))
        )
        .arg(
            Arg::new("block-size")
                .short('b')
                .short_alias('B')
                .long("block-size")
                .alias("blocksize")
                .value_name("SIZE")
                .help("Delta transfer block size in bytes (smaller = more precise, larger = faster)")
                .value_parser(clap::value_parser!(usize))
        )

        // File size threshold options
        .arg(
            Arg::new("small-file-threshold")
                .long("small-threshold")
                .value_name("SIZE")
                .help("Files smaller than SIZE use parallel batch processing (default: 256KB)")
                .value_parser(clap::value_parser!(u64))
        )
        .arg(
            Arg::new("medium-file-threshold")
                .long("medium-threshold")
                .value_name("SIZE")
                .help("Files between small and SIZE use optimized transfer (default: 16MB)")
                .value_parser(clap::value_parser!(u64))
        )
        .arg(
            Arg::new("large-file-threshold")
                .long("large-threshold")
                .value_name("SIZE")
                .help("Files larger than SIZE use memory-mapped I/O (default: 100MB)")
                .value_parser(clap::value_parser!(u64))
        )
        .arg(
            Arg::new("batch-file-count-threshold")
                .long("batch-count")
                .value_name("COUNT")
                .help("Use batch mode when directory has more than N small files (default: 100)")
                .value_parser(clap::value_parser!(usize))
        )

        // Special options
        .arg(
            Arg::new("recursive")
                .short('r')
                .long("recursive")
                .help("Recursive copy (auto-enabled for directories)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("archive")
                .short('a')
                .long("archive")
                .help("Archive mode: preserve all metadata (like rsync -a)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("compress")
                .short('z')
                .short_alias('Z')
                .help("Compress data during transfer (automatic algorithm selection)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("dry-run")
                .short('n')
                .short_alias('N')
                .long("dry-run")
                .help("Dry run: show what would be done without making changes")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("no-batch")
                .long("no-batch")
                .help("Don't use tar batching optimization for small files")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("force-tar")
                .long("force-tar")
                .help("Force tar streaming mode (skip analysis)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("checksum")
                .short('c')
                .short_alias('C')
                .help("Compare files using checksums instead of size/time")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("no-smart")
                .long("no-smart")
                .help("Disable automatic strategy selection")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("strategy")
                .long("strategy")
                .value_name("MODE")
                .help("Force strategy: delta, parallel, or mixed (default: auto-select)")
                .value_parser(["delta", "parallel", "mixed"])
        )

        // Linux-specific options
        .arg(
            Arg::new("linux-optimized")
                .long("linux-optimized")
                .help("Enable Linux-specific optimizations (io_uring, splice, FIEMAP)")
                .action(clap::ArgAction::SetTrue)
                .hide(cfg!(not(target_os = "linux")))
        )

        // Symlink handling options
        .arg(
            Arg::new("links")
                .long("links")
                .help("Copy symlinks as symlinks (default)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("deref")
                .long("deref")
                .help("Follow symlinks and copy target files")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("no-links")
                .long("no-links")
                .help("Don't process symlinks")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("reflink")
                .long("reflink")
                .value_name("MODE")
                .help("Reflink/COW mode: always, auto (default), or never")
                .value_parser(["always", "auto", "never"])
                .default_value("auto"),
        )
        .arg(
            Arg::new("enterprise")
                .long("enterprise")
                .help("Enterprise mode: maximum reliability with integrity checks and atomic operations")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("verify-integrity")
                .long("verify-integrity")
                .help("Verify file integrity after copying (enterprise feature)")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("atomic-operations")
                .long("atomic-operations") 
                .help("Use atomic operations for corruption prevention (enterprise feature)")
                .action(clap::ArgAction::SetTrue),
        )
}

/// Parse command-line arguments and build SyncOptions
fn parse_sync_options(matches: &clap::ArgMatches, config: &robosync::options::Config) -> Result<(PathBuf, PathBuf, SyncOptions, usize, usize)> {
    // For regular sync operations, source and destination are required
    let source_arg: PathBuf = matches
        .get_one::<PathBuf>("source")
        .ok_or_else(|| anyhow::anyhow!("Source path required"))?
        .clone();
    let destination: PathBuf = matches
        .get_one::<PathBuf>("destination")
        .ok_or_else(|| anyhow::anyhow!("Destination path required"))?
        .clone();

    // Use the source path as-is to maintain correct path structure
    // Don't canonicalize as it breaks path stripping when source is a symlink
    let source = source_arg.clone();

    // Check if the source exists
    if !source.exists() {
        eprintln!(
            "\n  Error: Source path does not exist: {}\n",
            source.display()
        );
        std::process::exit(1);
    }

    // Parse options
    let compress = matches.get_flag("compress");
    let verbose = matches.get_count("verbose");
    let confirm = matches.get_flag("confirm");
    let dry_run = matches.get_flag("dry-run") || matches.get_flag("list-only");
    
    // ALWAYS show spinner unless --no-progress is specified
    // User requirement: "spinner in EVERY scenario"
    let no_progress = matches.get_flag("no-progress");
    let show_progress = !no_progress;  // Show spinner in ALL scenarios unless explicitly disabled
    
    let no_report_errors = matches.get_flag("no-report-errors");
    let no_batch = matches.get_flag("no-batch");
    let force_tar = matches.get_flag("force-tar");
    let move_files = matches.get_flag("move-files");
    let checksum = matches.get_flag("checksum");
    let debug = matches.get_flag("debug");
    let no_smart = matches.get_flag("no-smart");
    let enterprise_mode = matches.get_flag("enterprise");
    let verify_integrity = matches.get_flag("verify-integrity") || enterprise_mode;
    let atomic_operations = matches.get_flag("atomic-operations") || enterprise_mode;
    let _smart_mode = !no_smart; // Smart mode is now default (unused but kept for clarity)
    let reflink = match matches.get_one::<String>("reflink").map(|s| s.as_str()) {
        Some("always") => ReflinkMode::Always,
        Some("never") => ReflinkMode::Never,
        _ => ReflinkMode::Auto, // Default to auto
    };

    // Parse symlink handling options
    let links = matches.get_flag("links");
    let deref = matches.get_flag("deref");
    let no_links = matches.get_flag("no-links");

    // Validate symlink options - only one can be specified
    let symlink_count = [links, deref, no_links].iter().filter(|&&x| x).count();
    if symlink_count > 1 {
        eprintln!(
            "\n  Error: Only one symlink option can be specified: --links, --deref, or --no-links\n"
        );
        std::process::exit(1);
    }

    // Determine symlink behavior (default is links)
    let symlink_behavior = if deref {
        SymlinkBehavior::Dereference
    } else if no_links {
        SymlinkBehavior::Skip
    } else {
        SymlinkBehavior::Preserve // Default behavior
    };
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
    let mut exclude_dirs: Vec<String> = matches
        .get_many::<String>("exclude-dirs")
        .unwrap_or_default()
        .cloned()
        .collect();
    if let Some(config_exclude_dirs) = &config.exclude_dirs {
        exclude_dirs.extend(config_exclude_dirs.clone());
    }
    let min_size = matches.get_one::<u64>("min-size").copied();
    let max_size = matches.get_one::<u64>("max-size").copied();

    // File size thresholds
    let small_file_threshold = matches.get_one::<u64>("small-file-threshold").copied();
    let medium_file_threshold = matches.get_one::<u64>("medium-file-threshold").copied();
    let large_file_threshold = matches.get_one::<u64>("large-file-threshold").copied();
    let batch_file_count_threshold = matches.get_one::<usize>("batch-file-count-threshold").copied();

    // Copy flags
    let copy_flags = matches
        .get_one::<String>("copy-flags")
        .expect("copy-flags has a default value");
    let copy_all = matches.get_flag("copy-all");

    // Performance
    let num_cpus = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);
    let threads = matches
        .get_one::<usize>("threads")
        .copied()
        .or(config.threads)
        .unwrap_or(num_cpus);
    let block_size = matches
        .get_one::<usize>("block-size")
        .copied()
        .unwrap_or(1024);

    // Validate thread count based on OS limits
    let max_threads = get_max_thread_count();
    if threads > max_threads {
        eprintln!(
            "\n  Error: Maximum thread count is {max_threads} to avoid system file handle limits."
        );
        eprintln!("     Requested: {threads}, Maximum allowed: {max_threads}\n");
        std::process::exit(1);
    }

    // Logging
    let log_file = matches.get_one::<String>("log-file");
    let show_eta = matches.get_flag("eta");

    // Retry options
    let retry_count = matches.get_one::<u32>("retry-count").copied().unwrap_or(0);
    let retry_wait = matches.get_one::<u32>("retry-wait").copied().unwrap_or(30);

    // Create sync options struct
    let sync_options = SyncOptions {
        recursive,
        purge,
        mirror,
        dry_run,
        verbose,
        confirm,
        show_progress,
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
        reflink,
        #[cfg(target_os = "linux")]
        linux_optimized,
        symlink_behavior,
        no_report_errors,
        debug,
        enterprise_mode,
        verify_integrity,
        atomic_operations,
        no_batch,
        force_tar,
        prefer_delta: false,
        force_no_delta: false,
        use_hybrid_dam: None, // TODO: Add CLI flag for Hybrid Dam
        small_file_threshold,
        medium_file_threshold,
        large_file_threshold,
        batch_file_count_threshold,
        buffer_memory_fraction: None,
    };

    Ok((source, destination, sync_options, threads, block_size))
}

/// Display sync configuration and warnings
fn display_sync_info(source: &PathBuf, destination: &PathBuf, sync_options: &SyncOptions, threads: usize) {
    // Always print header - user requirement: show app name and version on startup
    println!(
        "{} v{}: {}",
        "RoboSync".color_bold_if(Color::Cyan),
        env!("CARGO_PKG_VERSION").color_if(Color::White),
        "Fast parallel file synchronization".color_if(Color::White)
    );

    // Calculate the max width needed for all content
    let source_str = source.display().to_string();
    let dest_str = destination.display().to_string();

    // Build options string
    let mut all_options = Vec::new();
    if sync_options.mirror {
        all_options.push("--mir".to_string());
    } else if sync_options.recursive && source.is_dir() {
        if sync_options.copy_flags.contains('A') {
            all_options.push("-e".to_string());
        } else {
            all_options.push("-s".to_string());
        }
    }
    if sync_options.compress {
        all_options.push("-z".to_string());
    }
    if sync_options.checksum {
        all_options.push("-c".to_string());
    }
    if sync_options.move_files {
        all_options.push("--mov".to_string());
    }
    if sync_options.dry_run {
        all_options.push("-n".to_string());
    }
    if sync_options.verbose > 0 {
        all_options.push(format!("-{}", "v".repeat(sync_options.verbose as usize)));
    }
    let num_cpus = std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);
    if threads != num_cpus {
        all_options.push(format!("--mt {threads}"));
    }
    if sync_options.retry_count > 0 {
        all_options.push(format!("-r {} -w {}", sync_options.retry_count, sync_options.retry_wait));
    }
    if sync_options.copy_flags == "DATSOU" {
        all_options.push("--copyall".to_string());
    } else if sync_options.copy_flags != "DAT" {
        all_options.push(format!("--copy {}", sync_options.copy_flags));
    }

    // Add exclude patterns to options
    for excl in &sync_options.exclude_files {
        all_options.push(format!("--xf {excl}"));
    }
    for excl in &sync_options.exclude_dirs {
        all_options.push(format!("--xd {excl}"));
    }

    let options_str = all_options.join(" ");

    // Always display configuration - needed for user to see what's happening
    println!();
    println!(
        "{}  {}",
        "Source:".color_if(Color::White),
        source_str.as_str().color_if(Color::Green)
    );
    println!(
        "{}    {}",
        "Dest:".color_if(Color::White),
        dest_str.as_str().color_if(Color::Yellow)
    );
    if !options_str.is_empty() {
        println!(
            "{} {}",
            "Options:".color_if(Color::White),
            options_str.as_str().color_if(Color::DarkGrey)
        );
    }

    // Warn about dangerous combinations
    if sync_options.move_files && sync_options.mirror {
        eprintln!("  WARNING: Using --mov with --mir is dangerous!");
        eprintln!("If the sync is interrupted, source files already moved will be lost.");
        eprintln!("Consider using --mir without --mov for safety.");
        eprintln!();
    }

    // Show enterprise mode notification
    if sync_options.enterprise_mode && sync_options.verbose > 0 {
        eprintln!("Enterprise mode enabled: Data integrity verification, atomic operations, and audit trails active");
    }
}

/// Execute the synchronization
fn execute_sync(source: PathBuf, destination: PathBuf, sync_options: SyncOptions, threads: usize, block_size: usize) -> Result<()> {
    let start_time = std::time::Instant::now();
    if !sync_options.dry_run {
        // Intelligent auto-selection based on workload
        let config = ParallelSyncConfig {
            worker_threads: threads,
            io_threads: threads,
            block_size,
            max_parallel_files: threads * 2,
        };

        let mut syncer = ParallelSyncer::new(config);
        let stats = syncer.synchronize_smart(source, destination, sync_options)?;

        // Always print summary statistics
        print_sync_summary(&stats, start_time.elapsed());
    } else {
        // Dry run mode
        sync::synchronize_with_options(source, destination, threads, sync_options)?;
    }

    Ok(())
}

/// Print synchronization summary
fn print_sync_summary(stats: &robosync::sync_stats::SyncStats, duration: std::time::Duration) {
    println!("\nSummary:");
    println!("Files copied: {}", stats.files_copied());
    println!("Files deleted: {}", stats.files_deleted());
    println!("Bytes transferred: {}", stats.bytes_transferred());
    let reflinks = stats.reflinks_succeeded();
    let reflink_fallbacks = stats.reflinks_failed_fallback();
    if reflinks > 0 || reflink_fallbacks > 0 {
        println!("Reflinks succeeded: {}", reflinks);
        println!("Reflinks fallback: {}", reflink_fallbacks);
    }
    println!("Errors: {}", stats.errors());
    
    // Add timing and throughput stats like robocopy
    let elapsed_secs = duration.as_secs_f64();
    println!("Elapsed time: {:.2}s", elapsed_secs);
    
    // Show speed only if data was transferred
    if stats.bytes_transferred() > 0 && elapsed_secs > 0.0 {
        let bytes_per_sec = stats.bytes_transferred() as f64 / elapsed_secs;
        let mb_per_min = (stats.bytes_transferred() as f64 / (1024.0 * 1024.0)) / (elapsed_secs / 60.0);
        println!("Speed: {:.0} Bytes/sec", bytes_per_sec);
        println!("Speed: {:.3} MB/min", mb_per_min);
    }
}

fn main() -> Result<()> {
    // Set up panic handler to write crash logs
    std::panic::set_hook(Box::new(|panic_info| {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let crash_file = format!("robosync_crash_{}.log", timestamp);
        
        let crash_details = format!(
            "RoboSync Crash Report - {}\n\
            =================================\n\
            {}\n\n\
            Location: {}\n\n\
            Please report this crash with the above details.\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            panic_info,
            panic_info.location().map_or("Unknown".to_string(), |loc| {
                format!("{}:{}:{}", loc.file(), loc.line(), loc.column())
            })
        );
        
        if let Err(_) = std::fs::write(&crash_file, &crash_details) {
            eprintln!("Failed to write crash log to {}", crash_file);
        } else {
            eprintln!("Crash details saved to: {}", crash_file);
        }
        eprintln!("{}", crash_details);
    }));
    
    // Initialize global thread pool at application startup (Phase 1 optimization)
    // This eliminates the 27ms thread spawning overhead identified in performance analysis
    worker_pool::initialize_global_pool(None)?;
    
    let matches = build_cli().get_matches();
    
    // Load config from .robosync.toml if it exists
    let config = load_config()?.unwrap_or_default();
    
    // Parse options
    let (source, destination, sync_options, threads, block_size) = parse_sync_options(&matches, &config)?;
    
    // Display sync info
    display_sync_info(&source, &destination, &sync_options, threads);
    
    // Execute sync
    execute_sync(source, destination, sync_options, threads, block_size)?;
    
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
            "Max threads should be at least 16, got {max_threads}"
        );
        assert!(
            max_threads <= 512,
            "Max threads should be at most 512, got {max_threads}"
        );

        // Should be a power-friendly number for efficiency
        assert!(
            max_threads % 8 == 0 || max_threads == 64 || max_threads == 128 || max_threads == 256,
            "Max threads {max_threads} should be divisible by 8 or a common power"
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
        assert_eq!(max_threads, 256);
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