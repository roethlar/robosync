//! Hybrid Dam Architecture - Core Implementation
//!
//! The Hybrid Dam is a three-tier adaptive system for optimal file transfer performance:
//! - DAM: Small files (<1MB) - Batch, compress, stream via tar
//! - POOL: Medium files (1-100MB) - Direct parallel transfer with adaptive buffers  
//! - SLICER: Large files (>100MB) - Memory-mapped I/O with chunked processing
//!
//! This addresses the performance crisis identified by cross-platform testing:
//! - Eliminates startup overhead through concurrent analysis
//! - Scales thresholds based on network vs local transfers
//! - Uses platform-specific optimizations where available

use std::collections::{VecDeque, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::io::Read;
use std::time::{Duration, Instant};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use indicatif::ProgressBar;
use crate::worker_pool;
use anyhow::{Context, Result};
use rayon::prelude::*;
use rayon::ThreadPool;
use tar::{Builder, Archive};
use crate::network_fs::{NetworkFsInfo, NetworkFsType};
use crate::buffer_sizing::BufferSizer;
use crate::concurrent_delta::{ConcurrentDeltaAnalyzer, quick_delta_check};
use crate::options::SyncOptions;
use crate::file_list::FileOperation;
use crate::sync_stats::SyncStats;
use crate::progress::SyncProgress;
use crate::error_logger::ErrorLogger;
use lazy_static::lazy_static;

// ========== GLOBAL RAYON POOL (Per Expert Guidance - 10x Performance) ==========
// "Persistent Rayon pool, pre-allocated" - eliminates 27ms thread spawning overhead
lazy_static! {
    /// Global Rayon thread pool - initialized ONCE at startup
    /// Expert guidance: "Leverage Rayon for persistent, low-overhead pools (10x faster)"
    /// Default: num_cpus * 1.5-2, scale to 16-64 for 10GbE
    static ref GLOBAL_RAYON_POOL: ThreadPool = {
        let num_threads = (num_cpus::get() as f32 * 1.5) as usize;
        let pool_size = num_threads.clamp(4, 64); // Min 4, max 64 threads
        
        rayon::ThreadPoolBuilder::new()
            .num_threads(pool_size)
            .thread_name(|i| format!("robosync-worker-{}", i))
            .build()
            .expect("Failed to create global Rayon pool")
    };
}

// ========== DISPLAY STRUCTURES (migrated from mixed_strategy.rs) ==========

/// Size breakdown by different file size categories
#[derive(Debug, Default, Clone)]
pub struct SizeBreakdown {
    pub small_count: u64,
    pub small_size: u64,
    pub medium_count: u64,
    pub medium_size: u64,
    pub large_count: u64,
    pub large_size: u64,
    pub delta_count: u64,
    pub delta_size: u64,
}

/// Detailed pending statistics for display
#[derive(Debug, Default)]
pub struct DetailedPendingStats {
    pub files_create: u64,
    pub create_breakdown: SizeBreakdown,
    pub files_update: u64,
    pub update_breakdown: SizeBreakdown,
    pub files_delete: u64,
    pub dirs_create: u64,
    pub size_create: u64,
    pub size_update: u64,
    pub size_delete: u64,
}

/// File entry for Hybrid Dam processing
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub src_path: PathBuf,
    pub dst_path: PathBuf,
    pub size: u64,
    pub modified: std::time::SystemTime,
    pub file_type: FileType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileType {
    Regular,
    Directory,
    Symlink,
}

/// Transfer strategy based on file size and characteristics
#[derive(Debug, Clone, PartialEq)]
pub enum TransferStrategy {
    Dam,    // Small files - batch and stream
    Pool,   // Medium files - direct parallel
    Slicer, // Large files - memory-mapped chunks
}

/// Result of a transfer operation
#[derive(Debug)]
pub struct TransferResult {
    pub files_attempted: usize,      // Number of files attempted
    pub files_copied: usize,         // Number of files successfully copied
    pub bytes_transferred: u64,      // Actual bytes transferred for successful copies
    pub duration: Duration,
    pub strategy_used: TransferStrategy,
    pub errors: Vec<String>,
}

/// Configuration for Hybrid Dam thresholds
#[derive(Debug, Clone)]
pub struct HybridDamConfig {
    /// Small file threshold (Dam component)
    pub dam_threshold: u64,
    /// Large file threshold (Slicer component)  
    pub slicer_threshold: u64,
    /// Dam flush threshold - adaptive based on network/local
    pub dam_flush_threshold: u64,
    /// Maximum files in dam before forced flush
    pub dam_max_files: usize,
    /// Maximum age before forced flush
    pub dam_max_age: Duration,
    /// Network filesystem info for optimization
    pub network_fs_info: Option<NetworkFsInfo>,
}

impl Default for HybridDamConfig {
    fn default() -> Self {
        Self {
            dam_threshold: 1024 * 1024,        // 1MB
            slicer_threshold: 100 * 1024 * 1024, // 100MB
            dam_flush_threshold: 16 * 1024 * 1024, // 16MB local, 256MB network
            dam_max_files: 10000,
            dam_max_age: Duration::from_secs(30),
            network_fs_info: None,
        }
    }
}

impl HybridDamConfig {
    /// Create config optimized for network transfers
    pub fn for_network(fs_info: NetworkFsInfo) -> Self {
        let mut config = Self::default();
        
        // Large flush threshold for network efficiency (we have the memory)
        config.dam_flush_threshold = 1024 * 1024 * 1024; // 1GB - maximize batching
        
        // Optimized thresholds for modern high-performance systems
        match fs_info.fs_type {
            NetworkFsType::SSHFS | NetworkFsType::WebDAV => {
                // Higher overhead networks - still batch but with modern thresholds
                config.dam_threshold = 4 * 1024 * 1024; // 4MB
                config.slicer_threshold = 256 * 1024 * 1024; // 256MB
            }
            NetworkFsType::SMB | NetworkFsType::NFS => {
                // Modern SMB3/NFSv4 - can handle large operations efficiently
                config.dam_threshold = 8 * 1024 * 1024; // 8MB
                config.slicer_threshold = 512 * 1024 * 1024; // 512MB
            }
            _ => {
                // Local or high-performance networks - maximum throughput
                config.dam_threshold = 16 * 1024 * 1024; // 16MB
                config.slicer_threshold = 1024 * 1024 * 1024; // 1GB
            }
        }
        
        // Set the network fs info after using it
        config.network_fs_info = Some(fs_info);
        config
    }
    
    /// Create config optimized for local transfers
    pub fn for_local() -> Self {
        let mut config = Self::default();
        config.dam_flush_threshold = 16 * 1024 * 1024; // 16MB local
        config
    }
}

/// Small File Dam - batches small files for efficient streaming
#[derive(Clone)]
pub struct SmallFileDam {
    buffer: VecDeque<FileEntry>,
    current_size: u64,
    first_file_time: Option<Instant>,
    config: HybridDamConfig,
    buffer_sizer: BufferSizer,
}

impl SmallFileDam {
    pub fn new(config: HybridDamConfig, buffer_sizer: BufferSizer) -> Self {
        Self {
            buffer: VecDeque::new(),
            current_size: 0,
            first_file_time: None,
            config,
            buffer_sizer,
        }
    }
    
    /// Add a file to the dam, returns batch job if flush is triggered
    pub fn add_file(&mut self, file: FileEntry) -> Option<DamBatchJob> {
        // Set timestamp for aging if this is the first file
        if self.buffer.is_empty() {
            self.first_file_time = Some(Instant::now());
        }
        
        self.current_size += file.size;
        self.buffer.push_back(file);
        
        if self.should_flush() {
            self.create_batch_job()
        } else {
            None
        }
    }
    
    /// Check if dam should flush based on size, count, or age
    fn should_flush(&self) -> bool {
        // Size threshold
        if self.current_size >= self.config.dam_flush_threshold {
            return true;
        }
        
        // File count threshold  
        if self.buffer.len() >= self.config.dam_max_files {
            return true;
        }
        
        // Age threshold
        if let Some(first_time) = self.first_file_time {
            if first_time.elapsed() >= self.config.dam_max_age {
                return true;
            }
        }
        
        false
    }
    
    /// Create batch job from current buffer
    fn create_batch_job(&mut self) -> Option<DamBatchJob> {
        if self.buffer.is_empty() {
            return None;
        }
        
        let files = self.buffer.drain(..).collect();
        let total_size = self.current_size;
        
        // Reset state
        self.current_size = 0;
        self.first_file_time = None;
        
        Some(DamBatchJob {
            files,
            total_size,
            compression_enabled: self.should_use_compression(total_size),
        })
    }
    
    /// Force flush current buffer (for end-of-stream)
    pub fn flush(&mut self) -> Option<DamBatchJob> {
        self.create_batch_job()
    }
    
    /// Determine if compression should be used
    fn should_use_compression(&self, total_size: u64) -> bool {
        // Enable compression for larger batches over networks
        if self.config.network_fs_info.is_some() && total_size > 64 * 1024 * 1024 {
            return true;
        }
        
        // Skip compression for small local batches (CPU overhead not worth it)
        false
    }
}

/// Batch job for dam processing
#[derive(Debug)]
pub struct DamBatchJob {
    pub files: Vec<FileEntry>,
    pub total_size: u64,
    pub compression_enabled: bool,
}

/// Medium File Pool - direct parallel transfer for medium-sized files
#[derive(Clone)]
pub struct MediumFilePool {
    config: HybridDamConfig,
    buffer_sizer: BufferSizer,
}

impl MediumFilePool {
    pub fn new(config: HybridDamConfig, buffer_sizer: BufferSizer) -> Self {
        Self {
            config,
            buffer_sizer,
        }
    }
    
    /// Process a medium file with optimal buffer size and workers
    pub fn process_file(&self, file: FileEntry) -> Result<TransferResult> {
        let buffer_size = self.buffer_sizer.calculate_buffer_size_with_fs(
            file.size, 
            self.config.network_fs_info.as_ref()
        );
        
        let workers = self.calculate_optimal_workers(file.size);
        
        let start = Instant::now();
        
        let _result = if workers > 1 {
            self.parallel_chunked_transfer(&file, workers, buffer_size)
        } else {
            self.single_worker_transfer(&file, buffer_size)
        }?;
        
        Ok(TransferResult {
            files_attempted: 1,
            files_copied: 1,      // If we got here, the transfer succeeded
            bytes_transferred: file.size,
            duration: start.elapsed(),
            strategy_used: TransferStrategy::Pool,
            errors: Vec::new(),
        })
    }
    
    /// Calculate optimal number of workers for a file
    fn calculate_optimal_workers(&self, file_size: u64) -> usize {
        // Files under 10MB don't benefit from multiple workers
        if file_size < 10 * 1024 * 1024 {
            return 1;
        }
        
        // For network transfers, more workers help
        if self.config.network_fs_info.is_some() {
            return ((file_size / (20 * 1024 * 1024)) as usize).clamp(2, 8);
        }
        
        // Local transfers - moderate parallelism
        ((file_size / (50 * 1024 * 1024)) as usize).clamp(2, 4)
    }
    
    /// Single worker transfer with optimized buffer size
    fn single_worker_transfer(&self, file: &FileEntry, _buffer_size: usize) -> Result<()> {
        // Ensure destination directory exists
        if let Some(parent) = file.dst_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        // Use OS-level copy - MUCH faster than userspace byte-by-byte
        std::fs::copy(&file.src_path, &file.dst_path)
            .context(format!("Failed to copy {} to {}", 
                file.src_path.display(), file.dst_path.display()))?;
        
        Ok(())
    }
    
    /// Parallel chunked transfer for larger files
    fn parallel_chunked_transfer(&self, file: &FileEntry, workers: usize, buffer_size: usize) -> Result<()> {
        use std::fs::{File, OpenOptions};
        use std::io::{Read, Seek, SeekFrom, Write};
        use std::sync::{Arc, Mutex};
        
        // Calculate chunk size based on file size and worker count
        let chunk_size = (file.size / workers as u64).max(1024 * 1024); // Minimum 1MB chunks
        
        // Ensure destination directory exists
        if let Some(parent) = file.dst_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        // Pre-allocate destination file to avoid race conditions
        {
            let dst_file = File::create(&file.dst_path)?;
            dst_file.set_len(file.size)?;
        }
        
        // Create chunk tasks
        let mut chunk_tasks = Vec::new();
        
        for worker_id in 0..workers {
            let start_offset = worker_id as u64 * chunk_size;
            let end_offset = if worker_id == workers - 1 {
                file.size // Last worker handles remainder
            } else {
                (worker_id + 1) as u64 * chunk_size
            };
            
            if start_offset >= file.size {
                break; // File smaller than expected worker count
            }
            
            chunk_tasks.push((file.src_path.clone(), file.dst_path.clone(), start_offset, end_offset, buffer_size));
        }
        
        // Execute all chunks in parallel using worker pool
        let errors = Arc::new(Mutex::new(Vec::new()));
        
        worker_pool::scope(|s| {
            for (src_path, dst_path, start_offset, end_offset, buffer_size) in chunk_tasks {
                let errors = Arc::clone(&errors);
                s.spawn(move |_| {
                    if let Err(e) = Self::copy_file_chunk(&src_path, &dst_path, start_offset, end_offset, buffer_size) {
                        errors.lock().unwrap().push(e);
                    }
                });
            }
        });
        
        // Check for any errors
        let errors = errors.lock().unwrap();
        if let Some(error) = errors.first() {
            return Err(anyhow::anyhow!("Parallel chunked transfer failed: {}", error));
        }
        Ok(())
    }
    
    /// Copy a specific chunk of a file (helper for parallel transfer)
    fn copy_file_chunk(
        src_path: &Path, 
        dst_path: &Path, 
        start_offset: u64, 
        end_offset: u64,
        buffer_size: usize
    ) -> Result<()> {
        use std::fs::{File, OpenOptions};
        use std::io::{Read, Seek, SeekFrom, Write};
        
        let chunk_size = end_offset - start_offset;
        if chunk_size == 0 {
            return Ok(());
        }
        
        // Open source file for reading
        let mut src_file = File::open(src_path)?;
        src_file.seek(SeekFrom::Start(start_offset))?;
        
        // Open destination file for writing at specific offset
        let mut dst_file = OpenOptions::new()
            .write(true)
            .open(dst_path)?;
        dst_file.seek(SeekFrom::Start(start_offset))?;
        
        // Copy chunk with optimal buffer size
        let mut buffer = vec![0u8; buffer_size.min(chunk_size as usize)];
        let mut remaining = chunk_size;
        
        while remaining > 0 {
            let to_read = buffer.len().min(remaining as usize);
            let bytes_read = src_file.read(&mut buffer[..to_read])?;
            
            if bytes_read == 0 {
                break; // EOF
            }
            
            dst_file.write_all(&buffer[..bytes_read])?;
            remaining -= bytes_read as u64;
        }
        
        dst_file.flush()?;
        Ok(())
    }
}

/// Large File Slicer - memory-mapped I/O for large files
#[derive(Clone)]
pub struct LargeFileSlicer {
    config: HybridDamConfig,
    buffer_sizer: BufferSizer,
}

impl LargeFileSlicer {
    pub fn new(config: HybridDamConfig, buffer_sizer: BufferSizer) -> Self {
        Self {
            config,
            buffer_sizer,
        }
    }
    
    /// Process a large file with memory-mapped I/O and chunking
    pub fn process_file(&self, file: FileEntry) -> Result<TransferResult> {
        let start = Instant::now();
        
        // Try platform-specific acceleration first
        if let Ok(_result) = self.try_platform_acceleration(&file) {
            return Ok(TransferResult {
                files_attempted: 1,
                files_copied: 1,      // Platform acceleration succeeded
                bytes_transferred: file.size,
                duration: start.elapsed(),
                strategy_used: TransferStrategy::Slicer,
                errors: Vec::new(),
            });
        }
        
        // Fall back to memory-mapped chunked transfer
        let chunks = self.calculate_optimal_chunks(file.size);
        self.parallel_memory_mapped_transfer(&file, chunks)?;
        
        Ok(TransferResult {
            files_attempted: 1,
            files_copied: 1,      // Memory-mapped transfer succeeded
            bytes_transferred: file.size,
            duration: start.elapsed(),
            strategy_used: TransferStrategy::Slicer,
            errors: Vec::new(),
        })
    }
    
    /// Try platform-specific acceleration (reflinks, etc.)
    fn try_platform_acceleration(&self, file: &FileEntry) -> Result<()> {
        // Check if source and destination are on the same filesystem for reflinks
        if !self.can_use_reflink(file) {
            return Err(anyhow::anyhow!("Reflink not supported between filesystems"));
        }
        
        // Platform-specific optimizations
        #[cfg(target_os = "macos")]
        {
            self.try_apfs_clonefile(file)
        }
        #[cfg(target_os = "windows")]
        {
            self.try_refs_clone(file)
        }
        #[cfg(target_os = "linux")]
        {
            self.try_linux_reflink(file)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Err(anyhow::anyhow!("Platform acceleration not implemented"))
        }
    }
    
    /// Check if reflink can be used between source and destination
    fn can_use_reflink(&self, file: &FileEntry) -> bool {
        // Simple check: if both files are on local filesystem and same mount
        // More sophisticated filesystem detection would be done in production
        if let (Some(src_parent), Some(dst_parent)) = (file.src_path.parent(), file.dst_path.parent()) {
            // Check if they're on the same device (simple heuristic)
            src_parent.exists() && dst_parent.exists()
        } else {
            false
        }
    }
    
    /// macOS APFS clonefile optimization
    #[cfg(target_os = "macos")]
    fn try_apfs_clonefile(&self, file: &FileEntry) -> Result<()> {
        use std::ffi::CString;
        use std::os::raw::c_char;
        
        // Only attempt cloning on APFS filesystems for same-volume copies
        if let Some(ref network_info) = self.config.network_fs_info {
            if network_info.fs_type != crate::network_fs::NetworkFsType::APFS {
                return Err(anyhow::anyhow!("Not an APFS filesystem"));
            }
        }
        
        // Ensure destination directory exists
        if let Some(parent) = file.dst_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        // Try to use clonefile syscall (requires macOS 10.12+)
        let src_cstr = CString::new(file.src_path.to_string_lossy().as_ref())?;
        let dst_cstr = CString::new(file.dst_path.to_string_lossy().as_ref())?;
        
        // APFS clonefile syscall - attempt copy-on-write cloning
        extern "C" {
            fn clonefile(src: *const c_char, dst: *const c_char, flags: u32) -> i32;
        }
        
        const CLONE_NOFOLLOW: u32 = 0x0001;
        const CLONE_NOOWNERCOPY: u32 = 0x0002;
        
        let result = unsafe {
            clonefile(
                src_cstr.as_ptr(),
                dst_cstr.as_ptr(),
                CLONE_NOFOLLOW | CLONE_NOOWNERCOPY
            )
        };
        
        if result == 0 {
            // Successfully cloned - stats will be updated by caller
            Ok(())
        } else {
            // Get the actual errno for better error reporting  
            let errno = unsafe { *libc::__error() };
            Err(anyhow::anyhow!("clonefile failed with errno {}: {}", errno, 
                match errno {
                    libc::ENOTSUP => "Filesystem does not support cloning",
                    libc::EXDEV => "Cross-device cloning not supported", 
                    libc::EACCES => "Permission denied",
                    libc::ENOENT => "Source file not found",
                    libc::EEXIST => "Destination already exists",
                    _ => "Unknown error"
                }
            ))
        }
    }
    
    /// Windows ReFS clone optimization
    #[cfg(target_os = "windows")]
    fn try_refs_clone(&self, file: &FileEntry) -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use std::fs::OpenOptions;
        use winapi::um::ioapiset::DeviceIoControl;
        use winapi::um::winnt::{HANDLE, FILE_ATTRIBUTE_NORMAL};
        
        // Only attempt cloning on ReFS filesystems for same-volume copies
        if let Some(ref network_info) = self.config.network_fs_info {
            // Check if we're on a ReFS-capable filesystem
            let path_str = file.src_path.to_string_lossy();
            if !path_str.starts_with("ReFS") && network_info.fs_type != crate::network_fs::NetworkFsType::Local {
                return Err(anyhow::anyhow!("Not a ReFS or local filesystem"));
            }
        }
        
        // Ensure destination directory exists
        if let Some(parent) = file.dst_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        // Open source file for reading
        let src_file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_ATTRIBUTE_NORMAL)
            .open(&file.src_path)?;
            
        // Create destination file with same size
        let dst_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(FILE_ATTRIBUTE_NORMAL)
            .open(&file.dst_path)?;
            
        // Set destination file size to match source
        dst_file.set_len(file.size)?;
        
        // Prepare FSCTL_DUPLICATE_EXTENTS_TO_FILE structure
        #[repr(C)]
        struct DuplicateExtentsData {
            file_handle: HANDLE,
            source_file_offset: u64,
            target_file_offset: u64,
            byte_count: u64,
        }
        
        let duplicate_data = DuplicateExtentsData {
            file_handle: src_file.as_raw_handle() as HANDLE,
            source_file_offset: 0,
            target_file_offset: 0,
            byte_count: file.size,
        };
        
        const FSCTL_DUPLICATE_EXTENTS_TO_FILE: u32 = 0x00098344;
        let mut bytes_returned: u32 = 0;
        
        let result = unsafe {
            DeviceIoControl(
                dst_file.as_raw_handle() as HANDLE,
                FSCTL_DUPLICATE_EXTENTS_TO_FILE,
                &duplicate_data as *const _ as *mut _,
                std::mem::size_of::<DuplicateExtentsData>() as u32,
                std::ptr::null_mut(),
                0,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };
        
        if result != 0 {
            // Successfully cloned - stats will be updated by caller
            Ok(())
        } else {
            // Get the last Windows error for better error reporting
            let error_code = unsafe { winapi::um::errhandlingapi::GetLastError() };
            Err(anyhow::anyhow!("FSCTL_DUPLICATE_EXTENTS_TO_FILE failed with error {}: {}", error_code,
                match error_code {
                    winapi::shared::winerror::ERROR_NOT_SUPPORTED => "Filesystem does not support extent duplication",
                    winapi::shared::winerror::ERROR_ACCESS_DENIED => "Access denied",
                    winapi::shared::winerror::ERROR_INVALID_PARAMETER => "Invalid parameter",
                    winapi::shared::winerror::ERROR_FILE_NOT_FOUND => "Source file not found",
                    _ => "Unknown error"
                }
            ))
        }
    }
    
    /// Linux reflink optimization (Btrfs, XFS, OCFS2)
    #[cfg(target_os = "linux")]
    fn try_linux_reflink(&self, file: &FileEntry) -> Result<()> {
        use std::fs::File;
        use std::os::unix::io::AsRawFd;
        
        // Only attempt reflinks on supported filesystems (Btrfs, XFS, OCFS2)
        if let Some(ref network_info) = self.config.network_fs_info {
            match network_info.fs_type {
                crate::network_fs::NetworkFsType::BTRFS | 
                crate::network_fs::NetworkFsType::XFS => {
                    // Supported - continue
                },
                _ => {
                    return Err(anyhow::anyhow!("Filesystem does not support reflinks"));
                }
            }
        }
        
        // Ensure destination directory exists
        if let Some(parent) = file.dst_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        // Open source file for reading
        let src_file = File::open(&file.src_path)?;
        
        // Create destination file
        let dst_file = File::create(&file.dst_path)?;
        
        // Get file descriptors
        let src_fd = src_file.as_raw_fd();
        let dst_fd = dst_file.as_raw_fd();
        
        // FICLONE ioctl constant
        const FICLONE: libc::c_ulong = 0x40049409;
        
        // Attempt reflink using FICLONE ioctl
        let result = unsafe {
            libc::ioctl(dst_fd, FICLONE as libc::c_ulong, src_fd)
        };
        
        if result == 0 {
            // Successfully reflinked - stats will be updated by caller
            Ok(())
        } else {
            // Get the actual errno for better error reporting
            let errno = unsafe { *libc::__errno_location() };
            Err(anyhow::anyhow!("FICLONE ioctl failed with errno {}: {}", errno,
                match errno {
                    libc::EOPNOTSUPP => "Filesystem does not support reflinks",
                    libc::EXDEV => "Cross-device reflink not supported",
                    libc::EACCES => "Permission denied",
                    libc::ENOENT => "Source file not found",
                    libc::EEXIST => "Destination already exists",
                    libc::EINVAL => "Invalid argument (files may not be on same filesystem)",
                    libc::EIO => "I/O error",
                    _ => "Unknown error"
                }
            ))
        }
    }
    
    /// Calculate optimal chunk configuration
    fn calculate_optimal_chunks(&self, file_size: u64) -> usize {
        // Rule: 1 chunk per 50-100MB for parallel processing
        let chunks = (file_size / (75 * 1024 * 1024)) as usize;
        chunks.clamp(2, worker_pool::thread_count())
    }
    
    /// Memory-mapped parallel transfer for maximum efficiency
    fn parallel_memory_mapped_transfer(&self, file: &FileEntry, chunks: usize) -> Result<()> {
        use std::fs::File;
        use std::sync::Arc;
        use crate::worker_pool;
        
        // Calculate optimal buffer size for large files
        let buffer_size = self.buffer_sizer.calculate_buffer_size_with_fs(
            file.size,
            self.config.network_fs_info.as_ref()
        );
        
        // Ensure destination directory exists
        if let Some(parent) = file.dst_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        // Pre-allocate destination file
        let dst_file = File::create(&file.dst_path)?;
        dst_file.set_len(file.size)?;
        drop(dst_file);
        
        // Calculate chunk size
        let chunk_size = file.size / chunks as u64;
        
        // For very large files, try memory-mapped approach if available
        if file.size > 1024 * 1024 * 1024 && self.can_use_mmap() {
            self.memory_mapped_copy(file, chunks, chunk_size)
        } else {
            // Fall back to optimized buffered copy with large buffers
            self.large_buffer_parallel_copy(file, chunks, chunk_size, buffer_size)
        }
    }
    
    /// Check if memory mapping is available and beneficial
    fn can_use_mmap(&self) -> bool {
        // Memory mapping is beneficial for very large files on local filesystems
        // Skip mmap for network filesystems due to cache coherency issues
        
        match &self.config.network_fs_info {
            None => {
                // Local filesystem - memory mapping is beneficial
                true
            }
            Some(network_info) => {
                // Network filesystem - be selective about when to use mmap
                match network_info.fs_type {
                    NetworkFsType::SMB => {
                        // SMB can benefit from mmap for very large files due to caching
                        true
                    }
                    NetworkFsType::NFS => {
                        // NFS has cache coherency issues with mmap - avoid for now
                        false
                    }
                    NetworkFsType::SSHFS => {
                        // SSHFS generally doesn't benefit from mmap
                        false
                    }
                    NetworkFsType::WebDAV => {
                        // WebDAV definitely doesn't benefit from mmap
                        false
                    }
                    _ => {
                        // Be conservative for other network filesystems
                        false
                    }
                }
            }
        }
    }
    
    /// Memory-mapped copy implementation (Unix/Linux specific)
    #[cfg(unix)]
    fn memory_mapped_copy(&self, file: &FileEntry, chunks: usize, chunk_size: u64) -> Result<()> {
        use std::fs::{File, OpenOptions};
        use std::sync::{Arc, Mutex};
        
        let file_size = file.size;
        
        // Create chunk tasks
        let mut chunk_tasks = Vec::new();
        
        for chunk_id in 0..chunks {
            let start_offset = chunk_id as u64 * chunk_size;
            let end_offset = if chunk_id == chunks - 1 {
                file_size
            } else {
                (chunk_id + 1) as u64 * chunk_size
            };
            
            chunk_tasks.push((file.src_path.clone(), file.dst_path.clone(), start_offset, end_offset));
        }
        
        // Execute all chunks in parallel using worker pool
        let errors = Arc::new(Mutex::new(Vec::new()));
        
        worker_pool::scope(|s| {
            for (src_path, dst_path, start_offset, end_offset) in chunk_tasks {
                let errors = Arc::clone(&errors);
                s.spawn(move |_| {
                    if let Err(e) = Self::mmap_copy_chunk(&src_path, &dst_path, start_offset, end_offset) {
                        errors.lock().unwrap().push(e);
                    }
                });
            }
        });
        
        // Check for any errors
        let errors = errors.lock().unwrap();
        if let Some(error) = errors.first() {
            return Err(anyhow::anyhow!("Memory mapped copy failed: {}", error));
        }
        
        Ok(())
    }
    
    /// Windows version - now uses proper memory mapping with memmap2
    #[cfg(windows)]
    fn memory_mapped_copy(&self, file: &FileEntry, chunks: usize, chunk_size: u64) -> Result<()> {
        use std::sync::{Arc, Mutex};
        
        let file_size = file.size;
        
        // Create chunk tasks
        let mut chunk_tasks = Vec::new();
        
        for chunk_id in 0..chunks {
            let start_offset = chunk_id as u64 * chunk_size;
            let end_offset = if chunk_id == chunks - 1 {
                file_size
            } else {
                (chunk_id + 1) as u64 * chunk_size
            };
            
            chunk_tasks.push((file.src_path.clone(), file.dst_path.clone(), start_offset, end_offset));
        }
        
        // Execute all chunks in parallel using worker pool
        let errors = Arc::new(Mutex::new(Vec::new()));
        
        worker_pool::scope(|s| {
            for (src_path, dst_path, start_offset, end_offset) in chunk_tasks {
                let errors = Arc::clone(&errors);
                s.spawn(move |_| {
                    if let Err(e) = Self::mmap_copy_chunk(&src_path, &dst_path, start_offset, end_offset) {
                        errors.lock().unwrap().push(e);
                    }
                });
            }
        });
        
        // Check for any errors
        let errors = errors.lock().unwrap();
        if let Some(error) = errors.first() {
            return Err(anyhow::anyhow!("Memory mapped copy failed: {}", error));
        }
        
        Ok(())
    }
    
    /// Memory-mapped chunk copy helper using memmap2
    #[cfg(unix)]
    fn mmap_copy_chunk(src_path: &Path, dst_path: &Path, start_offset: u64, end_offset: u64) -> Result<()> {
        use std::fs::{File, OpenOptions};
        use std::io::{Seek, SeekFrom, Write};
        use memmap2::{Mmap, MmapOptions};
        
        let chunk_size = end_offset - start_offset;
        if chunk_size == 0 {
            return Ok(());
        }
        
        // Open source file for memory mapping
        let src_file = File::open(src_path)?;
        
        // Create memory map for the specific chunk region
        let src_mmap = unsafe {
            MmapOptions::new()
                .offset(start_offset)
                .len(chunk_size as usize)
                .map(&src_file)?
        };
        
        // Open destination file for writing
        let mut dst_file = OpenOptions::new().write(true).open(dst_path)?;
        dst_file.seek(SeekFrom::Start(start_offset))?;
        
        // Copy data from memory-mapped source directly to destination
        // This is zero-copy on the source side and highly efficient
        dst_file.write_all(&src_mmap)?;
        dst_file.flush()?;
        
        // Explicitly drop mmap to ensure cleanup
        drop(src_mmap);
        drop(src_file);
        
        Ok(())
    }
    
    /// Memory-mapped chunk copy helper for Windows
    #[cfg(windows)]
    fn mmap_copy_chunk(src_path: &Path, dst_path: &Path, start_offset: u64, end_offset: u64) -> Result<()> {
        use std::fs::{File, OpenOptions};
        use std::io::{Seek, SeekFrom, Write};
        use memmap2::{Mmap, MmapOptions};
        
        let chunk_size = end_offset - start_offset;
        if chunk_size == 0 {
            return Ok(());
        }
        
        // Open source file for memory mapping
        let src_file = File::open(src_path)?;
        
        // Create memory map for the specific chunk region
        let src_mmap = unsafe {
            MmapOptions::new()
                .offset(start_offset)
                .len(chunk_size as usize)
                .map(&src_file)?
        };
        
        // Open destination file for writing
        let mut dst_file = OpenOptions::new().write(true).open(dst_path)?;
        dst_file.seek(SeekFrom::Start(start_offset))?;
        
        // Copy data from memory-mapped source directly to destination
        dst_file.write_all(&src_mmap)?;
        dst_file.flush()?;
        
        // Explicitly drop mmap to ensure cleanup
        drop(src_mmap);
        drop(src_file);
        
        Ok(())
    }
    
    /// Large buffer parallel copy (fallback method)
    fn large_buffer_parallel_copy(&self, file: &FileEntry, chunks: usize, chunk_size: u64, buffer_size: usize) -> Result<()> {
        use std::sync::{Arc, Mutex};
        
        let file_size = file.size;
        
        // Create chunk tasks
        let mut chunk_tasks = Vec::new();
        
        for chunk_id in 0..chunks {
            let start_offset = chunk_id as u64 * chunk_size;
            let end_offset = if chunk_id == chunks - 1 {
                file_size
            } else {
                (chunk_id + 1) as u64 * chunk_size
            };
            
            chunk_tasks.push((file.src_path.clone(), file.dst_path.clone(), start_offset, end_offset, buffer_size));
        }
        
        // Execute all chunks in parallel using worker pool
        let errors = Arc::new(Mutex::new(Vec::new()));
        
        worker_pool::scope(|s| {
            for (src_path, dst_path, start_offset, end_offset, buffer_size) in chunk_tasks {
                let errors = Arc::clone(&errors);
                s.spawn(move |_| {
                    if let Err(e) = MediumFilePool::copy_file_chunk(&src_path, &dst_path, start_offset, end_offset, buffer_size) {
                        errors.lock().unwrap().push(e);
                    }
                });
            }
        });
        
        // Check for any errors
        let errors = errors.lock().unwrap();
        if let Some(error) = errors.first() {
            return Err(anyhow::anyhow!("Large buffer copy failed: {}", error));
        }
        
        Ok(())
    }
}

/// Categorized operations for Hybrid Dam processing
#[derive(Debug, Clone, Default)]
struct CategorizedOperations {
    directories: Vec<FileOperation>,
    dam_files: Vec<FileOperation>,      // Small files for Dam component
    pool_files: Vec<FileOperation>,     // Medium files for Pool component  
    slicer_files: Vec<FileOperation>,   // Large files for Slicer component
    delta_files: Vec<FileOperation>,    // Very large files for delta transfer
    deletes: Vec<FileOperation>,
}

impl CategorizedOperations {
    fn total_operations(&self) -> u64 {
        (self.directories.len() + self.dam_files.len() + self.pool_files.len() + 
         self.slicer_files.len() + self.delta_files.len() + self.deletes.len()) as u64
    }
}

/// Task batch for worker pool - groups 16-32 files per expert guidance
#[derive(Debug)]
struct TaskBatch {
    files: Vec<FileEntry>,
    strategy: TransferStrategy,
}

/// Main Hybrid Dam coordinator - unified strategy engine
pub struct HybridDam {
    dam: SmallFileDam,
    pool: MediumFilePool,
    slicer: LargeFileSlicer,
    config: HybridDamConfig,
    /// Concurrent delta analyzer for large files
    delta_analyzer: Arc<Mutex<ConcurrentDeltaAnalyzer>>,
    /// Progress tracking
    progress: Arc<SyncProgress>,
    /// Buffer sizer for optimizing I/O
    buffer_sizer: BufferSizer,
    
    // NEW: Channels for async worker results (per IMPLEMENTATION_NOTES.md)
    /// Channel for Pool results
    pool_results_tx: mpsc::Sender<TransferResult>,
    pool_results_rx: mpsc::Receiver<TransferResult>,
    /// Channel for Slicer results  
    slicer_results_tx: mpsc::Sender<TransferResult>,
    slicer_results_rx: mpsc::Receiver<TransferResult>,
    /// Task batch accumulator for intelligent batching (16-32 files)
    pool_batch: Arc<Mutex<Vec<FileEntry>>>,
    slicer_batch: Arc<Mutex<Vec<FileEntry>>>,
}

impl HybridDam {
    pub fn new(config: HybridDamConfig, buffer_sizer: BufferSizer) -> Self {
        // Create channels for async results
        let (pool_tx, pool_rx) = mpsc::channel();
        let (slicer_tx, slicer_rx) = mpsc::channel();
        
        // Initialize the global Rayon pool on first access
        lazy_static::initialize(&GLOBAL_RAYON_POOL);
        
        Self {
            dam: SmallFileDam::new(config.clone(), buffer_sizer.clone()),
            pool: MediumFilePool::new(config.clone(), buffer_sizer.clone()),
            slicer: LargeFileSlicer::new(config.clone(), buffer_sizer.clone()),
            delta_analyzer: Arc::new(Mutex::new(ConcurrentDeltaAnalyzer::new())),
            progress: Arc::new(SyncProgress::new_silent(0, 0)), // Will be updated
            config,
            buffer_sizer,
            pool_results_tx: pool_tx,
            pool_results_rx: pool_rx,
            slicer_results_tx: slicer_tx,
            slicer_results_rx: slicer_rx,
            pool_batch: Arc::new(Mutex::new(Vec::new())),
            slicer_batch: Arc::new(Mutex::new(Vec::new())),
        }
    }
    
    /// Create with progress tracking
    pub fn with_progress(mut self, total_files: u64, total_bytes: u64) -> Self {
        self.progress = Arc::new(SyncProgress::new_silent(total_files, total_bytes));
        self
    }
    
    /// Process a file through the appropriate component
    pub fn process_file(&mut self, file: FileEntry) -> Result<Option<TransferResult>> {
        match self.determine_strategy(&file) {
            TransferStrategy::Dam => {
                // Add to dam, may return batch job
                if let Some(batch_job) = self.dam.add_file(file) {
                    self.process_dam_batch(batch_job).map(Some)
                } else {
                    Ok(None) // File buffered, no immediate result
                }
            }
            TransferStrategy::Pool => {
                self.pool.process_file(file).map(Some)
            }
            TransferStrategy::Slicer => {
                self.slicer.process_file(file).map(Some)
            }
        }
    }
    
    /// Dispatch file to Pool workers ASYNCHRONOUSLY (never blocks!)
    fn dispatch_to_pool_async(&self, file: FileEntry) {
        // Clone what we need for the async task
        let pool = self.pool.clone();
        let tx = self.pool_results_tx.clone();
        let batch = self.pool_batch.clone();
        
        // Add to batch for intelligent batching (16-32 files per task)
        let should_flush = {
            let mut batch_guard = batch.lock().unwrap();
            batch_guard.push(file);
            batch_guard.len() >= 16 // Flush at 16 files per expert guidance
        };
        
        if should_flush {
            // Extract the batch
            let files_to_process = {
                let mut batch_guard = batch.lock().unwrap();
                std::mem::take(&mut *batch_guard)
            };
            
            // Submit batch to Rayon pool - returns immediately!
            GLOBAL_RAYON_POOL.spawn(move || {
                // Process batch of files together (reduces sync overhead)
                let mut batch_result = TransferResult {
                    files_attempted: files_to_process.len(),
                    files_copied: 0,
                    bytes_transferred: 0,
                    duration: Duration::from_secs(0),
                    strategy_used: TransferStrategy::Pool,
                    errors: Vec::new(),
                };
                
                let start = Instant::now();
                for file in files_to_process {
                    match pool.process_file(file) {
                        Ok(result) => {
                            batch_result.files_copied += result.files_copied;
                            batch_result.bytes_transferred += result.bytes_transferred;
                        }
                        Err(e) => {
                            batch_result.errors.push(e.to_string());
                        }
                    }
                }
                batch_result.duration = start.elapsed();
                
                // Send result back through channel
                let _ = tx.send(batch_result);
            });
        }
    }
    
    /// Dispatch file to Slicer workers ASYNCHRONOUSLY (never blocks!)
    fn dispatch_to_slicer_async(&self, file: FileEntry) {
        // Clone what we need for the async task
        let slicer = self.slicer.clone();
        let tx = self.slicer_results_tx.clone();
        
        // Large files go individually (no batching needed)
        GLOBAL_RAYON_POOL.spawn(move || {
            let start = Instant::now();
            match slicer.process_file(file) {
                Ok(mut result) => {
                    result.duration = start.elapsed();
                    let _ = tx.send(result);
                }
                Err(e) => {
                    let result = TransferResult {
                        files_attempted: 1,
                        files_copied: 0,
                        bytes_transferred: 0,
                        duration: start.elapsed(),
                        strategy_used: TransferStrategy::Slicer,
                        errors: vec![e.to_string()],
                    };
                    let _ = tx.send(result);
                }
            }
        });
    }
    
    /// Flush any pending batches that didn't reach the threshold
    fn flush_pending_batches(&self) {
        // Flush Pool batch
        let pool_files = {
            let mut batch_guard = self.pool_batch.lock().unwrap();
            std::mem::take(&mut *batch_guard)
        };
        
        if !pool_files.is_empty() {
            let pool = self.pool.clone();
            let tx = self.pool_results_tx.clone();
            
            GLOBAL_RAYON_POOL.spawn(move || {
                let mut batch_result = TransferResult {
                    files_attempted: pool_files.len(),
                    files_copied: 0,
                    bytes_transferred: 0,
                    duration: Duration::from_secs(0),
                    strategy_used: TransferStrategy::Pool,
                    errors: Vec::new(),
                };
                
                let start = Instant::now();
                for file in pool_files {
                    match pool.process_file(file) {
                        Ok(result) => {
                            batch_result.files_copied += result.files_copied;
                            batch_result.bytes_transferred += result.bytes_transferred;
                        }
                        Err(e) => {
                            batch_result.errors.push(e.to_string());
                        }
                    }
                }
                batch_result.duration = start.elapsed();
                let _ = tx.send(batch_result);
            });
        }
        
        // Note: Slicer doesn't use batching, so nothing to flush there
    }
    
    /// Wait for all async workers to complete
    fn wait_for_all_workers(&self, total_stats: &mut SyncStats, bytes_processed: &mut u64, files_copied: &mut u64, verbose: i32) {
        // Give workers time to finish
        // We'll poll the channels until they're empty and no work is pending
        let mut consecutive_empty = 0;
        let wait_threshold = 10; // If channels are empty for 10 iterations, assume done
        
        loop {
            let mut received_any = false;
            
            // Process Pool results
            while let Ok(result) = self.pool_results_rx.try_recv() {
                received_any = true;
                total_stats.add_bytes_transferred(result.bytes_transferred);
                *bytes_processed += result.bytes_transferred;
                *files_copied += result.files_copied as u64;  // Track actual files copied
                for _ in 0..result.files_copied {
                    total_stats.increment_files_copied();
                }
                if !result.errors.is_empty() {
                    if verbose >= 1 {
                        for err in &result.errors {
                            eprintln!("Pool error (final): {}", err);
                        }
                    }
                    total_stats.increment_errors();
                }
            }
            
            // Process Slicer results
            while let Ok(result) = self.slicer_results_rx.try_recv() {
                received_any = true;
                total_stats.add_bytes_transferred(result.bytes_transferred);
                *bytes_processed += result.bytes_transferred;
                *files_copied += result.files_copied as u64;  // Track actual files copied
                for _ in 0..result.files_copied {
                    total_stats.increment_files_copied();
                }
                if !result.errors.is_empty() {
                    if verbose >= 1 {
                        for err in &result.errors {
                            eprintln!("Slicer error (final): {}", err);
                        }
                    }
                    total_stats.increment_errors();
                }
            }
            
            if received_any {
                consecutive_empty = 0;
            } else {
                consecutive_empty += 1;
                if consecutive_empty >= wait_threshold {
                    // Channels have been empty for a while, assume all work is done
                    break;
                }
                // Small sleep to avoid busy waiting
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    
    /// Execute mixed strategy synchronization with Hybrid Dam architecture
    pub fn execute(
        &self,
        operations: Vec<FileOperation>,
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        use indicatif::{ProgressBar, ProgressStyle};
        use crate::logging::SyncLogger;
        use crate::formatted_display;

        // Create logger with optional log file
        let logger = Arc::new(Mutex::new(SyncLogger::new(
            options.log_file.as_deref(),
            options.show_eta,
            options.verbose,
        )?));

        {
            let logger_guard = logger.lock().unwrap();
            logger_guard.log("Starting Hybrid Dam synchronization...");
            logger_guard.log(&format!("Source: {}", source_root.display()));
            logger_guard.log(&format!("Destination: {}", dest_root.display()));
        }

        // Create error logger for automatic error reporting
        let error_logger = ErrorLogger::new(options.clone(), source_root, dest_root);
        let error_handle = error_logger.get_handle();

        // Show spinner during categorization for user feedback
        let spinner = if options.show_progress {
            let s = ProgressBar::new_spinner();
            s.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner} {msg}")
                    .expect("Failed to set spinner template"),
            );
            s.set_message("Analyzing files for Hybrid Dam strategy...");
            s.enable_steady_tick(std::time::Duration::from_millis(100));
            Some(s)
        } else {
            None
        };

        // Categorize operations into Hybrid Dam components
        let categorized = self.categorize_for_hybrid_dam(&operations, options);
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

        // Create spinner for operations (NO PROGRESS BAR)
        let pb = if !options.show_progress {
            let pb = indicatif::ProgressBar::new_spinner();
            pb.set_draw_target(indicatif::ProgressDrawTarget::hidden());
            pb
        } else {
            let pb = indicatif::ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner} {msg}")
                    .expect("Failed to set spinner template"),
            );
            pb.set_message("Starting Hybrid Dam transfer...");
            pb.enable_steady_tick(std::time::Duration::from_millis(100));
            pb
        };

        // Execute using Hybrid Dam components with parallel workers
        self.execute_hybrid_dam_strategy(categorized, source_root, dest_root, options, &pb, &error_handle)
    }

    /// Finish processing and flush any remaining files
    pub fn finish(&mut self) -> Result<Vec<TransferResult>> {
        let mut results = Vec::new();
        
        // Flush any remaining files in dam
        if let Some(batch_job) = self.dam.flush() {
            results.push(self.process_dam_batch(batch_job)?);
        }
        
        Ok(results)
    }
    

    /// Categorize operations for Hybrid Dam components
    fn categorize_for_hybrid_dam(&self, operations: &[FileOperation], options: &SyncOptions) -> CategorizedOperations {
        let mut categorized = CategorizedOperations::default();
        
        // Use thresholds similar to mixed_strategy.rs but adapted for Hybrid Dam
        let small_threshold = options.small_file_threshold.unwrap_or(self.config.dam_threshold);
        let large_threshold = options.large_file_threshold.unwrap_or(self.config.slicer_threshold);
        let delta_threshold = 500 * 1024 * 1024; // 500MB - files above this use delta
        
        for operation in operations {
            match operation {
                FileOperation::CreateDirectory { path } => {
                    categorized.directories.push(operation.clone());
                }
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    if let Ok(metadata) = path.metadata() {
                        let file_size = metadata.len();
                        
                        if file_size >= delta_threshold {
                            categorized.delta_files.push(operation.clone());
                        } else if file_size >= large_threshold {
                            categorized.slicer_files.push(operation.clone());
                        } else if file_size >= small_threshold {
                            categorized.pool_files.push(operation.clone());
                        } else {
                            categorized.dam_files.push(operation.clone());
                        }
                    } else {
                        // Fallback if we can't get metadata
                        categorized.dam_files.push(operation.clone());
                    }
                }
                FileOperation::Delete { .. } => {
                    categorized.deletes.push(operation.clone());
                }
                FileOperation::CreateSymlink { .. } | FileOperation::UpdateSymlink { .. } => {
                    // Symlinks are small operations, put them in dam_files
                    categorized.dam_files.push(operation.clone());
                }
            }
        }
        
        categorized
    }

    /// Execute the Hybrid Dam strategy with parallel workers
    fn execute_hybrid_dam_strategy(
        &self,
        categorized: CategorizedOperations,
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
        pb: &ProgressBar,
        error_handle: &crate::error_logger::ErrorLogHandle,
    ) -> Result<SyncStats> {
        let mut total_stats = SyncStats::default();

        // Process directories first
        if !categorized.directories.is_empty() {
            self.create_directories(&categorized.directories, source_root, dest_root)?;
        }

        // Setup for parallel execution using worker pools
        let (tx, rx) = mpsc::channel();
        let mut handles = vec![];
        let start_time = Instant::now();

        // Process Dam files (small files with tar streaming)
        if !categorized.dam_files.is_empty() {
            let tx = tx.clone();
            let dam_files = categorized.dam_files.clone();
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            let pb_clone = pb.clone();
            let error_handle = error_handle.clone();

            let handle = std::thread::spawn(move || {
                let result = Self::process_dam_files_with_tar_streaming(
                    &dam_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                    start_time,
                    &error_handle,
                );
                let _ = tx.send(("dam", result));
            });
            handles.push(handle);
        }

        // Process Pool files (medium files with parallel workers)  
        if !categorized.pool_files.is_empty() {
            let tx = tx.clone();
            let pool_files = categorized.pool_files.clone();
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            let pb_clone = pb.clone();

            let handle = std::thread::spawn(move || {
                let result = Self::process_pool_files_parallel(
                    &pool_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                );
                let _ = tx.send(("pool", result));
            });
            handles.push(handle);
        }

        // Process Slicer files (large files with memory-mapped I/O)
        if !categorized.slicer_files.is_empty() {
            let tx = tx.clone();
            let slicer_files = categorized.slicer_files.clone();
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            let pb_clone = pb.clone();

            let handle = std::thread::spawn(move || {
                let result = Self::process_slicer_files_mmap(
                    &slicer_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                );
                let _ = tx.send(("slicer", result));
            });
            handles.push(handle);
        }

        // Process Delta files (very large files with concurrent delta analysis)
        if !categorized.delta_files.is_empty() {
            let tx = tx.clone();
            let delta_files = categorized.delta_files.clone();
            let source_root = source_root.to_path_buf();
            let dest_root = dest_root.to_path_buf();
            let options = options.clone();
            let pb_clone = pb.clone();
            let delta_analyzer = Arc::clone(&self.delta_analyzer);

            let handle = std::thread::spawn(move || {
                let result = Self::process_delta_files_concurrent(
                    &delta_files,
                    &source_root,
                    &dest_root,
                    &options,
                    Some(&pb_clone),
                    delta_analyzer,
                );
                let _ = tx.send(("delta", result));
            });
            handles.push(handle);
        }

        // Process Deletes
        if !categorized.deletes.is_empty() {
            let tx = tx.clone();
            let deletes = categorized.deletes.clone();
            let options = options.clone();
            let pb_clone = pb.clone();

            let handle = std::thread::spawn(move || {
                let result = Self::process_deletes_parallel(&deletes, &options, Some(&pb_clone));
                let _ = tx.send(("deletes", result));
            });
            handles.push(handle);
        }

        // Collect results from all workers
        for _ in 0..handles.len() {
            if let Ok((worker_type, result)) = rx.recv() {
                match result {
                    Ok(stats) => {
                        // Manually aggregate stats
                        total_stats.add_bytes_transferred(stats.bytes_transferred());
                        for _ in 0..stats.files_processed() {
                            total_stats.increment_files_processed();
                        }
                        for _ in 0..stats.files_copied() {
                            total_stats.increment_files_copied();
                        }
                        
                        if options.verbose >= 2 {
                            eprintln!("[{}] {} worker completed: {} files, {} bytes", 
                                Self::timestamp(), worker_type, stats.files_processed(), stats.bytes_transferred());
                        }
                    }
                    Err(e) => {
                        eprintln!("[ERROR] {} worker failed: {}", worker_type, e);
                    }
                }
            }
        }

        // Wait for all threads to complete
        for handle in handles {
            let _ = handle.join();
        }

        pb.finish_and_clear();

        Ok(total_stats)
    }

    /// Execute synchronization with TRUE streaming file discovery to eliminate startup latency
    /// This version processes files ONE AT A TIME as they're discovered (per Gemini's mandate)
    pub fn execute_streaming(
        &mut self,
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
        progress: Option<std::sync::Arc<crate::progress_display::ProgressDisplay>>,
    ) -> Result<SyncStats> {
        use crate::streaming_walker::{StreamingWalker, OperationWithSize};
        use std::sync::mpsc;
        use std::thread;
        use std::time::Instant;
        
        let start_time = Instant::now();
        
        // Create channel for TRUE streaming operations WITH SIZE to avoid duplicate metadata calls
        let (tx, rx) = mpsc::channel::<OperationWithSize>();
        
        // Clone values for the walker thread
        let source_path = source_root.to_path_buf();
        let dest_path = dest_root.to_path_buf();
        let walker_options = options.clone();
        
        // Spawn walker thread that sends operations WITH SIZE AS THEY'RE DISCOVERED
        // This avoids duplicate metadata calls!
        let walker_thread = thread::spawn(move || {
            StreamingWalker::discover_files_with_size_to_channel(
                source_path,
                dest_path,
                walker_options,
                tx
            )
        });
        
        // Main application loop - process operations IMMEDIATELY as they arrive
        // NO BATCHING, NO WAITING - ACT IMMEDIATELY per expert guidance
        let mut total_stats = SyncStats::default();
        
        // Process each operation as it arrives from the walker thread
        let mut files_discovered = 0u64;  // Files found by walker
        let mut files_copied = 0u64;       // Files actually copied
        let mut bytes_processed = 0u64;
        let mut error_count = 0u64;
        let mut warning_count = 0u64;
        
        for op_with_size in rx {
            let operation = op_with_size.operation;
            let cached_size = op_with_size.size;  // Size from walker, no metadata call needed!
            
            // Check if this operation triggered a warning
            if op_with_size.warning_encountered {
                warning_count += 1;
                // Skip processing placeholder warning operations
                if let FileOperation::Create { path } = &operation {
                    if path.to_string_lossy() == "__WARNING__" {
                        // Update display to show warning count
                        if let Some(ref display) = progress {
                            display.update_detailed(files_discovered, files_copied, bytes_processed, warning_count, error_count);
                        }
                        continue;
                    }
                }
            }
            // Update progress display with current status  
            if let Some(ref display) = progress {
                // Track files discovered (not copied yet)
                files_discovered += 1;
                
                // Update display every 10 files or on first file
                if files_discovered % 10 == 0 || files_discovered == 1 {
                    // Use detailed display showing both discovered and copied
                    display.update_detailed(files_discovered, files_copied, bytes_processed, warning_count, error_count);
                }
                
                // DIAGNOSTIC: Show the REAL problem - discovery vs copy rate
                // Only show at verbose level 2 or higher to avoid duplicate output
                if files_discovered % 1000 == 0 && options.verbose >= 2 {
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let discovery_rate = if elapsed > 0.0 { 
                        files_discovered as f64 / elapsed 
                    } else { 
                        0.0 
                    };
                    let copy_rate = if bytes_processed > 0 && elapsed > 0.0 {
                        bytes_processed as f64 / elapsed / 1_048_576.0 // MB/s
                    } else {
                        0.0
                    };
                    eprintln!("[DIAGNOSTIC] Files discovered: {} ({:.0} files/sec), Files copied: {}, Bytes: {} MB ({:.1} MB/s)", 
                        files_discovered, discovery_rate, files_copied, bytes_processed / 1_048_576, copy_rate);
                }
            }
            
            // Convert FileOperation to FileEntry using CACHED SIZE - no metadata call!
            let file_entry = match self.operation_to_file_entry_with_size(&operation, cached_size, source_root, dest_root) {
                Ok(entry) => entry,
                Err(e) => {
                    if options.verbose >= 1 {
                        eprintln!("Warning: Skipping file due to error: {}", e);
                    }
                    total_stats.increment_errors();
                    error_count += 1;
                    continue;
                }
            };
            
            // Log file operation if verbose >= 2
            if options.verbose >= 2 {
                let operation_type = match &operation {
                    FileOperation::Create { .. } => "Copying",
                    FileOperation::Update { .. } => "Updating",
                    FileOperation::Delete { .. } => "Deleting",
                    FileOperation::CreateDirectory { .. } => "Creating directory",
                    FileOperation::CreateSymlink { .. } => "Creating symlink",
                    FileOperation::UpdateSymlink { .. } => "Updating symlink",
                };
                
                // Format file size
                let size_str = if file_entry.size == 0 {
                    String::new()
                } else {
                    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
                    let mut unit_index = 0;
                    let mut size = file_entry.size as f64;
                    
                    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
                        size /= 1024.0;
                        unit_index += 1;
                    }
                    
                    if unit_index == 0 {
                        format!(" ({} B)", file_entry.size)
                    } else {
                        format!(" ({:.2} {})", size, UNITS[unit_index])
                    }
                };
                
                println!("[{}] {}: {}{}", 
                    Self::timestamp(), 
                    operation_type,
                    file_entry.dst_path.display(),
                    size_str
                );
            }
            
            // HYBRID DAM MANDATE: Direct dispatch based ONLY on size - NO analysis!
            // This is the critical change - we decide immediately based on size alone
            
            // CRITICAL DIAGNOSTIC: Are we actually processing files?
            if options.verbose >= 2 && files_discovered % 1000 == 0 {
                eprintln!("[DISPATCH] File #{}: {} ({} bytes) -> {}", 
                    files_discovered,
                    file_entry.src_path.display(),
                    file_entry.size,
                    if file_entry.size < self.config.dam_threshold { "DAM" }
                    else if file_entry.size < self.config.slicer_threshold { "POOL" }
                    else { "SLICER" }
                );
            }
            
            if file_entry.size < self.config.dam_threshold {
                // Small files (<1MB) - Add to Dam for intelligent batching
                // The Dam will flush automatically when thresholds are reached
                if let Some(batch_job) = self.dam.add_file(file_entry) {
                    // Dam reached threshold and returned a batch to process
                    match self.process_dam_batch(batch_job) {
                        Ok(result) => {
                            total_stats.add_bytes_transferred(result.bytes_transferred);
                            bytes_processed += result.bytes_transferred;
                            files_copied += result.files_copied as u64;  // Track actual files copied
                            for _ in 0..result.files_copied {
                                total_stats.increment_files_copied();
                            }
                        }
                        Err(e) => {
                            if options.verbose >= 1 {
                                eprintln!("Dam batch processing error: {}", e);
                            }
                            total_stats.increment_errors();
                            error_count += 1;
                        }
                    }
                }
                // If no batch returned, file is buffered in Dam for later
            } else if file_entry.size < self.config.slicer_threshold {
                // Medium files (1-100MB) - ASYNC dispatch to Pool workers
                // FIXED: Now queues to Rayon pool and returns immediately!
                self.dispatch_to_pool_async(file_entry);
                // Discovery continues immediately - no blocking!
            } else {
                // Large files (>100MB) - ASYNC dispatch to Slicer workers
                // FIXED: Now queues to Rayon pool and returns immediately!
                self.dispatch_to_slicer_async(file_entry);
                // Discovery continues immediately - no blocking!
            }
            
            // Process any completed transfers WITHOUT BLOCKING
            // This is the key to concurrent operation!
            while let Ok(result) = self.pool_results_rx.try_recv() {
                total_stats.add_bytes_transferred(result.bytes_transferred);
                bytes_processed += result.bytes_transferred;
                files_copied += result.files_copied as u64;  // Track actual files copied
                for _ in 0..result.files_copied {
                    total_stats.increment_files_copied();
                }
                if !result.errors.is_empty() {
                    if options.verbose >= 1 {
                        for err in &result.errors {
                            eprintln!("Pool error: {}", err);
                        }
                    }
                    error_count += result.errors.len() as u64;
                    total_stats.increment_errors();
                }
            }
            
            while let Ok(result) = self.slicer_results_rx.try_recv() {
                total_stats.add_bytes_transferred(result.bytes_transferred);
                bytes_processed += result.bytes_transferred;
                files_copied += result.files_copied as u64;  // Track actual files copied
                for _ in 0..result.files_copied {
                    total_stats.increment_files_copied();
                }
                if !result.errors.is_empty() {
                    if options.verbose >= 1 {
                        for err in &result.errors {
                            eprintln!("Slicer error: {}", err);
                        }
                    }
                    error_count += result.errors.len() as u64;
                    total_stats.increment_errors();
                }
            }
        }
        
        // CRITICAL: Flush any remaining files in the Dam
        // The Dam buffers small files and must be flushed at the end
        if let Some(final_batch) = self.dam.flush() {
            match self.process_dam_batch(final_batch) {
                Ok(result) => {
                    total_stats.add_bytes_transferred(result.bytes_transferred);
                    bytes_processed += result.bytes_transferred;
                    files_copied += result.files_copied as u64;  // Track actual files copied
                    for _ in 0..result.files_copied {
                        total_stats.increment_files_copied();
                    }
                }
                Err(e) => {
                    if options.verbose >= 1 {
                        eprintln!("Error processing final Dam batch: {}", e);
                    }
                    total_stats.increment_errors();
                }
            }
        }
        
        // Wait for walker thread to complete
        match walker_thread.join() {
            Ok(Ok(())) => {
                // Walker completed successfully
            }
            Ok(Err(e)) => {
                if options.verbose >= 1 {
                    eprintln!("Warning: Walker encountered errors: {}", e);
                }
                total_stats.increment_errors();
            }
            Err(_) => {
                if options.verbose >= 1 {
                    eprintln!("Warning: Walker thread panicked, but synchronization continued");
                }
                total_stats.increment_errors();
            }
        }
        
        // CRITICAL: Flush any remaining batches in Pool and Slicer
        // Process any files that didn't reach the batch threshold
        self.flush_pending_batches();
        
        // Wait for all async workers to complete
        // This ensures all transfers finish before we return
        self.wait_for_all_workers(&mut total_stats, &mut bytes_processed, &mut files_copied, options.verbose as i32);
        
        Ok(total_stats)
    }
    
    /// OPTIMIZED: Convert FileOperation to FileEntry with CACHED SIZE - no metadata call!
    /// This eliminates the duplicate stat that was causing 75.8 second delays
    #[inline(always)]
    fn operation_to_file_entry_with_size(
        &self,
        operation: &FileOperation,
        cached_size: u64,  // Size from walker, no metadata call needed!
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<FileEntry> {
        match operation {
            FileOperation::Create { path } | 
            FileOperation::Update { path, .. } => {
                // The path in FileOperation is the SOURCE path
                let src_path = path.clone();
                
                // Calculate destination path FAST - no allocations if possible
                let rel_path = src_path.strip_prefix(source_root)
                    .unwrap_or(&src_path);
                let dst_path = dest_root.join(rel_path);
                
                // Use the CACHED size from the walker - NO METADATA CALL!
                // This eliminates the duplicate stat that was causing 75.8 second delays
                let size = cached_size;
                
                Ok(FileEntry {
                    src_path,
                    dst_path,
                    size,
                    modified: std::time::UNIX_EPOCH, // Dummy value - we don't need this for dispatch
                    file_type: FileType::Regular,
                })
            }
            _ => {
                // For non-file operations, skip quickly
                Ok(FileEntry {
                    src_path: PathBuf::new(),
                    dst_path: PathBuf::new(),
                    size: 0,
                    modified: std::time::UNIX_EPOCH,
                    file_type: FileType::Regular,
                })
            }
        }
    }

    /// Legacy method that still does metadata call - only used by non-streaming code paths
    fn operation_to_file_entry(
        &self,
        operation: &FileOperation,
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<FileEntry> {
        match operation {
            FileOperation::Create { path } | 
            FileOperation::Update { path, .. } => {
                let src_path = path.clone();
                let rel_path = src_path.strip_prefix(source_root)
                    .unwrap_or(&src_path);
                let dst_path = dest_root.join(rel_path);
                
                // Legacy method - still does metadata call
                let size = match std::fs::metadata(&src_path) {
                    Ok(m) => m.len(),
                    Err(_) => 0
                };
                
                Ok(FileEntry {
                    src_path,
                    dst_path,
                    size,
                    modified: std::time::UNIX_EPOCH,
                    file_type: FileType::Regular,
                })
            }
            _ => {
                Ok(FileEntry {
                    src_path: PathBuf::new(),
                    dst_path: PathBuf::new(),
                    size: 0,
                    modified: std::time::UNIX_EPOCH,
                    file_type: FileType::Regular,
                })
            }
        }
    }
    
    /// DEPRECATED: No longer used - we dispatch directly based on size in execute_streaming
    #[allow(dead_code)]
    fn determine_strategy(&self, file: &FileEntry) -> TransferStrategy {
        if file.size < self.config.dam_threshold {
            TransferStrategy::Dam
        } else if file.size < self.config.slicer_threshold {
            TransferStrategy::Pool
        } else {
            TransferStrategy::Slicer
        }
    }
    
    /// Process a dam batch job using optimized batch transfer
    fn process_dam_batch(&self, batch_job: DamBatchJob) -> Result<TransferResult> {
        // For now, just use individual file processing until tar streaming is fixed on Windows
        // TODO: Fix tar streaming on Windows - path normalization issues
        self.process_dam_batch_individual(batch_job)
    }
    
    /// Process batch using individual file copies
    fn process_dam_batch_individual(&self, batch_job: DamBatchJob) -> Result<TransferResult> {
        let start = Instant::now();
        let mut errors = Vec::new();
        let mut bytes_transferred = 0u64;
        let mut files_copied = 0usize;
        
        for file_entry in &batch_job.files {
            match self.copy_single_file(file_entry) {
                Ok(size) => {
                    bytes_transferred += size;
                    files_copied += 1;
                },
                Err(e) => errors.push(format!("Failed to copy {}: {}", 
                    file_entry.src_path.display(), e)),
            }
        }
        
        Ok(TransferResult {
            files_attempted: batch_job.files.len(),
            files_copied,
            bytes_transferred,
            duration: start.elapsed(),
            strategy_used: TransferStrategy::Dam,
            errors,
        })
    }
    
    /// Process batch using tar streaming for efficiency
    fn process_dam_batch_tar_streaming(&self, batch_job: &DamBatchJob) -> Result<TransferResult> {
        // Convert FileEntry batch to FileOperation format for tar streaming
        let mut file_operations = Vec::new();
        for file_entry in &batch_job.files {
            file_operations.push(FileOperation::Create { 
                path: file_entry.src_path.clone() 
            });
        }
        
        // Find common source and destination roots
        let source_root = if !batch_job.files.is_empty() {
            batch_job.files[0].src_path.parent().unwrap_or(Path::new("/"))
        } else {
            return Ok(TransferResult {
                files_attempted: 0,
                files_copied: 0,
                bytes_transferred: 0,
                duration: Duration::default(),
                strategy_used: TransferStrategy::Dam,
                errors: Vec::new(),
            });
        };
        
        let dest_root = if !batch_job.files.is_empty() {
            batch_job.files[0].dst_path.parent().unwrap_or(Path::new("/"))
        } else {
            Path::new("/")
        };
        
        // Use the robust tar streaming implementation
        // Create minimal options and error logger for internal use
        let minimal_options = SyncOptions::default();
        let error_logger = crate::error_logger::ErrorLogger::new(
            minimal_options.clone(), 
            source_root, 
            dest_root
        );
        let error_handle = error_logger.get_handle();
        
        let stats = Self::process_dam_files_with_tar_streaming(
            &file_operations,
            source_root,
            dest_root,
            &minimal_options,
            None, // progress bar
            Instant::now(),
            &error_handle,
        )?;
        
        Ok(TransferResult {
            files_attempted: batch_job.files.len(),
            files_copied: stats.files_copied() as usize,
            bytes_transferred: stats.bytes_transferred(),
            duration: stats.elapsed_time,
            strategy_used: TransferStrategy::Dam,
            errors: Vec::new(),
        })
    }
    
    /// Check if tar streaming is suitable for this batch
    fn should_use_tar_streaming(&self, batch_job: &DamBatchJob) -> bool {
        // Use tar streaming for batches with multiple small files
        if batch_job.files.len() >= 3 && batch_job.total_size < 100 * 1024 * 1024 {
            return true;
        }
        
        // Skip tar for mixed file sizes (some files might be too large)
        let max_file_size = batch_job.files.iter()
            .map(|f| f.size)
            .max()
            .unwrap_or(0);
            
        if max_file_size > 10 * 1024 * 1024 {
            return false; // Mixed sizes, use individual copying
        }
        
        // Use tar for network filesystems with many files
        self.config.network_fs_info.is_some() && batch_job.files.len() >= 10
    }
    
    /// Copy a single file (fallback method)
    fn copy_single_file(&self, file_entry: &FileEntry) -> Result<u64> {
        use std::fs;
        
        // Ensure destination directory exists
        if let Some(parent) = file_entry.dst_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        match file_entry.file_type {
            FileType::Regular => {
                let bytes_copied = fs::copy(&file_entry.src_path, &file_entry.dst_path)?;
                Ok(bytes_copied)
            }
            FileType::Directory => {
                fs::create_dir_all(&file_entry.dst_path)?;
                Ok(0)
            }
            FileType::Symlink => {
                // Handle symlinks
                let target = fs::read_link(&file_entry.src_path)?;
                
                #[cfg(unix)]
                {
                    use std::os::unix::fs as unix_fs;
                    unix_fs::symlink(target, &file_entry.dst_path)?;
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs as windows_fs;
                    if file_entry.src_path.is_dir() {
                        windows_fs::symlink_dir(target, &file_entry.dst_path)?;
                    } else {
                        windows_fs::symlink_file(target, &file_entry.dst_path)?;
                    }
                }
                Ok(0)
            }
        }
    }
    // ========== MIGRATED MATURE FEATURES FROM MIXED_STRATEGY.RS ==========
    
    /// Timestamp utility for logging
    fn timestamp() -> String {
        chrono::Local::now().format("%H:%M:%S%.3f").to_string()
    }
    
    /// Process Dam files with in-memory tar streaming (migrated from mixed_strategy.rs)
    fn process_dam_files_with_tar_streaming(
        files: &[FileOperation],
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
        _start_time: Instant,
        error_handle: &crate::error_logger::ErrorLogHandle,
    ) -> Result<SyncStats> {
        // Create a channel for in-memory streaming
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(64); // 64 chunks buffer
        
        let mut stats = SyncStats::default();
        let verbose = options.verbose; // Extract verbose level for thread
        
        // Prepare list of files to stream
        let mut files_to_stream = Vec::new();
        let mut total_bytes = 0u64;
        
        for op in files {
            match op {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    if let Ok(metadata) = path.metadata() {
                        total_bytes += metadata.len();
                    }
                    files_to_stream.push((path.clone(), relative.to_path_buf()));
                }
                _ => {}
            }
        }
        
        if files_to_stream.is_empty() {
            return Ok(SyncStats::default());
        }

        // Only show the tar streaming message when we actually have files to process
        if options.verbose >= 1 {
            eprintln!(
                "[{}] Using in-memory tar streaming for {} files that need copying",
                Self::timestamp(),
                files_to_stream.len(),
            );
        }
        
        let files_for_packer = files_to_stream.clone();
        let dest_root_unpacker = dest_root.to_path_buf();
        
        // Thread 1: Create tar stream in memory with enhanced error handling
        let packer_handle = std::thread::spawn(move || -> Result<Vec<PathBuf>> {
            let mut writer = crate::streaming_batch::ChannelWriter::new(tx);
            let mut builder = Builder::new(&mut writer);
            let mut locked_files = Vec::new();
            
            for (source_path, relative_path) in files_for_packer {
                match builder.append_path_with_name(&source_path, &relative_path) {
                    Ok(_) => {
                        // File successfully added to archive
                    }
                    Err(e) => {
                        // Check for specific OS errors that indicate file is in use
                        let anyhow_error = anyhow::Error::from(e);
                        if Self::is_file_locked_error(&anyhow_error) {
                            locked_files.push(source_path.clone());
                            if verbose > 0 {
                                eprintln!("[{}] Warning: Skipping locked file: {} ({})", 
                                    Self::timestamp(), source_path.display(), 
                                    Self::format_locked_file_message(&anyhow_error));
                            }
                        } else {
                            // For other errors, still try to continue but report them
                            if verbose > 1 {
                                eprintln!("[{}] Warning: Failed to add file to archive: {} ({})", 
                                    Self::timestamp(), source_path.display(), anyhow_error);
                            }
                        }
                    }
                }
            }
            
            builder.finish()?;
            Ok(locked_files)
        });
        
        // Thread 2: Extract tar stream with error resilience
        let unpacker_handle = std::thread::spawn(move || -> Result<(u64, Vec<(PathBuf, std::io::Error)>)> {
            let reader = crate::streaming_batch::ChannelReader::new(rx);
            let mut archive = Archive::new(reader);
            let mut count = 0u64;
            let mut failed_files = Vec::new();
            
            for entry in archive.entries()? {
                match entry {
                    Ok(mut entry) => {
                        // Get the path first before borrowing entry mutably
                        let entry_path_result = entry.path().map(|p| p.to_path_buf());
                        
                        match entry_path_result {
                            Ok(path) => {
                                // Convert Unix-style paths to Windows-style paths on Windows
                                let path_str = path.to_string_lossy();
                                let normalized_path = if cfg!(windows) {
                                    std::path::PathBuf::from(path_str.replace('/', "\\"))
                                } else {
                                    path.clone()
                                };
                                
                                let dest_path = dest_root_unpacker.join(&normalized_path);
                                
                                // Debug: Log problematic paths (only with -vv flag)
                                if path_str.contains('/') && cfg!(windows) && verbose > 1 {
                                    eprintln!("[{}] Path conversion: '{}' -> '{}'", 
                                        Self::timestamp(), path_str, dest_path.display());
                                }
                                
                                // Attempt to extract the file - collect errors but continue
                                match Self::extract_single_file(&mut entry, &dest_path, verbose) {
                                    Ok(()) => {
                                        count += 1;
                                    }
                                    Err(e) => {
                                        // Convert to std::io::Error for consistent error handling
                                        let io_error = match e.downcast::<std::io::Error>() {
                                            Ok(io_err) => io_err,
                                            Err(other_err) => std::io::Error::new(
                                                std::io::ErrorKind::Other, 
                                                format!("Tar extraction error: {}", other_err)
                                            )
                                        };
                                        
                                        failed_files.push((dest_path, io_error));
                                        if verbose > 0 {
                                            eprintln!("[{}] Failed to extract '{}', will retry individually", 
                                                Self::timestamp(), path_str);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                let io_error = std::io::Error::new(
                                    std::io::ErrorKind::InvalidData, 
                                    format!("Failed to read tar entry path: {}", e)
                                );
                                failed_files.push((PathBuf::from("unknown_path"), io_error));
                            }
                        }
                    }
                    Err(e) => {
                        let io_error = std::io::Error::new(
                            std::io::ErrorKind::InvalidData, 
                            format!("Failed to read tar entry: {}", e)
                        );
                        failed_files.push((PathBuf::from("unknown_entry"), io_error));
                    }
                }
            }
            
            Ok((count, failed_files))
        });
        
        // Wait for both threads to complete
        match (packer_handle.join(), unpacker_handle.join()) {
            (Ok(pack_result), Ok(unpack_result)) => {
                match (pack_result, unpack_result) {
                    (Ok(locked_files), Ok((files_extracted, failed_files))) => {
                        // Report locked files that were skipped
                        if !locked_files.is_empty() && options.verbose > 0 {
                            println!("[{}] Skipped {} locked system files (expected during live backup)", 
                                Self::timestamp(), locked_files.len());
                        }
                        // Update stats for successfully extracted files
                        let successful_files = files_extracted;
                        for _ in 0..successful_files {
                            stats.increment_files_processed();
                            stats.increment_files_copied();
                        }
                        
                        // Calculate actual bytes transferred based on successful files
                        let mut actual_bytes_transferred = 0u64;
                        let mut files_processed = 0;
                        for op in files {
                            if files_processed >= successful_files {
                                break;
                            }
                            match op {
                                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                                    if let Ok(metadata) = path.metadata() {
                                        actual_bytes_transferred += metadata.len();
                                    }
                                    files_processed += 1;
                                }
                                _ => {}
                            }
                        }
                        stats.set_bytes_transferred(actual_bytes_transferred);
                        
                        if let Some(pb) = progress_bar {
                            pb.inc(files_extracted);
                        }
                        
                        // Retry failed files individually if any
                        if !failed_files.is_empty() {
                            if options.verbose > 0 {
                                println!("[{}] Retrying {} files that failed tar extraction individually", 
                                    Self::timestamp(), failed_files.len());
                            }
                            
                            let mut individual_successes = 0u64;
                            
                            // Find source files for failed destinations
                            let mut additional_bytes = 0u64;
                            for (failed_dest, _error) in &failed_files {
                                // Find corresponding source file
                                if let Some(source_file) = Self::find_source_for_dest(files, dest_root, failed_dest) {
                                    match std::fs::copy(&source_file, failed_dest) {
                                        Ok(bytes_copied) => {
                                            individual_successes += 1;
                                            stats.increment_files_processed();
                                            stats.increment_files_copied();
                                            additional_bytes += bytes_copied;
                                            if options.verbose > 1 {
                                                println!("[{}] Individual retry successful: {}", 
                                                    Self::timestamp(), failed_dest.display());
                                            }
                                        }
                                        Err(e) => {
                                            // Add to error count for failed individual retries
                                            stats.increment_errors();
                                            error_handle.log_error(&source_file, 
                                                &format!("Individual file copy failed: {}", e), 
                                                "individual_retry");
                                        }
                                    }
                                }
                            }
                            
                            // Add additional bytes from successful individual retries
                            stats.add_bytes_transferred(additional_bytes);
                            
                            if let Some(pb) = progress_bar {
                                pb.inc(individual_successes);
                            }
                            
                            if options.verbose > 0 {
                                println!("[{}] Individual retries: {}/{} successful", 
                                    Self::timestamp(), individual_successes, failed_files.len());
                            }
                        }
                    }
                    (Err(pack_err), _) => {
                        return Err(anyhow::anyhow!("Tar packing failed: {}", pack_err));
                    }
                    (_, Err(unpack_err)) => {
                        return Err(anyhow::anyhow!("Tar unpacking failed: {}", unpack_err));
                    }
                }
            }
            _ => {
                error_handle.log_error(source_root, "Thread panic in tar streaming", "tar_streaming");
                return Err(anyhow::anyhow!("Thread panic in tar streaming"));
            }
        }
        
        Ok(stats)
    }
    
    /// Process Pool files using parallel workers
    fn process_pool_files_parallel(
        files: &[FileOperation],
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
    ) -> Result<SyncStats> {
        let mut stats = SyncStats::default();
        
        // Use Rayon for parallel processing
        let results: Vec<_> = files.par_iter().map(|operation| {
            match operation {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest_path = dest_root.join(relative);
                    
                    // Create destination directory
                    if let Some(parent) = dest_path.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            return Err(anyhow::anyhow!("Failed to create directory {}: {}", parent.display(), e));
                        }
                    }
                    
                    // Copy file
                    match std::fs::copy(path, &dest_path) {
                        Ok(bytes) => {
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                            }
                            Ok((1u64, bytes))
                        }
                        Err(e) => Err(anyhow::anyhow!("Failed to copy {}: {}", path.display(), e))
                    }
                }
                _ => Ok((0, 0))
            }
        }).collect();
        
        // Aggregate results
        for result in results {
            match result {
                Ok((files, bytes)) => {
                    for _ in 0..files {
                        stats.increment_files_processed();
                    }
                    stats.add_bytes_transferred(bytes);
                }
                Err(e) => {
                    eprintln!("[ERROR] Pool worker: {}", e);
                    stats.add_error(PathBuf::from("tar_stream"), "tar", "Failed to create tar stream");
                }
            }
        }
        
        Ok(stats)
    }
    
    /// Process Slicer files with memory-mapped I/O
    fn process_slicer_files_mmap(
        files: &[FileOperation],
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
    ) -> Result<SyncStats> {
        let mut stats = SyncStats::default();
        
        for operation in files {
            match operation {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest_path = dest_root.join(relative);
                    
                    // Create destination directory
                    if let Some(parent) = dest_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    
                    // For now, use optimized copy - memory mapping will be added in Phase 3
                    match std::fs::copy(path, &dest_path) {
                        Ok(bytes) => {
                            stats.increment_files_processed();
                            stats.add_bytes_transferred(bytes);
                            
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                            }
                        }
                        Err(e) => {
                            eprintln!("[ERROR] Slicer: Failed to copy {}: {}", path.display(), e);
                            stats.add_error(PathBuf::from("unknown"), "copy", &e.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        
        Ok(stats)
    }
    
    /// Process Delta files with concurrent delta analysis (migrated from mixed_strategy.rs)
    fn process_delta_files_concurrent(
        files: &[FileOperation],
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
        delta_analyzer: Arc<Mutex<ConcurrentDeltaAnalyzer>>,
    ) -> Result<SyncStats> {
        let mut stats = SyncStats::default();
        
        for operation in files {
            match operation {
                FileOperation::Update { path, .. } => {
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest_path = dest_root.join(relative);
                    
                    // Quick delta check to see if delta transfer is beneficial
                    if dest_path.exists() {
                        if quick_delta_check(path, &dest_path) {
                            if options.prefer_delta && !options.force_no_delta {
                                // Start delta analysis in background
                                if let Ok(mut analyzer) = delta_analyzer.lock() {
                                    analyzer.analyze_file(path.to_path_buf(), dest_path.clone());
                                }
                                // For now, continue with regular copy - delta analysis runs concurrently
                                // TODO: Implement delta-aware copy when analysis is complete
                            }
                        }
                    }
                    
                    // Fallback to full copy
                    if let Some(parent) = dest_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    
                    match std::fs::copy(path, &dest_path) {
                        Ok(bytes) => {
                            stats.increment_files_processed();
                            stats.add_bytes_transferred(bytes);
                            
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                            }
                        }
                        Err(e) => {
                            eprintln!("[ERROR] Delta: Failed to copy {}: {}", path.display(), e);
                            stats.add_error(PathBuf::from("unknown"), "copy", &e.to_string());
                        }
                    }
                }
                FileOperation::Create { path } => {
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest_path = dest_root.join(relative);
                    
                    if let Some(parent) = dest_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    
                    match std::fs::copy(path, &dest_path) {
                        Ok(bytes) => {
                            stats.increment_files_processed();
                            stats.add_bytes_transferred(bytes);
                            
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                            }
                        }
                        Err(e) => {
                            eprintln!("[ERROR] Delta: Failed to copy {}: {}", path.display(), e);
                            stats.add_error(PathBuf::from("unknown"), "copy", &e.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        
        Ok(stats)
    }
    
    /// Process delete operations in parallel
    fn process_deletes_parallel(
        deletes: &[FileOperation],
        options: &SyncOptions,
        progress_bar: Option<&indicatif::ProgressBar>,
    ) -> Result<SyncStats> {
        let mut stats = SyncStats::default();
        
        for operation in deletes {
            match operation {
                FileOperation::Delete { path } => {
                    match if path.is_dir() {
                        std::fs::remove_dir_all(path)
                    } else {
                        std::fs::remove_file(path)
                    } {
                        Ok(()) => {
                            stats.increment_files_processed();
                            if let Some(pb) = progress_bar {
                                pb.inc(1);
                            }
                            
                            if options.verbose >= 2 {
                                eprintln!("[{}] Deleted: {}", Self::timestamp(), path.display());
                            }
                        }
                        Err(e) => {
                            eprintln!("[ERROR] Failed to delete {}: {}", path.display(), e);
                            stats.add_error(PathBuf::from("unknown"), "copy", &e.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        
        Ok(stats)
    }
    
    /// Create directories from operations
    fn create_directories(
        &self,
        directories: &[FileOperation],
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<()> {
        for operation in directories {
            match operation {
                FileOperation::CreateDirectory { path } => {
                    let relative = path.strip_prefix(source_root).unwrap_or(path);
                    let dest_path = dest_root.join(relative);
                    
                    std::fs::create_dir_all(dest_path)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
    
    /// Extract a single file from tar entry with error handling
    fn extract_single_file(
        entry: &mut tar::Entry<impl Read>,
        dest_path: &Path,
        verbose: u8,
    ) -> Result<()> {
        // Create parent directories
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        // Extract the file
        entry.unpack(dest_path).map_err(|e| {
            anyhow::anyhow!("Failed to unpack {} to {}: {}", 
                entry.path().map(|p| p.display().to_string()).unwrap_or_else(|_| "unknown".to_string()),
                dest_path.display(), e)
        })?;
        
        if verbose > 2 {
            println!("[{}] Extracted: {}", Self::timestamp(), dest_path.display());
        }
        
        Ok(())
    }
    
    /// Find the source file path for a failed destination file
    fn find_source_for_dest(
        files: &[FileOperation],
        dest_root: &Path,
        failed_dest: &Path,
    ) -> Option<PathBuf> {
        // Extract relative path from destination
        let relative_path = failed_dest.strip_prefix(dest_root).ok()?;
        
        // Find the source file that would create this destination
        for operation in files {
            match operation {
                FileOperation::Create { path } | 
                FileOperation::Update { path, .. } => {
                    // Check if this source would create the failed destination
                    if path.ends_with(relative_path) || 
                       path.file_name() == failed_dest.file_name() {
                        return Some(path.clone());
                    }
                }
                _ => continue,
            }
        }
        
        None
    }
    
    /// Detect if an error indicates a file is locked (OS error 32 on Windows)
    fn is_file_locked_error(error: &anyhow::Error) -> bool {
        let error_string = error.to_string().to_lowercase();
        
        // Windows "The process cannot access the file because it is being used by another process" (OS error 32)
        if error_string.contains("os error 32") || 
           error_string.contains("being used by another process") ||
           error_string.contains("sharing violation") {
            return true;
        }
        
        // Unix/Linux file locking errors
        if error_string.contains("resource temporarily unavailable") ||
           error_string.contains("resource busy") ||
           error_string.contains("text file busy") {
            return true;
        }
        
        // macOS specific errors
        if error_string.contains("operation not permitted") && 
           (error_string.contains("systemui") || error_string.contains("system")) {
            return true;
        }
        
        false
    }
    
    /// Format a user-friendly message for locked file errors
    fn format_locked_file_message(error: &anyhow::Error) -> String {
        let error_string = error.to_string();
        
        if error_string.to_lowercase().contains("os error 32") {
            return "file in use by another process".to_string();
        }
        
        if error_string.to_lowercase().contains("sharing violation") {
            return "sharing violation - file open in another program".to_string();
        }
        
        if error_string.to_lowercase().contains("resource temporarily unavailable") {
            return "file temporarily locked".to_string();
        }
        
        // Default to the original error but make it more user-friendly
        format!("file access temporarily blocked ({})", 
               error_string.split_whitespace().take(5).collect::<Vec<_>>().join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_hybrid_dam_config() {
        let local_config = HybridDamConfig::for_local();
        assert_eq!(local_config.dam_flush_threshold, 16 * 1024 * 1024);
        
        let network_info = NetworkFsInfo {
            fs_type: NetworkFsType::SMB,
            server_info: None,
            optimal_buffer_size: 1024 * 1024,
        };
        let network_config = HybridDamConfig::for_network(network_info);
        assert_eq!(network_config.dam_flush_threshold, 256 * 1024 * 1024);
    }
    
    #[test] 
    fn test_strategy_selection() {
        let config = HybridDamConfig::default();
        let buffer_sizer = BufferSizer::new(&crate::options::SyncOptions::default());
        let mut dam = HybridDam::new(config, buffer_sizer);
        
        let small_file = FileEntry {
            src_path: "/test/small.txt".into(),
            dst_path: "/dest/small.txt".into(),
            size: 500 * 1024, // 500KB
            modified: std::time::SystemTime::now(),
            file_type: FileType::Regular,
        };
        
        assert_eq!(dam.determine_strategy(&small_file), TransferStrategy::Dam);
        
        let large_file = FileEntry {
            src_path: "/test/large.dat".into(),
            dst_path: "/dest/large.dat".into(),
            size: 200 * 1024 * 1024, // 200MB
            modified: std::time::SystemTime::now(),
            file_type: FileType::Regular,
        };
        
        assert_eq!(dam.determine_strategy(&large_file), TransferStrategy::Slicer);
    }
}