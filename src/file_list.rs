//! File list generation and management

use crate::options::{SymlinkBehavior, SyncOptions};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// Note: rayon is imported inside functions that use it to avoid conflicts

/// File metadata for synchronization
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub path: PathBuf,
    pub size: u64,
    pub modified: std::time::SystemTime,
    pub is_directory: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<PathBuf>,
    pub checksum: Option<Vec<u8>>,
}

/// Generate file list from a directory
pub fn generate_file_list(root: &Path) -> Result<Vec<FileInfo>> {
    let mut files = Vec::new();

    for entry in WalkDir::new(root).follow_links(false) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // Skip permission errors
                if let Some(io_err) = e.io_error() {
                    if io_err.kind() == std::io::ErrorKind::PermissionDenied {
                        eprintln!(
                            "Warning: Skipping inaccessible path: {}",
                            e.path().unwrap_or(Path::new("<unknown>")).display()
                        );
                        continue;
                    }
                }
                return Err(e.into());
            }
        };
        let path = entry.path();

        // Skip the root directory itself if it's the same as the root we're walking
        if path == root {
            continue;
        }

        // Use symlink_metadata to get info about the symlink itself, not its target
        let metadata = std::fs::symlink_metadata(path)?;
        let is_symlink = metadata.is_symlink();

        // Read symlink target if it's a symlink
        let symlink_target = if is_symlink {
            match std::fs::read_link(path) {
                Ok(target) => Some(target),
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to read symlink target for {}: {}",
                        path.display(),
                        e
                    );
                    None
                }
            }
        } else {
            None
        };

        // Ensure we don't store paths with doubled prefixes
        let clean_path = if path.starts_with(root) {
            path.to_path_buf()
        } else {
            // This shouldn't happen, but if it does, try to clean it up
            eprintln!(
                "WARNING: Path {} is not under root {}",
                path.display(),
                root.display()
            );
            path.to_path_buf()
        };

        let file_info = FileInfo {
            path: clean_path,
            size: metadata.len(),
            modified: metadata.modified()?,
            is_directory: metadata.is_dir(),
            is_symlink,
            symlink_target,
            checksum: None, // Will be computed later if needed
        };

        files.push(file_info);
    }

    Ok(files)
}

/// Generate file list from a directory with filtering
pub fn generate_file_list_with_options(
    root: &Path,
    options: &SyncOptions,
) -> Result<Vec<FileInfo>> {
    generate_file_list_with_options_and_progress(root, options, None::<fn(usize)>)
}

/// Generate file list from a directory with filtering and optional progress callback
pub fn generate_file_list_with_options_and_progress<F>(
    root: &Path,
    options: &SyncOptions,
    progress_callback: Option<F>,
) -> Result<Vec<FileInfo>>
where
    F: Fn(usize) + Send + Sync,
{
    use rayon::prelude::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // First, collect all entries without checksums
    let mut file_infos = Vec::new();
    let mut files_needing_checksums = Vec::new();
    let mut count = 0;

    // Clone exclude_dirs for the filter closure
    let exclude_dirs = options.exclude_dirs.clone();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |e| {
            // Always allow the root directory
            if e.path() == root {
                return true;
            }

            // Check if this is a directory that should be excluded
            if e.file_type().is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    for pattern in &exclude_dirs {
                        if name == pattern || name.contains(pattern) {
                            return false; // Skip this directory and all its contents
                        }
                    }
                }
            }
            true
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // Skip permission errors
                if let Some(io_err) = e.io_error() {
                    if io_err.kind() == std::io::ErrorKind::PermissionDenied {
                        eprintln!(
                            "Warning: Skipping inaccessible path: {}",
                            e.path().unwrap_or(Path::new("<unknown>")).display()
                        );
                        continue;
                    }
                }
                return Err(e.into());
            }
        };
        let path = entry.path();

        // Skip the root directory itself if it's the same as the root we're walking
        if path == root {
            continue;
        }

        // Use symlink_metadata to get info about the symlink itself, not its target
        let metadata = std::fs::symlink_metadata(path)?;
        let is_symlink = metadata.is_symlink();

        // Read symlink target if it's a symlink
        let symlink_target = if is_symlink {
            match std::fs::read_link(path) {
                Ok(target) => Some(target),
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to read symlink target for {}: {}",
                        path.display(),
                        e
                    );
                    None
                }
            }
        } else {
            None
        };

        // Handle symlink behavior first
        let mut skip_file = false;
        let actual_file_info = if is_symlink {
            // Found symlink
            // Symlink behavior
            match options.symlink_behavior {
                SymlinkBehavior::Skip => {
                    // Skip symlinks entirely
                    skip_file = true;
                    FileInfo {
                        path: path.to_path_buf(),
                        size: metadata.len(),
                        modified: metadata.modified()?,
                        is_directory: metadata.is_dir(),
                        is_symlink,
                        symlink_target,
                        checksum: None,
                    }
                }
                SymlinkBehavior::Preserve => {
                    // Keep as symlink (default behavior)
                    FileInfo {
                        path: path.to_path_buf(),
                        size: metadata.len(),
                        modified: metadata.modified()?,
                        is_directory: metadata.is_dir(),
                        is_symlink,
                        symlink_target,
                        checksum: None,
                    }
                }
                SymlinkBehavior::Dereference => {
                    // Dereference the symlink - get target file info
                    if let Some(ref target) = symlink_target {
                        match dereference_symlink(path, target) {
                            Ok(dereferenced_info) => dereferenced_info,
                            Err(_e) => {
                                // Warning: Failed to dereference symlink
                                skip_file = true;
                                FileInfo {
                                    path: path.to_path_buf(),
                                    size: metadata.len(),
                                    modified: metadata.modified()?,
                                    is_directory: metadata.is_dir(),
                                    is_symlink,
                                    symlink_target,
                                    checksum: None,
                                }
                            }
                        }
                    } else {
                        // No target found, skip
                        skip_file = true;
                        FileInfo {
                            path: path.to_path_buf(),
                            size: metadata.len(),
                            modified: metadata.modified()?,
                            is_directory: metadata.is_dir(),
                            is_symlink,
                            symlink_target,
                            checksum: None,
                        }
                    }
                }
            }
        } else {
            FileInfo {
                path: path.to_path_buf(),
                size: metadata.len(),
                modified: metadata.modified()?,
                is_directory: metadata.is_dir(),
                is_symlink,
                symlink_target,
                checksum: None,
            }
        };

        // Apply filters if not skipping
        if !skip_file && should_include_file(&actual_file_info, root, options) {
            // Check if we need to compute checksum for this file
            if options.checksum && !actual_file_info.is_symlink && !actual_file_info.is_directory {
                files_needing_checksums.push(file_infos.len());
            }
            file_infos.push(actual_file_info);
        }

        // Update progress
        count += 1;
        if let Some(ref callback) = progress_callback {
            callback(count);
        }
    }

    // Compute checksums in parallel if needed
    if !files_needing_checksums.is_empty() {
        let checksum_count = Arc::new(AtomicUsize::new(0));
        let progress_cb = progress_callback.as_ref();

        // Process checksums in parallel batches
        let checksums: Result<Vec<_>, _> = files_needing_checksums
            .par_iter()
            .map(|&index| {
                let path = &file_infos[index].path;
                let result = compute_file_checksum(path);

                // Update progress for checksum computation
                if let Some(callback) = progress_cb {
                    let current = checksum_count.fetch_add(1, Ordering::Relaxed);
                    callback(count + current + 1);
                }

                result.map(|checksum| (index, checksum))
            })
            .collect();

        // Apply computed checksums
        for (index, checksum) in checksums? {
            file_infos[index].checksum = checksum;
        }
    }

    Ok(file_infos)
}

/// Generate file list using parallel directory scanning (Linux optimized)
#[cfg(target_os = "linux")]
pub fn generate_file_list_parallel(root: &Path, options: &SyncOptions) -> Result<Vec<FileInfo>> {
    use crate::options::SymlinkBehavior;
    use jwalk::WalkDir as JWalkDir;
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let file_count = AtomicUsize::new(0);

    // Use jwalk for parallel directory traversal
    let entries: Vec<FileInfo> = JWalkDir::new(root)
        .parallelism(jwalk::Parallelism::RayonNewPool(num_cpus::get()))
        .skip_hidden(false)
        .follow_links(false)
        .into_iter()
        .par_bridge() // Convert to parallel iterator
        .filter_map(|entry| {
            match entry {
                Ok(entry) => {
                    let path = entry.path();

                    // Skip the root directory itself
                    if path == root {
                        return None;
                    }

                    // Get metadata
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

                    // Handle symlink behavior first
                    let mut skip_file = false;
                    let actual_file_info = if is_symlink {
                        match options.symlink_behavior {
                            SymlinkBehavior::Skip => {
                                // Skip symlinks entirely
                                skip_file = true;
                                FileInfo {
                                    path,
                                    size: metadata.len(),
                                    modified: metadata
                                        .modified()
                                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                                    is_directory: metadata.is_dir(),
                                    is_symlink,
                                    symlink_target,
                                    checksum: None,
                                }
                            }
                            SymlinkBehavior::Preserve => {
                                // Keep as symlink (default behavior)
                                FileInfo {
                                    path,
                                    size: metadata.len(),
                                    modified: metadata
                                        .modified()
                                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                                    is_directory: metadata.is_dir(),
                                    is_symlink,
                                    symlink_target,
                                    checksum: None,
                                }
                            }
                            SymlinkBehavior::Dereference => {
                                // Dereference the symlink - get target file info
                                if let Some(ref target) = symlink_target {
                                    match dereference_symlink(&path, target) {
                                        Ok(dereferenced_info) => dereferenced_info,
                                        Err(_) => {
                                            // Failed to dereference, skip
                                            skip_file = true;
                                            FileInfo {
                                                path,
                                                size: metadata.len(),
                                                modified: metadata
                                                    .modified()
                                                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                                                is_directory: metadata.is_dir(),
                                                is_symlink,
                                                symlink_target,
                                                checksum: None,
                                            }
                                        }
                                    }
                                } else {
                                    // No target found, skip
                                    skip_file = true;
                                    FileInfo {
                                        path,
                                        size: metadata.len(),
                                        modified: metadata
                                            .modified()
                                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                                        is_directory: metadata.is_dir(),
                                        is_symlink,
                                        symlink_target,
                                        checksum: None,
                                    }
                                }
                            }
                        }
                    } else {
                        FileInfo {
                            path,
                            size: metadata.len(),
                            modified: metadata
                                .modified()
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                            is_directory: metadata.is_dir(),
                            is_symlink,
                            symlink_target,
                            checksum: None,
                        }
                    };

                    // Apply filters if not skipping
                    if !skip_file && should_include_file(&actual_file_info, root, options) {
                        file_count.fetch_add(1, Ordering::Relaxed);
                        Some(actual_file_info)
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        })
        .collect();

    // If checksums are needed, compute them in parallel
    if options.checksum {
        let entries_with_checksums: Vec<FileInfo> = entries
            .into_par_iter()
            .map(|mut file_info| {
                if !file_info.is_directory && !file_info.is_symlink {
                    file_info.checksum = compute_file_checksum(&file_info.path)?;
                }
                Ok(file_info)
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(entries_with_checksums)
    } else {
        Ok(entries)
    }
}

/// Check if a file should be included based on filtering options
fn should_include_file(file_info: &FileInfo, root: &Path, options: &SyncOptions) -> bool {
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
            if matches_pattern(&file_name_str, pattern)
                || matches_pattern(&relative_path.to_string_lossy(), pattern)
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
                if matches_pattern(&dir_name_str, pattern)
                    || matches_pattern(&relative_path.to_string_lossy(), pattern)
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
                if matches_pattern(&dir_name_str, pattern)
                    || matches_pattern(&ancestor.to_string_lossy(), pattern)
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

/// Simple pattern matching with wildcards (* and ?)
fn matches_pattern(text: &str, pattern: &str) -> bool {
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
                    if matches_pattern(&remaining_text[i..], &remaining_pattern) {
                        return true;
                    }
                }
                // Also check matching empty string at the end
                if matches_pattern("", &remaining_pattern) {
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

/// Compare two file lists to find differences, normalizing paths relative to their roots
pub fn compare_file_lists_with_roots(
    source: &[FileInfo],
    target: &[FileInfo],
    source_root: &Path,
    dest_root: &Path,
    options: &SyncOptions,
) -> Vec<FileOperation> {
    compare_file_lists_with_roots_and_progress(
        source,
        target,
        source_root,
        dest_root,
        options,
        None::<fn(usize)>,
    )
}

/// Compare two file lists to find differences with progress callback, normalizing paths relative to their roots
pub fn compare_file_lists_with_roots_and_progress<F>(
    source: &[FileInfo],
    target: &[FileInfo],
    source_root: &Path,
    dest_root: &Path,
    options: &SyncOptions,
    progress_callback: Option<F>,
) -> Vec<FileOperation>
where
    F: Fn(usize) + Send + Sync,
{
    use rayon::prelude::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Pre-compute target map with relative paths for faster lookup
    let target_map: HashMap<PathBuf, &FileInfo> = target
        .par_iter()
        .filter_map(|file| {
            file.path
                .strip_prefix(dest_root)
                .ok()
                .map(|relative_path| (relative_path.to_path_buf(), file))
        })
        .collect();

    let processed_count = Arc::new(AtomicUsize::new(0));
    let progress_update_interval = std::cmp::max(1, source.len() / 100); // Update every 1% or at least every file

    // Process source files in parallel
    let (source_operations, processed_targets): (Vec<_>, Vec<_>) = source
        .par_iter()
        .filter_map(|source_file| {
            source_file
                .path
                .strip_prefix(source_root)
                .ok()
                .map(|relative_path| (source_file, relative_path.to_path_buf()))
        })
        .map(|(source_file, relative_path)| {
            // Update progress periodically, not for every file
            let count = processed_count.fetch_add(1, Ordering::Relaxed);
            if let Some(ref callback) = progress_callback {
                if count % progress_update_interval == 0 || count == source.len() - 1 {
                    callback(count + 1);
                }
            }

            let mut operations = Vec::new();
            let mut processed_target = None;

            if let Some(target_file) = target_map.get(&relative_path) {
                // File exists in both source and target
                processed_target = Some(relative_path.clone());

                // Handle symlinks first
                if source_file.is_symlink && target_file.is_symlink {
                    // Both are symlinks, check if they point to the same target
                    if source_file.symlink_target != target_file.symlink_target {
                        if let Some(ref target) = source_file.symlink_target {
                            operations.push(FileOperation::UpdateSymlink {
                                path: source_file.path.clone(),
                                target: target.clone(),
                            });
                        }
                    }
                } else if source_file.is_symlink && !target_file.is_symlink {
                    // Source is symlink, target is not - delete target and create symlink
                    operations.push(FileOperation::Delete {
                        path: source_file.path.clone(),
                    });
                    if let Some(ref target) = source_file.symlink_target {
                        operations.push(FileOperation::CreateSymlink {
                            path: source_file.path.clone(),
                            target: target.clone(),
                        });
                    }
                } else if !source_file.is_symlink && target_file.is_symlink {
                    // Source is not symlink, target is - delete symlink and create file/dir
                    operations.push(FileOperation::Delete {
                        path: source_file.path.clone(),
                    });
                    if source_file.is_directory {
                        operations.push(FileOperation::CreateDirectory {
                            path: source_file.path.clone(),
                        });
                    } else {
                        operations.push(FileOperation::Create {
                            path: source_file.path.clone(),
                        });
                    }
                } else if source_file.is_directory && target_file.is_directory {
                    // Both are directories, no action needed
                } else if source_file.is_directory && !target_file.is_directory {
                    // Source is directory, target is file - delete file and create directory
                    operations.push(FileOperation::Delete {
                        path: source_file.path.clone(),
                    });
                    operations.push(FileOperation::CreateDirectory {
                        path: source_file.path.clone(),
                    });
                } else if !source_file.is_directory && target_file.is_directory {
                    // Source is file, target is directory - delete directory and create file
                    operations.push(FileOperation::Delete {
                        path: source_file.path.clone(),
                    });
                    operations.push(FileOperation::Create {
                        path: source_file.path.clone(),
                    });
                } else if !source_file.is_directory && !target_file.is_directory {
                    // Both are files, check if update is needed
                    if needs_update(source_file, target_file, options) {
                        let use_delta = should_use_delta(source_file, target_file);
                        operations.push(FileOperation::Update {
                            path: source_file.path.clone(),
                            use_delta,
                        });
                    }
                }
            } else {
                // File exists only in source (new file)
                if source_file.is_symlink {
                    if let Some(ref target) = source_file.symlink_target {
                        operations.push(FileOperation::CreateSymlink {
                            path: source_file.path.clone(),
                            target: target.clone(),
                        });
                    }
                } else if source_file.is_directory {
                    operations.push(FileOperation::CreateDirectory {
                        path: source_file.path.clone(),
                    });
                } else {
                    operations.push(FileOperation::Create {
                        path: source_file.path.clone(),
                    });
                }
            }

            (operations, processed_target)
        })
        .unzip();

    // Flatten operations from parallel processing
    let mut operations: Vec<FileOperation> = source_operations.into_iter().flatten().collect();

    // Collect processed targets for deletion check
    let processed_targets: HashSet<PathBuf> = processed_targets.into_iter().flatten().collect();

    // Process files that exist only in target (deleted files) - also in parallel
    // Skip this if purge mode is enabled to avoid duplicates (purge operations are handled separately)
    let target_operations: Vec<FileOperation> = if !options.purge && !options.mirror {
        target
            .par_iter()
            .filter_map(|target_file| {
                target_file
                    .path
                    .strip_prefix(dest_root)
                    .ok()
                    .and_then(|relative_path| {
                        if !processed_targets.contains(relative_path) {
                            Some(FileOperation::Delete {
                                path: target_file.path.clone(),
                            })
                        } else {
                            None
                        }
                    })
            })
            .collect()
    } else {
        Vec::new()
    };

    // Update progress for target processing
    if let Some(ref callback) = progress_callback {
        callback(source.len() + target.len());
    }

    operations.extend(target_operations);
    operations
}

/// Compare two file lists to find differences (legacy function for backward compatibility)
pub fn compare_file_lists(source: &[FileInfo], target: &[FileInfo]) -> Vec<FileOperation> {
    // Use default options for backward compatibility
    let default_options = SyncOptions::default();
    compare_file_lists_with_options(source, target, &default_options)
}

/// Compare two file lists to find differences with options
pub fn compare_file_lists_with_options(
    source: &[FileInfo],
    target: &[FileInfo],
    options: &SyncOptions,
) -> Vec<FileOperation> {
    let mut operations = Vec::new();

    // Create maps for efficient lookup
    let mut target_map: HashMap<PathBuf, &FileInfo> = HashMap::new();
    for file in target {
        target_map.insert(file.path.clone(), file);
    }

    let mut processed_targets = HashSet::new();

    // Process each source file
    for source_file in source {
        if let Some(target_file) = target_map.get(&source_file.path) {
            // File exists in both source and target
            processed_targets.insert(&source_file.path);

            // Handle symlinks first
            if source_file.is_symlink && target_file.is_symlink {
                // Both are symlinks, check if they point to the same target
                if source_file.symlink_target != target_file.symlink_target {
                    if let Some(ref target) = source_file.symlink_target {
                        operations.push(FileOperation::UpdateSymlink {
                            path: source_file.path.clone(),
                            target: target.clone(),
                        });
                    }
                }
            } else if source_file.is_symlink && !target_file.is_symlink {
                // Source is symlink, target is not - delete target and create symlink
                operations.push(FileOperation::Delete {
                    path: source_file.path.clone(),
                });
                if let Some(ref target) = source_file.symlink_target {
                    operations.push(FileOperation::CreateSymlink {
                        path: source_file.path.clone(),
                        target: target.clone(),
                    });
                }
            } else if !source_file.is_symlink && target_file.is_symlink {
                // Source is not symlink, target is - delete symlink and create file/dir
                operations.push(FileOperation::Delete {
                    path: source_file.path.clone(),
                });
                if source_file.is_directory {
                    operations.push(FileOperation::CreateDirectory {
                        path: source_file.path.clone(),
                    });
                } else {
                    operations.push(FileOperation::Create {
                        path: source_file.path.clone(),
                    });
                }
            } else if source_file.is_directory && target_file.is_directory {
                // Both are directories, no action needed
                continue;
            } else if source_file.is_directory && !target_file.is_directory {
                // Source is directory, target is file - delete file and create directory
                operations.push(FileOperation::Delete {
                    path: source_file.path.clone(),
                });
                operations.push(FileOperation::CreateDirectory {
                    path: source_file.path.clone(),
                });
            } else if !source_file.is_directory && target_file.is_directory {
                // Source is file, target is directory - delete directory and create file
                operations.push(FileOperation::Delete {
                    path: source_file.path.clone(),
                });
                operations.push(FileOperation::Create {
                    path: source_file.path.clone(),
                });
            } else if !source_file.is_directory && !target_file.is_directory {
                // Both are files, check if update is needed
                if needs_update(source_file, target_file, options) {
                    let use_delta = should_use_delta(source_file, target_file);
                    operations.push(FileOperation::Update {
                        path: source_file.path.clone(),
                        use_delta,
                    });
                }
            }
        } else {
            // File exists only in source (new file)
            if source_file.is_symlink {
                if let Some(ref target) = source_file.symlink_target {
                    operations.push(FileOperation::CreateSymlink {
                        path: source_file.path.clone(),
                        target: target.clone(),
                    });
                }
            } else if source_file.is_directory {
                operations.push(FileOperation::CreateDirectory {
                    path: source_file.path.clone(),
                });
            } else {
                operations.push(FileOperation::Create {
                    path: source_file.path.clone(),
                });
            }
        }
    }

    // Process files that exist only in target (deleted files)
    for target_file in target {
        if !processed_targets.contains(&target_file.path) {
            operations.push(FileOperation::Delete {
                path: target_file.path.clone(),
            });
        }
    }

    // Sort operations: create directories first, then files, then deletions
    operations.sort_by(|a, b| {
        use FileOperation::*;
        match (a, b) {
            (CreateDirectory { .. }, CreateDirectory { .. }) => std::cmp::Ordering::Equal,
            (CreateDirectory { .. }, _) => std::cmp::Ordering::Less,
            (_, CreateDirectory { .. }) => std::cmp::Ordering::Greater,
            (Delete { .. }, Delete { .. }) => std::cmp::Ordering::Equal,
            (Delete { .. }, _) => std::cmp::Ordering::Greater,
            (_, Delete { .. }) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Equal,
        }
    });

    operations
}

/// Determine if a file needs to be updated
fn needs_update(source: &FileInfo, target: &FileInfo, options: &SyncOptions) -> bool {
    // If checksum mode is enabled, compare checksums if both are available
    if options.checksum {
        match (&source.checksum, &target.checksum) {
            (Some(source_checksum), Some(target_checksum)) => {
                // Both have checksums, compare them
                return source_checksum != target_checksum;
            }
            _ => {
                // If checksums are not available, fall back to traditional comparison
                // This can happen during the transition or if checksum calculation failed
            }
        }
    }

    // Traditional comparison: modification time and size
    source.modified > target.modified || source.size != target.size
}

/// Determine if delta algorithm should be used for update
fn should_use_delta(source: &FileInfo, target: &FileInfo) -> bool {
    // Use delta for files larger than 1KB where size difference is less than 50%
    const MIN_SIZE_FOR_DELTA: u64 = 1024;
    const MAX_SIZE_DIFFERENCE_RATIO: f64 = 0.5;

    if source.size < MIN_SIZE_FOR_DELTA || target.size < MIN_SIZE_FOR_DELTA {
        return false;
    }

    let size_diff = source.size.abs_diff(target.size);

    let size_diff_ratio = size_diff as f64 / target.size.max(source.size) as f64;

    size_diff_ratio < MAX_SIZE_DIFFERENCE_RATIO
}

/// Compute checksum for a file using Blake3 (fast, secure, default) with streaming
fn compute_file_checksum(path: &Path) -> Result<Option<Vec<u8>>> {
    use std::fs::File;
    use std::io::{BufReader, Read};

    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file); // 1MB buffer for better performance

    // Use Blake3 streaming hasher for memory efficiency
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 1024 * 1024];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let hash = hasher.finalize();
    Ok(Some(hash.as_bytes().to_vec()))
}

/// Operations that need to be performed during sync
#[derive(Debug, Clone)]
pub enum FileOperation {
    Create { path: PathBuf },
    Update { path: PathBuf, use_delta: bool },
    Delete { path: PathBuf },
    CreateDirectory { path: PathBuf },
    CreateSymlink { path: PathBuf, target: PathBuf },
    UpdateSymlink { path: PathBuf, target: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn create_test_file(path: &str, size: u64, modified_offset: u64, is_dir: bool) -> FileInfo {
        FileInfo {
            path: PathBuf::from(path),
            size,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(modified_offset),
            is_directory: is_dir,
            is_symlink: false,
            symlink_target: None,
            checksum: None,
        }
    }

    #[test]
    fn test_compare_identical_lists() {
        let files = vec![
            create_test_file("file1.txt", 100, 1000, false),
            create_test_file("dir1", 0, 1000, true),
        ];

        let operations = compare_file_lists(&files, &files);
        assert!(operations.is_empty());
    }

    #[test]
    fn test_compare_new_files() {
        let source = vec![
            create_test_file("file1.txt", 100, 1000, false),
            create_test_file("dir1", 0, 1000, true),
        ];
        let target = vec![];

        let operations = compare_file_lists(&source, &target);
        assert_eq!(operations.len(), 2);

        // Should create directory first, then file
        assert!(matches!(
            operations[0],
            FileOperation::CreateDirectory { .. }
        ));
        assert!(matches!(operations[1], FileOperation::Create { .. }));
    }

    #[test]
    fn test_needs_update() {
        let old_file = create_test_file("file.txt", 100, 1000, false);
        let new_file = create_test_file("file.txt", 100, 2000, false);
        let different_size = create_test_file("file.txt", 200, 1000, false);
        let options = SyncOptions::default(); // No checksum comparison

        assert!(needs_update(&new_file, &old_file, &options)); // Newer modification time
        assert!(needs_update(&different_size, &old_file, &options)); // Different size
        assert!(!needs_update(&old_file, &new_file, &options)); // Older file
    }

    #[test]
    fn test_should_use_delta() {
        let small_file = create_test_file("small.txt", 500, 1000, false);
        let large_file = create_test_file("large.txt", 10000, 1000, false);
        let similar_large = create_test_file("large.txt", 10100, 1000, false);
        let very_different = create_test_file("large.txt", 20000, 1000, false);

        assert!(!should_use_delta(&small_file, &small_file)); // Too small
        assert!(should_use_delta(&large_file, &similar_large)); // Similar size
        assert!(!should_use_delta(&large_file, &very_different)); // Too different
    }

    #[test]
    fn test_pattern_matching() {
        // Exact matches
        assert!(matches_pattern("file.txt", "file.txt"));
        assert!(!matches_pattern("file.txt", "other.txt"));

        // Wildcard * matches
        assert!(matches_pattern("file.txt", "*.txt"));
        assert!(matches_pattern("document.pdf", "*.pdf"));
        assert!(matches_pattern("backup_2023.txt", "backup_*.txt"));
        assert!(!matches_pattern("file.pdf", "*.txt"));

        // Wildcard ? matches
        assert!(matches_pattern("file1.txt", "file?.txt"));
        assert!(matches_pattern("fileA.txt", "file?.txt"));
        assert!(!matches_pattern("file12.txt", "file?.txt"));

        // Complex patterns
        assert!(matches_pattern("temp_file_123.tmp", "temp_*_*.tmp"));
        assert!(matches_pattern("log.2023-01-01", "log.????-??-??"));
    }

    #[test]
    fn test_file_filtering() {
        use crate::options::SyncOptions;

        let options = SyncOptions {
            recursive: true,
            purge: false,
            mirror: false,
            dry_run: false,
            verbose: 0,
            confirm: false,
            no_progress: false,
            move_files: false,
            exclude_files: vec!["*.tmp".to_string(), "*.log".to_string()],
            exclude_dirs: vec!["cache".to_string(), ".git".to_string()],
            min_size: Some(100),
            max_size: Some(10000),
            copy_flags: "DAT".to_string(),
            log_file: None,
            compress: false,
            compression_config: crate::compression::CompressionConfig::default(),
            show_eta: false,
            retry_count: 0,
            retry_wait: 30,
            checksum: false,
            forced_strategy: None,
            symlink_behavior: crate::options::SymlinkBehavior::Preserve,
            no_report_errors: false,
            small_file_threshold: None,
            medium_file_threshold: None,
            large_file_threshold: None,
            #[cfg(target_os = "linux")]
            linux_optimized: false,
        };

        let root = std::path::Path::new("/test");

        // File should be excluded by pattern
        let tmp_file = FileInfo {
            path: std::path::PathBuf::from("/test/temp.tmp"),
            size: 500,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            is_symlink: false,
            symlink_target: None,
            checksum: None,
        };
        assert!(!should_include_file(&tmp_file, root, &options));

        // File should be excluded by size (too small)
        let small_file = FileInfo {
            path: std::path::PathBuf::from("/test/small.txt"),
            size: 50,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            is_symlink: false,
            symlink_target: None,
            checksum: None,
        };
        assert!(!should_include_file(&small_file, root, &options));

        // File should be excluded by size (too large)
        let large_file = FileInfo {
            path: std::path::PathBuf::from("/test/large.txt"),
            size: 20000,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            is_symlink: false,
            symlink_target: None,
            checksum: None,
        };
        assert!(!should_include_file(&large_file, root, &options));

        // File should be included
        let good_file = FileInfo {
            path: std::path::PathBuf::from("/test/document.txt"),
            size: 5000,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            is_symlink: false,
            symlink_target: None,
            checksum: None,
        };
        assert!(should_include_file(&good_file, root, &options));

        // Directory should be excluded
        let cache_dir = FileInfo {
            path: std::path::PathBuf::from("/test/cache"),
            size: 0,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: true,
            is_symlink: false,
            symlink_target: None,
            checksum: None,
        };
        assert!(!should_include_file(&cache_dir, root, &options));
    }

    #[test]
    fn test_symlink_operations() {
        use std::path::PathBuf;

        // Create test symlink file info
        let symlink_file = FileInfo {
            path: PathBuf::from("/test/link.txt"),
            size: 0,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            is_symlink: true,
            symlink_target: Some(PathBuf::from("target.txt")),
            checksum: None,
        };

        let target_regular_file = FileInfo {
            path: PathBuf::from("/test/link.txt"),
            size: 100,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            is_symlink: false,
            symlink_target: None,
            checksum: None,
        };

        // Test symlink to regular file replacement
        let operations =
            compare_file_lists(&[symlink_file.clone()], &[target_regular_file.clone()]);
        assert_eq!(operations.len(), 2);
        // Operations are sorted: directories first, then other operations, then deletions last
        assert!(matches!(operations[0], FileOperation::CreateSymlink { .. }));
        assert!(matches!(operations[1], FileOperation::Delete { .. }));

        // Test regular file to symlink replacement
        let operations = compare_file_lists(&[target_regular_file], &[symlink_file.clone()]);
        assert_eq!(operations.len(), 2);
        // Operations are sorted: directories first, then other operations, then deletions last
        assert!(matches!(operations[0], FileOperation::Create { .. }));
        assert!(matches!(operations[1], FileOperation::Delete { .. }));

        // Test symlink target change
        let symlink_file2 = FileInfo {
            path: PathBuf::from("/test/link.txt"),
            size: 0,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            is_symlink: true,
            symlink_target: Some(PathBuf::from("different_target.txt")),
            checksum: None,
        };

        let operations = compare_file_lists(&[symlink_file2], &[symlink_file]);
        assert_eq!(operations.len(), 1);
        assert!(matches!(operations[0], FileOperation::UpdateSymlink { .. }));
    }

    #[test]
    fn test_checksum_comparison() {
        use std::path::PathBuf;

        // Create files with same size and time but different content (different checksums)
        let checksum1 = Some(vec![1, 2, 3, 4]);
        let checksum2 = Some(vec![5, 6, 7, 8]);

        let file1 = FileInfo {
            path: PathBuf::from("/test/file.txt"),
            size: 100,
            modified: std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000),
            is_directory: false,
            is_symlink: false,
            symlink_target: None,
            checksum: checksum1.clone(),
        };

        let file2 = FileInfo {
            path: PathBuf::from("/test/file.txt"),
            size: 100, // Same size
            modified: std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000), // Same time
            is_directory: false,
            is_symlink: false,
            symlink_target: None,
            checksum: checksum2, // Different checksum
        };

        // Test with checksum mode enabled
        let checksum_options = SyncOptions {
            checksum: true,
            ..Default::default()
        };

        // Should detect difference with checksum mode
        assert!(needs_update(&file1, &file2, &checksum_options));

        // Test with checksum mode disabled (default behavior)
        let normal_options = SyncOptions::default();

        // Should NOT detect difference without checksum mode (same size and time)
        assert!(!needs_update(&file1, &file2, &normal_options));

        // Test with identical checksums
        let file3 = FileInfo {
            checksum: checksum1.clone(),
            ..file2.clone()
        };

        assert!(!needs_update(&file1, &file3, &checksum_options));
    }
}

/// Dereference a symlink and create FileInfo for the target
fn dereference_symlink(symlink_path: &Path, target_path: &Path) -> Result<FileInfo> {
    // Resolve the target path (could be relative to the symlink)
    let resolved_target = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        // Relative path - resolve relative to the symlink's parent directory
        if let Some(parent) = symlink_path.parent() {
            parent.join(target_path)
        } else {
            target_path.to_path_buf()
        }
    };

    // Get metadata of the target (following the symlink)
    let metadata = std::fs::metadata(&resolved_target).with_context(|| {
        format!(
            "Failed to get metadata for symlink target: {}",
            resolved_target.display()
        )
    })?;

    // Create FileInfo for the target, but keep the original symlink path
    // This way the file will be copied to the symlink's location but with the target's content
    Ok(FileInfo {
        path: symlink_path.to_path_buf(), // Keep original symlink path as destination
        size: metadata.len(),
        modified: metadata.modified()?,
        is_directory: metadata.is_dir(),
        is_symlink: false,    // This is now treated as a regular file/directory
        symlink_target: None, // No longer a symlink
        checksum: None,       // Will be computed later if needed
    })
}
