//! Multithreaded synchronization implementation

use anyhow::{Context, Result};

/// Get current timestamp for logging
fn timestamp() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::fast_file_list::{compare_file_lists_fast, FastEnumConfig, FastFileListGenerator};
use crate::file_list::{
    compare_file_lists_with_roots, compare_file_lists_with_roots_and_progress,
    generate_file_list_with_options, generate_file_list_with_options_and_progress, FileInfo,
    FileOperation,
};
// Pattern export functionality moved to separate shimmer project
use crate::color_output::ConditionalColor;
#[cfg(target_os = "linux")]
use crate::file_list::generate_file_list_parallel;
#[cfg(target_os = "linux")]
use crate::linux_fast_copy::IO_URING_BATCH_SIZE;
use crate::logging::SyncLogger;
use crate::metadata::{copy_file_with_metadata_and_reflink, CopyFlags};
use crate::network_fs::{NetworkFsDetector, NetworkFsType};
use crate::reflink::ReflinkOptions;
use crate::hybrid_dam::{HybridDam, HybridDamConfig};
use crate::buffer_sizing::BufferSizer;
use crate::native_tools::NativeToolExecutor;
use crate::options::SyncOptions;
use crate::parallel_dirs::ParallelDirCreator;
// use crate::platform_api::PlatformCopier; // TODO: Used for platform-specific optimizations
use crate::progress::SyncProgress;
use crate::strategy::{CopyStrategy, FileStats, StrategySelector};
use crate::sync_stats::SyncStats;
use crossterm::style::Color;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use crate::compression::StreamingCompressor;
use crate::worker_pool;
use std::io::{BufReader, BufWriter};

/// Process file operations in batches to reduce synchronization overhead
/// This implements the "intelligent task batching" optimization from Phase 1
fn process_operations_batched<F>(
    operations: Vec<FileOperation>,
    batch_size: usize,
    mut process_fn: F,
) -> Result<()>
where
    F: FnMut(&FileOperation) -> Result<()> + Send + Sync,
{
    use std::sync::{Arc, Mutex};
    
    let process_fn = Arc::new(Mutex::new(process_fn));
    
    worker_pool::execute_batched(operations, batch_size, |batch| {
        let process_fn = Arc::clone(&process_fn);
        
        for operation in batch {
            if let Err(e) = process_fn.lock().unwrap()(operation) {
                eprintln!("Batch processing error: {}", e);
                // Continue with other files in the batch
            }
        }
    });
    
    Ok(())
}

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
    #[allow(dead_code)]
    fs_detector: NetworkFsDetector,
}

impl ParallelSyncer {
    pub fn new(config: ParallelSyncConfig) -> Self {
        Self { 
            config,
            fs_detector: NetworkFsDetector::new(),
        }
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
        println!("\nPending Operation Summary:");
        if new_files > 0 {
            println!("New Files: {new_files}");
        }
        if new_dirs > 0 {
            println!("New Directories: {new_dirs}");
        }
        if updates > 0 {
            println!("Updates: {updates}");
        }
        if deletions > 0 {
            println!("Deletions: {deletions}");
        }
        if symlinks > 0 {
            println!("Symlinks: {symlinks}");
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

    /// Execute with reflink/clonefile priority for same-volume operations
    fn execute_with_reflink_priority(
        &mut self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        // Use mixed mode but with reflink forced to "always"
        self.execute_mixed_mode_direct(source, destination, options)
    }
    
    /// Quick profile of directory to determine best strategy
    fn quick_profile_directory(&self, path: &Path, sample_size: usize) -> Result<crate::streaming_batch::WorkloadProfile> {
        use std::fs;
        
        let mut file_count = 0;
        let mut total_size = 0u64;
        let mut sampled = 0;
        
        // Quick sampling - just check first N files
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.take(sample_size) {
                if let Ok(entry) = entry {
                    if let Ok(metadata) = entry.metadata() {
                        if metadata.is_file() {
                            file_count += 1;
                            total_size += metadata.len();
                            sampled += 1;
                        }
                    }
                }
            }
        }
        
        let avg_file_size = if file_count > 0 {
            total_size / file_count as u64
        } else {
            0
        };
        
        Ok(crate::streaming_batch::WorkloadProfile {
            file_count,
            avg_file_size,
            total_size,
            sample_size: sampled,
        })
    }
    
    /// Handle single file copying - fix for critical bug identified by mac_claude
    fn handle_single_file_copy(&mut self, source: &Path, destination: &Path, options: &SyncOptions) -> Result<SyncStats> {
        use crate::metadata::{copy_file_with_metadata_and_reflink, CopyFlags};
        use crate::reflink::ReflinkOptions;
        
        let stats = SyncStats::default();
        let copy_flags = CopyFlags::from_string(&options.copy_flags);
        let reflink_options = ReflinkOptions { mode: options.reflink };
        
        // Determine actual destination path
        let dest_path = if destination.is_dir() {
            // Copy to directory - use source filename
            let file_name = source.file_name()
                .ok_or_else(|| anyhow::anyhow!("Source file has no name"))?;
            destination.join(file_name)
        } else {
            // Copy to specific file
            destination.to_path_buf()
        };
        
        // Create parent directory if needed
        if let Some(parent) = dest_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        
        // Copy the file with all optimizations
        match copy_file_with_metadata_and_reflink(source, &dest_path, &copy_flags, &reflink_options, Some(&stats)) {
            Ok(bytes_copied) => {
                stats.increment_files_copied();
                stats.add_bytes_transferred(bytes_copied);
                if options.show_progress {
                    println!("Copied {} bytes", bytes_copied);
                }
            }
            Err(e) => {
                stats.increment_errors();
                return Err(e);
            }
        }
        
        Ok(stats)
    }

    /// Execute mixed mode directly without file analysis
    fn execute_mixed_mode_direct(
        &mut self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        // Handle file-to-file copying - critical fix from mac_claude
        if source.is_file() {
            return self.handle_single_file_copy(&source, &destination, &options);
        }
        
        // Check if streaming batch mode should be used for small files - MOVED UP IN PRIORITY
        if !options.no_batch && !options.purge && source.is_dir() {
            // Force tar if requested by user
            if options.force_tar {
                if options.verbose >= 1 {
                    println!("🚀 Force-tar: Using tar streaming as requested");
                }
                let stats = SyncStats::new();
                return crate::streaming_batch_fast::fast_tar_transfer(
                    &source, 
                    &destination,
                    &stats, 
                    &options
                );
            }
            
            // Fast-path: Check for known tar candidates first (no analysis needed)
            if crate::speculative_tar::is_known_tar_candidate(&source) {
                if options.verbose >= 1 {
                    println!("🚀 Fast-path: Known tar candidate detected ({})", 
                        source.file_name().unwrap_or_default().to_string_lossy());
                }
                let stats = SyncStats::new();
                return crate::streaming_batch_fast::fast_tar_transfer(
                    &source, 
                    &destination,
                    &stats, 
                    &options
                );
            }
            
            // Try speculative tar execution with parallel analysis
            let stats = SyncStats::new();
            match crate::speculative_tar::execute_speculative_tar(&source, &destination, &stats, &options) {
                Ok(result_stats) => {
                    if options.verbose >= 1 {
                        println!("✅ Speculative tar streaming completed successfully");
                    }
                    return Ok(result_stats);
                }
                Err(e) => {
                    // Check if this is a strategy rejection (expected) or an actual error
                    if e.to_string().contains("strategy rejected") {
                        if options.verbose >= 2 {
                            println!("Tar streaming not suitable, falling back to mixed strategy");
                        }
                        // Continue to mixed strategy below
                    } else {
                        // Actual error, but we can still try mixed strategy as fallback
                        if options.verbose >= 1 {
                            eprintln!("Warning: Speculative tar failed: {}", e);
                            eprintln!("Falling back to mixed strategy...");
                        }
                    }
                }
            }
        }
        
        // Check if any filters are specified
        let has_filters = !options.exclude_files.is_empty() || 
                         !options.exclude_dirs.is_empty() || 
                         options.min_size.is_some() || 
                         options.max_size.is_some();
        
        // Ultra-fast path for simple directory copies - beats all native tools (only if no filters)
        if !options.purge && !options.dry_run && crate::ultra_fast_copy::is_simple_copy_scenario(&source, &destination, has_filters) {
            if options.show_progress {
                println!("Ultra-fast copy mode detected");
            }
            return crate::ultra_fast_copy::ultra_fast_directory_copy(&source, &destination);
        }
        
        // Fast path for small files scenario - bypass most overhead (only if no filters)
        if !options.purge && !has_filters && source.is_dir() && crate::small_file_optimizer::is_small_files_scenario(&source).unwrap_or(false) {
            if options.show_progress {
                println!("Fast path for small files detected");
            }
            return crate::small_file_optimizer::sync_small_files_fast(&source, &destination);
        }

        // TIMER: Start filesystem detection
        let fs_detect_start = std::time::Instant::now();
        println!("[TIMER] Starting filesystem detection...");
        
        // Detect network filesystem types for optimization
        let mut fs_detector = NetworkFsDetector::new();
        let src_fs_info = fs_detector.detect_filesystem(&source);
        let dst_fs_info = fs_detector.detect_filesystem(&destination);
        
        let fs_detect_elapsed = fs_detect_start.elapsed();
        println!("[TIMER] Filesystem detection completed in {:.2}s", fs_detect_elapsed.as_secs_f64());

        // Filesystem detection for optimization

        // Log filesystem information
        if src_fs_info.fs_type != NetworkFsType::Local {
            println!("  Source filesystem: {:?} ({})", 
                src_fs_info.fs_type, src_fs_info.mount_point);
            if let Some(server) = &src_fs_info.server {
                println!("    Server: {}", server);
            }
        }
        
        if dst_fs_info.fs_type != NetworkFsType::Local {
            println!("  Destination filesystem: {:?} ({})", 
                dst_fs_info.fs_type, dst_fs_info.mount_point);
            if let Some(server) = &dst_fs_info.server {
                println!("    Server: {}", server);
            }
        }

        // Provide optimization recommendations
        if src_fs_info.fs_type != crate::network_fs::NetworkFsType::Local 
            || dst_fs_info.fs_type != crate::network_fs::NetworkFsType::Local {
            println!("Network filesystem detected - optimization recommendations:");
            
            if src_fs_info.fs_type != NetworkFsType::Local {
                let recommendations = fs_detector.get_optimization_recommendations(&src_fs_info);
                for rec in recommendations {
                    println!("  Source: {}", rec);
                }
            }
            
            if dst_fs_info.fs_type != NetworkFsType::Local {
                let recommendations = fs_detector.get_optimization_recommendations(&dst_fs_info);
                for rec in recommendations {
                    println!("  Destination: {}", rec);
                }
            }
        }

        // Check if source is a single file
        if source.is_file() {
            // Single file sync - call sync_single_file directly
            return self.sync_single_file(&source, &destination, &options);
        }

        // Go straight to mixed mode execution for directories
        // Show spinners during scanning, they'll be cleared before mixed strategy progress bar
        let operations =
            self.collect_operations_with_progress(&source, &destination, &options, true)?;

        // Check if confirmation is needed
        if options.confirm && !operations.is_empty() && !self.confirm_operations(&operations)? {
            println!("Operation cancelled by user.");
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

        // Create Hybrid Dam configuration based on filesystem detection
        let mut fs_detector = NetworkFsDetector::new();
        let src_fs_info = fs_detector.detect_filesystem(&source);
        let dst_fs_info = fs_detector.detect_filesystem(&destination);
        
        // Choose configuration based on network vs local filesystem
        let config = if src_fs_info.fs_type != NetworkFsType::Local || dst_fs_info.fs_type != NetworkFsType::Local {
            // Use network-optimized configuration
            let fs_info = if dst_fs_info.fs_type != NetworkFsType::Local {
                dst_fs_info
            } else {
                src_fs_info
            };
            HybridDamConfig::for_network(fs_info)
        } else {
            HybridDamConfig::for_local()
        };
        
        // Create buffer sizer
        let buffer_sizer = BufferSizer::new(&options);
        
        // Create Hybrid Dam executor
        let executor = HybridDam::new(config, buffer_sizer).with_progress(total_files, total_bytes);
        
        executor.execute(operations, &source, &destination, &options)
    }

    /// Synchronize using intelligent strategy selection
    pub fn synchronize_smart(
        &mut self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        // CRITICAL: Per Gemini's FINAL MANDATE - go DIRECTLY to streaming
        // NO profiling, NO scanning, NO analysis before streaming starts
        // This eliminates ALL startup latency
        return self.synchronize_hybrid_dam(source, destination, options);
        
        /* REMOVED: All blocking pre-analysis per Gemini's mandate
        // FAST PATH 1: Check for platform accelerators FIRST (before any analysis)
        if source.is_dir() && destination.parent().is_some() {
            // Check if source and destination are on the same volume for instant cloning
            if let Ok(source_meta) = source.metadata() {
                let dest_parent = destination.parent().unwrap_or(&destination);
                if let Ok(dest_meta) = dest_parent.metadata() {
                    // Check if same device/volume for reflink/clonefile opportunities
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        if source_meta.dev() == dest_meta.dev() {
                            // Same device - try reflink/clonefile first
                            if options.verbose >= 1 {
                                println!("⚡ Fast path: Same volume detected - attempting instant clone");
                            }
                            // Set reflink to always for this operation
                            let mut fast_options = options.clone();
                            fast_options.reflink = crate::reflink::ReflinkMode::Always;
                            
                            // Try the operation with reflink forced
                            match self.execute_with_reflink_priority(source.clone(), destination.clone(), fast_options) {
                                Ok(stats) => {
                                    if stats.reflinks_succeeded() > 0 {
                                        if options.verbose >= 1 {
                                            println!("✅ Fast clone completed: {} files cloned instantly", stats.reflinks_succeeded());
                                        }
                                        return Ok(stats);
                                    }
                                    // Reflink didn't work, continue with normal path
                                }
                                Err(_) => {} // Continue with normal path
                            }
                        }
                    }
                    
                    #[cfg(windows)]
                    {
                        // On Windows, check if files are on same drive for ReFS block cloning
                        // We can use a simple heuristic based on path roots
                        let source_root = source.components().next();
                        let dest_root = destination.components().next();
                        
                        if source_root == dest_root {
                            if options.verbose >= 1 {
                                println!("⚡ Fast path: Same volume detected - checking for ReFS block cloning");
                            }
                            // Try ReFS block cloning
                            let mut fast_options = options.clone();
                            fast_options.reflink = crate::reflink::ReflinkMode::Always;
                            
                            match self.execute_with_reflink_priority(source.clone(), destination.clone(), fast_options) {
                                Ok(stats) => {
                                    if stats.reflinks_succeeded() > 0 {
                                        if options.verbose >= 1 {
                                            println!("✅ ReFS block cloning completed: {} files cloned", stats.reflinks_succeeded());
                                        }
                                        return Ok(stats);
                                    }
                                }
                                Err(_) => {} // Continue with normal path
                            }
                        }
                    }
                }
            }
        }
        
        // FAST PATH 2: Single file copy - no analysis needed
        if source.is_file() {
            return self.handle_single_file_copy(&source, &destination, &options);
        }
        
        // FAST PATH 3: Tar streaming for known small file directories
        if !options.no_batch && source.is_dir() {
            // Force tar if requested
            if options.force_tar {
                if options.verbose >= 1 {
                    println!("🚀 Force-tar: Using tar streaming as requested");
                }
                let stats = SyncStats::new();
                return crate::streaming_batch_fast::fast_tar_transfer(&source, &destination, &stats, &options);
            }
            
            // Check for known tar candidates (node_modules, .git, etc)
            if crate::speculative_tar::is_known_tar_candidate(&source) {
                if options.verbose >= 1 {
                    println!("🚀 Fast path: Known small files pattern detected");
                }
                let stats = SyncStats::new();
                return crate::streaming_batch_fast::fast_tar_transfer(&source, &destination, &stats, &options);
            }
        }
        
        // INTELLIGENT SELECTION: Quick sampling to choose best strategy
        // Sample the source to make intelligent decision
        if source.is_dir() {
            if let Ok(profile) = self.quick_profile_directory(&source, 20) {
                // Choose strategy based on profile
                if profile.avg_file_size < 10_240 && profile.file_count > 50 {
                    // Many small files - use tar streaming
                    if options.verbose >= 1 {
                        println!("📊 Auto-selected: Tar streaming ({} small files detected)", profile.file_count);
                    }
                    let stats = SyncStats::new();
                    return crate::streaming_batch_fast::fast_tar_transfer(&source, &destination, &stats, &options);
                } else if profile.avg_file_size > 100_000_000 {
                    // Large files - NEVER force delta upfront!
                    // The mixed strategy will intelligently decide PER FILE
                    // based on whether the destination exists and needs updating
                    if options.verbose >= 1 {
                        println!("[{}] 📊 Auto-selected: Mixed strategy (large files detected)", timestamp());
                    }
                    // Fall through to mixed strategy which will:
                    // 1. Check each file individually
                    // 2. Use delta ONLY when destination exists AND file needs updating
                    // 3. Use fast copy for new files or when delta doesn't make sense
                }
                // Fall through to mixed mode for everything else
            }
        }
        
        // ALWAYS use Hybrid Dam with streaming walker to eliminate startup latency
        // This is now the default path per Gemini's mandate
        return self.synchronize_hybrid_dam(source, destination, options);
        */
    }

    /// Synchronize using Hybrid Dam three-tier architecture
    /// 
    /// This implements the Phase 2 Hybrid Dam system:
    /// - DAM: Small files (<1MB) batched and streamed
    /// - POOL: Medium files (1-100MB) parallel direct transfer
    /// - SLICER: Large files (>100MB) memory-mapped chunks + platform acceleration
    pub fn synchronize_hybrid_dam(
        &mut self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        let _start_time = Instant::now();
        
        if options.verbose >= 1 {
            // Internal: Using Hybrid Dam architecture
            if options.verbose >= 2 {
                println!("Using optimized streaming architecture");
            }
        }
        
        // Handle file-to-file copying
        if source.is_file() {
            return self.handle_single_file_copy(&source, &destination, &options);
        }
        
        // Detect network filesystem for configuration
        let mut fs_detector = NetworkFsDetector::new();
        let fs_info = fs_detector.detect_filesystem(&destination);
        
        // Configure Hybrid Dam based on network vs local
        let dam_config = if fs_info.fs_type != NetworkFsType::Local {
            if options.verbose >= 2 {
                println!("Network filesystem detected: {:?}, using network optimizations", fs_info.fs_type);
            }
            HybridDamConfig::for_network(fs_info.clone())
        } else {
            HybridDamConfig::for_local()
        };
        
        // Create buffer sizer
        let buffer_sizer = BufferSizer::new(&options);
        
        // Skip file operations collection - we'll use streaming walker instead
        // This eliminates the startup latency from exhaustive directory scanning
        
        // Use full Hybrid Dam implementation
        if options.verbose >= 1 {
            // Internal: Hybrid Dam enabled
            if options.verbose >= 2 {
                println!("Streaming file discovery enabled for reduced startup latency");
            }
        }
        
        // For streaming, we don't know the exact counts upfront
        // Use estimates or zero for progress tracking (will update as we go)
        let total_files = 0u64; // Will be updated during streaming
        let total_bytes = 0u64; // Will be updated during streaming

        // Initialize Hybrid Dam with progress tracking
        let mut hybrid_dam = HybridDam::new(dam_config, buffer_sizer).with_progress(total_files, total_bytes);
        
        // START SPINNER BEFORE STREAMING (per Gemini's mandate)
        // This provides immediate feedback to the user
        // ALWAYS show spinner - this is critical for user feedback
        use crate::progress_display::ProgressDisplay;
        let progress = ProgressDisplay::new();
        progress.set_message("Discovering files and starting synchronization...");
        
        // Use streaming execution to eliminate startup latency
        // This processes files as they're discovered instead of waiting for full scan
        // The progress spinner is already showing status - no need for duplicate output
        let stats = hybrid_dam.execute_streaming(&source, &destination, &options, Some(progress.clone()))?;
        progress.finish_and_clear();
        
        Ok(stats)
    }

    /// Synchronize with strategy analysis (for forced strategies other than mixed)

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
        let multi_progress = if show_progress && options.show_progress {
            Some(MultiProgress::new())
        } else {
            None
        };

        // Show progress during source enumeration
        let source_files = if let Some(ref mp) = multi_progress {
            let source_pb = mp.add(ProgressBar::new_spinner());
            source_pb.set_style(ProgressStyle::default_spinner()
                .template("{spinner:.green} Scanning source: {msg}")
                .expect("Failed to set progress style"));
            source_pb.enable_steady_tick(std::time::Duration::from_millis(100));

            // Enumerating source files
            let source_pb_clone = source_pb.clone();
            let file_count = Arc::new(AtomicUsize::new(0));
            let file_count_clone = Arc::clone(&file_count);
            
            // Create a generator with progress tracking
            let gen_with_progress = generator.clone().with_progress_interval(1000); // Update every 1000 files
            
            // Start a thread to update the progress bar
            let progress_handle = std::thread::spawn(move || {
                loop {
                    let count = file_count_clone.load(Ordering::Relaxed);
                    source_pb_clone.set_message(format!("{} files found...", count));
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if source_pb_clone.is_finished() {
                        break;
                    }
                }
            });
            
            let files = gen_with_progress.generate_file_list_with_counter(source, options, Arc::clone(&file_count))?;
            source_pb.finish_with_message(format!("     ✅ Found {} source files", files.len()));
            let _ = progress_handle.join();

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
                dest_pb.set_style(ProgressStyle::default_spinner()
                    .template("{spinner:.yellow} Scanning destination: {msg}")
                    .expect("Failed to set progress style"));
                dest_pb.enable_steady_tick(std::time::Duration::from_millis(100));

                // Enumerating destination files
                let dest_pb_clone = dest_pb.clone();
                let dest_file_count = Arc::new(AtomicUsize::new(0));
                let dest_file_count_clone = Arc::clone(&dest_file_count);
                
                // Create a generator with progress tracking
                let gen_with_progress = generator.clone().with_progress_interval(1000); // Update every 1000 files
                
                // Start a thread to update the progress bar
                let progress_handle = std::thread::spawn(move || {
                    loop {
                        let count = dest_file_count_clone.load(Ordering::Relaxed);
                        dest_pb_clone.set_message(format!("{} files found...", count));
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        if dest_pb_clone.is_finished() {
                            break;
                        }
                    }
                });
                
                let files = gen_with_progress.generate_file_list_with_counter(destination, options, Arc::clone(&dest_file_count))?;
                dest_pb.finish_with_message(format!(
                    "     ✅ Found {} destination files",
                    files.len()
                ));
                let _ = progress_handle.join();
                files
            } else {
                generator.generate_file_list(destination, options)?
            }
        } else {
            if let Some(ref mp) = multi_progress {
                let _ = mp.println("     📁 Destination directory doesn't exist - will be created");
            } else if show_progress && options.show_progress {
                println!("📁 Destination directory doesn't exist - will be created");
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
        &mut self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        let _start_time = Instant::now();

        println!("Starting parallel synchronization...");
        println!("Source: {}", source.display());
        println!("Destination: {}", destination.display());
        println!("Threads: {}", self.config.worker_threads);

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
        &mut self,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        let start_time = Instant::now();
        let mut logger = SyncLogger::new(options.log_file.as_deref(), options.show_eta, options.verbose)?;
        logger.initialize_progress(1, std::fs::metadata(source)?.len());

        let dest_path = if destination.exists() && destination.is_dir() {
            let file_name = source
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Source file has no name"))?;
            destination.join(file_name)
        } else {
            destination.to_path_buf()
        };

        let mut fs_detector = NetworkFsDetector::new();
        let stats = self.sync_file_pair(source, &dest_path, options, &mut fs_detector)?;
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
        let mut logger = SyncLogger::new(options.log_file.as_deref(), options.show_eta, options.verbose)?;

        // Create MultiProgress for analysis phase - always use for scanning progress
        let multi_progress = if !options.show_progress {
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
        let mut operations = if options.show_progress {
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
            if options.show_progress {
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
        let progress = if !options.show_progress || options.verbose >= 2 {
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
        // Use the global thread pool (Phase 1 optimization - eliminates 27ms spawn overhead)
        let pool = worker_pool::global_pool();
        let effective_threads = pool.current_num_threads();
        
        println!("DEBUG: Using {} threads from global pool for parallel operations", effective_threads);

        // Separate operations by type for optimal ordering
        let (dir_ops, file_ops): (Vec<_>, Vec<_>) = operations
            .into_iter()
            .partition(|op| matches!(op, FileOperation::CreateDirectory { .. }));

        // Separate delete operations to run last
        let (file_ops, delete_ops): (Vec<_>, Vec<_>) = file_ops
            .into_iter()
            .partition(|op| !matches!(op, FileOperation::Delete { .. }));

        // Create directories first using parallel creation
        if !dir_ops.is_empty() {
            println!("DEBUG: Creating {} directories in parallel...", dir_ops.len());
            if options.verbose >= 1 {
                logger.log(&format!("Creating {} directories in parallel...", dir_ops.len()));
            }
            
            // Extract directory paths from operations
            let dir_paths: Vec<PathBuf> = dir_ops.iter()
                .filter_map(|op| match op {
                    FileOperation::CreateDirectory { path } => {
                        Some(self.map_source_to_dest(path, source, destination).ok()?)
                    }
                    _ => None,
                })
                .collect();
            
            // Create progress bar for directory creation
            let dir_progress = if let Some(ref mp) = multi_progress {
                let pb = mp.add(indicatif::ProgressBar::new(dir_paths.len() as u64));
                pb.set_style(
                    indicatif::ProgressStyle::default_bar()
                        .template("{spinner:.green} Creating directories: {pos}/{len} {bar:40.cyan/blue} {msg}")
                        .unwrap()
                        .progress_chars("=>-"),
                );
                Some(pb)
            } else {
                None
            };
            
            // Use parallel directory creator
            let dir_creator = ParallelDirCreator::new();
            let (successes, errors) = dir_creator.create_directories(dir_paths, dir_progress.as_ref())?;
            
            // Update stats - count directories as processed files
            for _ in 0..successes.len() {
                stats.increment_files_processed();
            }
            
            // Report errors if any
            if !errors.is_empty() {
                for (path, err) in &errors {
                    logger.log(&format!("Failed to create directory {}: {}", path.display(), err));
                }
                if options.verbose >= 1 {
                    logger.log(&format!(
                        "Created {} directories successfully, {} errors",
                        successes.len(),
                        errors.len()
                    ));
                }
            } else if options.verbose >= 1 {
                logger.log(&format!("Created {} directories successfully", successes.len()));
            }
            
            // Update overall progress
            logger.update_progress(dir_ops.len() as u64, 0);
            if let Some(ref progress) = progress {
                if let Ok(mut p) = progress.lock() {
                    for _ in 0..dir_ops.len() {
                        p.update_file_complete(0);
                    }
                }
            }
            
            // Remove progress bar
            if let Some(pb) = dir_progress {
                pb.finish_and_clear();
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
            println!("DEBUG: Starting directory pre-creation for {small_files_count} small files");
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
                use io_uring::{opcode, types, IoUring};
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
                                                if verbose >= 2 {
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
                                                        crate::windows_symlinks::create_symlink(&dest_path, &_target)
                                                            .with_context(|| {
                                                                format!(
                                                                    "Failed to create symlink: {} -> {}",
                                                                    dest_path.display(),
                                                                    _target.display()
                                                                )
                                                            })?;
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
                                            if verbose >= 2 {
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
                                                    if verbose >= 2 {
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
                                            if verbose >= 2 {
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
                                            if verbose >= 2 {
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
                                                    if verbose >= 2 {
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
                                            if verbose >= 2 {
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

                if options.verbose >= 2 {
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
                let reflink_options = ReflinkOptions {
                    mode: options.reflink,
                };
                let bytes_copied = copy_file_with_metadata_and_reflink(
                    &path,
                    &dest_path,
                    &copy_flags,
                    &reflink_options,
                    Some(&stats),
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
                let reflink_options = ReflinkOptions {
                    mode: options.reflink,
                };
                let bytes_copied = copy_file_with_metadata_and_reflink(
                    &path,
                    &dest_path,
                    &copy_flags,
                    &reflink_options,
                    Some(&stats),
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

    /// Synchronize a single file pair (source to destination)
    fn sync_file_pair(
        &self,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
        fs_detector: &mut NetworkFsDetector,
    ) -> Result<SyncStats> {
        let stats = SyncStats::default();
        let copy_flags = CopyFlags::from_string(&options.copy_flags);
        let reflink_options = ReflinkOptions { mode: options.reflink };

        // Create parent directory if needed
        if let Some(parent) = destination.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let dst_fs_info = fs_detector.detect_filesystem(destination);

        // Check if compression is enabled and destination is a network filesystem
        if options.compress && dst_fs_info.fs_type != NetworkFsType::Local {
            if options.verbose >= 1 {
                println!("Compressing {} to network destination {}", source.display(), destination.display());
            }
            // Open source and destination files
            let mut source_file = BufReader::new(fs::File::open(source)?);
            let mut dest_file = BufWriter::new(fs::File::create(destination)?);

            // Compress and transfer
            let compressor = StreamingCompressor::new(options.compression_config);
            let bytes_written = compressor.compress_stream(&mut source_file, &mut dest_file)?;
            
            stats.increment_files_copied();
            stats.add_bytes_transferred(bytes_written);
        } else {
            // Copy the file with all optimizations (no compression or local destination)
            match copy_file_with_metadata_and_reflink(source, destination, &copy_flags, &reflink_options, Some(&stats)) {
                Ok(bytes_copied) => {
                    stats.increment_files_copied();
                    stats.add_bytes_transferred(bytes_copied);
                }
                Err(e) => {
                    stats.increment_errors();
                    return Err(e);
                }
            }
        }

        Ok(stats)
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
            // Use our comprehensive Windows symlink implementation
            crate::windows_symlinks::create_symlink(destination, target)?;
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
