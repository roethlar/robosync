//! Mission-critical operations for enterprise reliability
//! 
//! This module integrates all safety features for production use:
//! - Data integrity verification
//! - Atomic operations
//! - Comprehensive error handling and recovery
//! - Audit trail logging
//! - Disk space validation

use std::path::{Path, PathBuf};
use std::fs;
use std::time::SystemTime;
use anyhow::{Result, Context, bail};
use serde::{Serialize, Deserialize};

use crate::integrity::{IntegrityManager, IntegrityCheck, ChecksumAlgorithm};
use crate::sync_stats::SyncStats;

/// Mission-critical file synchronization operations
pub struct MissionCriticalSync {
    integrity_manager: IntegrityManager,
    verify_integrity: bool,
    atomic_operations: bool,
    require_checksums: bool,
    max_retry_attempts: u32,
    audit_log: Vec<AuditEntry>,
}

/// Audit log entry for compliance and debugging
#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: SystemTime,
    pub operation: String,
    pub source: Option<PathBuf>,
    pub destination: Option<PathBuf>,
    pub status: OperationStatus,
    pub bytes_transferred: u64,
    pub checksum_verified: bool,
    pub retry_count: u32,
    pub error_message: Option<String>,
}

/// Operation status for audit trail
#[derive(Debug, Serialize, Deserialize)]
pub enum OperationStatus {
    Success,
    Failed,
    Retried,
    Skipped,
    PartialSuccess,
}

/// Configuration for mission-critical operations
#[derive(Debug, Clone)]
pub struct MissionCriticalConfig {
    pub verify_integrity: bool,
    pub atomic_operations: bool,
    pub require_checksums: bool,
    pub checksum_algorithm: ChecksumAlgorithm,
    pub max_retry_attempts: u32,
    pub minimum_free_space_mb: u64,
}

impl Default for MissionCriticalConfig {
    fn default() -> Self {
        Self {
            verify_integrity: true,
            atomic_operations: true,
            require_checksums: true,
            checksum_algorithm: ChecksumAlgorithm::Sha256,
            max_retry_attempts: 3,
            minimum_free_space_mb: 1024, // 1GB minimum free space
        }
    }
}

impl MissionCriticalSync {
    /// Create new mission-critical sync with enterprise defaults
    pub fn new(config: MissionCriticalConfig) -> Self {
        let integrity_manager = IntegrityManager::new()
            .with_checksum_algorithm(config.checksum_algorithm)
            .with_verification(config.verify_integrity)
            .with_atomic_operations(config.atomic_operations);

        Self {
            integrity_manager,
            verify_integrity: config.verify_integrity,
            atomic_operations: config.atomic_operations,
            require_checksums: config.require_checksums,
            max_retry_attempts: config.max_retry_attempts,
            audit_log: Vec::new(),
        }
    }

    /// Perform enterprise-grade file copy with all safety features
    pub fn copy_file_enterprise(&mut self, source: &Path, dest: &Path) -> Result<u64> {
        let start_time = SystemTime::now();
        let mut retry_count = 0;
        let mut last_error: Option<anyhow::Error>;

        // Pre-flight validation
        self.validate_operation(source, dest)?;

        loop {
            match self.attempt_copy(source, dest) {
                Ok(bytes_transferred) => {
                    // Log successful operation
                    self.audit_log.push(AuditEntry {
                        timestamp: start_time,
                        operation: "copy_file".to_string(),
                        source: Some(source.to_path_buf()),
                        destination: Some(dest.to_path_buf()),
                        status: OperationStatus::Success,
                        bytes_transferred,
                        checksum_verified: self.verify_integrity,
                        retry_count,
                        error_message: None,
                    });

                    return Ok(bytes_transferred);
                }
                Err(error) => {
                    last_error = Some(error);
                    retry_count += 1;

                    if retry_count >= self.max_retry_attempts {
                        break;
                    }

                    // Log retry attempt
                    self.audit_log.push(AuditEntry {
                        timestamp: SystemTime::now(),
                        operation: "copy_file_retry".to_string(),
                        source: Some(source.to_path_buf()),
                        destination: Some(dest.to_path_buf()),
                        status: OperationStatus::Retried,
                        bytes_transferred: 0,
                        checksum_verified: false,
                        retry_count,
                        error_message: Some(last_error.as_ref().unwrap().to_string()),
                    });

                    // Wait before retry (exponential backoff)
                    let wait_ms = 100 * (1 << retry_count.min(5)); // Cap at ~3 seconds
                    std::thread::sleep(std::time::Duration::from_millis(wait_ms));

                    // Attempt recovery
                    if let Some(error) = last_error.as_ref() {
                        let _ = self.integrity_manager.attempt_recovery(source, dest, error);
                    }
                }
            }
        }

        // Log final failure
        let error = last_error.unwrap();
        self.audit_log.push(AuditEntry {
            timestamp: start_time,
            operation: "copy_file".to_string(),
            source: Some(source.to_path_buf()),
            destination: Some(dest.to_path_buf()),
            status: OperationStatus::Failed,
            bytes_transferred: 0,
            checksum_verified: false,
            retry_count,
            error_message: Some(error.to_string()),
        });

        Err(error).with_context(|| {
            format!("Failed to copy {} to {} after {} attempts", 
                   source.display(), dest.display(), retry_count)
        })
    }

    /// Validate operation before attempting
    fn validate_operation(&mut self, source: &Path, dest: &Path) -> Result<()> {
        // Validate paths
        crate::integrity::validate_path(source)?;
        crate::integrity::validate_path(dest)?;

        // Pre-flight checks
        crate::integrity::preflight_checks(source, dest)?;

        // Check disk space
        self.integrity_manager.check_disk_space(source, dest)?;

        Ok(())
    }

    /// Single copy attempt with all safety features
    fn attempt_copy(&mut self, source: &Path, dest: &Path) -> Result<u64> {
        // Use atomic copy if enabled
        let bytes_transferred = if self.atomic_operations {
            self.integrity_manager.atomic_copy(source, dest)?
        } else {
            self.simple_copy(source, dest)?
        };

        // Verify integrity if required
        if self.verify_integrity {
            match self.integrity_manager.verify_integrity(source, dest)? {
                IntegrityCheck::Passed => {},
                IntegrityCheck::Failed { expected, actual } => {
                    // Clean up corrupted file
                    let _ = fs::remove_file(dest);
                    bail!("Data integrity verification failed: expected {}, got {}", expected, actual);
                }
                IntegrityCheck::Skipped { reason } => {
                    if self.require_checksums {
                        bail!("Checksum verification was skipped but is required: {}", reason);
                    }
                }
            }
        }

        Ok(bytes_transferred)
    }

    /// Simple copy operation (fallback)
    fn simple_copy(&self, source: &Path, dest: &Path) -> Result<u64> {
        // Ensure destination directory exists
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create destination directory: {}", parent.display()))?;
        }

        // Perform the copy
        fs::copy(source, dest)
            .with_context(|| format!("Failed to copy {} to {}", source.display(), dest.display()))
    }

    /// Copy directory with enterprise reliability
    pub fn copy_directory_enterprise(&mut self, source: &Path, dest: &Path) -> Result<SyncStats> {
        let mut stats = SyncStats::default();
        let start_time = std::time::Instant::now();

        // Validate directory operation
        if !source.is_dir() {
            bail!("Source is not a directory: {}", source.display());
        }

        // Create destination directory
        fs::create_dir_all(dest)
            .with_context(|| format!("Failed to create destination directory: {}", dest.display()))?;

        // Recursively copy contents
        self.copy_directory_contents(source, dest, &mut stats)?;

        stats.elapsed_time = start_time.elapsed();
        Ok(stats)
    }

    /// Recursively copy directory contents
    fn copy_directory_contents(&mut self, source: &Path, dest: &Path, stats: &mut SyncStats) -> Result<()> {
        for entry in fs::read_dir(source)
            .with_context(|| format!("Failed to read directory: {}", source.display()))? {
            
            let entry = entry.with_context(|| format!("Failed to read directory entry in: {}", source.display()))?;
            let src_path = entry.path();
            let file_name = entry.file_name();
            let dest_path = dest.join(file_name);

            if src_path.is_dir() {
                // Create directory and recurse
                fs::create_dir_all(&dest_path)
                    .with_context(|| format!("Failed to create directory: {}", dest_path.display()))?;
                self.copy_directory_contents(&src_path, &dest_path, stats)?;
            } else if src_path.is_file() {
                // Copy file with enterprise reliability
                match self.copy_file_enterprise(&src_path, &dest_path) {
                    Ok(bytes) => {
                        stats.increment_files_copied();
                        stats.add_bytes_transferred(bytes);
                    }
                    Err(e) => {
                        eprintln!("Error copying {:?}: {}", src_path, e);
                        stats.increment_errors();
                    }
                }
            }
            // Skip symlinks and special files in enterprise mode for safety
        }

        Ok(())
    }

    /// Get comprehensive audit log
    pub fn get_audit_log(&self) -> &[AuditEntry] {
        &self.audit_log
    }

    /// Export audit log to JSON for compliance
    pub fn export_audit_log(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.audit_log)
            .with_context(|| "Failed to serialize audit log")
    }

    /// Get operation statistics for reporting
    pub fn get_operation_stats(&self) -> OperationStats {
        let mut stats = OperationStats::default();
        
        for entry in &self.audit_log {
            match entry.status {
                OperationStatus::Success => stats.successful_operations += 1,
                OperationStatus::Failed => stats.failed_operations += 1,
                OperationStatus::Retried => stats.retry_attempts += 1,
                OperationStatus::Skipped => stats.skipped_operations += 1,
                OperationStatus::PartialSuccess => stats.partial_success_operations += 1,
            }
            
            stats.total_bytes_transferred += entry.bytes_transferred;
            
            if entry.checksum_verified {
                stats.integrity_verifications += 1;
            }
        }
        
        stats
    }

    /// Check if system is in good state for operations
    pub fn system_health_check(&self) -> Result<HealthStatus> {
        let mut status = HealthStatus {
            overall_status: "healthy".to_string(),
            issues: Vec::new(),
            recommendations: Vec::new(),
        };

        // Check for critical errors in integrity manager
        if self.integrity_manager.has_critical_errors() {
            status.overall_status = "degraded".to_string();
            status.issues.push("Critical integrity errors detected".to_string());
            status.recommendations.push("Review integrity error log and resolve data corruption issues".to_string());
        }

        // Check error rate
        let total_ops = self.audit_log.len();
        let failed_ops = self.audit_log.iter().filter(|e| matches!(e.status, OperationStatus::Failed)).count();
        
        if total_ops > 0 {
            let failure_rate = (failed_ops as f64) / (total_ops as f64) * 100.0;
            if failure_rate > 10.0 {
                status.overall_status = "unhealthy".to_string();
                status.issues.push(format!("High failure rate: {:.1}%", failure_rate));
                status.recommendations.push("Investigate and resolve recurring errors".to_string());
            } else if failure_rate > 5.0 {
                status.overall_status = "degraded".to_string();
                status.issues.push(format!("Elevated failure rate: {:.1}%", failure_rate));
            }
        }

        Ok(status)
    }

    /// Clear audit log (use with caution for compliance)
    pub fn clear_audit_log(&mut self) {
        self.audit_log.clear();
        self.integrity_manager.clear_error_log();
    }
}

/// Operation statistics for monitoring and reporting
#[derive(Debug, Default)]
pub struct OperationStats {
    pub successful_operations: usize,
    pub failed_operations: usize,
    pub retry_attempts: usize,
    pub skipped_operations: usize,
    pub partial_success_operations: usize,
    pub total_bytes_transferred: u64,
    pub integrity_verifications: usize,
}

/// System health status for monitoring
#[derive(Debug)]
pub struct HealthStatus {
    pub overall_status: String,
    pub issues: Vec<String>,
    pub recommendations: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

    #[test]
    fn test_mission_critical_sync_creation() {
        let config = MissionCriticalConfig::default();
        let sync = MissionCriticalSync::new(config);
        assert!(sync.verify_integrity);
        assert!(sync.atomic_operations);
        assert!(sync.require_checksums);
    }

    #[test]
    fn test_enterprise_file_copy() {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let source = temp_dir.path().join("source.txt");
        let dest = temp_dir.path().join("dest.txt");
        
        fs::write(&source, b"test data").expect("Failed to write source");
        
        let config = MissionCriticalConfig::default();
        let mut sync = MissionCriticalSync::new(config);
        
        let bytes = sync.copy_file_enterprise(&source, &dest).expect("Copy failed");
        assert_eq!(bytes, 9);
        assert_eq!(fs::read(&dest).expect("Failed to read dest"), b"test data");
        
        // Check audit log
        let audit_log = sync.get_audit_log();
        assert_eq!(audit_log.len(), 1);
        assert!(matches!(audit_log[0].status, OperationStatus::Success));
    }

    #[test]
    fn test_health_check() {
        let config = MissionCriticalConfig::default();
        let sync = MissionCriticalSync::new(config);
        
        let health = sync.system_health_check().expect("Health check failed");
        assert_eq!(health.overall_status, "healthy");
        assert!(health.issues.is_empty());
    }
}