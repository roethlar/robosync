//! File metadata handling for copy operations

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use crate::error_report::ErrorReportHandle;
use std::sync::atomic::{AtomicUsize, Ordering};

// Global counters for metadata warnings
static METADATA_WARNING_COUNT: AtomicUsize = AtomicUsize::new(0);

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
    /// Automatically filters out unsupported flags based on the current platform
    pub fn from_string(flags: &str) -> Self {
        let flags_upper = flags.to_uppercase();
        Self {
            data: flags_upper.contains('D'),
            attributes: flags_upper.contains('A'),
            timestamps: flags_upper.contains('T'),
            security: flags_upper.contains('S'),
            owner: flags_upper.contains('O'),
            // Auditing (U flag) is not supported on any current platform, so always false
            auditing: false,
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
    copy_file_with_metadata_and_reporter(source, destination, flags, None)
}

/// Copy a file with specified metadata preservation and error reporting
pub fn copy_file_with_metadata_and_reporter(
    source: &Path,
    destination: &Path,
    flags: &CopyFlags,
    error_reporter: Option<&ErrorReportHandle>,
) -> Result<u64> {
    // Check if source is a symlink first
    let source_metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Failed to read source metadata: {}", source.display()))?;
    
    if source_metadata.is_symlink() {
        return copy_symlink_with_metadata(source, destination, flags);
    }
    
    // First, let's copy the file data - this is the critical part
    let bytes_copied = if flags.data {
        streaming_copy_optimized(source, destination)?
    } else {
        return Err(anyhow::anyhow!("Data flag (D) must be set for file copying"));
    };
    
    // Now apply metadata - these are less critical and some may fail due to permissions
    let metadata = fs::metadata(source)
        .with_context(|| format!("Failed to read source metadata: {}", source.display()))?;
    
    // Apply each type of metadata, but don't fail the whole operation for non-critical errors
    if flags.timestamps {
        if let Err(e) = copy_timestamps(source, destination, &metadata) {
            if error_reporter.is_none() {
                METADATA_WARNING_COUNT.fetch_add(1, Ordering::Relaxed);
            } else {
                let msg = format!("Failed to preserve timestamps: {}", e);
                error_reporter.unwrap().add_warning(destination, &msg);
            }
        }
    }
    
    if flags.security {
        if let Err(e) = copy_permissions(source, destination, &metadata) {
            if error_reporter.is_none() {
                METADATA_WARNING_COUNT.fetch_add(1, Ordering::Relaxed);
            } else {
                let msg = format!("Failed to preserve permissions: {}", e);
                error_reporter.unwrap().add_warning(destination, &msg);
            }
        }
    }
    
    if flags.attributes {
        if let Err(e) = copy_attributes(source, destination, &metadata) {
            if error_reporter.is_none() {
                METADATA_WARNING_COUNT.fetch_add(1, Ordering::Relaxed);
            } else {
                let msg = format!("Failed to preserve attributes: {}", e);
                error_reporter.unwrap().add_warning(destination, &msg);
            }
        }
    }
    
    #[cfg(unix)]
    if flags.owner {
        if let Err(e) = copy_ownership(source, destination, &metadata) {
            if error_reporter.is_none() {
                METADATA_WARNING_COUNT.fetch_add(1, Ordering::Relaxed);
            } else {
                let msg = format!("Failed to preserve ownership: {} (requires appropriate privileges)", e);
                error_reporter.unwrap().add_warning(destination, &msg);
            }
        }
    }
    
    Ok(bytes_copied)
}

/// Get the current metadata warning count and reset it
pub fn get_and_reset_metadata_warning_count() -> usize {
    METADATA_WARNING_COUNT.swap(0, Ordering::Relaxed)
}

/// Fast copy that only copies data without metadata for maximum performance
pub fn copy_file_data_only(source: &Path, destination: &Path) -> Result<u64> {
    streaming_copy_optimized(source, destination)
}

/// Copy a file with specified metadata preservation, with optional warnings collector
pub fn copy_file_with_metadata_with_warnings(
    source: &Path,
    destination: &Path,
    flags: &CopyFlags,
    _warnings: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) -> Result<u64> {
    // Just use the regular function which now handles warnings internally
    copy_file_with_metadata(source, destination, flags)
}

// Internal implementation removed - logic moved to public function

/// Copy a symlink with specified metadata preservation
pub fn copy_symlink_with_metadata(
    source: &Path,
    destination: &Path,
    #[cfg_attr(not(unix), allow(unused_variables))] flags: &CopyFlags,
) -> Result<u64> {
    // Read the symlink target
    let target = fs::read_link(source)
        .with_context(|| format!("Failed to read symlink target: {}", source.display()))?;

    // Create the symlink
    create_symlink_cross_platform(&target, destination)?;

    // Get source symlink metadata (using symlink_metadata to get info about the link itself)
    #[cfg_attr(not(unix), allow(unused_variables))]
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
            // Try to copy ownership, but don't fail if it doesn't work (requires privileges)
            let _ = copy_symlink_ownership(source, destination, &source_metadata);
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
            #[allow(clippy::permissions_set_readonly_false)]
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

/// Filesystem type information
#[derive(Debug, Clone, PartialEq)]
pub enum FilesystemType {
    Local,
    Network,
    Tmpfs,
    Unknown,
}

/// Filesystem capability information
#[derive(Debug, Clone)]
pub struct FilesystemCapabilities {
    pub supports_ownership: bool,
    pub supports_permissions: bool,
    pub supports_timestamps: bool,
    pub supports_extended_attributes: bool,
    pub filesystem_type: FilesystemType,
}

impl Default for FilesystemCapabilities {
    fn default() -> Self {
        Self {
            supports_ownership: true,
            supports_permissions: true,
            supports_timestamps: true,
            supports_extended_attributes: true,
            filesystem_type: FilesystemType::Local,
        }
    }
}

/// Detect filesystem type and capabilities for a given path
pub fn detect_filesystem_capabilities(path: &Path) -> Result<FilesystemCapabilities> {
    // First try to get filesystem information using statfs (Unix) or GetVolumeInformation (Windows)
    #[cfg(unix)]
    {
        detect_unix_filesystem_capabilities(path)
    }
    
    #[cfg(windows)]
    {
        detect_windows_filesystem_capabilities(path)
    }
}

/// Detect filesystem capabilities on Unix systems
#[cfg(unix)]
fn detect_unix_filesystem_capabilities(path: &Path) -> Result<FilesystemCapabilities> {
    // Try to get the mount point for this path
    let mount_info = get_mount_info(path)?;
    
    let mut caps = FilesystemCapabilities::default();
    caps.filesystem_type = classify_unix_filesystem(&mount_info.fstype, &mount_info.mount_point);
    
    // Set capabilities based on filesystem type
    match mount_info.fstype.as_str() {
        // Network filesystems - limited capabilities
        "nfs" | "nfs4" | "cifs" | "smb" | "smbfs" | "fuse.sshfs" => {
            caps.supports_ownership = false; // Usually fails due to uid mapping
            caps.supports_extended_attributes = false;
            caps.filesystem_type = FilesystemType::Network;
        },
        // Temporary filesystems
        "tmpfs" | "ramfs" => {
            caps.filesystem_type = FilesystemType::Tmpfs;
        },
        // FAT filesystems - very limited
        "vfat" | "fat32" | "msdos" => {
            caps.supports_ownership = false;
            caps.supports_permissions = false; // Only basic read-only attribute
            caps.supports_extended_attributes = false;
        },
        // NTFS via ntfs-3g - mixed capabilities
        "ntfs" | "fuseblk" => {
            caps.supports_ownership = false; // Usually mapped to mount user
            caps.supports_extended_attributes = false;
        },
        // Full-featured filesystems (ext4, xfs, btrfs, zfs)
        _ => {
            // Keep default full capabilities
        }
    }
    
    Ok(caps)
}

/// Get mount information for a path on Unix
#[cfg(unix)]
fn get_mount_info(path: &Path) -> Result<MountInfo> {
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    
    let canonical_path = std::fs::canonicalize(path)
        .with_context(|| format!("Failed to canonicalize path: {}", path.display()))?;
    
    let file = File::open("/proc/mounts")
        .context("Failed to open /proc/mounts")?;
    let reader = BufReader::new(file);
    
    let mut best_match = MountInfo {
        mount_point: "/".to_string(),
        fstype: "unknown".to_string(),
    };
    let mut best_match_len = 0;
    
    for line in reader.lines() {
        let line = line.context("Failed to read line from /proc/mounts")?;
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            let mount_point = parts[1];
            let fstype = parts[2];
            
            // Find the longest matching mount point
            if canonical_path.starts_with(mount_point) && mount_point.len() > best_match_len {
                best_match = MountInfo {
                    mount_point: mount_point.to_string(),
                    fstype: fstype.to_string(),
                };
                best_match_len = mount_point.len();
            }
        }
    }
    
    Ok(best_match)
}

#[cfg(unix)]
struct MountInfo {
    mount_point: String,
    fstype: String,
}

/// Classify Unix filesystem type
#[cfg(unix)]
fn classify_unix_filesystem(fstype: &str, mount_point: &str) -> FilesystemType {
    match fstype {
        "nfs" | "nfs4" | "cifs" | "smb" | "smbfs" | "fuse.sshfs" => FilesystemType::Network,
        "tmpfs" | "ramfs" => FilesystemType::Tmpfs,
        _ => {
            // Also check mount point patterns for network mounts
            if mount_point.starts_with("/mnt/") || mount_point.starts_with("/media/") 
                || mount_point.starts_with("/net/") || mount_point.starts_with("/smb/") {
                FilesystemType::Network
            } else {
                FilesystemType::Local
            }
        }
    }
}

/// Detect filesystem capabilities on Windows
#[cfg(windows)]
fn detect_windows_filesystem_capabilities(path: &Path) -> Result<FilesystemCapabilities> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    
    let mut caps = FilesystemCapabilities::default();
    
    // Check if it's a UNC path (network)
    if let Some(path_str) = path.to_str() {
        if path_str.starts_with("\\\\") {
            caps.filesystem_type = FilesystemType::Network;
            caps.supports_ownership = false; // Usually fails on network shares
            caps.supports_extended_attributes = false;
            return Ok(caps);
        }
    }
    
    // For local paths, try to get volume information
    let root_path = get_volume_root(path)?;
    let root_wide: Vec<u16> = OsStr::new(&root_path).encode_wide().chain(std::iter::once(0)).collect();
    
    let mut fs_name = [0u16; 256];
    let mut volume_flags = 0u32;
    
    unsafe {
        let success = winapi::um::fileapi::GetVolumeInformationW(
            root_wide.as_ptr(),
            std::ptr::null_mut(), 0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut volume_flags,
            fs_name.as_mut_ptr(), fs_name.len() as u32,
        );
        
        if success != 0 {
            let fs_name_str = String::from_utf16_lossy(&fs_name);
            let fs_name_clean = fs_name_str.trim_end_matches('\0').to_lowercase();
            
            match fs_name_clean.as_str() {
                "fat" | "fat32" | "exfat" => {
                    caps.supports_ownership = false;
                    caps.supports_permissions = false;
                    caps.supports_extended_attributes = false;
                },
                _ => {
                    // NTFS and other filesystems usually support most features
                }
            }
        }
    }
    
    Ok(caps)
}

#[cfg(windows)]
fn get_volume_root(path: &Path) -> Result<String> {
    let path_str = path.to_str().context("Invalid path encoding")?;
    
    if path_str.len() >= 2 && path_str.chars().nth(1) == Some(':') {
        Ok(format!("{}:\\", path_str.chars().nth(0).unwrap().to_uppercase()))
    } else {
        Ok("\\".to_string())
    }
}

/// Filter copy flags based on filesystem capabilities
pub fn filter_copy_flags_for_filesystem(flags: &CopyFlags, caps: &FilesystemCapabilities) -> CopyFlags {
    CopyFlags {
        data: flags.data, // Always preserve data
        attributes: flags.attributes && caps.supports_extended_attributes,
        timestamps: flags.timestamps && caps.supports_timestamps,
        security: flags.security && caps.supports_permissions,
        owner: flags.owner && caps.supports_ownership,
        auditing: false, // Always filtered out
    }
}

/// Check if a path is a network path (UNC path on Windows or mounted network drive)
pub fn is_network_path(path: &Path) -> bool {
    match detect_filesystem_capabilities(path) {
        Ok(caps) => caps.filesystem_type == FilesystemType::Network,
        Err(_) => {
            // Fallback to simple heuristics
            #[cfg(windows)]
            {
                if let Some(path_str) = path.to_str() {
                    return path_str.starts_with("\\\\");
                }
            }
            
            #[cfg(unix)]
            {
                if let Some(path_str) = path.to_str() {
                    return path_str.starts_with("/mnt/") || path_str.starts_with("/media/") 
                        || path_str.starts_with("/net/") || path_str.starts_with("/smb/");
                }
            }
            
            false
        }
    }
}

/// Optimized streaming copy for network transfers
fn streaming_copy_optimized(source: &Path, destination: &Path) -> Result<u64> {
    // For Windows, use native APIs for maximum performance
    #[cfg(windows)]
    {
        // Try to use Windows native copy first for optimal performance
        match windows_native_copy(source, destination) {
            Ok(bytes) => return Ok(bytes),
            Err(_) => {
                // Fall back to standard copy if native fails
            }
        }
    }
    
    // Use unbuffered I/O for better performance on large files
    use std::fs::File;
    use std::io::{Read, Write};
    
    // Use much larger buffer for network transfers (32MB)
    const NETWORK_BUFFER_SIZE: usize = 32 * 1024 * 1024;
    
    let mut source_file = File::open(source)
        .with_context(|| format!("Failed to open source file: {}", source.display()))?;
    let mut dest_file = File::create(destination)
        .with_context(|| format!("Failed to create destination file: {}", destination.display()))?;
    
    // Pre-allocate destination file for better performance
    if let Ok(metadata) = source_file.metadata() {
        let _ = dest_file.set_len(metadata.len());
    }
    
    // Direct I/O without buffering for maximum throughput
    let mut buffer = vec![0u8; NETWORK_BUFFER_SIZE];
    let mut total_bytes = 0u64;
    
    loop {
        let bytes_read = source_file.read(&mut buffer)
            .with_context(|| format!("Failed to read from source: {}", source.display()))?;
        
        if bytes_read == 0 {
            break;
        }
        
        dest_file.write_all(&buffer[..bytes_read])
            .with_context(|| format!("Failed to write to destination: {}", destination.display()))?;
        
        total_bytes += bytes_read as u64;
    }
    
    dest_file.sync_all()
        .with_context(|| format!("Failed to sync destination: {}", destination.display()))?;
    
    Ok(total_bytes)
}

#[cfg(windows)]
fn windows_native_copy(source: &Path, destination: &Path) -> Result<u64> {
    // Use standard fs::copy which on Windows uses CopyFileW internally
    fs::copy(source, destination)
        .with_context(|| format!("Failed to copy file: {} -> {}", source.display(), destination.display()))
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
        assert!(!all_flags.auditing); // Auditing is always false (unsupported)
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
        assert!(!flags.auditing); // Auditing is always false (unsupported)
    }
}
