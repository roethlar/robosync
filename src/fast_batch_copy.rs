//! Fast batch copy for small files without parallel overhead

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use anyhow::Result;

const BUFFER_SIZE: usize = 64 * 1024; // 64KB buffer

/// Ultra-fast sequential batch copy for small files
/// Avoids parallel overhead for better performance than rsync
pub fn sequential_batch_copy(operations: Vec<(PathBuf, PathBuf)>) -> Result<crate::linux_fast_copy::BatchCopyStats> {
    let total_files = operations.len();
    let mut files_copied = 0u64;
    let mut bytes_copied = 0u64;
    let start = std::time::Instant::now();
    
    // Reuse buffer for all operations
    let mut buffer = vec![0u8; BUFFER_SIZE];
    
    for (src, dst) in operations {
        // Create parent directory if needed
        if let Some(parent) = dst.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        
        // Direct copy with reused buffer
        match copy_with_buffer(&src, &dst, &mut buffer) {
            Ok(bytes) => {
                files_copied += 1;
                bytes_copied += bytes;
            }
            Err(e) => {
                eprintln!("Error copying {:?}: {}", src, e);
            }
        }
    }
    
    Ok(crate::linux_fast_copy::BatchCopyStats {
        total_files,
        files_copied,
        bytes_copied,
        elapsed: start.elapsed(),
    })
}

/// Copy a file using a pre-allocated buffer
fn copy_with_buffer(src: &PathBuf, dst: &PathBuf, buffer: &mut [u8]) -> io::Result<u64> {
    let mut src_file = File::open(src)?;
    let mut dst_file = File::create(dst)?;
    let mut total_bytes = 0u64;
    
    loop {
        let bytes_read = src_file.read(buffer)?;
        if bytes_read == 0 {
            break;
        }
        dst_file.write_all(&buffer[..bytes_read])?;
        total_bytes += bytes_read as u64;
    }
    
    // Copy metadata
    if let Ok(metadata) = src.metadata() {
        let _ = dst_file.set_permissions(metadata.permissions());
    }
    
    Ok(total_bytes)
}

/// Even faster version using std::fs::copy for files that fit criteria
pub fn hybrid_batch_copy(operations: Vec<(PathBuf, PathBuf)>) -> Result<crate::linux_fast_copy::BatchCopyStats> {
    let total_files = operations.len();
    let mut files_copied = 0u64;
    let mut bytes_copied = 0u64;
    let start = std::time::Instant::now();
    
    // Process all files sequentially - no parallel overhead
    for (src, dst) in operations {
        // Create parent directory if needed
        if let Some(parent) = dst.parent() {
            if !parent.exists() {
                let _ = fs::create_dir_all(parent);
            }
        }
        
        // Use std::fs::copy which is optimized on Linux
        match fs::copy(&src, &dst) {
            Ok(bytes) => {
                files_copied += 1;
                bytes_copied += bytes;
            }
            Err(e) => {
                eprintln!("Error copying {:?}: {}", src, e);
            }
        }
    }
    
    Ok(crate::linux_fast_copy::BatchCopyStats {
        total_files,
        files_copied,
        bytes_copied,
        elapsed: start.elapsed(),
    })
}