//! Platform-specific file copy APIs for optimal performance
//!
//! This module provides fast, native file copying using OS-specific APIs:
//! - Windows: CopyFileEx with progress callback
//! - Linux: copy_file_range for zero-copy transfers
//! - macOS: copyfile preserving all metadata and resource forks

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

use crate::progress::ProgressTracker;
use crate::sync_stats::SyncStats;

/// Platform-specific file copier
pub struct PlatformCopier {
    progress: Option<Arc<dyn ProgressTracker>>,
}

impl Default for PlatformCopier {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformCopier {
    pub fn new() -> Self {
        Self { progress: None }
    }

    pub fn with_progress(mut self, progress: Arc<dyn ProgressTracker>) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Copy a single file using the best platform-specific method
    pub fn copy_file(&self, source: &Path, dest: &Path) -> Result<u64> {
        #[cfg(target_os = "windows")]
        {
            self.copy_file_windows(source, dest)
        }

        #[cfg(target_os = "linux")]
        {
            self.copy_file_linux(source, dest)
        }

        #[cfg(target_os = "macos")]
        {
            self.copy_file_macos(source, dest)
        }

        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        {
            // Fallback to standard copy
            std::fs::copy(source, dest).context("Failed to copy file")
        }
    }

    /// Windows implementation using CopyFileEx
    #[cfg(target_os = "windows")]
    fn copy_file_windows(&self, source: &Path, dest: &Path) -> Result<u64> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use winapi::shared::minwindef::{DWORD, LPVOID};
        use winapi::um::winbase::CopyFileExW;
        use winapi::um::winnt::LARGE_INTEGER;

        // Convert paths to wide strings
        let source_wide: Vec<u16> = OsStr::new(source)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let dest_wide: Vec<u16> = OsStr::new(dest)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // Progress callback data
        struct CallbackData {
            progress: Option<Arc<dyn ProgressTracker>>,
            total_size: u64,
        }

        let callback_data = CallbackData {
            progress: self.progress.clone(),
            total_size: std::fs::metadata(source)?.len(),
        };

        // Progress callback function
        unsafe extern "system" fn progress_callback(
            total_file_size: LARGE_INTEGER,
            total_bytes_transferred: LARGE_INTEGER,
            _stream_size: LARGE_INTEGER,
            _stream_bytes_transferred: LARGE_INTEGER,
            _stream_number: DWORD,
            _callback_reason: DWORD,
            _source_file: *mut winapi::ctypes::c_void,
            _destination_file: *mut winapi::ctypes::c_void,
            data: LPVOID,
        ) -> DWORD {
            if !data.is_null() {
                let callback_data = unsafe { &*(data as *const CallbackData) };
                if let Some(ref progress) = callback_data.progress {
                    let bytes = unsafe { *total_bytes_transferred.QuadPart() } as u64;
                    let total = unsafe { *total_file_size.QuadPart() } as u64;
                    if total > 0 {
                        let percentage = (bytes * 100) / total;
                        progress.update_percentage(percentage);
                    }
                }
            }

            // PROGRESS_CONTINUE
            0
        }

        // Perform the copy
        let result = unsafe {
            CopyFileExW(
                source_wide.as_ptr(),
                dest_wide.as_ptr(),
                Some(progress_callback),
                &callback_data as *const _ as LPVOID,
                std::ptr::null_mut(),
                0, // Copy flags (0 = default behavior)
            )
        };

        if result == 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        Ok(callback_data.total_size)
    }

    /// Linux implementation using copy_file_range
    #[cfg(target_os = "linux")]
    fn copy_file_linux(&self, source: &Path, dest: &Path) -> Result<u64> {
        use std::fs::{File, OpenOptions};

        let src_file = File::open(source)?;
        let mut dst_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(dest)?;

        let src_metadata = src_file.metadata()?;
        let total_size = src_metadata.len();

        // Try copy_file_range first (kernel 4.5+)
        let result = self.try_copy_file_range(&src_file, &mut dst_file, total_size);

        match result {
            Ok(bytes) => {
                // Copy metadata
                self.copy_metadata_unix(source, dest)?;
                Ok(bytes)
            }
            Err(_) => {
                // Fall back to sendfile (older kernels)
                self.copy_with_sendfile(&src_file, &mut dst_file, total_size)
            }
        }
    }

    /// Try to use copy_file_range (Linux 4.5+)
    #[cfg(target_os = "linux")]
    fn try_copy_file_range(
        &self,
        src: &std::fs::File,
        dst: &mut std::fs::File,
        total_size: u64,
    ) -> Result<u64> {
        use std::os::unix::io::AsRawFd;

        let src_fd = src.as_raw_fd();
        let dst_fd = dst.as_raw_fd();

        let mut total_copied = 0u64;
        let mut last_progress_update = 0u64;

        loop {
            // Use libc::copy_file_range if available
            let bytes_copied = unsafe {
                libc::syscall(
                    libc::SYS_copy_file_range,
                    src_fd,
                    std::ptr::null::<libc::off_t>(),
                    dst_fd,
                    std::ptr::null::<libc::off_t>(),
                    total_size - total_copied,
                    0,
                )
            };

            if bytes_copied < 0 {
                return Err(std::io::Error::last_os_error().into());
            }

            if bytes_copied == 0 {
                break;
            }

            total_copied += bytes_copied as u64;

            // Update progress
            if let Some(ref progress) = self.progress {
                if total_copied - last_progress_update > 1024 * 1024 {
                    let percentage = (total_copied * 100) / total_size;
                    progress.update_percentage(percentage);
                    last_progress_update = total_copied;
                }
            }
        }

        Ok(total_copied)
    }

    /// Fall back to sendfile for older Linux kernels
    #[cfg(target_os = "linux")]
    fn copy_with_sendfile(
        &self,
        src: &std::fs::File,
        dst: &mut std::fs::File,
        total_size: u64,
    ) -> Result<u64> {
        use std::os::unix::io::AsRawFd;

        let src_fd = src.as_raw_fd();
        let dst_fd = dst.as_raw_fd();

        let mut total_copied = 0u64;
        let mut offset = 0i64;

        while total_copied < total_size {
            let to_copy = std::cmp::min(total_size - total_copied, 1024 * 1024 * 16) as usize; // 16MB chunks

            let result =
                unsafe { libc::sendfile(dst_fd, src_fd, &mut offset as *mut libc::off_t, to_copy) };

            if result < 0 {
                return Err(std::io::Error::last_os_error().into());
            }

            if result == 0 {
                break;
            }

            total_copied += result as u64;

            // Update progress
            if let Some(ref progress) = self.progress {
                let percentage = (total_copied * 100) / total_size;
                progress.update_percentage(percentage);
            }
        }

        Ok(total_copied)
    }

    /// macOS implementation using copyfile
    #[cfg(target_os = "macos")]
    fn copy_file_macos(&self, source: &Path, dest: &Path) -> Result<u64> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        // Convert paths to C strings
        let source_cstr = CString::new(source.as_os_str().as_bytes())?;
        let dest_cstr = CString::new(dest.as_os_str().as_bytes())?;

        // Get file size for progress tracking
        let metadata = std::fs::metadata(source)?;
        let total_size = metadata.len();

        // copyfile flags
        const COPYFILE_ALL: u32 = 0x0001;
        const COPYFILE_EXCL: u32 = 0x0002;
        const COPYFILE_NOFOLLOW: u32 = 0x0004;

        // Link to copyfile
        #[link(name = "c")]
        extern "C" {
            fn copyfile(
                from: *const libc::c_char,
                to: *const libc::c_char,
                state: *mut libc::c_void,
                flags: u32,
            ) -> libc::c_int;
        }

        let result = unsafe {
            copyfile(
                source_cstr.as_ptr(),
                dest_cstr.as_ptr(),
                std::ptr::null_mut(),
                COPYFILE_ALL | COPYFILE_NOFOLLOW,
            )
        };

        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        // Update progress to 100%
        if let Some(ref progress) = self.progress {
            progress.update_percentage(100);
        }

        Ok(total_size)
    }

    /// Copy Unix metadata (permissions, timestamps, etc.)
    #[cfg(unix)]
    fn copy_metadata_unix(&self, source: &Path, dest: &Path) -> Result<()> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let metadata = std::fs::metadata(source)?;
        let permissions = std::fs::Permissions::from_mode(metadata.mode());
        std::fs::set_permissions(dest, permissions)?;

        // Try to preserve timestamps
        let atime = metadata.atime();
        let mtime = metadata.mtime();

        let times = [
            libc::timespec {
                tv_sec: atime,
                tv_nsec: 0,
            },
            libc::timespec {
                tv_sec: mtime,
                tv_nsec: 0,
            },
        ];

        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let dest_cstr = CString::new(dest.as_os_str().as_bytes())?;
        unsafe {
            libc::utimensat(libc::AT_FDCWD, dest_cstr.as_ptr(), times.as_ptr(), 0);
        }

        Ok(())
    }

    /// Copy multiple files using platform APIs
    pub fn copy_files(&self, files: &[(PathBuf, PathBuf)]) -> Result<SyncStats> {
        let stats = SyncStats::default();
        let _total_files = files.len();

        for (i, (source, dest)) in files.iter().enumerate() {
            // Don't update percentage here - let the main progress logic handle it

            // Create parent directory if needed
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Copy the file
            match self.copy_file(source, dest) {
                Ok(bytes) => {
                    stats.add_bytes_transferred(bytes);
                    stats.increment_files_copied();

                    // Update progress
                    if let Some(ref progress) = self.progress {
                        progress.update_file_count((i + 1) as u64);
                        progress.update_bytes(stats.bytes_transferred());
                    }
                }
                Err(e) => {
                    eprintln!("Failed to copy {source:?}: {e}");
                    stats.increment_errors();
                }
            }
        }

        Ok(stats)
    }
}

use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_platform_copy() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let source = temp_dir.path().join("source.txt");
        let dest = temp_dir.path().join("dest.txt");

        // Create test file
        std::fs::write(&source, "Hello, World!").expect("Failed to write test file");

        // Copy using platform API
        let copier = PlatformCopier::new();
        let bytes = copier
            .copy_file(&source, &dest)
            .expect("Failed to copy file");

        assert_eq!(bytes, 13);
        assert!(dest.exists());
        assert_eq!(
            std::fs::read_to_string(&dest).expect("Failed to read copied file"),
            "Hello, World!"
        );
    }
}
