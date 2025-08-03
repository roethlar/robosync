//! Utility functions for common file operation patterns
//!
//! This module consolidates frequently duplicated logic across the codebase
//! to improve maintainability and reduce code duplication.

use anyhow::Result;
use indicatif::ProgressBar;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::progress::SyncProgress;

/// Simple humanize bytes function for progress display
fn humanize_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit_index = 0;

    while value >= 1024.0 && unit_index < UNITS.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{:.1} {}", value, UNITS[unit_index])
    }
}

/// Resolve a file path from source to destination directory
///
/// This consolidates the common pattern:
/// ```ignore
/// let relative = path.strip_prefix(source_root).unwrap_or(path);
/// let dest = dest_root.join(relative);
/// ```
pub fn resolve_destination_path(file_path: &Path, source_root: &Path, dest_root: &Path) -> PathBuf {
    let relative = file_path.strip_prefix(source_root).unwrap_or(file_path);
    dest_root.join(relative)
}

/// Ensure parent directory exists for the given path
///
/// This consolidates the common pattern:
/// ```ignore
/// if let Some(parent) = dest.parent() {
///     let _ = std::fs::create_dir_all(parent);
/// }
/// ```
pub fn ensure_parent_dir(dest_path: &Path) -> Result<()> {
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

/// Update progress tracking for successful file operations
///
/// This consolidates the common pattern of updating progress trackers
/// and progress bars with throughput calculation
pub fn update_progress(
    progress_tracker: &SyncProgress,
    progress_bar: Option<&ProgressBar>,
    bytes_transferred: u64,
    start_time: Instant,
) {
    // Update progress tracker
    progress_tracker.add_file();
    progress_tracker.add_bytes(bytes_transferred);

    // Update progress bar with throughput calculation
    if let Some(pb) = progress_bar {
        pb.inc(1);
        let elapsed = start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            let total_bytes = progress_tracker.get_bytes_transferred();
            let throughput = (total_bytes as f64 / elapsed) as u64;
            pb.set_message(format!("{}/s", humanize_bytes(throughput)));
        }
    }
}

/// Update progress with current file information
pub fn update_progress_with_file(
    progress_tracker: &SyncProgress,
    progress_bar: Option<&ProgressBar>,
    bytes_transferred: u64,
    start_time: Instant,
    current_file: &Path,
) {
    // Update progress tracker
    progress_tracker.add_file();
    progress_tracker.add_bytes(bytes_transferred);
    progress_tracker.set_current_file(&current_file.display().to_string());

    // Update progress bar with file info and throughput
    if let Some(pb) = progress_bar {
        pb.inc(1);
        let elapsed = start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            let total_bytes = progress_tracker.get_bytes_transferred();
            let throughput = (total_bytes as f64 / elapsed) as u64;
            let file_name = current_file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            pb.set_message(format!("{} | {}/s", file_name, humanize_bytes(throughput)));
        }
    }
}

/// Combined operation: resolve path and ensure parent directory exists
///
/// This combines the two most common operations when preparing to copy a file
pub fn prepare_destination_path(
    file_path: &Path,
    source_root: &Path,
    dest_root: &Path,
) -> Result<PathBuf> {
    let dest_path = resolve_destination_path(file_path, source_root, dest_root);
    ensure_parent_dir(&dest_path)?;
    Ok(dest_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_resolve_destination_path() {
        let source_root = Path::new("/source");
        let dest_root = Path::new("/dest");
        let file_path = Path::new("/source/subdir/file.txt");

        let result = resolve_destination_path(file_path, source_root, dest_root);
        assert_eq!(result, PathBuf::from("/dest/subdir/file.txt"));
    }

    #[test]
    fn test_resolve_destination_path_no_prefix() {
        let source_root = Path::new("/source");
        let dest_root = Path::new("/dest");
        let file_path = Path::new("/other/file.txt");

        let result = resolve_destination_path(file_path, source_root, dest_root);
        // When path can't be stripped from source_root, joining with absolute path replaces dest_root
        assert_eq!(result, PathBuf::from("/other/file.txt"));
    }
}
