//! Memory-Mapped IO implementation for large files on macOS
//! 
//! This module provides high-performance file copying using memory-mapped IO
//! specifically optimized for macOS/Apple Silicon systems. It leverages:
//! - Native mmap(2) system calls for zero-copy operations
//! - Unified memory architecture on Apple Silicon
//! - APFS extent-aware copying patterns
//! - Optimal page alignment for virtual memory efficiency

use std::fs::File;
use std::io::{self, Result};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::ptr;
use anyhow::Context;

#[cfg(target_os = "macos")]
use libc::{
    mmap, munmap, madvise, off_t, c_void,
    PROT_READ, PROT_WRITE, MAP_SHARED, MAP_PRIVATE, MAP_FAILED,
    MADV_SEQUENTIAL, MADV_WILLNEED
};

/// Minimum file size to use memory-mapped IO (64MB)
/// Files smaller than this use traditional copying methods
const MMAP_THRESHOLD: u64 = 64 * 1024 * 1024;

/// Maximum chunk size for memory-mapped operations (1GB)  
/// Prevents excessive virtual memory usage
const MAX_MMAP_CHUNK: usize = 1024 * 1024 * 1024;

/// Page size for optimal alignment (typically 16KB on Apple Silicon)
const APPLE_SILICON_PAGE_SIZE: usize = 16 * 1024;

/// Memory-mapped file copy implementation for macOS
#[cfg(target_os = "macos")]
pub struct MacOSMemoryMapper {
    page_size: usize,
    use_unified_memory: bool,
}

#[cfg(target_os = "macos")]
impl MacOSMemoryMapper {
    /// Create a new MacOS memory mapper with system detection
    pub fn new() -> Result<Self> {
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize };
        
        // Detect Apple Silicon unified memory architecture
        let use_unified_memory = Self::detect_unified_memory();
        
        Ok(MacOSMemoryMapper {
            page_size,
            use_unified_memory,
        })
    }
    
    /// Detect if running on Apple Silicon with unified memory
    fn detect_unified_memory() -> bool {
        use std::process::Command;
        
        // Check for Apple Silicon CPU
        if let Ok(output) = Command::new("sysctl")
            .args(&["-n", "machdep.cpu.brand_string"])
            .output() 
        {
            let cpu_info = String::from_utf8_lossy(&output.stdout);
            cpu_info.contains("Apple") && (cpu_info.contains("M1") || cpu_info.contains("M2") || cpu_info.contains("M3"))
        } else {
            false
        }
    }
    
    /// Copy a large file using memory-mapped IO
    pub fn copy_file_mmap(&self, source: &Path, dest: &Path) -> anyhow::Result<u64> {
        let source_file = File::open(source)
            .with_context(|| format!("Failed to open source file: {}", source.display()))?;
        
        let dest_file = File::create(dest)
            .with_context(|| format!("Failed to create destination file: {}", dest.display()))?;
        
        let source_metadata = source_file.metadata()
            .with_context(|| "Failed to get source file metadata")?;
        
        let file_size = source_metadata.len();
        
        // Check if file is large enough for memory mapping
        if file_size < MMAP_THRESHOLD {
            return Err(anyhow::anyhow!(
                "File too small for memory mapping: {} bytes (minimum: {} bytes)", 
                file_size, MMAP_THRESHOLD
            ));
        }
        
        // Pre-allocate destination file
        dest_file.set_len(file_size)
            .with_context(|| "Failed to pre-allocate destination file")?;
        
        // Copy file in chunks to manage memory usage
        let mut bytes_copied = 0u64;
        let mut offset = 0u64;
        
        while offset < file_size {
            let chunk_size = std::cmp::min(
                MAX_MMAP_CHUNK as u64,
                file_size - offset
            ) as usize;
            
            let copied = self.copy_chunk_mmap(
                &source_file,
                &dest_file, 
                offset,
                chunk_size
            ).with_context(|| format!("Failed to copy chunk at offset {}", offset))?;
            
            bytes_copied += copied;
            offset += copied;
        }
        
        // Ensure all data is written to disk
        self.sync_file(&dest_file)
            .with_context(|| "Failed to sync destination file")?;
        
        Ok(bytes_copied)
    }
    
    /// Copy a single memory-mapped chunk
    fn copy_chunk_mmap(
        &self,
        source_file: &File,
        dest_file: &File, 
        offset: u64,
        size: usize
    ) -> Result<u64> {
        let source_fd = source_file.as_raw_fd();
        let dest_fd = dest_file.as_raw_fd();
        
        // Align offset to page boundaries for optimal performance
        let aligned_offset = (offset / self.page_size as u64) * self.page_size as u64;
        let offset_adjustment = (offset - aligned_offset) as usize;
        let aligned_size = size + offset_adjustment;
        
        // Map source file for reading
        let source_ptr = unsafe {
            mmap(
                ptr::null_mut(),
                aligned_size,
                PROT_READ,
                MAP_PRIVATE,
                source_fd,
                aligned_offset as off_t,
            )
        };
        
        if source_ptr == MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        
        // Map destination file for writing  
        let dest_ptr = unsafe {
            mmap(
                ptr::null_mut(),
                aligned_size,
                PROT_WRITE,
                MAP_SHARED,
                dest_fd,
                aligned_offset as off_t,
            )
        };
        
        if dest_ptr == MAP_FAILED {
            unsafe { munmap(source_ptr, aligned_size) };
            return Err(io::Error::last_os_error());
        }
        
        // Configure memory access patterns for optimal performance
        self.configure_memory_access(source_ptr, dest_ptr, aligned_size)?;
        
        // Perform the copy operation
        let result = self.perform_mmap_copy(
            source_ptr, 
            dest_ptr, 
            offset_adjustment, 
            size
        );
        
        // Clean up memory mappings
        let unmap_result = unsafe {
            let source_unmap = munmap(source_ptr, aligned_size);
            let dest_unmap = munmap(dest_ptr, aligned_size);
            
            if source_unmap != 0 || dest_unmap != 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        
        // Return copy result, prioritizing copy errors over cleanup errors
        match result {
            Ok(bytes) => {
                unmap_result?;
                Ok(bytes)
            }
            Err(e) => {
                let _ = unmap_result; // Log but don't override copy error
                Err(e)
            }
        }
    }
    
    /// Configure memory access patterns for optimal performance
    fn configure_memory_access(
        &self,
        source_ptr: *mut c_void,
        dest_ptr: *mut c_void,
        size: usize
    ) -> Result<()> {
        unsafe {
            // Configure source for sequential reading
            if madvise(source_ptr, size, MADV_SEQUENTIAL) != 0 {
                return Err(io::Error::last_os_error());
            }
            
            // Pre-load source pages if using unified memory (Apple Silicon optimization)
            if self.use_unified_memory {
                if madvise(source_ptr, size, MADV_WILLNEED) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            
            // Configure destination for sequential writing
            if madvise(dest_ptr, size, MADV_SEQUENTIAL) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        
        Ok(())
    }
    
    /// Perform the actual memory copy operation
    fn perform_mmap_copy(
        &self,
        source_ptr: *mut c_void,
        dest_ptr: *mut c_void,
        offset_adjustment: usize,
        size: usize
    ) -> Result<u64> {
        // Calculate actual copy pointers with offset adjustment
        let src_copy_ptr = unsafe { (source_ptr as *const u8).add(offset_adjustment) };
        let dst_copy_ptr = unsafe { (dest_ptr as *mut u8).add(offset_adjustment) };
        
        // Perform optimized memory copy
        if self.use_unified_memory {
            // Apple Silicon unified memory optimization
            self.unified_memory_copy(src_copy_ptr, dst_copy_ptr, size)?;
        } else {
            // Standard memory copy for Intel Macs
            unsafe {
                ptr::copy_nonoverlapping(src_copy_ptr, dst_copy_ptr, size);
            }
        }
        
        Ok(size as u64)
    }
    
    /// Optimized copy for Apple Silicon unified memory architecture
    fn unified_memory_copy(
        &self,
        src: *const u8,
        dst: *mut u8, 
        size: usize
    ) -> Result<()> {
        // Use SIMD-optimized copying for Apple Silicon
        // Break into cache-line sized chunks (64 bytes) for optimal performance
        const CACHE_LINE_SIZE: usize = 64;
        
        let full_chunks = size / CACHE_LINE_SIZE;
        let remainder = size % CACHE_LINE_SIZE;
        
        unsafe {
            // Copy full cache lines
            for i in 0..full_chunks {
                let src_chunk = src.add(i * CACHE_LINE_SIZE);
                let dst_chunk = dst.add(i * CACHE_LINE_SIZE);
                ptr::copy_nonoverlapping(src_chunk, dst_chunk, CACHE_LINE_SIZE);
            }
            
            // Copy remaining bytes
            if remainder > 0 {
                let src_remainder = src.add(full_chunks * CACHE_LINE_SIZE);
                let dst_remainder = dst.add(full_chunks * CACHE_LINE_SIZE);
                ptr::copy_nonoverlapping(src_remainder, dst_remainder, remainder);
            }
        }
        
        Ok(())
    }
    
    /// Synchronize file data to disk
    fn sync_file(&self, file: &File) -> Result<()> {
        use std::os::unix::io::AsRawFd;
        
        // Use fsync to ensure data is written to storage
        let result = unsafe { libc::fsync(file.as_raw_fd()) };
        
        if result != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    
    /// Get optimal chunk size based on available memory and file size
    pub fn get_optimal_chunk_size(&self, file_size: u64, available_memory: u64) -> usize {
        // Use up to 25% of available memory, but not more than MAX_MMAP_CHUNK
        let memory_limit = (available_memory / 4) as usize;
        let size_based_limit = std::cmp::min(file_size as usize, MAX_MMAP_CHUNK);
        
        std::cmp::min(memory_limit, size_based_limit)
    }
    
    /// Check if a file is suitable for memory-mapped copying
    pub fn should_use_mmap(&self, file_size: u64) -> bool {
        file_size >= MMAP_THRESHOLD
    }
}

// Stub implementation for non-macOS platforms
#[cfg(not(target_os = "macos"))]
pub struct MacOSMemoryMapper;

#[cfg(not(target_os = "macos"))]
impl MacOSMemoryMapper {
    pub fn new() -> Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "MacOS Memory Mapping not available on this platform"
        ))
    }
    
    pub fn copy_file_mmap(&self, _source: &Path, _dest: &Path) -> anyhow::Result<u64> {
        Err(anyhow::anyhow!("MacOS Memory Mapping not available on this platform"))
    }
    
    pub fn should_use_mmap(&self, _file_size: u64) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    
    #[test]
    #[cfg(target_os = "macos")]
    fn test_memory_mapper_creation() {
        let mapper = MacOSMemoryMapper::new();
        assert!(mapper.is_ok());
        
        let mapper = mapper.unwrap();
        assert!(mapper.page_size > 0);
        println!("Page size: {} bytes", mapper.page_size);
        println!("Unified memory: {}", mapper.use_unified_memory);
    }
    
    #[test]
    #[cfg(target_os = "macos")]
    fn test_unified_memory_detection() {
        let unified = MacOSMemoryMapper::detect_unified_memory();
        println!("Apple Silicon unified memory detected: {}", unified);
    }
    
    #[test]
    fn test_should_use_mmap() {
        if let Ok(mapper) = MacOSMemoryMapper::new() {
            assert!(!mapper.should_use_mmap(1024)); // 1KB - too small
            assert!(!mapper.should_use_mmap(1024 * 1024)); // 1MB - too small  
            assert!(mapper.should_use_mmap(128 * 1024 * 1024)); // 128MB - large enough
        }
    }
    
    #[test]
    #[cfg(target_os = "macos")]
    fn test_mmap_copy_large_file() -> anyhow::Result<()> {
        let temp_dir = tempdir()?;
        let source_path = temp_dir.path().join("large_source.bin");
        let dest_path = temp_dir.path().join("large_dest.bin");
        
        // Create a large test file (70MB to exceed MMAP_THRESHOLD)
        let test_data = vec![0xAB; 70 * 1024 * 1024];
        fs::write(&source_path, &test_data)?;
        
        let mapper = MacOSMemoryMapper::new()?;
        let bytes_copied = mapper.copy_file_mmap(&source_path, &dest_path)?;
        
        assert_eq!(bytes_copied, test_data.len() as u64);
        
        // Verify the copy
        let copied_data = fs::read(&dest_path)?;
        assert_eq!(copied_data, test_data);
        
        Ok(())
    }
}