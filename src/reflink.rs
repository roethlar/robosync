//! Copy-on-write (reflink) support for instant file copies on supported filesystems
//!
//! This module provides platform-specific implementations for creating reflinks
//! (copy-on-write clones) of files on filesystems that support this feature.
//! 
//! Supported platforms and filesystems:
//! - Linux: BTRFS, XFS (via FICLONE ioctl)
//! - macOS: APFS (via clonefile syscall)
//! - Future: Windows ReFS, FreeBSD ZFS

use std::fmt;
use std::io;
use std::path::Path;

use crate::filesystem_info::{are_on_same_filesystem, get_filesystem_info};

/// Specific error types for reflink operations
#[derive(Debug)]
pub enum ReflinkError {
    /// Source and destination are on different filesystems
    CrossDevice,
    /// Filesystem doesn't support reflink operations
    NotSupported(String), // filesystem type name
    /// Not enough space for reflink metadata
    NoSpace,
    /// Generic I/O error
    Io(io::Error),
}

impl fmt::Display for ReflinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReflinkError::CrossDevice => write!(f, "Cannot reflink across different filesystems"),
            ReflinkError::NotSupported(fs) => write!(f, "Filesystem '{}' does not support reflinks", fs),
            ReflinkError::NoSpace => write!(f, "Not enough space for reflink metadata"),
            ReflinkError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for ReflinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReflinkError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ReflinkError {
    fn from(error: io::Error) -> Self {
        ReflinkError::Io(error)
    }
}

/// Result of a reflink operation
#[derive(Debug)]
pub enum ReflinkResult {
    /// Reflink succeeded - file was cloned instantly
    Success,
    /// Reflink not supported or failed - caller should use regular copy
    Fallback,
    /// Hard error occurred - operation should not be retried
    Error(ReflinkError),
}

/// Reflink operation mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReflinkMode {
    /// Always attempt reflink, fail if not possible
    Always,
    /// Attempt reflink, fallback to regular copy if not possible (default)
    Auto,
    /// Never attempt reflink, always use regular copy
    Never,
}

impl Default for ReflinkMode {
    fn default() -> Self {
        ReflinkMode::Auto
    }
}

/// Options for reflink operations
#[derive(Debug, Clone)]
pub struct ReflinkOptions {
    /// The reflink mode to use
    pub mode: ReflinkMode,
}

impl Default for ReflinkOptions {
    fn default() -> Self {
        ReflinkOptions {
            mode: ReflinkMode::default(),
        }
    }
}

/// Attempt to create a reflink (copy-on-write clone) of a file
///
/// This function will:
/// 1. Check if reflink is enabled (mode != Never)
/// 2. Verify source and destination are on the same filesystem
/// 3. Check if the filesystem supports reflinks
/// 4. Attempt the platform-specific reflink operation
/// 5. Return Success, Fallback, or Error based on the result
///
/// # Arguments
/// * `src` - Source file path
/// * `dst` - Destination file path  
/// * `options` - Reflink options including mode
///
/// # Returns
/// * `ReflinkResult::Success` - Reflink succeeded
/// * `ReflinkResult::Fallback` - Reflink not possible, use regular copy
/// * `ReflinkResult::Error` - Hard error occurred
pub fn try_reflink(src: &Path, dst: &Path, options: &ReflinkOptions) -> ReflinkResult {
    // Never mode - immediate fallback
    if options.mode == ReflinkMode::Never {
        return ReflinkResult::Fallback;
    }

    // Check if files are on the same filesystem
    match are_on_same_filesystem(src, dst) {
        Ok(false) => {
            // Different filesystems - can't reflink
            return if options.mode == ReflinkMode::Always {
                ReflinkResult::Error(ReflinkError::CrossDevice)
            } else {
                ReflinkResult::Fallback
            };
        }
        Err(e) => {
            // Error checking filesystem - for Auto mode, treat as fallback
            // This handles cases where destination doesn't exist yet
            return if options.mode == ReflinkMode::Always {
                ReflinkResult::Error(e.into())
            } else {
                ReflinkResult::Fallback  
            };
        }
        Ok(true) => {} // Same filesystem - continue
    }

    // Get filesystem info to check reflink support
    let fs_info = match get_filesystem_info(src) {
        Ok(info) => info,
        Err(e) => {
            // Error getting filesystem info - for Auto mode, treat as fallback
            return if options.mode == ReflinkMode::Always {
                ReflinkResult::Error(e.into())
            } else {
                ReflinkResult::Fallback
            };
        }
    };

    // Check if filesystem supports reflinks
    if !fs_info.supports_reflinks {
        return if options.mode == ReflinkMode::Always {
            ReflinkResult::Error(ReflinkError::NotSupported(
                format!("{:?}", fs_info.filesystem_type)
            ))
        } else {
            ReflinkResult::Fallback
        };
    }

    // Attempt platform-specific reflink
    #[cfg(target_os = "linux")]
    {
        reflink_linux(src, dst, options)
    }

    #[cfg(target_os = "macos")]
    {
        reflink_macos(src, dst, options)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // Platform not supported
        if options.mode == ReflinkMode::Always {
            ReflinkResult::Error(ReflinkError::NotSupported("platform".to_string()))
        } else {
            ReflinkResult::Fallback
        }
    }
}

#[cfg(target_os = "linux")]
fn reflink_linux(src: &Path, dst: &Path, options: &ReflinkOptions) -> ReflinkResult {
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;
    use nix::libc::FICLONE;

    // Open source file for reading
    let src_file = match OpenOptions::new().read(true).open(src) {
        Ok(f) => f,
        Err(e) => return ReflinkResult::Error(e.into()),
    };

    // Create/open destination file for writing
    let dst_file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(dst)
    {
        Ok(f) => f,
        Err(e) => return ReflinkResult::Error(e.into()),
    };

    // Perform the FICLONE ioctl
    let result = unsafe {
        nix::libc::ioctl(dst_file.as_raw_fd(), FICLONE as _, src_file.as_raw_fd())
    };

    if result == 0 {
        ReflinkResult::Success
    } else {
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(nix::libc::EXDEV) => {
                // Cross-device
                if options.mode == ReflinkMode::Always {
                    ReflinkResult::Error(ReflinkError::CrossDevice)
                } else {
                    ReflinkResult::Fallback
                }
            }
            Some(nix::libc::EOPNOTSUPP) => {
                // Not supported
                if options.mode == ReflinkMode::Always {
                    ReflinkResult::Error(ReflinkError::NotSupported("filesystem".to_string()))
                } else {
                    ReflinkResult::Fallback
                }
            }
            Some(nix::libc::ENOSPC) => {
                // No space
                ReflinkResult::Error(ReflinkError::NoSpace)
            }
            _ => {
                // Other errors are generic I/O errors
                ReflinkResult::Error(error.into())
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn reflink_macos(src: &Path, dst: &Path, options: &ReflinkOptions) -> ReflinkResult {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // Convert paths to C strings
    let src_cstr = match CString::new(src.as_os_str().as_bytes()) {
        Ok(s) => s,
        Err(_) => {
            return ReflinkResult::Error(ReflinkError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source path contains null byte",
            )))
        }
    };

    let dst_cstr = match CString::new(dst.as_os_str().as_bytes()) {
        Ok(s) => s,
        Err(_) => {
            return ReflinkResult::Error(ReflinkError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination path contains null byte",
            )))
        }
    };

    // Call clonefile
    let result = unsafe { clonefile(src_cstr.as_ptr(), dst_cstr.as_ptr(), 0) };

    if result == 0 {
        ReflinkResult::Success
    } else {
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EXDEV) => {
                // Cross-device
                if options.mode == ReflinkMode::Always {
                    ReflinkResult::Error(ReflinkError::CrossDevice)
                } else {
                    ReflinkResult::Fallback
                }
            }
            Some(libc::ENOTSUP) => {
                // Not supported
                if options.mode == ReflinkMode::Always {
                    ReflinkResult::Error(ReflinkError::NotSupported("filesystem".to_string()))
                } else {
                    ReflinkResult::Fallback
                }
            }
            Some(libc::ENOSPC) => {
                // No space
                ReflinkResult::Error(ReflinkError::NoSpace)
            }
            _ => {
                // Other errors are generic I/O errors
                ReflinkResult::Error(error.into())
            }
        }
    }
}

// External function declaration for macOS clonefile
#[cfg(target_os = "macos")]
extern "C" {
    fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32) -> libc::c_int;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use tempfile::tempdir;

    #[test]
    fn test_reflink_mode_default() {
        assert_eq!(ReflinkMode::default(), ReflinkMode::Auto);
    }

    #[test]
    fn test_reflink_never_mode() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        
        File::create(&src).unwrap();
        
        let options = ReflinkOptions {
            mode: ReflinkMode::Never,
        };
        
        let result = try_reflink(&src, &dst, &options);
        match result {
            ReflinkResult::Fallback => {} // Expected
            _ => panic!("Expected Fallback for Never mode"),
        }
    }

    #[test]
    fn test_reflink_different_filesystems() {
        // This test would need to be adjusted based on the test environment
        // For now, we'll create a simple test that verifies the logic
        let options = ReflinkOptions {
            mode: ReflinkMode::Auto,
        };
        
        // Test with non-existent paths to trigger filesystem check error
        let result = try_reflink(Path::new("/nonexistent/src"), Path::new("/nonexistent/dst"), &options);
        match result {
            ReflinkResult::Error(_) => {} // Expected
            _ => panic!("Expected error for non-existent paths"),
        }
    }

    #[test]
    fn test_reflink_always_mode() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        
        // Create source file
        fs::write(&src, b"test content").unwrap();
        
        let options = ReflinkOptions {
            mode: ReflinkMode::Always,
        };
        
        // Try reflink with Always mode
        let result = try_reflink(&src, &dst, &options);
        
        // On most test environments (like /tmp), reflink won't be supported
        // With Always mode, this should return an error, not fallback
        match result {
            ReflinkResult::Error(_) => {} // Expected - reflink not supported on test filesystem
            ReflinkResult::Fallback => panic!("Always mode should not fallback"),
            ReflinkResult::Success => {
                // If it succeeded, verify the file exists
                assert!(dst.exists());
                assert_eq!(fs::read(&dst).unwrap(), b"test content");
            }
        }
    }

    #[test]
    fn test_reflink_auto_mode_fallback() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        
        // Create source file
        fs::write(&src, b"test content").unwrap();
        
        let options = ReflinkOptions {
            mode: ReflinkMode::Auto,
        };
        
        // Try reflink with Auto mode
        let result = try_reflink(&src, &dst, &options);
        
        // On most test environments, this should either fallback or error due to filesystem checks
        match result {
            ReflinkResult::Fallback => {} // Expected - should fallback gracefully
            ReflinkResult::Error(ReflinkError::NotSupported(_)) => {} // Also acceptable - filesystem doesn't support reflinks
            ReflinkResult::Error(ReflinkError::Io(_)) => {} // Also acceptable - IO error during filesystem check
            ReflinkResult::Error(e) => panic!("Auto mode unexpected error: {}", e),
            ReflinkResult::Success => {
                // If it succeeded, verify the file exists
                assert!(dst.exists());
                assert_eq!(fs::read(&dst).unwrap(), b"test content");
            }
        }
    }

    #[test]
    fn test_reflink_options_default() {
        let options = ReflinkOptions::default();
        assert_eq!(options.mode, ReflinkMode::Auto);
    }

    #[test]
    fn test_reflink_cross_device_error() {
        // Test that cross-device reflinks are handled properly
        let options_always = ReflinkOptions {
            mode: ReflinkMode::Always,
        };
        
        let options_auto = ReflinkOptions {
            mode: ReflinkMode::Auto,
        };
        
        // Test with paths that would be on different filesystems if they existed
        // This tests the error path handling
        let src = Path::new("/definitely/not/a/real/path/src.txt");
        let dst = Path::new("/another/fake/path/dst.txt");
        
        // Always mode should error
        match try_reflink(src, dst, &options_always) {
            ReflinkResult::Error(_) => {} // Expected
            _ => panic!("Expected error for Always mode with non-existent paths"),
        }
        
        // Auto mode should also error (can't check filesystem for non-existent paths)
        match try_reflink(src, dst, &options_auto) {
            ReflinkResult::Error(_) => {} // Expected
            _ => panic!("Expected error for Auto mode with non-existent paths"),
        }
    }
}