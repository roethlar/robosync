//! Ultra-fast copy for simple scenarios to beat native tools

use std::fs;
use std::path::Path;
use anyhow::Result;
use crate::sync_stats::SyncStats;

/// Detect if this is a simple directory copy that can use ultra-fast path
pub fn is_simple_copy_scenario(source: &Path, dest: &Path, has_filters: bool) -> bool {
    // Cannot use ultra-fast path if any filters are specified
    if has_filters {
        return false;
    }
    
    // Must be directory to directory
    if !source.is_dir() {
        return false;
    }
    
    // CRITICAL FIX: Also allow when destination exists but we're doing simple sync
    // This fixes the major performance regression for existing destinations
    if dest.exists() && dest.is_dir() {
        // Quick check - if source directory is small/medium, use fast path
        if let Ok(entries) = fs::read_dir(source) {
            let count = entries.count();
            // Expanded threshold - most directories benefit from fast path
            if count < 50000 {
                return true;
            }
        }
        return false;
    }
    
    // For new destinations, definitely use fast path
    if !dest.exists() {
        if let Ok(entries) = fs::read_dir(source) {
            let count = entries.count();
            if count < 50000 {
                return true;
            }
        }
    }
    
    false
}

/// Ultra-fast directory copy - minimal overhead, maximum speed
pub fn ultra_fast_directory_copy(source: &Path, dest: &Path) -> Result<SyncStats> {
    let start = std::time::Instant::now();
    let mut stats = SyncStats::default();
    
    // Create destination directory if it doesn't exist
    if !dest.exists() {
        fs::create_dir_all(dest)?;
    }
    
    // Recursively copy everything with minimal processing
    // This handles both new destinations and syncing to existing ones
    copy_directory_contents(source, dest, &mut stats)?;
    
    stats.elapsed_time = start.elapsed();
    Ok(stats)
}

/// Recursive directory copy with minimal overhead
fn copy_directory_contents(source: &Path, dest: &Path, stats: &mut SyncStats) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let src_path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dest.join(&file_name);
        
        if src_path.is_dir() {
            // Create directory and recurse
            fs::create_dir_all(&dest_path)?;
            copy_directory_contents(&src_path, &dest_path, stats)?;
        } else if src_path.is_file() {
            // Direct file copy - fastest possible on each platform
            match fs::copy(&src_path, &dest_path) {
                Ok(bytes) => {
                    stats.increment_files_copied();
                    stats.add_bytes_transferred(bytes);
                }
                Err(e) => {
                    eprintln!("Error copying {:?}: {}", src_path, e);
                    stats.increment_errors();
                }
            }
        }
        // Ignore symlinks and other special files for ultra-fast mode
    }
    
    Ok(())
}