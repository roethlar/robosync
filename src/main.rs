use anyhow::Result;
use clap::{Arg, Command};
use crossterm::style::Color;
use robosync::color_output::ConditionalColor;
use std::path::PathBuf;

use robosync::compression::CompressionConfig;
use robosync::options::{SymlinkBehavior, SyncOptions};
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
                .value_parser(clap::value_parser!(PathBuf))
        )
        .arg(
            Arg::new("destination")
                .help("Destination directory or file")
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
            Arg::new("no-report-errors")
                .long("no-report-errors")
                .help("Disable automatic error report file generation")
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
        )

        // Symlink handling options
        .arg(
            Arg::new("links")
                .long("links")
                .help("Copy symlinks as symlinks (default behavior)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("deref")
                .long("deref")
                .help("Dereference symlinks - copy the target file/directory instead of the symlink")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("no-links")
                .long("no-links")
                .help("Skip symlinks entirely - do not copy symlinks or their targets")
                .action(clap::ArgAction::SetTrue)
        );

    #[cfg(target_os = "linux")]
    let matches = matches.arg(
        Arg::new("linux-optimized")
            .long("linux-optimized")
            .help("Enable Linux-specific optimizations for small files")
            .action(clap::ArgAction::SetTrue),
    );

    #[cfg(not(target_os = "linux"))]
    let matches = matches;

    let matches = matches.arg(
        Arg::new("no-smart")
            .long("no-smart")
            .help("Diagnostic: use basic parallel mode instead of mixed mode")
            .action(clap::ArgAction::SetTrue),
    );

    let matches = matches.arg(
            Arg::new("strategy")
                .long("strategy")
                .value_name("METHOD")
                .help("Diagnostic override: force a specific strategy (delta, mixed, rsync, robocopy, platform)")
                .value_parser(["delta", "mixed", "rsync", "robocopy", "platform", "io_uring", "parallel"])
        );

    let matches = matches
        .arg(
            Arg::new("dry-run")
                .short('n')
                .short_alias('N')
                .long("dry-run")
                .alias("dryrun")
                .help("Show what would be done without actually doing it")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("checksum")
                .short('c')
                .long("checksum")
                .alias("Checksum")
                .alias("CHECKSUM")
                .help("Skip based on checksum, not mod-time & size")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

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
            "\n  ❌ Error: Source path does not exist: {}\n",
            source.display()
        );
        std::process::exit(1);
    }

    // Parse options
    let compress = matches.get_flag("compress");
    let verbose = matches.get_count("verbose");
    let confirm = matches.get_flag("confirm");
    let dry_run = matches.get_flag("dry-run") || matches.get_flag("list-only");
    let no_progress = matches.get_flag("no-progress");
    let no_report_errors = matches.get_flag("no-report-errors");
    let move_files = matches.get_flag("move-files");
    let checksum = matches.get_flag("checksum");
    let no_smart = matches.get_flag("no-smart");
    let _smart_mode = !no_smart; // Smart mode is now default (unused but kept for clarity)
    let forced_strategy = matches.get_one::<String>("strategy").cloned();
    let _has_forced_strategy = forced_strategy.is_some();

    // Parse symlink handling options
    let links = matches.get_flag("links");
    let deref = matches.get_flag("deref");
    let no_links = matches.get_flag("no-links");

    // Validate symlink options - only one can be specified
    let symlink_count = [links, deref, no_links].iter().filter(|&&x| x).count();
    if symlink_count > 1 {
        eprintln!(
            "\n  ❌ Error: Only one symlink option can be specified: --links, --deref, or --no-links\n"
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
    let exclude_dirs: Vec<String> = matches
        .get_many::<String>("exclude-dirs")
        .unwrap_or_default()
        .cloned()
        .collect();
    let min_size = matches.get_one::<u64>("min-size").copied();
    let max_size = matches.get_one::<u64>("max-size").copied();

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
        .unwrap_or(num_cpus);
    let block_size = matches
        .get_one::<usize>("block-size")
        .copied()
        .unwrap_or(1024);

    // Validate thread count based on OS limits
    let max_threads = get_max_thread_count();
    if threads > max_threads {
        eprintln!(
            "\n  ❌ Error: Maximum thread count is {max_threads} to avoid system file handle limits."
        );
        eprintln!(
            "     Requested: {threads}, Maximum allowed: {max_threads}\n"
        );
        std::process::exit(1);
    }

    // Logging
    let log_file = matches.get_one::<String>("log-file");
    let show_eta = matches.get_flag("eta");

    // Retry options
    let retry_count = matches.get_one::<u32>("retry-count").copied().unwrap_or(0);
    let retry_wait = matches.get_one::<u32>("retry-wait").copied().unwrap_or(30);

    // Print header without lines
    if !no_progress {
        println!(
            "     {} v{}: {}",
            "RoboSync".color_bold_if(Color::Cyan),
            env!("CARGO_PKG_VERSION").color_if(Color::White),
            "Fast parallel file synchronization".color_if(Color::White)
        );
    }

    // Calculate the max width needed for all content
    let source_str = source.display().to_string();
    let dest_str = destination.display().to_string();

    // Build options string
    let mut all_options = Vec::new();
    if mirror {
        all_options.push("--mir".to_string());
    } else if empty_dirs {
        all_options.push("-e".to_string());
    } else if subdirs {
        all_options.push("-s".to_string());
    }
    if archive {
        all_options.push("-a".to_string());
    }
    if compress {
        all_options.push("-z".to_string());
    }
    if checksum {
        all_options.push("-c".to_string());
    }
    if move_files {
        all_options.push("--mov".to_string());
    }
    if dry_run {
        all_options.push("-n".to_string());
    }
    if verbose > 0 {
        all_options.push(format!("-{}", "v".repeat(verbose as usize)));
    }
    if threads != num_cpus {
        all_options.push(format!("--mt {threads}"));
    }
    if retry_count > 0 {
        all_options.push(format!("-r {retry_count} -w {retry_wait}"));
    }
    if copy_all || archive {
        all_options.push("--copyall".to_string());
    } else if copy_flags != "DAT" {
        all_options.push(format!("--copy {copy_flags}"));
    }

    // Add exclude patterns to options
    for excl in &exclude_files {
        all_options.push(format!("--xf {excl}"));
    }
    for excl in &exclude_dirs {
        all_options.push(format!("--xd {excl}"));
    }

    let options_str = all_options.join(" ");

    // Find max width needed
    let max_len = source_str.len().max(dest_str.len()).max(options_str.len());
    let _table_width = max_len.max(44) + 2; // At least 44 chars, plus padding

    // Display configuration
    if !no_progress {
        println!();
        println!(
            "     {}  {}",
            "Source:".color_if(Color::White),
            source_str.as_str().color_if(Color::Green)
        );
        println!(
            "     {}    {}",
            "Dest:".color_if(Color::White),
            dest_str.as_str().color_if(Color::Yellow)
        );
        if !options_str.is_empty() {
            println!(
                "     {} {}",
                "Options:".color_if(Color::White),
                options_str.as_str().color_if(Color::DarkGrey)
            );
        }
    }

    // Warn about dangerous combinations
    if move_files && mirror {
        eprintln!("  ⚠️  WARNING: Using --mov with --mir is dangerous!");
        eprintln!("     If the sync is interrupted, source files already moved will be lost.");
        eprintln!("     Consider using --mir without --mov for safety.");
        eprintln!();
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
        symlink_behavior,
        no_report_errors,
    };

    if !dry_run {
        if sync_options.forced_strategy.is_some() {
            // Diagnostic override - use legacy smart mode with specific strategy
            let config = ParallelSyncConfig {
                worker_threads: threads,
                io_threads: threads,
                block_size,
                max_parallel_files: threads * 2,
            };

            let syncer = ParallelSyncer::new(config);
            if let Some(ref strategy) = sync_options.forced_strategy {
                println!("     Diagnostic mode: using {strategy} strategy");
            }
            let _stats = syncer.synchronize_smart(source, destination, sync_options)?;
        } else {
            // Default: mixed mode (optimal for all scenarios)
            // Force mixed mode in sync options and use smart mode infrastructure
            let mut mixed_options = sync_options.clone();
            mixed_options.forced_strategy = Some("mixed".to_string());

            let config = ParallelSyncConfig {
                worker_threads: threads,
                io_threads: threads,
                block_size,
                max_parallel_files: threads * 2,
            };

            let syncer = ParallelSyncer::new(config);
            let _stats = syncer.synchronize_smart(source, destination, mixed_options)?;
        }
    } else {
        // Dry run mode
        sync::synchronize_with_options(source, destination, threads, sync_options)?;
    }

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
