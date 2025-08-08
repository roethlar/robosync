//! Windows-specific fast file enumeration using FindFirstFileEx
//!
//! This module provides optimized directory traversal for Windows using native APIs
//! with FIND_FIRST_EX_LARGE_FETCH for improved performance on NTFS.

#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::mem;
#[cfg(windows)]
use winapi::um::fileapi::{FindFirstFileExW, FindNextFileW, FindClose};
#[cfg(windows)]
use winapi::um::minwinbase::{FINDEX_INFO_LEVELS, FindExInfoBasic, FINDEX_SEARCH_OPS, FindExSearchNameMatch};
#[cfg(windows)]
use winapi::um::minwinbase::WIN32_FIND_DATAW;
#[cfg(windows)]
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
#[cfg(windows)]
use winapi::um::winnt::FILE_ATTRIBUTE_DIRECTORY;
#[cfg(windows)]
use winapi::shared::winerror::ERROR_NO_MORE_FILES;
#[cfg(windows)]
use anyhow::{Result, Context};

// Define FIND_FIRST_EX_LARGE_FETCH if not available in winapi
#[cfg(windows)]
const FIND_FIRST_EX_LARGE_FETCH: u32 = 0x00000002;

/// Windows-optimized directory entry collector
#[cfg(windows)]
pub fn collect_entries_windows(root: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = Vec::with_capacity(10000);
    let mut dirs_to_process = vec![root.to_path_buf()];
    
    while let Some(dir) = dirs_to_process.pop() {
        // Skip if we can't read the directory
        match enumerate_directory_windows(&dir) {
            Ok((files, subdirs)) => {
                entries.extend(files);
                dirs_to_process.extend(subdirs);
            }
            Err(_) => {
                // Silently skip directories we can't access
                continue;
            }
        }
    }
    
    Ok(entries)
}

#[cfg(windows)]
fn enumerate_directory_windows(dir: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let search_path = dir.join("*");
    let search_path_wide: Vec<u16> = OsStr::new(&search_path)
        .encode_wide()
        .chain(Some(0))
        .collect();
    
    let mut find_data: WIN32_FIND_DATAW = unsafe { mem::zeroed() };
    let mut files = Vec::with_capacity(1000);
    let mut dirs = Vec::with_capacity(100);
    
    // Use FindFirstFileExW with optimized parameters
    let handle = unsafe {
        FindFirstFileExW(
            search_path_wide.as_ptr(),
            FindExInfoBasic,
            &mut find_data as *mut _ as *mut _,
            FindExSearchNameMatch,
            std::ptr::null_mut(),
            FIND_FIRST_EX_LARGE_FETCH, // Key optimization: larger buffer for directory entries
        )
    };
    
    if handle == INVALID_HANDLE_VALUE {
        return Ok((files, dirs));
    }
    
    loop {
        // Convert wide string to PathBuf
        let file_name_wide = &find_data.cFileName;
        let len = file_name_wide.iter().position(|&c| c == 0).unwrap_or(260);
        let file_name = String::from_utf16_lossy(&file_name_wide[..len]);
        
        // Skip . and ..
        if file_name != "." && file_name != ".." {
            let full_path = dir.join(&file_name);
            
            if find_data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                dirs.push(full_path);
            } else {
                files.push(full_path);
            }
        }
        
        // Get next file
        if unsafe { FindNextFileW(handle, &mut find_data) } == 0 {
            let error = unsafe { winapi::um::errhandlingapi::GetLastError() };
            if error != ERROR_NO_MORE_FILES {
                // Actual error occurred
                unsafe { FindClose(handle) };
                return Err(anyhow::anyhow!("FindNextFileW failed with error: {}", error));
            }
            break;
        }
    }
    
    unsafe { FindClose(handle) };
    Ok((files, dirs))
}

/// Fallback implementation for non-Windows platforms
#[cfg(not(windows))]
pub fn collect_entries_windows(_root: &Path) -> Result<Vec<PathBuf>> {
    Err(anyhow::anyhow!("Windows-specific enumeration not available on this platform"))
}