//! File metadata handling for copy operations

use anyhow::{Result, Context};
use std::path::Path;
use std::fs;
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

/// Copy flags that determine what metadata to copy
#[derive(Debug, Clone)]
pub struct CopyFlags {
    pub data: bool,        // D - File data
    pub attributes: bool,  // A - Attributes 
    pub timestamps: bool,  // T - Timestamps
    pub security: bool,    // S - Security (permissions)
    pub owner: bool,       // O - Owner info
    pub auditing: bool,    // U - Auditing info
}

impl CopyFlags {
    /// Parse copy flags from string (e.g., "DAT", "DATSOU")
    pub fn from_string(flags: &str) -> Self {
        let flags_upper = flags.to_uppercase();
        Self {
            data: flags_upper.contains('D'),
            attributes: flags_upper.contains('A'),
            timestamps: flags_upper.contains('T'),
            security: flags_upper.contains('S'),
            owner: flags_upper.contains('O'),
            auditing: flags_upper.contains('U'),
        }
    }

    /// Get default copy flags (DAT - Data, Attributes, Timestamps)
    pub fn default() -> Self {
        Self::from_string("DAT")
    }

    /// Get all copy flags (DATSOU)
    pub fn all() -> Self {
        Self::from_string("DATSOU")
    }
}

/// Copy a file with specified metadata preservation
pub fn copy_file_with_metadata(
    source: &Path,
    destination: &Path,
    flags: &CopyFlags,
) -> Result<u64> {
    // Always copy data if D flag is set (which it should be for file copies)
    if !flags.data {
        return Err(anyhow::anyhow!("Data flag (D) must be set for file copying"));
    }

    // Copy the file data
    let bytes_copied = fs::copy(source, destination)
        .with_context(|| format!("Failed to copy file data: {} -> {}", source.display(), destination.display()))?;

    // Get source metadata
    let source_metadata = fs::metadata(source)
        .with_context(|| format!("Failed to read source metadata: {}", source.display()))?;

    // Apply metadata based on flags
    if flags.timestamps {
        copy_timestamps(source, destination, &source_metadata)?;
    }

    if flags.security {
        copy_permissions(source, destination, &source_metadata)?;
    }

    if flags.attributes {
        copy_attributes(source, destination, &source_metadata)?;
    }

    #[cfg(unix)]
    if flags.owner {
        copy_ownership(source, destination, &source_metadata)?;
    }

    // Auditing info (U flag) is typically not supported on most filesystems
    // We'll just log that it was requested but not implemented
    if flags.auditing {
        eprintln!("Warning: Auditing info copying (U flag) not supported on this platform");
    }

    Ok(bytes_copied)
}

/// Copy file timestamps (modification and access times)
pub fn copy_timestamps(
    source: &Path,
    destination: &Path,
    source_metadata: &fs::Metadata,
) -> Result<()> {
    let modified = source_metadata.modified()
        .context("Failed to get source modification time")?;
    
    let accessed = source_metadata.accessed()
        .context("Failed to get source access time")?;

    // Set modification time
    set_file_mtime(destination, modified)
        .with_context(|| format!("Failed to set modification time: {}", destination.display()))?;

    // Note: Setting access time is not commonly supported/needed in most cases
    // and can cause issues on some filesystems, so we'll skip it for now
    let _ = accessed; // Suppress unused variable warning

    Ok(())
}

/// Copy file permissions
pub fn copy_permissions(
    source: &Path,
    destination: &Path,
    source_metadata: &fs::Metadata,
) -> Result<()> {
    let permissions = source_metadata.permissions();
    
    fs::set_permissions(destination, permissions)
        .with_context(|| format!("Failed to set permissions: {}", destination.display()))?;

    Ok(())
}

/// Copy file attributes (currently limited - could be extended for Windows extended attributes)
pub fn copy_attributes(
    _source: &Path,
    _destination: &Path,
    _source_metadata: &fs::Metadata,
) -> Result<()> {
    // Basic attributes are typically handled by permissions
    // Extended attributes would require platform-specific code
    // For now, this is a no-op but provides a place for future enhancement
    Ok(())
}

/// Copy file ownership (Unix only)
#[cfg(unix)]
pub fn copy_ownership(
    source: &Path,
    destination: &Path,
    source_metadata: &fs::Metadata,
) -> Result<()> {
    use std::os::unix::fs::fchown;
    use std::fs::File;

    let uid = source_metadata.uid();
    let gid = source_metadata.gid();

    // Open destination file to get file descriptor
    let file = File::open(destination)
        .with_context(|| format!("Failed to open destination for ownership change: {}", destination.display()))?;

    // Change ownership (requires appropriate privileges)
    fchown(&file, Some(uid), Some(gid))
        .with_context(|| format!("Failed to change ownership: {}", destination.display()))?;

    Ok(())
}

/// Set file modification time (cross-platform)
fn set_file_mtime(path: &Path, mtime: SystemTime) -> Result<()> {
    // Use filetime crate for cross-platform timestamp setting
    let filetime_mtime = filetime::FileTime::from(mtime);
    
    // Get current access time to preserve it
    let metadata = fs::metadata(path)
        .context("Failed to read file metadata for timestamp update")?;
    let atime = metadata.accessed()
        .context("Failed to get access time")?;
    let filetime_atime = filetime::FileTime::from(atime);
    
    // Set access time and modification time
    filetime::set_file_times(path, filetime_atime, filetime_mtime)
        .context("Failed to set file times")?;
    
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copy_flags_parsing() {
        let flags = CopyFlags::from_string("DAT");
        assert!(flags.data);
        assert!(flags.attributes);
        assert!(flags.timestamps);
        assert!(!flags.security);
        assert!(!flags.owner);
        assert!(!flags.auditing);

        let all_flags = CopyFlags::from_string("DATSOU");
        assert!(all_flags.data);
        assert!(all_flags.attributes);
        assert!(all_flags.timestamps);
        assert!(all_flags.security);
        assert!(all_flags.owner);
        assert!(all_flags.auditing);
    }

    #[test]
    fn test_default_flags() {
        let flags = CopyFlags::default();
        assert!(flags.data);
        assert!(flags.attributes);
        assert!(flags.timestamps);
        assert!(!flags.security);
        assert!(!flags.owner);
        assert!(!flags.auditing);
    }

    #[test]
    fn test_all_flags() {
        let flags = CopyFlags::all();
        assert!(flags.data);
        assert!(flags.attributes);
        assert!(flags.timestamps);
        assert!(flags.security);
        assert!(flags.owner);
        assert!(flags.auditing);
    }
}