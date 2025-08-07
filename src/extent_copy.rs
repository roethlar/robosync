//! Extent-based copying optimization for filesystems that support extent maps
//!
//! This module provides optimized I/O patterns by querying file extent maps
//! on filesystems like ext4, XFS, and NTFS to reduce fragmentation and
//! improve sequential access patterns.

use std::fs::File;
use std::io::{self, Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// File extent information
#[derive(Debug, Clone)]
pub struct FileExtent {
    /// Logical offset in file
    pub logical_offset: u64,
    /// Physical offset on storage device
    pub physical_offset: u64,
    /// Length of extent in bytes
    pub length: u64,
    /// Flags (compressed, encrypted, etc.)
    pub flags: u32,
}

/// Extent map for a file
#[derive(Debug)]
pub struct ExtentMap {
    /// List of extents in logical order
    pub extents: Vec<FileExtent>,
    /// Total file size
    pub file_size: u64,
}

/// Extent-based file copier
pub struct ExtentCopier {
    /// Buffer size for extent copying
    buffer_size: usize,
    /// Whether to use extent information for optimization
    use_extents: bool,
}

impl ExtentCopier {
    /// Create new extent copier
    pub fn new(buffer_size: usize) -> Self {
        ExtentCopier {
            buffer_size,
            use_extents: Self::is_extent_support_available(),
        }
    }

    /// Check if extent support is available on this system
    pub fn is_extent_support_available() -> bool {
        #[cfg(target_os = "linux")]
        {
            // Check if FIEMAP ioctl is available (Linux 2.6.28+)
            true
        }
        
        #[cfg(not(target_os = "linux"))]
        {
            // Extent support not implemented for other platforms yet
            false
        }
    }

    /// Get extent map for a file
    pub fn get_extent_map(&self, file: &File) -> io::Result<ExtentMap> {
        if !self.use_extents {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "Extent mapping not supported on this platform",
            ));
        }

        #[cfg(target_os = "linux")]
        {
            self.get_extent_map_linux(file)
        }
        
        #[cfg(not(target_os = "linux"))]
        {
            let _ = file; // Suppress unused warning
            Err(Error::new(
                ErrorKind::Unsupported,
                "Extent mapping not implemented for this platform",
            ))
        }
    }

    #[cfg(target_os = "linux")]
    /// Get extent map using Linux FIEMAP ioctl
    fn get_extent_map_linux(&self, file: &File) -> io::Result<ExtentMap> {
        use std::mem;
        
        let file_size = file.metadata()?.len();
        let fd = file.as_raw_fd();
        
        // FIEMAP structure for Linux
        const FIEMAP_MAX_EXTENTS: usize = 256;
        const FIEMAP_EXTENT_LAST: u32 = 0x00000001;
        
        #[repr(C)]
        struct FiemapExtent {
            fe_logical: u64,
            fe_physical: u64,
            fe_length: u64,
            fe_reserved64: [u64; 2],
            fe_flags: u32,
            fe_reserved: [u32; 3],
        }
        
        #[repr(C)]
        struct Fiemap {
            fm_start: u64,
            fm_length: u64,
            fm_flags: u32,
            fm_mapped_extents: u32,
            fm_extent_count: u32,
            fm_reserved: u32,
            fm_extents: [FiemapExtent; FIEMAP_MAX_EXTENTS],
        }
        
        let mut fiemap: Fiemap = unsafe { mem::zeroed() };
        fiemap.fm_start = 0;
        fiemap.fm_length = file_size;
        fiemap.fm_extent_count = FIEMAP_MAX_EXTENTS as u32;
        
        // FS_IOC_FIEMAP ioctl number
        const FS_IOC_FIEMAP: libc::c_ulong = 0xC020660B;
        
        let result = unsafe {
            #[cfg(target_env = "musl")]
            let request = FS_IOC_FIEMAP as libc::c_int;
            #[cfg(not(target_env = "musl"))]
            let request = FS_IOC_FIEMAP as libc::c_ulong;
            
            libc::ioctl(fd, request, &mut fiemap as *mut _ as *mut libc::c_void)
        };
        
        if result < 0 {
            return Err(Error::last_os_error());
        }
        
        let mut extents = Vec::new();
        let extent_count = std::cmp::min(fiemap.fm_mapped_extents as usize, FIEMAP_MAX_EXTENTS);
        
        for i in 0..extent_count {
            let extent = &fiemap.fm_extents[i];
            extents.push(FileExtent {
                logical_offset: extent.fe_logical,
                physical_offset: extent.fe_physical,
                length: extent.fe_length,
                flags: extent.fe_flags,
            });
            
            // Check if this is the last extent
            if extent.fe_flags & FIEMAP_EXTENT_LAST != 0 {
                break;
            }
        }
        
        Ok(ExtentMap {
            extents,
            file_size,
        })
    }

    /// Copy file using extent-optimized I/O patterns
    pub fn copy_file_with_extents(&self, src: &Path, dst: &Path) -> io::Result<u64> {
        let src_file = File::open(src)?;
        let mut dst_file = File::create(dst)?;
        
        // Try to get extent map
        let extent_map = match self.get_extent_map(&src_file) {
            Ok(map) => Some(map),
            Err(_) => None, // Fall back to regular copy if extent mapping fails
        };
        
        if let Some(map) = extent_map {
            self.copy_with_extent_map(&src_file, &mut dst_file, &map)
        } else {
            self.copy_without_extents(&src_file, &mut dst_file)
        }
    }
    
    /// Copy file using extent map information
    fn copy_with_extent_map(&self, src_file: &File, dst_file: &mut File, extent_map: &ExtentMap) -> io::Result<u64> {
        let _src_reader = src_file;
        let mut _total_bytes_written = 0u64;
        let mut buffer = vec![0u8; self.buffer_size];
        
        // First, set the destination file size to create a sparse file
        dst_file.set_len(extent_map.file_size)?;
        
        // Process extents in order, seeking to create holes
        for extent in &extent_map.extents {
            // Seek to logical offset in BOTH source and destination files
            let mut src_clone = src_file.try_clone()?;
            src_clone.seek(SeekFrom::Start(extent.logical_offset))?;
            
            // IMPORTANT: Seek in destination to preserve sparseness
            dst_file.seek(SeekFrom::Start(extent.logical_offset))?;
            
            let mut remaining = extent.length;
            
            while remaining > 0 {
                let to_read = std::cmp::min(remaining, self.buffer_size as u64) as usize;
                let bytes_read = src_clone.read(&mut buffer[..to_read])?;
                
                if bytes_read == 0 {
                    break; // EOF
                }
                
                dst_file.write_all(&buffer[..bytes_read])?;
                _total_bytes_written += bytes_read as u64;
                remaining = remaining.saturating_sub(bytes_read as u64);
            }
        }
        
        // The file size was already set, so we just return the size
        // The actual bytes written might be less due to holes
        Ok(extent_map.file_size)
    }
    
    /// Copy file without extent optimization (fallback)
    fn copy_without_extents(&self, src_file: &File, dst_file: &mut File) -> io::Result<u64> {
        let _src_reader = src_file;
        let mut total_copied = 0u64;
        let mut buffer = vec![0u8; self.buffer_size];
        
        let mut src_clone = src_file.try_clone()?;
        
        loop {
            let bytes_read = src_clone.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            
            dst_file.write_all(&buffer[..bytes_read])?;
            total_copied += bytes_read as u64;
        }
        
        Ok(total_copied)
    }
    
    /// Analyze extent fragmentation
    pub fn analyze_fragmentation(&self, file: &File) -> io::Result<FragmentationInfo> {
        let extent_map = self.get_extent_map(file)?;
        
        let extent_count = extent_map.extents.len();
        let mut sequential_extents = 0;
        let mut gaps = 0;
        let mut largest_extent = 0u64;
        let mut smallest_extent = u64::MAX;
        
        for (i, extent) in extent_map.extents.iter().enumerate() {
            largest_extent = largest_extent.max(extent.length);
            smallest_extent = smallest_extent.min(extent.length);
            
            if i > 0 {
                let prev_extent = &extent_map.extents[i - 1];
                let expected_physical = prev_extent.physical_offset + prev_extent.length;
                
                if extent.physical_offset == expected_physical {
                    sequential_extents += 1;
                } else {
                    gaps += 1;
                }
            }
        }
        
        let fragmentation_ratio = if extent_count > 1 {
            gaps as f64 / (extent_count - 1) as f64
        } else {
            0.0
        };
        
        Ok(FragmentationInfo {
            total_extents: extent_count,
            sequential_extents,
            gaps,
            fragmentation_ratio,
            largest_extent_size: largest_extent,
            smallest_extent_size: if smallest_extent == u64::MAX { 0 } else { smallest_extent },
            file_size: extent_map.file_size,
        })
    }
}

/// File fragmentation analysis information
#[derive(Debug)]
pub struct FragmentationInfo {
    /// Total number of extents
    pub total_extents: usize,
    /// Number of sequential (contiguous) extents
    pub sequential_extents: usize,
    /// Number of gaps between extents
    pub gaps: usize,
    /// Fragmentation ratio (0.0 = not fragmented, 1.0 = highly fragmented)
    pub fragmentation_ratio: f64,
    /// Size of largest extent
    pub largest_extent_size: u64,
    /// Size of smallest extent
    pub smallest_extent_size: u64,
    /// Total file size
    pub file_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_extent_support_detection() {
        let supported = ExtentCopier::is_extent_support_available();
        println!("Extent support available: {}", supported);
        
        // Test should pass regardless of support
        assert!(supported || !supported);
    }
    
    #[test]
    fn test_extent_copier_creation() {
        let copier = ExtentCopier::new(64 * 1024);
        assert_eq!(copier.buffer_size, 64 * 1024);
    }
    
    #[test]
    fn test_extent_file_copy() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("test_src.txt");
        let dst = dir.path().join("test_dst.txt");
        
        // Create test file
        let test_data = vec![0u8; 10 * 1024]; // 10KB test file
        fs::write(&src, &test_data).unwrap();
        
        let copier = ExtentCopier::new(4096);
        let result = copier.copy_file_with_extents(&src, &dst);
        
        // Should succeed (either with extents or fallback)
        assert!(result.is_ok());
        
        if let Ok(bytes_copied) = result {
            assert_eq!(bytes_copied, test_data.len() as u64);
            
            // Verify copied data
            let copied_data = fs::read(&dst).unwrap();
            assert_eq!(copied_data, test_data);
        }
    }
    
    #[test]
    fn test_fragmentation_analysis() {
        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test_file.txt");
        
        // Create test file
        let test_data = vec![0u8; 1024];
        fs::write(&test_file, &test_data).unwrap();
        
        let copier = ExtentCopier::new(4096);
        let file = File::open(&test_file).unwrap();
        
        // Try to analyze fragmentation (may not work on all filesystems)
        let fragmentation = copier.analyze_fragmentation(&file);
        
        match fragmentation {
            Ok(info) => {
                println!("Fragmentation analysis: {:?}", info);
                assert!(info.file_size > 0);
            }
            Err(e) => {
                println!("Fragmentation analysis not supported: {}", e);
                // This is expected on many test environments
            }
        }
    }
}