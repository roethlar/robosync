//! File metadata handling for copy operations

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// Copy flags that determine what metadata to copy
#[derive(Debug, Clone)]
pub struct CopyFlags {
    pub data: bool,       // D - File data
    pub attributes: bool, // A - Attributes
    pub timestamps: bool, // T - Timestamps
    pub security: bool,   // S - Security (permissions)
    pub owner: bool,      // O - Owner info
    pub auditing: bool,   // U - Auditing info
}

impl Default for CopyFlags {
    fn default() -> Self {
        Self::from_string("DAT")
    }
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

    /// Get all copy flags (DATSOU)
    #[allow(dead_code)]
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
    copy_file_with_metadata_internal(source, destination, flags, None)
}

/// Copy a file with specified metadata preservation, with optional warnings collector
pub fn copy_file_with_metadata_with_warnings(
    source: &Path,
    destination: &Path,
    flags: &CopyFlags,
    warnings: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) -> Result<u64> {
    copy_file_with_metadata_internal(source, destination, flags, Some(warnings))
}

/// Internal implementation for copy_file_with_metadata
fn copy_file_with_metadata_internal(
    source: &Path,
    destination: &Path,
    flags: &CopyFlags,
    warnings: Option<&std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
) -> Result<u64> {
    // Check if source is a symlink - if so, use symlink-specific handling
    let source_metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Failed to read source metadata: {}", source.display()))?;

    if source_metadata.is_symlink() {
        return copy_symlink_with_metadata(source, destination, flags);
    }

    // Always copy data if D flag is set (which it should be for file copies)
    if !flags.data {
        return Err(anyhow::anyhow!(
            "Data flag (D) must be set for file copying"
        ));
    }

    // Copy the file data
    let bytes_copied = fs::copy(source, destination).with_context(|| {
        format!(
            "Failed to copy file data: {} -> {}",
            source.display(),
            destination.display()
        )
    })?;

    // Get source metadata (now we know it's not a symlink, so we can use regular metadata)
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
        let warning = "Warning: Auditing info copying (U flag) not supported on this platform";
        if let Some(warnings) = warnings {
            if let Ok(mut w) = warnings.lock() {
                w.push(warning.to_string());
            }
        } else {
            eprintln!("{warning}");
        }
    }

    Ok(bytes_copied)
}

/// Copy a symlink with specified metadata preservation
pub fn copy_symlink_with_metadata(
    source: &Path,
    destination: &Path,
    flags: &CopyFlags,
) -> Result<u64> {
    // Read the symlink target
    let target = fs::read_link(source)
        .with_context(|| format!("Failed to read symlink target: {}", source.display()))?;

    // Create the symlink
    create_symlink_cross_platform(&target, destination)?;

    // Get source symlink metadata (using symlink_metadata to get info about the link itself)
    let source_metadata = fs::symlink_metadata(source).with_context(|| {
        format!(
            "Failed to read source symlink metadata: {}",
            source.display()
        )
    })?;

    // Apply metadata based on flags (limited for symlinks)
    // Note: Most metadata operations don't apply to symlinks or behave differently

    #[cfg(unix)]
    {
        // On Unix, we can set ownership and some timestamps for symlinks
        if flags.owner {
            copy_symlink_ownership(source, destination, &source_metadata)?;
        }

        // Timestamps for symlinks are generally not preserved as they're not very meaningful
        // and can't be set reliably across platforms
    }

    // Symlinks don't have "size" in the traditional sense, so we return 0
    Ok(0)
}

/// Create a symlink in a cross-platform way
fn create_symlink_cross_platform(target: &Path, destination: &Path) -> Result<()> {
    // Create parent directories if needed
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent directory: {}", parent.display()))?;
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(target, destination).with_context(|| {
        format!(
            "Failed to create symlink: {} -> {}",
            destination.display(),
            target.display()
        )
    })?;

    #[cfg(windows)]
    {
        // On Windows, we need to determine if the target is a directory or file
        let target_path = if target.is_absolute() {
            target.to_path_buf()
        } else if let Some(parent) = destination.parent() {
            parent.join(target)
        } else {
            target.to_path_buf()
        };

        if target_path.is_dir() {
            std::os::windows::fs::symlink_dir(target, destination).with_context(|| {
                format!(
                    "Failed to create directory symlink: {} -> {}",
                    destination.display(),
                    target.display()
                )
            })?;
        } else {
            std::os::windows::fs::symlink_file(target, destination).with_context(|| {
                format!(
                    "Failed to create file symlink: {} -> {}",
                    destination.display(),
                    target.display()
                )
            })?;
        }
    }

    Ok(())
}

/// Copy symlink ownership (Unix only)
#[cfg(unix)]
fn copy_symlink_ownership(
    _source: &Path,
    destination: &Path,
    source_metadata: &fs::Metadata,
) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::fs::MetadataExt;

    let uid = source_metadata.uid();
    let gid = source_metadata.gid();

    // Convert path to CString for lchown
    let dest_cstring = CString::new(destination.as_os_str().to_string_lossy().as_ref())
        .with_context(|| {
            format!(
                "Failed to convert path to CString: {}",
                destination.display()
            )
        })?;

    // Use lchown to change ownership of the symlink itself (not its target)
    unsafe {
        if libc::lchown(dest_cstring.as_ptr(), uid, gid) != 0 {
            return Err(anyhow::anyhow!(
                "Failed to change symlink ownership: {}",
                destination.display()
            ));
        }
    }

    Ok(())
}

/// Copy file timestamps (modification and access times)
pub fn copy_timestamps(
    _source: &Path,
    destination: &Path,
    source_metadata: &fs::Metadata,
) -> Result<()> {
    let modified = source_metadata
        .modified()
        .context("Failed to get source modification time")?;

    let accessed = source_metadata
        .accessed()
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
    _source: &Path,
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
    _source: &Path,
    destination: &Path,
    source_metadata: &fs::Metadata,
) -> Result<()> {
    use std::fs::File;
    use std::os::unix::fs::fchown;

    let uid = source_metadata.uid();
    let gid = source_metadata.gid();

    // Open destination file to get file descriptor
    let file = File::open(destination).with_context(|| {
        format!(
            "Failed to open destination for ownership change: {}",
            destination.display()
        )
    })?;

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
    let metadata =
        fs::metadata(path).context("Failed to read file metadata for timestamp update")?;
    let atime = metadata.accessed().context("Failed to get access time")?;
    let filetime_atime = filetime::FileTime::from(atime);

    // On Windows, we may need to temporarily remove readonly attribute
    #[cfg(windows)]
    {
        let permissions = metadata.permissions();
        if permissions.readonly() {
            // Temporarily remove readonly attribute
            let mut new_permissions = permissions.clone();
            new_permissions.set_readonly(false);
            fs::set_permissions(path, new_permissions)
                .context("Failed to temporarily remove readonly attribute")?;

            // Set the times
            let result = filetime::set_file_times(path, filetime_atime, filetime_mtime);

            // Restore original permissions
            fs::set_permissions(path, permissions)
                .context("Failed to restore readonly attribute")?;

            result.context("Failed to set file times")?;
            return Ok(());
        }
    }

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
