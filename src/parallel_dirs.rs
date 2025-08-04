//! Parallel directory creation module for optimized directory structure setup
//!
//! This module provides efficient parallel creation of directories by grouping them
//! by depth level and creating all directories at the same depth in parallel.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Result;
use indicatif::ProgressBar;
use rayon::prelude::*;

/// A parallel directory creator that uses a thread pool to create directories efficiently
pub struct ParallelDirCreator;

impl ParallelDirCreator {
    /// Create a new ParallelDirCreator
    pub fn new() -> Self {
        Self
    }

    /// Create multiple directories in parallel, respecting parent-child dependencies
    ///
    /// # Arguments
    /// * `dirs` - Vector of directory paths to create
    /// * `progress` - Optional progress bar for reporting progress
    ///
    /// # Returns
    /// * `Ok((successes, errors))` - Tuple of successfully created paths and errors
    pub fn create_directories(
        &self,
        dirs: Vec<PathBuf>,
        progress: Option<&ProgressBar>,
    ) -> Result<(Vec<PathBuf>, Vec<(PathBuf, io::Error)>)> {
        if dirs.is_empty() {
            return Ok((vec![], vec![]));
        }

        // Deduplicate and canonicalize paths
        let unique_dirs = deduplicate_and_canonicalize(dirs)?;
        
        // Group directories by depth
        let depth_groups = group_by_depth(unique_dirs);
        
        let total_dirs = depth_groups.values().map(|v| v.len()).sum::<usize>();
        let mut created_count = 0;
        
        if let Some(pb) = progress {
            pb.set_message(format!("Creating {} directories...", total_dirs));
            pb.set_length(total_dirs as u64);
        }

        let mut all_successes = Vec::new();
        let mut all_errors = Vec::new();

        // Process each depth level sequentially, but create directories within each level in parallel
        for (depth, dirs_at_depth) in depth_groups {
            if let Some(pb) = progress {
                pb.set_message(format!("Creating directories at depth {}...", depth));
            }

            let results = self.create_depth_level(&dirs_at_depth);
            
            for result in results {
                match result {
                    Ok(path) => {
                        all_successes.push(path);
                        created_count += 1;
                        if let Some(pb) = progress {
                            pb.set_position(created_count as u64);
                        }
                    }
                    Err((path, err)) => {
                        all_errors.push((path, err));
                        created_count += 1;
                        if let Some(pb) = progress {
                            pb.set_position(created_count as u64);
                        }
                    }
                }
            }
        }

        if let Some(pb) = progress {
            if all_errors.is_empty() {
                pb.finish_with_message(format!("Created {} directories", all_successes.len()));
            } else {
                pb.finish_with_message(format!(
                    "Created {} directories, {} errors",
                    all_successes.len(),
                    all_errors.len()
                ));
            }
        }

        Ok((all_successes, all_errors))
    }

    /// Create all directories at a specific depth level in parallel
    fn create_depth_level(
        &self,
        dirs: &[PathBuf],
    ) -> Vec<Result<PathBuf, (PathBuf, io::Error)>> {
        dirs.par_iter()
            .map(|dir| {
                match fs::create_dir(dir) {
                    Ok(_) => Ok(dir.clone()),
                    Err(e) => {
                        // If directory already exists, consider it a success
                        if e.kind() == io::ErrorKind::AlreadyExists {
                            Ok(dir.clone())
                        } else {
                            Err((dir.clone(), e))
                        }
                    }
                }
            })
            .collect()
    }
}

/// Group directories by their depth (number of path components)
fn group_by_depth(dirs: Vec<PathBuf>) -> BTreeMap<usize, Vec<PathBuf>> {
    let mut groups: BTreeMap<usize, Vec<PathBuf>> = BTreeMap::new();
    
    for dir in dirs {
        let depth = dir.components().count();
        groups.entry(depth).or_insert_with(Vec::new).push(dir);
    }
    
    groups
}

/// Deduplicate and canonicalize paths to handle different representations
fn deduplicate_and_canonicalize(dirs: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut unique_paths = std::collections::HashSet::new();
    let mut result = Vec::new();
    
    for dir in dirs {
        // We can't canonicalize paths that don't exist yet, so we'll normalize them instead
        let normalized = normalize_path(&dir);
        
        if unique_paths.insert(normalized.clone()) {
            result.push(normalized);
        }
    }
    
    // Also ensure we have all parent directories
    let mut all_dirs = std::collections::HashSet::new();
    for dir in &result {
        let mut current = dir.clone();
        while let Some(parent) = current.parent() {
            if parent == Path::new("") || parent == Path::new("/") {
                break;
            }
            all_dirs.insert(parent.to_path_buf());
            current = parent.to_path_buf();
        }
    }
    
    // Add parent directories to the result
    for parent in all_dirs {
        if unique_paths.insert(parent.clone()) {
            result.push(parent);
        }
    }
    
    Ok(result)
}

/// Normalize a path without requiring it to exist
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {
                // Skip "." components
            }
            std::path::Component::ParentDir => {
                // Handle ".." by removing the last component if possible
                if !components.is_empty() {
                    components.pop();
                }
            }
            other => {
                components.push(other.as_os_str().to_owned());
            }
        }
    }
    
    // Reconstruct the path
    let mut result = PathBuf::new();
    for component in components {
        result.push(component);
    }
    
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_group_by_depth() {
        let dirs = vec![
            PathBuf::from("a"),
            PathBuf::from("b"),
            PathBuf::from("a/b"),
            PathBuf::from("a/c"),
            PathBuf::from("a/b/c"),
        ];
        
        let groups = group_by_depth(dirs);
        
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[&1].len(), 2); // "a" and "b"
        assert_eq!(groups[&2].len(), 2); // "a/b" and "a/c"
        assert_eq!(groups[&3].len(), 1); // "a/b/c"
    }

    #[test]
    fn test_deduplicate_and_canonicalize() {
        let dirs = vec![
            PathBuf::from("a/b"),
            PathBuf::from("a/./b"),
            PathBuf::from("a/b/../b"),
            PathBuf::from("c/d"),
        ];
        
        let result = deduplicate_and_canonicalize(dirs).unwrap();
        
        // Should have "a", "a/b", "c", "c/d" (includes parents)
        assert!(result.contains(&PathBuf::from("a")));
        assert!(result.contains(&PathBuf::from("a/b")));
        assert!(result.contains(&PathBuf::from("c")));
        assert!(result.contains(&PathBuf::from("c/d")));
    }

    #[test]
    fn test_parallel_directory_creation() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();
        
        let dirs = vec![
            base.join("a"),
            base.join("b"),
            base.join("a/b"),
            base.join("a/c"),
            base.join("b/d"),
            base.join("a/b/c"),
        ];
        
        let creator = ParallelDirCreator::new();
        let (successes, errors) = creator.create_directories(dirs, None).unwrap();
        
        // All directories should be created successfully
        assert!(errors.is_empty());
        assert!(successes.len() >= 6); // May include parent directories
        
        // Verify directories exist
        assert!(base.join("a").exists());
        assert!(base.join("b").exists());
        assert!(base.join("a/b").exists());
        assert!(base.join("a/c").exists());
        assert!(base.join("b/d").exists());
        assert!(base.join("a/b/c").exists());
    }

    #[test]
    fn test_error_handling() {
        let creator = ParallelDirCreator::new();
        
        // Try to create directories in a read-only location (should fail)
        let dirs = vec![
            PathBuf::from("/root/test_dir_that_should_fail"),
        ];
        
        let (successes, errors) = creator.create_directories(dirs, None).unwrap();
        
        // Should have an error
        assert!(successes.is_empty());
        assert_eq!(errors.len(), 1);
    }
}