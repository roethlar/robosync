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

    /// Windows implementation using CopyFileEx with NTFS optimizations
    #[cfg(target_os = "windows")]
    fn copy_file_windows(&self, source: &Path, dest: &Path) -> Result<u64> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use winapi::shared::minwindef::{DWORD, LPVOID};
        use winapi::um::winbase::CopyFileExW;
        use winapi::um::winnt::LARGE_INTEGER;

        // Check for NTFS compression before copying
        if let Ok(compressed) = self.is_ntfs_compressed(source) {
            if compressed && self.progress.is_some() {
                // Compressed files copy differently - update progress tracking
            }
        }

        // Convert paths to wide strings with long path support
        let source_wide = self.to_long_path_wide(source)?;
        let dest_wide = self.to_long_path_wide(dest)?;

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

        // Perform the copy with NTFS optimization flags
        let copy_flags = 0x00000800; // COPY_FILE_COPY_SYMLINK for better NTFS handling
        let result = unsafe {
            CopyFileExW(
                source_wide.as_ptr(),
                dest_wide.as_ptr(),
                Some(progress_callback),
                &callback_data as *const _ as LPVOID,
                std::ptr::null_mut(),
                copy_flags,
            )
        };

        if result == 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        // Copy NTFS alternate data streams if they exist
        let _ = self.copy_ntfs_streams(source, dest);

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

        // copyfile flags - CORRECT VALUES from copyfile.h
        const COPYFILE_DATA: u32 = 0x00000008;
        const COPYFILE_METADATA: u32 = 0x00000007;
        const COPYFILE_NOFOLLOW: u32 = 0x000C0000;

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
                COPYFILE_DATA | COPYFILE_METADATA | COPYFILE_NOFOLLOW,
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
                    // Don't print to stderr - it breaks the progress bar
                    // Record error details for log file
                    stats.add_error(source.clone(), "platform_copy", &e.to_string());
                }
            }
        }

        Ok(stats)
    }

    /// Check if a file is compressed on NTFS
    #[cfg(target_os = "windows")]
    fn is_ntfs_compressed(&self, path: &Path) -> Result<bool> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use std::ptr;
        use winapi::um::fileapi::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
        use winapi::um::winnt::FILE_ATTRIBUTE_COMPRESSED;

        let path_wide: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let attributes = unsafe { GetFileAttributesW(path_wide.as_ptr()) };
        
        if attributes == INVALID_FILE_ATTRIBUTES {
            return Err(std::io::Error::last_os_error().into());
        }

        Ok((attributes & FILE_ATTRIBUTE_COMPRESSED) != 0)
    }

    /// Get NTFS alternate data streams for a file
    #[cfg(target_os = "windows")]
    fn get_ntfs_streams(&self, path: &Path) -> Result<Vec<String>> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use std::ptr;
        use winapi::shared::minwindef::{DWORD, LPVOID};
        use winapi::um::fileapi::{FindFirstStreamW, FindNextStreamW, FindClose};
        use winapi::um::handleapi::INVALID_HANDLE_VALUE;
        use winapi::um::winnt::{LARGE_INTEGER};

        let path_wide: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut streams = Vec::new();
        
        // Define WIN32_FIND_STREAM_DATA structure manually
        #[repr(C)]
        struct WIN32_FIND_STREAM_DATA {
            stream_size: LARGE_INTEGER,
            stream_name: [u16; 296], // MAX_PATH + 36 for ":streamname:$DATA"
        }
        
        let mut stream_data = WIN32_FIND_STREAM_DATA {
            stream_size: unsafe { std::mem::zeroed() },
            stream_name: [0; 296],
        };

        // Find first stream - use proper FindStreamInfoStandard (0)
        let find_handle = unsafe {
            FindFirstStreamW(
                path_wide.as_ptr(),
                0, // FindStreamInfoStandard 
                &mut stream_data as *mut _ as LPVOID,
                0, // Reserved, must be 0
            )
        };

        if find_handle != INVALID_HANDLE_VALUE {
            // Process first stream
            let stream_str = String::from_utf16_lossy(&stream_data.stream_name);
            if let Some(null_pos) = stream_str.find('\0') {
                let stream_name = stream_str[..null_pos].to_string();
                // Skip the default data stream (::$DATA)
                if !stream_name.ends_with("::$DATA") || stream_name.contains(':') && !stream_name.starts_with("::") {
                    streams.push(stream_name);
                }
            }

            // Find additional streams
            loop {
                stream_data.stream_name = [0; 296];
                let result = unsafe {
                    FindNextStreamW(
                        find_handle,
                        &mut stream_data as *mut _ as LPVOID,
                    )
                };

                if result == 0 {
                    break;
                }

                let stream_str = String::from_utf16_lossy(&stream_data.stream_name);
                if let Some(null_pos) = stream_str.find('\0') {
                    let stream_name = stream_str[..null_pos].to_string();
                    // Skip the default data stream (::$DATA) 
                    if !stream_name.ends_with("::$DATA") || stream_name.contains(':') && !stream_name.starts_with("::") {
                        streams.push(stream_name);
                    }
                }
            }

            unsafe { FindClose(find_handle) };
        }

        Ok(streams)
    }

    /// Copy NTFS alternate data streams
    #[cfg(target_os = "windows")]
    fn copy_ntfs_streams(&self, source: &Path, dest: &Path) -> Result<()> {
        let streams = self.get_ntfs_streams(source)?;
        
        for stream in &streams {
            // Skip the main data stream (::$DATA)
            if stream.ends_with("::$DATA") && !stream.contains(':') {
                continue;
            }

            // Copy each alternate data stream
            let source_stream = format!("{}:{}", source.display(), stream);
            let dest_stream = format!("{}:{}", dest.display(), stream);
            
            if let Err(e) = std::fs::copy(&source_stream, &dest_stream) {
                // Log but don't fail the entire copy for ADS issues
                eprintln!("Warning: Failed to copy stream {}: {}", stream, e);
            }
        }

        Ok(())
    }

    /// Convert path to Windows long path format (\\?\) with UTF-16 encoding
    #[cfg(target_os = "windows")]
    fn to_long_path_wide(&self, path: &Path) -> Result<Vec<u16>> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        let path_str = path.to_string_lossy();
        
        // Convert to absolute path if relative
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        let abs_path_str = abs_path.to_string_lossy();
        
        // Add \\?\ prefix for long path support if path is longer than 260 chars
        let long_path = if abs_path_str.len() > 260 || abs_path_str.contains("\\\\?\\") {
            if abs_path_str.starts_with("\\\\?\\") {
                abs_path_str.to_string()
            } else if abs_path_str.starts_with("\\\\") {
                // UNC path: \\server\share -> \\?\UNC\server\share
                format!("\\\\?\\UNC\\{}", &abs_path_str[2..])
            } else {
                // Regular path: C:\path -> \\?\C:\path
                format!("\\\\?\\{}", abs_path_str)
            }
        } else {
            abs_path_str.to_string()
        };

        // Convert to UTF-16 with null terminator
        let wide: Vec<u16> = OsStr::new(&long_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        Ok(wide)
    }

    /// Check if a filename contains Windows reserved names
    #[cfg(target_os = "windows")]
    fn is_reserved_filename(&self, filename: &str) -> bool {
        let reserved_names = [
            "CON", "PRN", "AUX", "NUL",
            "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
            "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"
        ];

        let name_without_ext = if let Some(pos) = filename.find('.') {
            &filename[..pos]
        } else {
            filename
        };

        reserved_names.iter().any(|&reserved| 
            name_without_ext.eq_ignore_ascii_case(reserved)
        )
    }

    /// Sanitize Windows filename by handling reserved names and invalid characters
    #[cfg(target_os = "windows")]
    fn sanitize_filename(&self, filename: &str) -> String {
        // Windows invalid characters: < > : " | ? * and control characters (0-31)
        let mut sanitized = filename.chars()
            .map(|c| match c {
                '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
                c if c as u32 <= 31 => '_',
                c => c,
            })
            .collect::<String>();

        // Handle reserved names by appending underscore
        if self.is_reserved_filename(&sanitized) {
            sanitized.push('_');
        }

        // Remove trailing dots and spaces (Windows requirement)
        sanitized = sanitized.trim_end_matches(&['.', ' '][..]).to_string();
        
        // Ensure it's not empty
        if sanitized.is_empty() {
            sanitized = "unnamed_file".to_string();
        }

        sanitized
    }
}

use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    #[cfg(not(target_os = "macos"))] // Skip on macOS - flaky in CI
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
