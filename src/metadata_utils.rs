//! Utility functions for metadata operations
//! 
//! This module consolidates duplicated metadata copying patterns
//! found throughout the codebase.

use std::fs::Metadata;
use std::path::Path;
use anyhow::Result;

use crate::metadata::{CopyFlags, copy_timestamps, copy_permissions, copy_attributes, copy_ownership};

/// Apply all metadata based on copy flags in the correct order
/// 
/// This consolidates the common pattern of applying multiple metadata
/// operations based on CopyFlags. The order is important for some filesystems.
pub fn apply_metadata_from_flags(
    source: &Path,
    destination: &Path,
    source_metadata: &Metadata,
    copy_flags: &CopyFlags,
) -> Result<()> {
    // Apply in the order: timestamps, permissions, attributes, ownership
    // This order ensures best compatibility across different filesystems
    
    if copy_flags.timestamps {
        copy_timestamps(source, destination, source_metadata)?;
    }
    
    if copy_flags.security {
        copy_permissions(source, destination, source_metadata)?;
    }
    
    if copy_flags.attributes {
        copy_attributes(source, destination, source_metadata)?;
    }
    
    if copy_flags.owner {
        copy_ownership(source, destination, source_metadata)?;
    }
    
    Ok(())
}

/// Apply metadata after delta transfer operations
/// 
/// This is specifically for delta transfers where the file content
/// has been updated but metadata needs to be copied separately.
pub fn apply_metadata_after_delta(
    source: &Path,
    destination: &Path,
    copy_flags: &CopyFlags,
) -> Result<()> {
    if let Ok(metadata) = std::fs::metadata(source) {
        apply_metadata_from_flags(source, destination, &metadata, copy_flags)?;
    }
    Ok(())
}

/// Get file metadata with error handling
/// 
/// Helper function that provides consistent error handling for metadata retrieval
pub fn get_file_metadata(path: &Path) -> Result<Metadata> {
    std::fs::metadata(path).map_err(|e| {
        anyhow::anyhow!("Failed to get metadata for {}: {}", path.display(), e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

    #[test]
    fn test_apply_metadata_with_minimal_flags() {
        let temp_dir = tempdir().unwrap();
        let source = temp_dir.path().join("source.txt");
        let dest = temp_dir.path().join("dest.txt");
        
        // Create test files
        fs::write(&source, "test content").unwrap();
        fs::write(&dest, "test content").unwrap();
        
        let metadata = fs::metadata(&source).unwrap();
        let flags = CopyFlags {
            data: true,
            timestamps: true,
            security: false,
            attributes: false,
            owner: false,
            auditing: false,
        };
        
        // Should not fail even with minimal flags
        let result = apply_metadata_from_flags(&source, &dest, &metadata, &flags);
        assert!(result.is_ok());
    }
}