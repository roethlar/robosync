//! File list generation and management

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use walkdir::WalkDir;
use crate::options::SyncOptions;

/// File metadata for synchronization
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub path: PathBuf,
    pub size: u64,
    pub modified: std::time::SystemTime,
    pub is_directory: bool,
    pub checksum: Option<Vec<u8>>,
}

/// Generate file list from a directory
pub fn generate_file_list(root: &Path) -> Result<Vec<FileInfo>> {
    let mut files = Vec::new();
    
    for entry in WalkDir::new(root) {
        let entry = entry?;
        let metadata = entry.metadata()?;
        
        let file_info = FileInfo {
            path: entry.path().to_path_buf(),
            size: metadata.len(),
            modified: metadata.modified()?,
            is_directory: metadata.is_dir(),
            checksum: None, // Will be computed later if needed
        };
        
        files.push(file_info);
    }
    
    Ok(files)
}

/// Generate file list from a directory with filtering
pub fn generate_file_list_with_options(root: &Path, options: &SyncOptions) -> Result<Vec<FileInfo>> {
    let mut files = Vec::new();
    
    for entry in WalkDir::new(root) {
        let entry = entry?;
        let metadata = entry.metadata()?;
        
        let file_info = FileInfo {
            path: entry.path().to_path_buf(),
            size: metadata.len(),
            modified: metadata.modified()?,
            is_directory: metadata.is_dir(),
            checksum: None, // Will be computed later if needed
        };
        
        // Apply filters
        if should_include_file(&file_info, root, options) {
            files.push(file_info);
        }
    }
    
    Ok(files)
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
            if matches_pattern(&file_name_str, pattern) || matches_pattern(&relative_path.to_string_lossy(), pattern) {
                return false;
            }
        }
    }
    
    // Check directory patterns (/XD)
    if file_info.is_directory {
        if let Some(dir_name) = file_info.path.file_name() {
            let dir_name_str = dir_name.to_string_lossy();
            
            for pattern in &options.exclude_dirs {
                if matches_pattern(&dir_name_str, pattern) || matches_pattern(&relative_path.to_string_lossy(), pattern) {
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
                if matches_pattern(&dir_name_str, pattern) || matches_pattern(&ancestor.to_string_lossy(), pattern) {
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
                
                for i in 0..=remaining_text.len() {
                    if matches_pattern(&remaining_text[i..], &remaining_pattern) {
                        return true;
                    }
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
    dest_root: &Path
) -> Vec<FileOperation> {
    let mut operations = Vec::new();
    
    // Create maps for efficient lookup using relative paths
    let mut target_map: HashMap<PathBuf, &FileInfo> = HashMap::new();
    for file in target {
        if let Ok(relative_path) = file.path.strip_prefix(dest_root) {
            target_map.insert(relative_path.to_path_buf(), file);
        }
    }
    
    let mut processed_targets = HashSet::new();
    
    // Process each source file
    for source_file in source {
        if let Ok(relative_path) = source_file.path.strip_prefix(source_root) {
            if let Some(target_file) = target_map.get(relative_path) {
                // File exists in both source and target
                processed_targets.insert(relative_path.to_path_buf());
                
                if source_file.is_directory && target_file.is_directory {
                    // Both are directories, no action needed
                    continue;
                } else if source_file.is_directory && !target_file.is_directory {
                    // Source is directory, target is file - delete file and create directory
                    operations.push(FileOperation::Delete { path: source_file.path.clone() });
                    operations.push(FileOperation::CreateDirectory { path: source_file.path.clone() });
                } else if !source_file.is_directory && target_file.is_directory {
                    // Source is file, target is directory - delete directory and create file
                    operations.push(FileOperation::Delete { path: source_file.path.clone() });
                    operations.push(FileOperation::Create { path: source_file.path.clone() });
                } else if !source_file.is_directory && !target_file.is_directory {
                    // Both are files, check if update is needed
                    if needs_update(source_file, target_file) {
                        let use_delta = should_use_delta(source_file, target_file);
                        operations.push(FileOperation::Update { 
                            path: source_file.path.clone(), 
                            use_delta 
                        });
                    }
                }
            } else {
                // File exists only in source (new file)
                if source_file.is_directory {
                    operations.push(FileOperation::CreateDirectory { path: source_file.path.clone() });
                } else {
                    operations.push(FileOperation::Create { path: source_file.path.clone() });
                }
            }
        }
    }
    
    // Process files that exist only in target (deleted files) 
    for target_file in target {
        if let Ok(relative_path) = target_file.path.strip_prefix(dest_root) {
            if !processed_targets.contains(relative_path) {
                operations.push(FileOperation::Delete { path: target_file.path.clone() });
            }
        }
    }
    
    operations
}

/// Compare two file lists to find differences (legacy function for backward compatibility)
pub fn compare_file_lists(source: &[FileInfo], target: &[FileInfo]) -> Vec<FileOperation> {
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
            
            if source_file.is_directory && target_file.is_directory {
                // Both are directories, no action needed
                continue;
            } else if source_file.is_directory && !target_file.is_directory {
                // Source is directory, target is file - delete file and create directory
                operations.push(FileOperation::Delete { path: source_file.path.clone() });
                operations.push(FileOperation::CreateDirectory { path: source_file.path.clone() });
            } else if !source_file.is_directory && target_file.is_directory {
                // Source is file, target is directory - delete directory and create file
                operations.push(FileOperation::Delete { path: source_file.path.clone() });
                operations.push(FileOperation::Create { path: source_file.path.clone() });
            } else if !source_file.is_directory && !target_file.is_directory {
                // Both are files, check if update is needed
                if needs_update(source_file, target_file) {
                    let use_delta = should_use_delta(source_file, target_file);
                    operations.push(FileOperation::Update { 
                        path: source_file.path.clone(), 
                        use_delta 
                    });
                }
            }
        } else {
            // File exists only in source (new file)
            if source_file.is_directory {
                operations.push(FileOperation::CreateDirectory { path: source_file.path.clone() });
            } else {
                operations.push(FileOperation::Create { path: source_file.path.clone() });
            }
        }
    }
    
    // Process files that exist only in target (deleted files)
    for target_file in target {
        if !processed_targets.contains(&target_file.path) {
            operations.push(FileOperation::Delete { path: target_file.path.clone() });
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
fn needs_update(source: &FileInfo, target: &FileInfo) -> bool {
    // Compare modification time and size
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
    
    let size_diff = if source.size > target.size {
        source.size - target.size
    } else {
        target.size - source.size
    };
    
    let size_diff_ratio = size_diff as f64 / target.size.max(source.size) as f64;
    
    size_diff_ratio < MAX_SIZE_DIFFERENCE_RATIO
}

/// Operations that need to be performed during sync
#[derive(Debug, Clone)]
pub enum FileOperation {
    Create { path: PathBuf },
    Update { path: PathBuf, use_delta: bool },
    Delete { path: PathBuf },
    CreateDirectory { path: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, Duration};

    fn create_test_file(path: &str, size: u64, modified_offset: u64, is_dir: bool) -> FileInfo {
        FileInfo {
            path: PathBuf::from(path),
            size,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(modified_offset),
            is_directory: is_dir,
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
        assert!(matches!(operations[0], FileOperation::CreateDirectory { .. }));
        assert!(matches!(operations[1], FileOperation::Create { .. }));
    }

    #[test]
    fn test_needs_update() {
        let old_file = create_test_file("file.txt", 100, 1000, false);
        let new_file = create_test_file("file.txt", 100, 2000, false);
        let different_size = create_test_file("file.txt", 200, 1000, false);
        
        assert!(needs_update(&new_file, &old_file)); // Newer modification time
        assert!(needs_update(&different_size, &old_file)); // Different size
        assert!(!needs_update(&old_file, &new_file)); // Older file
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
            verbose: false,
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
        };
        
        let root = std::path::Path::new("/test");
        
        // File should be excluded by pattern
        let tmp_file = FileInfo {
            path: std::path::PathBuf::from("/test/temp.tmp"),
            size: 500,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            checksum: None,
        };
        assert!(!should_include_file(&tmp_file, root, &options));
        
        // File should be excluded by size (too small)
        let small_file = FileInfo {
            path: std::path::PathBuf::from("/test/small.txt"),
            size: 50,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            checksum: None,
        };
        assert!(!should_include_file(&small_file, root, &options));
        
        // File should be excluded by size (too large)
        let large_file = FileInfo {
            path: std::path::PathBuf::from("/test/large.txt"),
            size: 20000,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            checksum: None,
        };
        assert!(!should_include_file(&large_file, root, &options));
        
        // File should be included
        let good_file = FileInfo {
            path: std::path::PathBuf::from("/test/document.txt"),
            size: 5000,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: false,
            checksum: None,
        };
        assert!(should_include_file(&good_file, root, &options));
        
        // Directory should be excluded
        let cache_dir = FileInfo {
            path: std::path::PathBuf::from("/test/cache"),
            size: 0,
            modified: std::time::SystemTime::UNIX_EPOCH,
            is_directory: true,
            checksum: None,
        };
        assert!(!should_include_file(&cache_dir, root, &options));
    }
}