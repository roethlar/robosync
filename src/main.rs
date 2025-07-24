use anyhow::Result;
use clap::{Arg, Command};
use std::path::PathBuf;

mod algorithm;
mod checksum;
mod file_list;
mod progress;
mod options;
mod metadata;
mod logging;
mod compression;
mod retry;
mod sync;
mod parallel_sync;

use parallel_sync::{ParallelSyncer, ParallelSyncConfig};
use options::SyncOptions;
use compression::CompressionConfig;

fn main() -> Result<()> {
    let matches = Command::new("RoboSync")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Fast, parallel file synchronization with delta-transfer algorithm")
        .arg(
            Arg::new("source")
                .help("Source directory or file")
                .required(true)
                .value_parser(clap::value_parser!(PathBuf))
        )
        .arg(
            Arg::new("destination")
                .help("Destination directory or file")
                .required(true)
                .value_parser(clap::value_parser!(PathBuf))
        )
        
        // Core copy options (RoboCopy style)
        .arg(
            Arg::new("subdirs")
                .long("s")
                .alias("S")
                .help("Copy subdirectories, but not empty ones (/S)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("empty-dirs")
                .long("e")
                .alias("E")
                .help("Copy subdirectories, including empty ones (/E)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("mirror")
                .long("mir")
                .alias("MIR")
                .alias("Mirror")
                .alias("MIRROR")
                .help("Mirror a directory tree (equivalent to /E plus /PURGE)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("purge")
                .long("purge")
                .alias("PURGE")
                .alias("Purge")
                .help("Delete dest files/dirs that no longer exist in source")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("list-only")
                .long("l")
                .alias("L")
                .help("List only - don't copy, timestamp or delete any files")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("move-files")
                .long("mov")
                .alias("MOV")
                .alias("Move")
                .alias("MOVE")
                .help("Move files (delete source after successful copy)")
                .action(clap::ArgAction::SetTrue)
        )
        
        // File filtering options
        .arg(
            Arg::new("exclude-files")
                .long("xf")
                .alias("XF")
                .alias("Xf")
                .value_name("PATTERN")
                .help("Exclude files matching given patterns")
                .action(clap::ArgAction::Append)
        )
        .arg(
            Arg::new("exclude-dirs")
                .long("xd")
                .alias("XD")
                .alias("Xd")
                .value_name("PATTERN")
                .help("Exclude directories matching given patterns")
                .action(clap::ArgAction::Append)
        )
        .arg(
            Arg::new("min-size")
                .long("min")
                .alias("MIN")
                .alias("Min")
                .value_name("SIZE")
                .help("Minimum file size - exclude files smaller than SIZE bytes")
                .value_parser(clap::value_parser!(u64))
        )
        .arg(
            Arg::new("max-size")
                .long("max")
                .alias("MAX")
                .alias("Max")
                .value_name("SIZE")
                .help("Maximum file size - exclude files bigger than SIZE bytes")
                .value_parser(clap::value_parser!(u64))
        )
        
        // Copy flags
        .arg(
            Arg::new("copy-flags")
                .long("copy")
                .alias("COPY")
                .alias("Copy")
                .value_name("FLAGS")
                .help("What to copy: D=Data, A=Attributes, T=Timestamps, S=Security, O=Owner, U=aUditing (default: DAT)")
                .default_value("DAT")
        )
        .arg(
            Arg::new("copy-all")
                .long("copyall")
                .alias("COPYALL")
                .alias("CopyAll")
                .alias("Copyall")
                .help("Copy all file info including security/ownership (equivalent to /COPY:DATSOU)")
                .action(clap::ArgAction::SetTrue)
        )
        
        // Logging and verbosity
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .alias("Verbose")
                .alias("VERBOSE")
                .help("Produce verbose output, showing skipped files (/V)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("no-progress")
                .long("np")
                .alias("NP")
                .alias("Np")
                .help("No progress - don't display percentage copied")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("log-file")
                .long("log")
                .alias("LOG")
                .alias("Log")
                .value_name("FILE")
                .help("Output status to log file (overwrite existing)")
        )
        .arg(
            Arg::new("eta")
                .long("eta")
                .alias("ETA")
                .alias("Eta")
                .help("Show estimated time of arrival and progress updates")
                .action(clap::ArgAction::SetTrue)
        )
        
        // Retry options
        .arg(
            Arg::new("retry-count")
                .long("r")
                .alias("R")
                .value_name("NUM")
                .help("Number of retries on failed copies (default: 0)")
                .default_value("0")
                .value_parser(clap::value_parser!(u32))
        )
        .arg(
            Arg::new("retry-wait")
                .long("w")
                .alias("W")
                .value_name("SECONDS")
                .help("Wait time between retries in seconds (default: 30)")
                .default_value("30")
                .value_parser(clap::value_parser!(u32))
        )
        
        // Performance options
        .arg(
            Arg::new("threads")
                .long("mt")
                .alias("MT")
                .alias("Mt")
                .value_name("NUM")
                .help("Do multi-threaded copies with NUM threads (default: CPU cores)")
                .value_parser(clap::value_parser!(usize))
        )
        .arg(
            Arg::new("block-size")
                .short('B')
                .short('b')
                .long("block-size")
                .alias("Block-Size")
                .alias("blocksize")
                .alias("BlockSize")
                .value_name("SIZE")
                .help("Block size for delta algorithm in bytes (default: 1024). Smaller blocks find more matches but use more CPU/memory. Larger blocks are faster but may transfer more data.")
                .value_parser(clap::value_parser!(usize))
        )
        .arg(
            Arg::new("sequential")
                .long("sequential")
                .alias("Sequential")
                .alias("SEQUENTIAL")
                .help("Force sequential processing (disables parallelism)")
                .action(clap::ArgAction::SetTrue)
        )
        
        // Legacy rsync options
        .arg(
            Arg::new("archive")
                .short('a')
                .short('A')
                .long("archive")
                .alias("Archive")
                .alias("ARCHIVE")
                .help("Archive mode (equivalent to -rlptgoD)")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("recursive")
                .short('r')
                .short('R')
                .long("recursive")
                .alias("Recursive")
                .alias("RECURSIVE")
                .help("Recurse into directories")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("compress")
                .short('z')
                .short('Z')
                .long("compress")
                .alias("Compress")
                .alias("COMPRESS")
                .help("Compress file data during transfer")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("dry-run")
                .short('n')
                .short('N')
                .long("dry-run")
                .alias("Dry-Run")
                .alias("dryrun")
                .alias("DryRun")
                .help("Show what would be done without actually doing it")
                .action(clap::ArgAction::SetTrue)
        )
        .get_matches();

    let source: PathBuf = matches.get_one::<PathBuf>("source").unwrap().clone();
    let destination: PathBuf = matches.get_one::<PathBuf>("destination").unwrap().clone();
    
    // Parse options
    let compress = matches.get_flag("compress");
    let sequential = matches.get_flag("sequential");
    let parallel = !sequential;
    let verbose = matches.get_flag("verbose");
    let dry_run = matches.get_flag("dry-run") || matches.get_flag("list-only");
    let no_progress = matches.get_flag("no-progress");
    let move_files = matches.get_flag("move-files");
    
    // Copy options
    let subdirs = matches.get_flag("subdirs");
    let empty_dirs = matches.get_flag("empty-dirs");
    let mirror = matches.get_flag("mirror");
    let purge = matches.get_flag("purge") || mirror;
    let archive = matches.get_flag("archive");
    // Mirror mode implies empty directories (/E), which implies subdirectories (/S)
    // For directory operations, recursive is always enabled by default
    let recursive = source.is_dir() || matches.get_flag("recursive") || archive || subdirs || empty_dirs || mirror;
    
    // File filtering
    let exclude_files: Vec<String> = matches.get_many::<String>("exclude-files")
        .unwrap_or_default()
        .map(|s| s.clone())
        .collect();
    let exclude_dirs: Vec<String> = matches.get_many::<String>("exclude-dirs")
        .unwrap_or_default()
        .map(|s| s.clone())
        .collect();
    let min_size = matches.get_one::<u64>("min-size").copied();
    let max_size = matches.get_one::<u64>("max-size").copied();
    
    // Copy flags
    let copy_flags = matches.get_one::<String>("copy-flags").unwrap();
    let copy_all = matches.get_flag("copy-all");
    
    // Performance
    let num_cpus = std::thread::available_parallelism().unwrap().get();
    let threads = matches.get_one::<usize>("threads").copied()
        .unwrap_or(num_cpus);
    let block_size = matches.get_one::<usize>("block-size").copied()
        .unwrap_or(1024);

    // Validate thread count to avoid system limits
    if threads > 64 {
        eprintln!("Error: Maximum thread count is 64 to avoid system file handle limits.");
        eprintln!("Requested: {}, Maximum allowed: 64", threads);
        std::process::exit(1);
    }

    // Logging
    let log_file = matches.get_one::<String>("log-file");
    let show_eta = matches.get_flag("eta");
    
    // Retry options
    let retry_count = matches.get_one::<u32>("retry-count").copied()
        .unwrap_or(0);
    let retry_wait = matches.get_one::<u32>("retry-wait").copied()
        .unwrap_or(30);

    println!("RoboSync v{}: Fast parallel file synchronization", env!("CARGO_PKG_VERSION"));
    println!("Source: {}", source.display());
    println!("Destination: {}", destination.display());
    
    if dry_run {
        println!("Mode: DRY RUN (list only)");
    } else {
        println!("Mode: {}", if parallel { "parallel" } else { "sequential" });
    }
    
    if parallel && !dry_run {
        println!("Threads: {}", threads);
        println!("Block size: {} bytes", block_size);
    }
    
    // Show active options
    let mut options = Vec::new();
    if recursive { options.push("recursive"); }
    if purge { options.push("purge"); }
    if mirror { options.push("mirror"); }
    if verbose { options.push("verbose"); }
    if compress { options.push("compress"); }
    if move_files { options.push("move-files"); }
    if !exclude_files.is_empty() { options.push("exclude-files"); }
    if !exclude_dirs.is_empty() { options.push("exclude-dirs"); }
    if min_size.is_some() { options.push("min-size"); }
    if max_size.is_some() { options.push("max-size"); }
    
    if !options.is_empty() {
        println!("Options: {}", options.join(", "));
    }
    
    if copy_all {
        println!("Copy flags: DATSOU (all)");
    } else {
        println!("Copy flags: {}", copy_flags);
    }
    
    if retry_count > 0 {
        println!("Retry: {} retries, {} seconds wait", retry_count, retry_wait);
    }

    // Create sync options struct
    let sync_options = SyncOptions {
        recursive,
        purge,
        mirror,
        dry_run,
        verbose,
        no_progress,
        move_files,
        exclude_files,
        exclude_dirs,
        min_size,
        max_size,
        copy_flags: if copy_all { "DATSOU".to_string() } else { copy_flags.clone() },
        log_file: log_file.cloned(),
        compress,
        compression_config: if compress { CompressionConfig::balanced() } else { CompressionConfig::default() },
        show_eta,
        retry_count,
        retry_wait,
    };

    if parallel && !dry_run {
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