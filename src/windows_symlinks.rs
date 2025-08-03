//! Windows symlink support
//!
//! This module provides full symlink support on Windows, handling both
//! file and directory symbolic links with proper permission management.

#[cfg(windows)]
use std::os::windows::fs as windows_fs;
use std::path::Path;
use std::io;
use anyhow::{Result, Context};

/// Windows symlink types
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SymlinkType {
    /// File symbolic link
    File,
    /// Directory symbolic link  
    Directory,
}

/// Determine the type of symlink to create based on the target
#[cfg(windows)]
pub fn determine_symlink_type(target: &Path) -> Result<SymlinkType> {
    // If the target exists, check its type
    if target.exists() {
        let metadata = std::fs::metadata(target)
            .with_context(|| format!("Failed to read metadata for symlink target: {}", target.display()))?;
        
        if metadata.is_dir() {
            Ok(SymlinkType::Directory)
        } else {
            Ok(SymlinkType::File)
        }
    } else {
        // If target doesn't exist, try to infer from the path
        // If it ends with a separator or has no extension, assume directory
        if target.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR) {
            Ok(SymlinkType::Directory)
        } else if target.extension().is_none() && !target.to_string_lossy().contains('.') {
            // No extension often means directory
            Ok(SymlinkType::Directory)
        } else {
            // Default to file
            Ok(SymlinkType::File)
        }
    }
}

/// Create a symbolic link on Windows
/// 
/// This function handles both file and directory symlinks and manages
/// the Windows-specific requirements for creating symbolic links.
#[cfg(windows)]
pub fn create_symlink(link: &Path, target: &Path) -> Result<()> {
    // Determine symlink type
    let symlink_type = determine_symlink_type(target)?;
    
    // Ensure parent directory exists
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent directory for symlink: {}", parent.display()))?;
    }
    
    // Remove existing file/link if it exists
    if link.exists() {
        if link.symlink_metadata()?.file_type().is_symlink() {
            std::fs::remove_file(link)
                .or_else(|_| std::fs::remove_dir(link))
                .with_context(|| format!("Failed to remove existing symlink: {}", link.display()))?;
        }
    }
    
    // Create the symlink based on type
    match symlink_type {
        SymlinkType::File => {
            windows_fs::symlink_file(target, link)
                .map_err(|e| handle_symlink_error(e, link, target, "file"))?;
        }
        SymlinkType::Directory => {
            windows_fs::symlink_dir(target, link)
                .map_err(|e| handle_symlink_error(e, link, target, "directory"))?;
        }
    }
    
    Ok(())
}

/// Handle Windows symlink creation errors with helpful context
#[cfg(windows)]
fn handle_symlink_error(error: io::Error, link: &Path, target: &Path, link_type: &str) -> anyhow::Error {
    use std::io::ErrorKind;
    
    match error.kind() {
        ErrorKind::PermissionDenied => {
            anyhow::anyhow!(
                "Permission denied creating {} symlink '{}' -> '{}'. \
                Windows requires either: \
                1) Run as Administrator, or \
                2) Developer Mode enabled (Windows 10+), or \
                3) SeCreateSymbolicLinkPrivilege granted to user",
                link_type,
                link.display(),
                target.display()
            )
        }
        ErrorKind::NotFound => {
            anyhow::anyhow!(
                "Failed to create {} symlink '{}' -> '{}': target path or parent directory not found",
                link_type,
                link.display(),
                target.display()
            )
        }
        _ => {
            anyhow::anyhow!(
                "Failed to create {} symlink '{}' -> '{}': {}",
                link_type,
                link.display(),
                target.display(),
                error
            )
        }
    }
}

/// Read a symbolic link target on Windows
#[cfg(windows)]
pub fn read_symlink(link: &Path) -> Result<std::path::PathBuf> {
    std::fs::read_link(link)
        .with_context(|| format!("Failed to read symlink target: {}", link.display()))
}

/// Check if a path is a symbolic link on Windows
#[cfg(windows)]
pub fn is_symlink(path: &Path) -> Result<bool> {
    let metadata = path.symlink_metadata()
        .with_context(|| format!("Failed to read symlink metadata: {}", path.display()))?;
    Ok(metadata.file_type().is_symlink())
}

/// Copy a symbolic link preserving its target
#[cfg(windows)]
pub fn copy_symlink(source: &Path, dest: &Path) -> Result<()> {
    let target = read_symlink(source)?;
    create_symlink(dest, &target)?;
    Ok(())
}

// Non-Windows stub implementations
#[cfg(not(windows))]
pub fn create_symlink(_link: &Path, _target: &Path) -> Result<()> {
    Err(anyhow::anyhow!("Windows symlink functions called on non-Windows platform"))
}

#[cfg(not(windows))]
pub fn read_symlink(_link: &Path) -> Result<std::path::PathBuf> {
    Err(anyhow::anyhow!("Windows symlink functions called on non-Windows platform"))
}

#[cfg(not(windows))]
pub fn is_symlink(_path: &Path) -> Result<bool> {
    Err(anyhow::anyhow!("Windows symlink functions called on non-Windows platform"))
}

#[cfg(not(windows))]
pub fn copy_symlink(_source: &Path, _dest: &Path) -> Result<()> {
    Err(anyhow::anyhow!("Windows symlink functions called on non-Windows platform"))
}

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_determine_symlink_type() {
        // Test with directory path
        assert_eq!(
            determine_symlink_type(Path::new("C:\\some\\dir\\")).unwrap(),
            SymlinkType::Directory
        );
        
        // Test with file path
        assert_eq!(
            determine_symlink_type(Path::new("C:\\some\\file.txt")).unwrap(),
            SymlinkType::File
        );
    }

    #[test]
    #[ignore] // Requires admin privileges
    fn test_create_file_symlink() {
        let temp_dir = tempdir().unwrap();
        let target = temp_dir.path().join("target.txt");
        let link = temp_dir.path().join("link.txt");
        
        // Create target file
        std::fs::write(&target, "test content").unwrap();
        
        // Create symlink
        create_symlink(&link, &target).unwrap();
        
        // Verify symlink
        assert!(is_symlink(&link).unwrap());
        assert_eq!(read_symlink(&link).unwrap(), target);
    }

    #[test]
    #[ignore] // Requires admin privileges
    fn test_create_directory_symlink() {
        let temp_dir = tempdir().unwrap();
        let target = temp_dir.path().join("target_dir");
        let link = temp_dir.path().join("link_dir");
        
        // Create target directory
        std::fs::create_dir(&target).unwrap();
        
        // Create symlink
        create_symlink(&link, &target).unwrap();
        
        // Verify symlink
        assert!(is_symlink(&link).unwrap());
        assert_eq!(read_symlink(&link).unwrap(), target);
    }
}