//! Fast file enumeration optimized for large directories
//!
//! This module provides optimized file enumeration that can handle tens of thousands
//! of files efficiently by using multiple strategies:
//! - Parallel directory traversal
//! - Batched metadata reading
//! - Progress reporting with minimal overhead
//! - Memory-efficient processing

use crate::file_list::{FileInfo, FileOperation};
use crate::options::SyncOptions;
use crate::progress::SyncProgress;
use anyhow::Result;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Configuration for fast file enumeration
#[derive(Debug, Clone)]
pub struct FastEnumConfig {
    /// Number of parallel threads for directory scanning
    pub scan_threads: usize,
    /// Batch size for processing entries
    pub batch_size: usize,
    /// Whether to pre-scan for directory count estimates
    pub pre_scan: bool,
    /// Update progress every N files
    pub progress_interval: usize,
}

impl Default for FastEnumConfig {
    fn default() -> Self {
        let num_cpus = num_cpus::get();
        Self {
            scan_threads: num_cpus * 2, // More threads for I/O bound work
            batch_size: 2000,
            pre_scan: true,
            progress_interval: 5000,
        }
    }
}

/// Fast file list generator with progress tracking
pub struct FastFileListGenerator {
    config: FastEnumConfig,
    progress: Option<Arc<SyncProgress>>,
}

impl FastFileListGenerator {
    pub fn new(config: FastEnumConfig) -> Self {
        Self {
            config,
            progress: None,
        }
    }

    pub fn with_progress(mut self, progress: Arc<SyncProgress>) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Generate file list with fast enumeration
    pub fn generate_file_list(&self, root: &Path, options: &SyncOptions) -> Result<Vec<FileInfo>> {
        let _start_time = Instant::now();

        // Starting fast file enumeration

        // Use platform-optimized implementation if available
        #[cfg(target_os = "linux")]
        if let Ok(files) = self.generate_with_jwalk(root, options) {
            let _elapsed = _start_time.elapsed();
            // Fast enumeration completed
            return Ok(files);
        }

        // Fallback to optimized rayon-based implementation
        self.generate_with_rayon(root, options)
    }

    /// Linux-specific optimized implementation using jwalk
    #[cfg(target_os = "linux")]
    fn generate_with_jwalk(&self, root: &Path, options: &SyncOptions) -> Result<Vec<FileInfo>> {
        use jwalk::{Parallelism, WalkDir as JWalkDir};

        let file_count = AtomicUsize::new(0);
        let last_update = Arc::new(Mutex::new(Instant::now()));

        // Clone exclude_dirs for use in closure
        let exclude_dirs = options.exclude_dirs.clone();

        // Configure jwalk for optimal performance
        let entries: Result<Vec<FileInfo>, _> = JWalkDir::new(root)
            .parallelism(Parallelism::RayonNewPool(self.config.scan_threads))
            .skip_hidden(false)
            .follow_links(false)
            .process_read_dir(move |_depth, path, _read_dir_state, children| {
                // Check if this directory should be excluded
                if let Some(dir_name) = path.file_name() {
                    let dir_name_str = dir_name.to_string_lossy();
                    for pattern in &exclude_dirs {
                        // Simple pattern matching for directory names
                        if pattern == &dir_name_str || dir_name_str.contains(pattern) {
                            // Clear children to prevent traversing into excluded directory
                            children.clear();
                            return;
                        }
                    }
                }

                // Skip permission errors during directory traversal
                children.retain(|entry| {
                    if let Err(e) = entry {
                        if let Some(io_err) = e.io_error() {
                            if io_err.kind() == std::io::ErrorKind::PermissionDenied {
                                // Silently skip permission denied errors
                                return false;
                            }
                        }
                    }
                    true
                });
            })
            .into_iter()
            .par_bridge()
            .filter_map(|entry| {
                match entry {
                    Ok(entry) => {
                        let path = entry.path();

                        // Skip the root directory itself
                        if path == root {
                            return None;
                        }

                        // Get metadata efficiently
                        let metadata = match entry.metadata() {
                            Ok(m) => m,
                            Err(_) => return None,
                        };

                        let is_symlink = metadata.is_symlink();
                        let symlink_target = if is_symlink {
                            std::fs::read_link(&path).ok()
                        } else {
                            None
                        };

                        let file_info = FileInfo {
                            path: path.clone(),
                            size: metadata.len(),
                            modified: metadata
                                .modified()
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                            is_directory: metadata.is_dir(),
                            is_symlink,
                            symlink_target,
                            checksum: None,
                        };

                        // Debug suspicious paths
                        if file_info
                            .path
                            .to_string_lossy()
                            .contains("/home/michael/Documents/home/")
                        {
                            eprintln!(
                                "JWALK ERROR: Enumerated file with doubled path: {}",
                                file_info.path.display()
                            );
                            eprintln!("  Root: {}", root.display());
                            eprintln!("  Entry path: {}", path.display());
                        }

                        // Apply filters
                        if self.should_include_file(&file_info, root, options) {
                            // Update progress periodically
                            let count = file_count.fetch_add(1, Ordering::Relaxed);
                            if count % self.config.progress_interval == 0 {
                                if let Some(ref progress) = self.progress {
                                    // Only update if enough time has passed to avoid overhead
                                    let now = Instant::now();
                                    let should_update = {
                                        if let Ok(last) = last_update.lock() {
                                            now.duration_since(*last) >= Duration::from_millis(500)
                                        } else {
                                            false
                                        }
                                    };

                                    if should_update {
                                        if let Ok(mut last) = last_update.lock() {
                                            *last = now;
                                        }
                                        progress.print_update();
                                    }
                                }
                            }

                            Some(Ok(file_info))
                        } else {
                            None
                        }
                    }
                    Err(e) => {
                        // Skip permission errors silently
                        if let Some(io_err) = e.io_error() {
                            if io_err.kind() == std::io::ErrorKind::PermissionDenied {
                                return None;
                            }
                        }
                        Some(Err(anyhow::anyhow!("Walk error: {}", e)))
                    }
                }
            })
            .collect();

        entries
    }

    /// Cross-platform optimized implementation using rayon
    fn generate_with_rayon(&self, root: &Path, options: &SyncOptions) -> Result<Vec<FileInfo>> {
        // First, quickly enumerate all directory entries
        // println!("Scanning directory structure...");
        let entries = self.collect_entries_fast(root)?;

        // println!("Processing {} entries...", entries.len());

        if let Some(ref progress) = self.progress {
            progress.print_update();
        }

        // Process entries in parallel batches
        let file_count = AtomicUsize::new(0);
        let last_update = Arc::new(Mutex::new(Instant::now()));

        let files: Result<Vec<_>, _> = entries
            .par_chunks(self.config.batch_size)
            .map(|chunk| {
                let mut batch_files = Vec::with_capacity(chunk.len());

                for path in chunk {
                    // Get metadata
                    match std::fs::symlink_metadata(path) {
                        Ok(metadata) => {
                            let is_symlink = metadata.is_symlink();
                            let symlink_target = if is_symlink {
                                std::fs::read_link(path).ok()
                            } else {
                                None
                            };

                            let file_info = FileInfo {
                                path: path.to_path_buf(),
                                size: metadata.len(),
                                modified: metadata
                                    .modified()
                                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                                is_directory: metadata.is_dir(),
                                is_symlink,
                                symlink_target,
                                checksum: None,
                            };

                            // Debug suspicious paths
                            if file_info
                                .path
                                .to_string_lossy()
                                .contains("/home/michael/Documents/home/")
                            {
                                eprintln!(
                                    "RAYON ERROR: Enumerated file with doubled path: {}",
                                    file_info.path.display()
                                );
                                eprintln!("  Root: {}", root.display());
                            }

                            // Apply filters
                            if self.should_include_file(&file_info, root, options) {
                                batch_files.push(file_info);
                            }
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to read metadata for {path:?}: {e}");
                        }
                    }
                }

                // Update progress
                let count = file_count.fetch_add(batch_files.len(), Ordering::Relaxed);
                if count % self.config.progress_interval < batch_files.len() {
                    if let Some(ref progress) = self.progress {
                        let now = Instant::now();
                        let should_update = {
                            if let Ok(last) = last_update.lock() {
                                now.duration_since(*last) >= Duration::from_millis(500)
                            } else {
                                false
                            }
                        };

                        if should_update {
                            if let Ok(mut last) = last_update.lock() {
                                *last = now;
                            }
                            progress.print_update();
                        }
                    }
                }

                Ok::<Vec<FileInfo>, anyhow::Error>(batch_files)
            })
            .collect();

        let file_batches = files?;
        let all_files: Vec<FileInfo> = file_batches.into_iter().flatten().collect();

        // println!("Enumerated {} files", all_files.len());
        Ok(all_files)
    }

    /// Fast directory entry collection using optimized traversal
    fn collect_entries_fast(&self, root: &Path) -> Result<Vec<PathBuf>> {
        // Use walkdir with optimizations
        use walkdir::WalkDir;

        let entries: Vec<PathBuf> = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| {
                match entry {
                    Ok(entry) => {
                        let path = entry.path().to_path_buf();
                        // Skip the root directory itself
                        if path == root { None } else { Some(path) }
                    }
                    Err(_) => None,
                }
            })
            .collect();

        Ok(entries)
    }

    /// Apply file filtering logic (copied from file_list.rs for performance)
    fn should_include_file(
        &self,
        file_info: &FileInfo,
        root: &Path,
        options: &SyncOptions,
    ) -> bool {
        // Get relative path for pattern matching
        let relative_path = match file_info.path.strip_prefix(root) {
            Ok(path) => path,
            Err(_) => return true, // If we can't get relative path, include it
        };

        // Check file name patterns
        if let Some(file_name) = file_info.path.file_name() {
            let file_name_str = file_name.to_string_lossy();

            // Check exclude file patterns (/XF)
            for pattern in &options.exclude_files {
                if self.matches_pattern(&file_name_str, pattern)
                    || self.matches_pattern(&relative_path.to_string_lossy(), pattern)
                {
                    return false;
                }
            }
        }

        // Check directory patterns (/XD)
        if file_info.is_directory {
            if let Some(dir_name) = file_info.path.file_name() {
                let dir_name_str = dir_name.to_string_lossy();

                for pattern in &options.exclude_dirs {
                    if self.matches_pattern(&dir_name_str, pattern)
                        || self.matches_pattern(&relative_path.to_string_lossy(), pattern)
                    {
                        return false;
                    }
                }
            }
        }

        // Check for files in excluded directories
        for ancestor in relative_path.ancestors() {
            if let Some(dir_name) = ancestor.file_name() {
                let dir_name_str = dir_name.to_string_lossy();

                for pattern in &options.exclude_dirs {
                    if self.matches_pattern(&dir_name_str, pattern)
                        || self.matches_pattern(&ancestor.to_string_lossy(), pattern)
                    {
                        return false;
                    }
                }
            }
        }

        // Check file size filters (/MIN, /MAX)
        if !file_info.is_directory {
            if let Some(min_size) = options.min_size {
                if file_info.size < min_size {
                    return false;
                }
            }

            if let Some(max_size) = options.max_size {
                if file_info.size > max_size {
                    return false;
                }
            }
        }

        true
    }

    /// Simple pattern matching with wildcards (copied from file_list.rs)
    fn matches_pattern(&self, text: &str, pattern: &str) -> bool {
        // Convert pattern to regex-like matching
        // * matches any sequence of characters
        // ? matches any single character

        let mut pattern_chars = pattern.chars().peekable();
        let mut text_chars = text.chars().peekable();

        loop {
            match (pattern_chars.peek(), text_chars.peek()) {
                (None, None) => return true,
                (None, Some(_)) => return false,
                (Some('*'), _) => {
                    pattern_chars.next(); // consume '*'

                    // If * is at the end of pattern, it matches everything remaining
                    if pattern_chars.peek().is_none() {
                        return true;
                    }

                    // Try to match the rest of the pattern at each position in text
                    let remaining_pattern: String = pattern_chars.collect();
                    let remaining_text: String = text_chars.collect();

                    // Use character indices instead of byte indices
                    for (i, _) in remaining_text.char_indices() {
                        if self.matches_pattern(&remaining_text[i..], &remaining_pattern) {
                            return true;
                        }
                    }
                    // Also check matching empty string at the end
                    if self.matches_pattern("", &remaining_pattern) {
                        return true;
                    }
                    return false;
                }
                (Some('?'), Some(_)) => {
                    pattern_chars.next();
                    text_chars.next();
                }
                (Some(p), Some(t)) if p == t => {
                    pattern_chars.next();
                    text_chars.next();
                }
                _ => return false,
            }
        }
    }
}

/// Optimized file comparison for large file sets
pub fn compare_file_lists_fast(
    source: &[FileInfo],
    target: &[FileInfo],
    source_root: &Path,
    dest_root: &Path,
    options: &SyncOptions,
    progress: Option<Arc<SyncProgress>>,
) -> Vec<FileOperation> {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Comparing source and target files

    // Build target map in parallel for faster lookup
    let target_map: HashMap<PathBuf, &FileInfo> = target
        .par_iter()
        .filter_map(|file| {
            file.path
                .strip_prefix(dest_root)
                .ok()
                .map(|relative_path| (relative_path.to_path_buf(), file))
        })
        .collect();

    // Built target lookup map

    let processed_count = AtomicUsize::new(0);
    let last_update = Arc::new(Mutex::new(Instant::now()));

    // Process source files in parallel chunks for better performance
    let chunk_size = 1000; // Process in chunks to avoid memory overhead
    let (source_operations, processed_targets): (Vec<_>, Vec<_>) = source
        .par_chunks(chunk_size)
        .map(|chunk| {
            let mut chunk_operations = Vec::new();
            let mut chunk_targets = Vec::new();

            for source_file in chunk {
                if let Ok(relative_path) = source_file.path.strip_prefix(source_root) {
                    let relative_path = relative_path.to_path_buf();

                    // Skip any paths that would create a home directory structure
                    if relative_path.starts_with("home/") {
                        eprintln!(
                            "WARNING: Skipping file with suspicious path: {} (relative: {})",
                            source_file.path.display(),
                            relative_path.display()
                        );
                        continue;
                    }

                    if let Some(target_file) = target_map.get(&relative_path) {
                        // File exists in both source and target
                        chunk_targets.push(relative_path);

                        // Handle symlinks first
                        if source_file.is_symlink && target_file.is_symlink {
                            // Both are symlinks, check if they point to the same target
                            if source_file.symlink_target != target_file.symlink_target {
                                if let Some(ref target) = source_file.symlink_target {
                                    chunk_operations.push(FileOperation::UpdateSymlink {
                                        path: source_file.path.clone(),
                                        target: target.clone(),
                                    });
                                }
                            }
                        } else if source_file.is_symlink && !target_file.is_symlink {
                            // Source is symlink, target is not - replace with symlink
                            chunk_operations.push(FileOperation::Delete {
                                path: target_file.path.clone(),
                            });
                            if let Some(ref target) = source_file.symlink_target {
                                chunk_operations.push(FileOperation::CreateSymlink {
                                    path: source_file.path.clone(),
                                    target: target.clone(),
                                });
                            }
                        } else if !source_file.is_symlink && target_file.is_symlink {
                            // Source is not symlink, target is - replace symlink
                            chunk_operations.push(FileOperation::Delete {
                                path: target_file.path.clone(),
                            });
                            if source_file.is_directory {
                                chunk_operations.push(FileOperation::CreateDirectory {
                                    path: source_file.path.clone(),
                                });
                            } else {
                                chunk_operations.push(FileOperation::Create {
                                    path: source_file.path.clone(),
                                });
                            }
                        } else if !source_file.is_directory && !target_file.is_directory {
                            // Both are regular files
                            if needs_update_fast(source_file, target_file, options) {
                                let use_delta = should_use_delta_fast(source_file, target_file);
                                chunk_operations.push(FileOperation::Update {
                                    path: source_file.path.clone(),
                                    use_delta,
                                });
                            }
                        } else if source_file.is_directory && !target_file.is_directory {
                            chunk_operations.push(FileOperation::Delete {
                                path: target_file.path.clone(),
                            });
                            chunk_operations.push(FileOperation::CreateDirectory {
                                path: source_file.path.clone(),
                            });
                        } else if !source_file.is_directory && target_file.is_directory {
                            chunk_operations.push(FileOperation::Delete {
                                path: target_file.path.clone(),
                            });
                            chunk_operations.push(FileOperation::Create {
                                path: source_file.path.clone(),
                            });
                        }
                    } else {
                        // File exists only in source (new file)
                        if source_file.is_directory {
                            chunk_operations.push(FileOperation::CreateDirectory {
                                path: source_file.path.clone(),
                            });
                        } else if source_file.is_symlink {
                            if let Some(ref target) = source_file.symlink_target {
                                chunk_operations.push(FileOperation::CreateSymlink {
                                    path: source_file.path.clone(),
                                    target: target.clone(),
                                });
                            }
                        } else {
                            chunk_operations.push(FileOperation::Create {
                                path: source_file.path.clone(),
                            });
                        }
                    }
                }
            }

            // Update progress
            let _count = processed_count.fetch_add(chunk.len(), Ordering::Relaxed);
            if let Some(ref progress) = progress {
                let now = Instant::now();
                let should_update = {
                    if let Ok(last) = last_update.lock() {
                        now.duration_since(*last) >= Duration::from_millis(1000)
                    } else {
                        false
                    }
                };

                if should_update {
                    if let Ok(mut last) = last_update.lock() {
                        *last = now;
                    }
                    progress.print_update();
                }
            }

            (chunk_operations, chunk_targets)
        })
        .unzip();

    // Flatten results
    let mut operations: Vec<FileOperation> = source_operations.into_iter().flatten().collect();
    let processed_targets: HashSet<PathBuf> = processed_targets.into_iter().flatten().collect();

    // Generated operations from source comparison

    // Handle deletions if in purge mode
    if options.purge || options.mirror {
        let delete_operations: Vec<FileOperation> = target
            .par_iter()
            .filter_map(|target_file| {
                if let Ok(relative_path) = target_file.path.strip_prefix(dest_root) {
                    if !processed_targets.contains(relative_path) {
                        Some(FileOperation::Delete {
                            path: target_file.path.clone(),
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        // println!("Added {} delete operations", delete_operations.len());
        operations.extend(delete_operations);
    }

    operations
}

/// Fast file update check without full metadata comparison
fn needs_update_fast(source: &FileInfo, target: &FileInfo, options: &SyncOptions) -> bool {
    if options.checksum {
        // If checksums are available, use them
        match (&source.checksum, &target.checksum) {
            (Some(source_checksum), Some(target_checksum)) => {
                return source_checksum != target_checksum;
            }
            _ => {
                // Fall back to size/time comparison
            }
        }
    }

    // Quick size check first (most common difference)
    if source.size != target.size {
        return true;
    }

    // Then check modification time
    source.modified > target.modified
}

/// Fast delta algorithm decision
fn should_use_delta_fast(source: &FileInfo, target: &FileInfo) -> bool {
    const MIN_SIZE_FOR_DELTA: u64 = 1024;
    const MAX_SIZE_DIFFERENCE_RATIO: f64 = 0.5;

    if source.size < MIN_SIZE_FOR_DELTA || target.size < MIN_SIZE_FOR_DELTA {
        return false;
    }

    let size_diff = source.size.abs_diff(target.size);
    let size_diff_ratio = size_diff as f64 / target.size.max(source.size) as f64;

    size_diff_ratio < MAX_SIZE_DIFFERENCE_RATIO
}
