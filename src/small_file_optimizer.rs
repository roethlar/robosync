//! Special optimizer for small file scenarios to beat rsync

use std::fs;
use std::path::Path;
use anyhow::Result;
use crate::sync_stats::SyncStats;

/// Check if all files in a directory are small
pub fn is_small_files_scenario(source: &Path) -> Result<bool> {
    // CRITICAL PERFORMANCE FIX: Avoid expensive metadata() calls per file
    // Instead, use heuristics based on directory size and file count
    
    let entries: Vec<_> = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    let file_count = entries.len();
    
    // If directory has many files, likely small files scenario
    if file_count > 100 {
        return Ok(true);
    }
    
    // For smaller directories, do a quick sampling check (max 10 files)
    let mut large_file_found = false;
    let sample_size = file_count.min(10);
    
    for entry in entries.iter().take(sample_size) {
        if let Ok(metadata) = entry.metadata() {
            if metadata.is_file() && metadata.len() > 10 * 1024 * 1024 {
                large_file_found = true;
                break;
            }
        }
    }
    
    // If no large files found in sample, treat as small files scenario
    Ok(!large_file_found)
}

/// Fast path for small files - minimal overhead
pub fn sync_small_files_fast(source: &Path, dest: &Path) -> Result<SyncStats> {
    let mut stats = SyncStats::default();
    let start = std::time::Instant::now();
    
    // Ensure destination exists
    fs::create_dir_all(dest)?;
    
    // Collect all operations first (minimal processing)
    let mut operations = Vec::new();
    
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let src_path = entry.path();
        
        if src_path.is_file() {
            let file_name = entry.file_name();
            let dst_path = dest.join(&file_name);
            operations.push((src_path, dst_path));
        }
    }
    
    // Use the fastest copy method - direct fs::copy
    for (src, dst) in operations {
        match fs::copy(&src, &dst) {
            Ok(bytes) => {
                stats.increment_files_copied();
                stats.add_bytes_transferred(bytes);
            }
            Err(e) => {
                eprintln!("Error copying {:?}: {}", src, e);
                stats.increment_errors();
            }
        }
    }
    
    stats.elapsed_time = start.elapsed();
    Ok(stats)
}