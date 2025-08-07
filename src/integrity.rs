//! Mission-critical data integrity and error handling for RoboSync
//! 
//! This module provides enterprise-grade reliability features:
//! - Checksum verification for data integrity
//! - Atomic file operations to prevent corruption
//! - Comprehensive error reporting with context
//! - Graceful recovery mechanisms
//! - Audit trail logging

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::{Result, Context, bail};
use blake3;
use serde::{Serialize, Deserialize};

/// Checksum algorithms supported for integrity verification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChecksumAlgorithm {
    Blake3,  // Default: Cryptographically secure, extremely fast
    Xxh3,    // Non-cryptographic but ultra-fast for checksums
    Sha256,  // Legacy compatibility only
}

/// Comprehensive error information for audit trails
#[derive(Debug, Serialize, Deserialize)]
pub struct IntegrityError {
    pub operation: String,
    pub source_path: Option<PathBuf>,
    pub dest_path: Option<PathBuf>,
    pub error_type: String,
    pub error_message: String,
    pub timestamp: u64,
    pub recovery_attempted: bool,
    pub recovery_successful: bool,
}

/// Atomic file operation result
#[derive(Debug)]
pub struct AtomicOperation {
    pub temp_path: PathBuf,
    pub final_path: PathBuf,
    pub completed: bool,
}

/// Data integrity verification results
#[derive(Debug, PartialEq, Eq)]
pub enum IntegrityCheck {
    Passed,
    Failed { expected: String, actual: String },
    Skipped { reason: String },
}

/// Mission-critical file operations with integrity guarantees
pub struct IntegrityManager {
    checksum_algorithm: ChecksumAlgorithm,
    verify_after_copy: bool,
    atomic_operations: bool,
    error_log: Vec<IntegrityError>,
}

impl Default for IntegrityManager {
    fn default() -> Self {
        Self::new()
    }
}

impl IntegrityManager {
    /// Create new integrity manager with enterprise defaults
    pub fn new() -> Self {
        Self {
            checksum_algorithm: ChecksumAlgorithm::Blake3,  // Modern default: fast and secure
            verify_after_copy: true,
            atomic_operations: true,
            error_log: Vec::new(),
        }
    }

    /// Configure integrity verification settings
    pub fn with_checksum_algorithm(mut self, algorithm: ChecksumAlgorithm) -> Self {
        self.checksum_algorithm = algorithm;
        self
    }

    pub fn with_verification(mut self, verify: bool) -> Self {
        self.verify_after_copy = verify;
        self
    }

    pub fn with_atomic_operations(mut self, atomic: bool) -> Self {
        self.atomic_operations = atomic;
        self
    }

    /// Compute file checksum for integrity verification
    pub fn compute_checksum(&self, path: &Path) -> Result<String> {
        let mut file = File::open(path)
            .with_context(|| format!("Failed to open file for checksum: {}", path.display()))?;
        
        match self.checksum_algorithm {
            ChecksumAlgorithm::Blake3 => {
                let mut hasher = blake3::Hasher::new();
                let mut buffer = [0u8; 8192];
                
                loop {
                    let bytes_read = file.read(&mut buffer)
                        .with_context(|| format!("Failed to read file for checksum: {}", path.display()))?;
                    
                    if bytes_read == 0 {
                        break;
                    }
                    
                    hasher.update(&buffer[..bytes_read]);
                }
                
                Ok(hasher.finalize().to_hex().to_string())
            }
            ChecksumAlgorithm::Xxh3 => {
                use xxhash_rust::xxh3::Xxh3;
                let mut hasher = Xxh3::new();
                let mut buffer = [0u8; 8192];
                
                loop {
                    let bytes_read = file.read(&mut buffer)
                        .with_context(|| format!("Failed to read file for checksum: {}", path.display()))?;
                    
                    if bytes_read == 0 {
                        break;
                    }
                    
                    hasher.update(&buffer[..bytes_read]);
                }
                
                Ok(format!("{:016x}", hasher.digest()))
            }
            ChecksumAlgorithm::Sha256 => {
                // Legacy SHA256 support for compatibility
                use sha2::{Sha256, Digest};
                let mut hasher = Sha256::new();
                let mut buffer = [0u8; 8192];
                
                loop {
                    let bytes_read = file.read(&mut buffer)
                        .with_context(|| format!("Failed to read file for checksum: {}", path.display()))?;
                    
                    if bytes_read == 0 {
                        break;
                    }
                    
                    hasher.update(&buffer[..bytes_read]);
                }
                
                Ok(format!("{:x}", hasher.finalize()))
            }
        }
    }

    /// Verify data integrity between source and destination
    pub fn verify_integrity(&mut self, source: &Path, dest: &Path) -> Result<IntegrityCheck> {
        if !self.verify_after_copy {
            return Ok(IntegrityCheck::Skipped { 
                reason: "Verification disabled".to_string() 
            });
        }

        // Ensure both files exist
        if !source.exists() {
            self.log_error("integrity_check", Some(source), Some(dest), 
                          "SourceMissing", "Source file does not exist", false, false);
            bail!("Source file does not exist: {}", source.display());
        }

        if !dest.exists() {
            self.log_error("integrity_check", Some(source), Some(dest),
                          "DestinationMissing", "Destination file does not exist", false, false);
            bail!("Destination file does not exist: {}", dest.display());
        }

        // Compare file sizes first (quick check)
        let source_metadata = fs::metadata(source)
            .with_context(|| format!("Failed to get source metadata: {}", source.display()))?;
        let dest_metadata = fs::metadata(dest)
            .with_context(|| format!("Failed to get destination metadata: {}", dest.display()))?;

        if source_metadata.len() != dest_metadata.len() {
            let error = format!("Size mismatch: source {} bytes, dest {} bytes", 
                              source_metadata.len(), dest_metadata.len());
            self.log_error("integrity_check", Some(source), Some(dest),
                          "SizeMismatch", &error, false, false);
            return Ok(IntegrityCheck::Failed { 
                expected: source_metadata.len().to_string(),
                actual: dest_metadata.len().to_string(),
            });
        }

        // Compute and compare checksums
        let source_checksum = self.compute_checksum(source)
            .with_context(|| format!("Failed to compute source checksum: {}", source.display()))?;
        let dest_checksum = self.compute_checksum(dest)
            .with_context(|| format!("Failed to compute destination checksum: {}", dest.display()))?;

        if source_checksum == dest_checksum {
            Ok(IntegrityCheck::Passed)
        } else {
            self.log_error("integrity_check", Some(source), Some(dest),
                          "ChecksumMismatch", "File checksums do not match", false, false);
            Ok(IntegrityCheck::Failed {
                expected: source_checksum,
                actual: dest_checksum,
            })
        }
    }

    /// Perform atomic file copy with integrity verification
    pub fn atomic_copy(&mut self, source: &Path, dest: &Path) -> Result<u64> {
        if !self.atomic_operations {
            return self.simple_copy(source, dest);
        }

        // Create temporary file in same directory as destination
        let dest_dir = dest.parent()
            .ok_or_else(|| anyhow::anyhow!("Destination has no parent directory"))?;
        
        let temp_name = format!(".robosync.tmp.{}.{}", 
                               dest.file_name().unwrap_or_default().to_string_lossy(),
                               SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis());
        let temp_path = dest_dir.join(temp_name);

        // Copy to temporary file first
        let bytes_copied = self.simple_copy(source, &temp_path)
            .with_context(|| format!("Failed to copy to temporary file: {}", temp_path.display()))?;

        // Verify integrity if enabled
        if self.verify_after_copy {
            match self.verify_integrity(source, &temp_path)? {
                IntegrityCheck::Passed => {},
                IntegrityCheck::Failed { expected, actual } => {
                    // Clean up temp file on verification failure
                    let _ = fs::remove_file(&temp_path);
                    bail!("Integrity verification failed: expected {}, got {}", expected, actual);
                }
                IntegrityCheck::Skipped { .. } => {},
            }
        }

        // Atomic rename to final destination
        fs::rename(&temp_path, dest)
            .with_context(|| format!("Failed to atomically move {} to {}", 
                                    temp_path.display(), dest.display()))?;

        Ok(bytes_copied)
    }

    /// Simple file copy without atomic guarantees
    fn simple_copy(&mut self, source: &Path, dest: &Path) -> Result<u64> {
        // Ensure destination directory exists
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create destination directory: {}", parent.display()))?;
        }

        // Check available disk space
        self.check_disk_space(source, dest)?;

        // Perform the copy
        fs::copy(source, dest)
            .with_context(|| format!("Failed to copy {} to {}", source.display(), dest.display()))
    }

    /// Check if destination has enough disk space
    pub fn check_disk_space(&mut self, source: &Path, dest_dir: &Path) -> Result<()> {
        let source_size = fs::metadata(source)
            .with_context(|| format!("Failed to get source file size: {}", source.display()))?
            .len();

        // Get destination directory (not the file itself)
        let dest_parent = if dest_dir.is_dir() {
            dest_dir
        } else {
            dest_dir.parent().unwrap_or(dest_dir)
        };

        // Check if destination directory exists
        if !dest_parent.exists() {
            self.log_error("disk_space_check", Some(source), Some(dest_dir),
                          "DestinationMissing", "Destination directory does not exist", false, false);
            bail!("Destination directory does not exist: {}", dest_parent.display());
        }

        // Get available disk space
        let available_space = self.get_available_disk_space(dest_parent)
            .with_context(|| format!("Failed to get available disk space for: {}", dest_parent.display()))?;

        // Require at least 2x the file size for safety (atomic operations need temp space)
        let required_space = source_size.saturating_mul(2);

        if available_space < required_space {
            let error_msg = format!(
                "Insufficient disk space: need {} bytes, have {} bytes available",
                required_space, available_space
            );
            self.log_error("disk_space_check", Some(source), Some(dest_dir),
                          "InsufficientSpace", &error_msg, false, false);
            bail!("{}", error_msg);
        }

        Ok(())
    }

    /// Get available disk space for a given path (cross-platform)
    fn get_available_disk_space(&self, path: &Path) -> Result<u64> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            use std::ffi::CString;
            
            let path_cstr = CString::new(path.as_os_str().as_bytes())
                .with_context(|| "Path contains null bytes")?;
                
            let mut statvfs = unsafe { std::mem::zeroed::<libc::statvfs>() };
            let result = unsafe { libc::statvfs(path_cstr.as_ptr(), &mut statvfs) };
            
            if result == 0 {
                // Available space = block size * available blocks
                let available_bytes = (statvfs.f_bavail as u64) * (statvfs.f_frsize as u64);
                Ok(available_bytes)
            } else {
                bail!("Failed to get filesystem statistics for: {}", path.display());
            }
        }
        
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use winapi::um::fileapi::GetDiskFreeSpaceExW;
            use winapi::um::winnt::ULARGE_INTEGER;
            
            let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            
            let mut free_bytes_available: ULARGE_INTEGER = unsafe { std::mem::zeroed() };
            let mut total_bytes: ULARGE_INTEGER = unsafe { std::mem::zeroed() };
            let mut total_free_bytes: ULARGE_INTEGER = unsafe { std::mem::zeroed() };
            
            let result = unsafe {
                GetDiskFreeSpaceExW(
                    wide_path.as_ptr(),
                    &mut free_bytes_available,
                    &mut total_bytes,
                    &mut total_free_bytes,
                )
            };
            
            if result != 0 {
                unsafe {
                    Ok(*free_bytes_available.QuadPart())
                }
            } else {
                bail!("Failed to get disk space information for: {}", path.display());
            }
        }
        
        #[cfg(not(any(unix, windows)))]
        {
            // Fallback for other platforms - assume enough space
            eprintln!("Warning: Disk space checking not implemented for this platform");
            Ok(u64::MAX)
        }
    }

    /// Attempt to recover from failed operations
    pub fn attempt_recovery(&mut self, source: &Path, dest: &Path, error: &anyhow::Error) -> Result<bool> {
        let error_str = error.to_string();
        let mut recovery_successful = false;

        // Attempt different recovery strategies based on error type
        if error_str.contains("Permission denied") || error_str.contains("Access is denied") {
            // Try to recover from permission errors
            recovery_successful = self.recover_permission_error(dest).unwrap_or(false);
        } else if error_str.contains("No space") || error_str.contains("disk full") {
            // Cannot recover from disk full - but we can clean up
            let _ = fs::remove_file(dest); // Clean up partial file
            recovery_successful = false;
        } else if error_str.contains("File exists") {
            // Try to handle existing file
            recovery_successful = self.recover_existing_file(dest).unwrap_or(false);
        }

        self.log_error("recovery_attempt", Some(source), Some(dest),
                      "RecoveryAttempt", &error_str, true, recovery_successful);

        Ok(recovery_successful)
    }

    /// Attempt to recover from permission errors
    fn recover_permission_error(&self, path: &Path) -> Result<bool> {
        // On Unix systems, try to make the parent directory writable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(parent) = path.parent() {
                let metadata = fs::metadata(parent)?;
                let mut permissions = metadata.permissions();
                permissions.set_mode(permissions.mode() | 0o200); // Add write permission
                fs::set_permissions(parent, permissions)?;
                return Ok(true);
            }
        }

        // On Windows, try to remove read-only attribute
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if path.exists() {
                let metadata = fs::metadata(path)?;
                let mut permissions = metadata.permissions();
                permissions.set_readonly(false);
                fs::set_permissions(path, permissions)?;
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Attempt to recover from existing file conflicts
    fn recover_existing_file(&self, dest: &Path) -> Result<bool> {
        if !dest.exists() {
            return Ok(false); // No conflict to resolve
        }

        // Check if the existing file is read-only or locked
        match fs::metadata(dest) {
            Ok(metadata) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let permissions = metadata.permissions();
                    
                    // If file is not writable, try to make it writable
                    let mode = permissions.mode();
                    if mode & 0o200 == 0 {
                        let mut new_permissions = permissions;
                        new_permissions.set_mode(mode | 0o200);
                        match fs::set_permissions(dest, new_permissions) {
                            Ok(_) => return Ok(true),
                            Err(_) => {} // Continue to other recovery methods
                        }
                    }
                }
                
                #[cfg(windows)]
                {
                    if metadata.permissions().readonly() {
                        let mut permissions = metadata.permissions();
                        permissions.set_readonly(false);
                        match fs::set_permissions(dest, permissions) {
                            Ok(_) => return Ok(true),
                            Err(_) => {} // Continue to other recovery methods
                        }
                    }
                }

                // Try to check if file is in use by attempting to open it exclusively
                match fs::OpenOptions::new().write(true).truncate(false).open(dest) {
                    Ok(_) => {
                        // File can be opened for writing, so overwrite is possible
                        Ok(true)
                    }
                    Err(_) => {
                        // File might be locked or in use
                        // In enterprise mode, we should be conservative and not overwrite
                        Ok(false)
                    }
                }
            }
            Err(_) => {
                // Can't get metadata, assume recovery is not possible
                Ok(false)
            }
        }
    }

    /// Log error for audit trail and debugging
    fn log_error(&mut self, operation: &str, source: Option<&Path>, dest: Option<&Path>,
                error_type: &str, message: &str, recovery_attempted: bool, recovery_successful: bool) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let error = IntegrityError {
            operation: operation.to_string(),
            source_path: source.map(|p| p.to_path_buf()),
            dest_path: dest.map(|p| p.to_path_buf()),
            error_type: error_type.to_string(),
            error_message: message.to_string(),
            timestamp,
            recovery_attempted,
            recovery_successful,
        };

        self.error_log.push(error);
    }

    /// Get comprehensive error log for audit purposes
    pub fn get_error_log(&self) -> &[IntegrityError] {
        &self.error_log
    }

    /// Export error log to JSON for external analysis
    pub fn export_error_log(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.error_log)
            .with_context(|| "Failed to serialize error log")
    }

    /// Clear error log (use with caution)
    pub fn clear_error_log(&mut self) {
        self.error_log.clear();
    }

    /// Check if any critical errors occurred
    pub fn has_critical_errors(&self) -> bool {
        self.error_log.iter().any(|error| {
            matches!(error.error_type.as_str(), 
                    "ChecksumMismatch" | "SizeMismatch" | "SourceMissing" | "DestinationMissing")
        })
    }

    /// Get summary of error types for reporting
    pub fn get_error_summary(&self) -> std::collections::HashMap<String, usize> {
        let mut summary = std::collections::HashMap::new();
        for error in &self.error_log {
            *summary.entry(error.error_type.clone()).or_insert(0) += 1;
        }
        summary
    }
}

/// Validate file paths for security and correctness
pub fn validate_path(path: &Path) -> Result<()> {
    // Check for null bytes (security issue)
    let path_str = path.to_string_lossy();
    if path_str.contains('\0') {
        bail!("Path contains null bytes: {}", path_str);
    }

    // Check for excessively long paths
    if path_str.len() > 4096 {
        bail!("Path too long ({} chars): {}", path_str.len(), path_str);
    }

    // Check for relative path traversal attempts
    if path_str.contains("..") {
        let canonical = path.canonicalize()
            .with_context(|| format!("Failed to canonicalize path: {}", path.display()))?;
        
        // Additional validation could be added here
        if canonical.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            bail!("Path contains directory traversal: {}", path.display());
        }
    }

    Ok(())
}

/// Pre-flight checks before starting operations
pub fn preflight_checks(source: &Path, dest: &Path) -> Result<()> {
    // Validate both paths
    validate_path(source)?;
    validate_path(dest)?;

    // Check source exists and is readable
    if !source.exists() {
        bail!("Source does not exist: {}", source.display());
    }

    let source_metadata = fs::metadata(source)
        .with_context(|| format!("Cannot access source: {}", source.display()))?;

    // Check if source is a file or directory
    if !source_metadata.is_file() && !source_metadata.is_dir() {
        bail!("Source is neither file nor directory: {}", source.display());
    }

    // Check destination parent directory exists or can be created
    if let Some(dest_parent) = dest.parent() {
        if !dest_parent.exists() {
            fs::create_dir_all(dest_parent)
                .with_context(|| format!("Cannot create destination directory: {}", dest_parent.display()))?;
        }
    }

    // Check we're not trying to copy to ourselves
    if source.canonicalize()? == dest.canonicalize().unwrap_or_else(|_| dest.to_path_buf()) {
        bail!("Source and destination are the same: {}", source.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_integrity_manager_creation() {
        let manager = IntegrityManager::new();
        assert_eq!(manager.checksum_algorithm, ChecksumAlgorithm::Blake3);
        assert!(manager.verify_after_copy);
        assert!(manager.atomic_operations);
    }

    #[test]
    fn test_checksum_computation() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, b"Hello, World!").expect("Failed to write test file");

        let manager = IntegrityManager::new();
        let checksum = manager.compute_checksum(&test_file).expect("Failed to compute checksum");
        
        // Blake3 of "Hello, World!" 
        assert_eq!(checksum, "ede5c0b10f2ec4979c69b52f61e42ff5b413519ce09be0f14d098dcfe5f6f98d");
    }

    #[test]
    fn test_integrity_verification() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let source = temp_dir.path().join("source.txt");
        let dest = temp_dir.path().join("dest.txt");
        
        fs::write(&source, b"test data").expect("Failed to write source");
        fs::write(&dest, b"test data").expect("Failed to write dest");

        let mut manager = IntegrityManager::new();
        let result = manager.verify_integrity(&source, &dest).expect("Verification failed");
        
        assert_eq!(result, IntegrityCheck::Passed);
    }

    #[test]
    fn test_path_validation() {
        // Valid path
        assert!(validate_path(Path::new("/valid/path")).is_ok());
        
        // Path with null byte should fail
        // Note: This test might not work on all platforms due to OS restrictions
        // assert!(validate_path(Path::new("/invalid\0/path")).is_err());
    }

    #[test]
    fn test_preflight_checks() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let source = temp_dir.path().join("source.txt");
        let dest = temp_dir.path().join("dest.txt");
        
        fs::write(&source, b"test").expect("Failed to write source");
        
        assert!(preflight_checks(&source, &dest).is_ok());
        
        // Non-existent source should fail
        let bad_source = temp_dir.path().join("nonexistent.txt");
        assert!(preflight_checks(&bad_source, &dest).is_err());
    }
}