//! Intelligent copy strategy selection and execution
//!
//! This module implements the smart decision engine that chooses the optimal
//! copying method based on file characteristics, platform, and operation type.

use std::path::Path;
use std::process::Command;

use crate::file_list::FileInfo;
use crate::options::SyncOptions;

/// Represents different copy strategies available
#[derive(Debug, Clone)]
pub enum CopyStrategy {
    /// Use native rsync command (Unix)
    NativeRsync {
        extra_args: Vec<String>,
    },
    /// Use native robocopy command (Windows)
    NativeRobocopy {
        extra_args: Vec<String>,
    },
    /// Use platform-specific APIs for optimal performance
    PlatformApi {
        method: PlatformMethod,
    },
    /// Use our custom delta transfer algorithm
    DeltaTransfer {
        block_size: usize,
    },
    /// Use our parallel copy implementation
    ParallelCustom {
        threads: usize,
    },
    /// Use io_uring on Linux for batch operations
    #[cfg(target_os = "linux")]
    IoUringBatch {
        batch_size: usize,
    },
    /// Mixed mode - uses different strategies for different file types
    MixedMode,
}

/// Platform-specific copy methods
#[derive(Debug, Clone)]
pub enum PlatformMethod {
    /// Windows CopyFileEx API
    #[cfg(target_os = "windows")]
    WindowsCopyFileEx,
    /// Linux copy_file_range/sendfile
    #[cfg(target_os = "linux")]
    LinuxCopyFileRange,
    /// macOS copyfile/clonefile
    #[cfg(target_os = "macos")]
    MacOSCopyFile,
    /// Generic fallback
    StandardCopy,
}

/// Statistics about files to be copied
#[derive(Debug, Default)]
pub struct FileStats {
    pub total_files: usize,
    pub total_size: u64,
    pub avg_size: u64,
    pub small_files: usize,  // < 64KB
    pub medium_files: usize, // 64KB - 10MB
    pub large_files: usize,  // > 10MB
    pub updates: usize,      // Files that exist in destination
    pub creates: usize,      // New files
    pub directories: usize,
}

impl FileStats {
    /// Analyze a list of file operations to gather statistics
    pub fn from_operations(operations: &[FileInfo]) -> Self {
        let mut stats = FileStats::default();
        
        for op in operations {
            stats.total_files += 1;
            stats.total_size += op.size;
            
            match op.size {
                0..=65536 => stats.small_files += 1,
                65537..=10485760 => stats.medium_files += 1,
                _ => stats.large_files += 1,
            }
            
            if op.is_directory {
                stats.directories += 1;
            }
            
            // TODO: Determine if update or create based on destination existence
            stats.creates += 1;
        }
        
        if stats.total_files > 0 {
            stats.avg_size = stats.total_size / stats.total_files as u64;
        }
        
        stats
    }
}

/// Strategy selector that chooses the optimal copy method
pub struct StrategySelector {
    force_strategy: Option<CopyStrategy>,
    available_tools: AvailableTools,
}

#[derive(Debug, Default)]
struct AvailableTools {
    has_rsync: bool,
    has_robocopy: bool,
    rsync_version: Option<String>,
    robocopy_version: Option<String>,
}

impl StrategySelector {
    /// Create a new strategy selector
    pub fn new() -> Self {
        let available_tools = Self::detect_available_tools();
        
        Self {
            force_strategy: None,
            available_tools,
        }
    }
    
    /// Force a specific strategy (useful for testing or user override)
    pub fn force_strategy(mut self, strategy: CopyStrategy) -> Self {
        self.force_strategy = Some(strategy);
        self
    }
    
    /// Detect which native tools are available on the system
    fn detect_available_tools() -> AvailableTools {
        let mut tools = AvailableTools::default();
        
        // Check for rsync
        #[cfg(unix)]
        {
            if let Ok(output) = Command::new("rsync").arg("--version").output() {
                tools.has_rsync = output.status.success();
                if tools.has_rsync {
                    if let Ok(version) = String::from_utf8(output.stdout) {
                        tools.rsync_version = version.lines().next().map(String::from);
                    }
                }
            }
        }
        
        // Check for robocopy
        #[cfg(target_os = "windows")]
        {
            if let Ok(output) = Command::new("robocopy").arg("/?").output() {
                tools.has_robocopy = output.status.success();
                if tools.has_robocopy {
                    tools.robocopy_version = Some("Built-in Windows tool".to_string());
                }
            }
        }
        
        tools
    }
    
    /// Choose the optimal strategy based on file statistics and operation type
    pub fn choose_strategy(
        &self,
        stats: &FileStats,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
    ) -> CopyStrategy {
        // If strategy is forced, use it
        if let Some(ref strategy) = self.force_strategy {
            return strategy.clone();
        }
        
        // Determine if this is a local or network operation
        let is_network = is_network_path(source) || is_network_path(destination);
        
        // Decision tree for strategy selection
        match (stats.total_files, stats.avg_size, is_network) {
            // Large diverse file set - use mixed mode for optimal performance
            (1000.., _, false) if stats.small_files > 100 && stats.large_files > 0 => {
                CopyStrategy::MixedMode
            }
            
            // Medium diverse file set - also use mixed mode
            (100.., _, false) if stats.small_files > 50 && (stats.medium_files > 10 || stats.large_files > 0) => {
                CopyStrategy::MixedMode
            }
            
            // Thousands of small files locally - use mixed mode for best performance
            (1000.., 0..=65536, false) => {
                CopyStrategy::MixedMode
            }
            
            // Large files with updates - use delta transfer
            (_, 10485761.., _) if stats.updates > 0 && options.checksum => {
                CopyStrategy::DeltaTransfer {
                    block_size: self.optimal_block_size(stats.avg_size),
                }
            }
            
            // Network operations with large files - use mixed mode
            (_, 1048576.., true) => {
                CopyStrategy::MixedMode
            }
            
            // Linux with many medium files - use mixed mode
            #[cfg(target_os = "linux")]
            (100.., 65537..=10485760, false) if options.linux_optimized => {
                CopyStrategy::MixedMode
            }
            
            // Default to mixed mode for everything else
            _ => CopyStrategy::MixedMode,
        }
    }
    
    /// Get the platform-specific API strategy
    pub fn platform_api_strategy(&self) -> CopyStrategy {
        CopyStrategy::PlatformApi {
            method: {
                #[cfg(target_os = "windows")]
                { PlatformMethod::WindowsCopyFileEx }
                
                #[cfg(target_os = "linux")]
                { PlatformMethod::LinuxCopyFileRange }
                
                #[cfg(target_os = "macos")]
                { PlatformMethod::MacOSCopyFile }
                
                #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
                { PlatformMethod::StandardCopy }
            },
        }
    }
    
    /// Build rsync arguments from our options
    pub fn build_rsync_args(&self, options: &SyncOptions) -> Vec<String> {
        let mut args = vec!["-a".to_string()]; // Archive mode
        
        if options.verbose > 0 {
            args.push("-v".to_string());
        }
        
        if options.dry_run {
            args.push("-n".to_string());
        }
        
        if options.compress {
            args.push("-z".to_string());
        }
        
        if options.checksum {
            args.push("-c".to_string());
        }
        
        if options.purge {
            args.push("--delete".to_string());
        }
        
        // Add exclude patterns
        for pattern in &options.exclude_files {
            args.push(format!("--exclude={}", pattern));
        }
        
        for pattern in &options.exclude_dirs {
            args.push(format!("--exclude={}/", pattern));
        }
        
        args
    }
    
    /// Build robocopy arguments from our options
    pub fn build_robocopy_args(&self, options: &SyncOptions) -> Vec<String> {
        let mut args = Vec::new();
        
        if options.mirror {
            args.push("/MIR".to_string());
        } else if options.recursive {
            args.push("/E".to_string());
        }
        
        if options.verbose > 0 {
            args.push("/V".to_string());
        }
        
        if options.dry_run {
            args.push("/L".to_string());
        }
        
        if options.move_files {
            args.push("/MOV".to_string());
        }
        
        // Copy flags
        args.push(format!("/COPY:{}", options.copy_flags));
        
        // Retry settings
        if options.retry_count > 0 {
            args.push(format!("/R:{}", options.retry_count));
            args.push(format!("/W:{}", options.retry_wait));
        }
        
        // Multi-threading
        args.push(format!("/MT:{}", num_cpus::get()));
        
        // Exclude patterns
        for pattern in &options.exclude_files {
            args.push("/XF".to_string());
            args.push(pattern.clone());
        }
        
        for pattern in &options.exclude_dirs {
            args.push("/XD".to_string());
            args.push(pattern.clone());
        }
        
        args
    }
    
    /// Determine optimal block size for delta transfer
    pub fn optimal_block_size(&self, avg_file_size: u64) -> usize {
        match avg_file_size {
            0..=1048576 => 1024,        // 1KB blocks for files up to 1MB
            1048577..=10485760 => 4096, // 4KB blocks for files up to 10MB
            10485761..=104857600 => 16384, // 16KB blocks for files up to 100MB
            _ => 65536,                 // 64KB blocks for larger files
        }
    }
    
    /// Determine optimal thread count based on operation type
    pub fn optimal_thread_count(&self, is_network: bool) -> usize {
        let cpu_count = num_cpus::get();
        
        if is_network {
            // More threads for network operations to hide latency
            (cpu_count * 2).min(32)
        } else {
            // Fewer threads for local disk to avoid contention
            cpu_count.min(8)
        }
    }
    
    /// Get a description of the chosen strategy
    pub fn describe_strategy(&self, strategy: &CopyStrategy) -> String {
        match strategy {
            CopyStrategy::NativeRsync { .. } => "Mixed mode".to_string(), // Shouldn't happen anymore
            CopyStrategy::NativeRobocopy { .. } => "Mixed mode".to_string(), // Shouldn't happen anymore
            CopyStrategy::PlatformApi { method } => match method {
                #[cfg(target_os = "windows")]
                PlatformMethod::WindowsCopyFileEx => "Windows CopyFileEx API".to_string(),
                #[cfg(target_os = "linux")]
                PlatformMethod::LinuxCopyFileRange => "Linux copy_file_range".to_string(),
                #[cfg(target_os = "macos")]
                PlatformMethod::MacOSCopyFile => "macOS copyfile API".to_string(),
                PlatformMethod::StandardCopy => "Standard file copy".to_string(),
            },
            CopyStrategy::DeltaTransfer { block_size } => {
                format!("Delta transfer ({}KB blocks)", block_size / 1024)
            }
            CopyStrategy::ParallelCustom { .. } => "Mixed mode".to_string(),
            #[cfg(target_os = "linux")]
            CopyStrategy::IoUringBatch { .. } => "Mixed mode".to_string(),
            CopyStrategy::MixedMode => "Mixed mode".to_string(),
        }
    }
}

/// Check if a path is a network location
pub fn is_network_path(path: &Path) -> bool {
    if let Some(path_str) = path.to_str() {
        // Windows UNC paths
        if path_str.starts_with("\\\\") {
            return true;
        }
        
        // Unix network mounts (common patterns)
        if path_str.starts_with("/mnt/") || path_str.starts_with("/media/") {
            // This is a heuristic - could be improved
            return path_str.contains("smb") || path_str.contains("nfs") || path_str.contains("cifs");
        }
    }
    
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_strategy_selection_small_files() {
        let selector = StrategySelector::new();
        let stats = FileStats {
            total_files: 5000,
            avg_size: 1024,
            small_files: 5000,
            ..Default::default()
        };
        
        let strategy = selector.choose_strategy(
            &stats,
            Path::new("/tmp/src"),
            Path::new("/tmp/dst"),
            &SyncOptions::default(),
        );
        
        // Should choose native tools for many small files
        #[cfg(unix)]
        assert!(matches!(strategy, CopyStrategy::NativeRsync { .. }));
    }
    
    #[test]
    fn test_strategy_selection_large_files() {
        let selector = StrategySelector::new();
        let stats = FileStats {
            total_files: 10,
            avg_size: 100 * 1024 * 1024, // 100MB average
            large_files: 10,
            updates: 5,
            ..Default::default()
        };
        
        let mut options = SyncOptions::default();
        options.checksum = true;
        
        let strategy = selector.choose_strategy(
            &stats,
            Path::new("/tmp/src"),
            Path::new("/tmp/dst"),
            &options,
        );
        
        // Should choose delta transfer for large files with updates
        assert!(matches!(strategy, CopyStrategy::DeltaTransfer { .. }));
    }
}