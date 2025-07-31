//! Comprehensive error handling for RoboSync
//!
//! This module provides structured error types that replace the generic anyhow::Result
//! with specific, actionable error information for library consumers.

use std::io;
use std::path::PathBuf;
use thiserror::Error;

/// Main RoboSync error type that encompasses all possible failure modes
#[derive(Error, Debug)]
pub enum RoboSyncError {
    /// File system I/O errors
    #[error("I/O error: {message}")]
    Io {
        message: String,
        #[source]
        source: io::Error,
        path: Option<PathBuf>,
    },

    /// File access permission errors
    #[error("Permission denied accessing {path}")]
    Permission {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// File not found errors
    #[error("File or directory not found: {path}")]
    NotFound { path: PathBuf },

    /// File synchronization specific errors
    #[error("Synchronization failed: {reason}")]
    SyncFailed {
        reason: String,
        source_path: Option<PathBuf>,
        dest_path: Option<PathBuf>,
    },

    /// Delta algorithm errors
    #[error("Delta transfer failed: {message}")]
    DeltaFailed { message: String, file_path: PathBuf },

    /// Compression/decompression errors
    #[error("Compression error: {operation} failed")]
    Compression {
        operation: String, // "compress" or "decompress"
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Checksum validation errors
    #[error("Checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    /// Strategy selection errors
    #[error("Strategy selection failed: {reason}")]
    StrategyError {
        reason: String,
        attempted_strategy: Option<String>,
    },

    /// Thread pool or parallel processing errors
    #[error("Parallel processing error: {message}")]
    ParallelError {
        message: String,
        thread_count: Option<usize>,
    },

    /// Configuration and validation errors
    #[error("Configuration error: {field} is invalid")]
    ConfigError {
        field: String,
        value: String,
        reason: String,
    },

    /// Network-related errors for remote sync operations
    #[error("Network error during sync: {message}")]
    Network {
        message: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Retry exhaustion errors
    #[error("Operation failed after {attempts} retries: {last_error}")]
    RetryExhausted {
        attempts: u32,
        last_error: String,
        operation: String,
    },

    /// Pattern export/import errors for AI training
    #[error("Pattern processing error: {operation} failed")]
    PatternError {
        operation: String, // "export", "import", "parse", etc.
        path: Option<PathBuf>,
        reason: String,
    },

    /// Generic operation errors for edge cases
    #[error("Operation failed: {operation} - {message}")]
    OperationFailed { operation: String, message: String },
}

impl RoboSyncError {
    /// Create an I/O error with optional path context
    pub fn io_error(source: io::Error, path: Option<PathBuf>) -> Self {
        Self::Io {
            message: source.to_string(),
            source,
            path,
        }
    }

    /// Create a permission error with path context
    pub fn permission_error(source: io::Error, path: PathBuf) -> Self {
        Self::Permission { path, source }
    }

    /// Create a file not found error
    pub fn not_found(path: PathBuf) -> Self {
        Self::NotFound { path }
    }

    /// Create a sync failure error
    pub fn sync_failed(
        reason: impl Into<String>,
        source_path: Option<PathBuf>,
        dest_path: Option<PathBuf>,
    ) -> Self {
        Self::SyncFailed {
            reason: reason.into(),
            source_path,
            dest_path,
        }
    }

    /// Create a delta transfer error
    pub fn delta_failed(message: impl Into<String>, file_path: PathBuf) -> Self {
        Self::DeltaFailed {
            message: message.into(),
            file_path,
        }
    }

    /// Create a compression error
    pub fn compression_error(
        operation: impl Into<String>,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self::Compression {
            operation: operation.into(),
            source,
        }
    }

    /// Create a checksum mismatch error
    pub fn checksum_mismatch(
        path: PathBuf,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self::ChecksumMismatch {
            path,
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// Create a strategy selection error
    pub fn strategy_error(reason: impl Into<String>, attempted_strategy: Option<String>) -> Self {
        Self::StrategyError {
            reason: reason.into(),
            attempted_strategy,
        }
    }

    /// Create a parallel processing error
    pub fn parallel_error(message: impl Into<String>, thread_count: Option<usize>) -> Self {
        Self::ParallelError {
            message: message.into(),
            thread_count,
        }
    }

    /// Create a configuration error
    pub fn config_error(
        field: impl Into<String>,
        value: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::ConfigError {
            field: field.into(),
            value: value.into(),
            reason: reason.into(),
        }
    }

    /// Create a network error
    pub fn network_error(
        message: impl Into<String>,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self::Network {
            message: message.into(),
            source,
        }
    }

    /// Create a retry exhaustion error
    pub fn retry_exhausted(
        attempts: u32,
        last_error: impl Into<String>,
        operation: impl Into<String>,
    ) -> Self {
        Self::RetryExhausted {
            attempts,
            last_error: last_error.into(),
            operation: operation.into(),
        }
    }

    /// Create a pattern processing error
    pub fn pattern_error(
        operation: impl Into<String>,
        path: Option<PathBuf>,
        reason: impl Into<String>,
    ) -> Self {
        Self::PatternError {
            operation: operation.into(),
            path,
            reason: reason.into(),
        }
    }

    /// Create a generic operation failure error
    pub fn operation_failed(operation: impl Into<String>, message: impl Into<String>) -> Self {
        Self::OperationFailed {
            operation: operation.into(),
            message: message.into(),
        }
    }

    /// Create a serialization error
    pub fn serialization(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::operation_failed("serialization", format!("{}: {}", context.into(), source))
    }

    /// Create a deserialization error
    pub fn deserialization(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::operation_failed("deserialization", format!("{}: {}", context.into(), source))
    }
}

/// Result type alias for RoboSync operations
pub type Result<T> = std::result::Result<T, RoboSyncError>;

/// Convert from io::Error to RoboSyncError
impl From<io::Error> for RoboSyncError {
    fn from(error: io::Error) -> Self {
        Self::io_error(error, None)
    }
}

/// Convert from anyhow::Error to RoboSyncError for backward compatibility
impl From<anyhow::Error> for RoboSyncError {
    fn from(error: anyhow::Error) -> Self {
        Self::operation_failed("anyhow_conversion", error.to_string())
    }
}

/// Convert from serde_json::Error to RoboSyncError
impl From<serde_json::Error> for RoboSyncError {
    fn from(error: serde_json::Error) -> Self {
        Self::operation_failed("json_serialization", error.to_string())
    }
}

/// Trait for adding path context to I/O errors
pub trait IoErrorExt<T> {
    /// Add path context to an I/O error
    fn with_path(self, path: PathBuf) -> Result<T>;

    /// Add path context for permission errors
    fn with_permission_context(self, path: PathBuf) -> Result<T>;
}

/// Trait for adding context to any error (similar to anyhow::Context)
pub trait ErrorContext<T> {
    /// Add context to any error
    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> String;
}

impl<T, E> ErrorContext<T> for std::result::Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> String,
    {
        self.map_err(|e| {
            RoboSyncError::operation_failed("context_operation", format!("{}: {}", f(), e))
        })
    }
}

impl<T> IoErrorExt<T> for std::result::Result<T, io::Error> {
    fn with_path(self, path: PathBuf) -> Result<T> {
        self.map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => RoboSyncError::not_found(path),
            io::ErrorKind::PermissionDenied => RoboSyncError::permission_error(e, path),
            _ => RoboSyncError::io_error(e, Some(path)),
        })
    }

    fn with_permission_context(self, path: PathBuf) -> Result<T> {
        self.map_err(|e| RoboSyncError::permission_error(e, path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let path = PathBuf::from("/test/path");

        // Test I/O error creation
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let error = RoboSyncError::io_error(io_err, Some(path.clone()));
        assert!(matches!(error, RoboSyncError::Io { .. }));

        // Test not found error
        let error = RoboSyncError::not_found(path.clone());
        assert!(matches!(error, RoboSyncError::NotFound { .. }));

        // Test sync failure
        let error = RoboSyncError::sync_failed("test failure", Some(path.clone()), None);
        assert!(matches!(error, RoboSyncError::SyncFailed { .. }));
    }

    #[test]
    fn test_io_error_ext() {
        let path = PathBuf::from("/test/path");

        // Test successful operation
        let result: std::result::Result<i32, io::Error> = Ok(42);
        assert_eq!(
            result.with_path(path.clone()).expect("Result should be Ok"),
            42
        );

        // Test error conversion
        let io_err = io::Error::new(io::ErrorKind::NotFound, "not found");
        let result: std::result::Result<i32, io::Error> = Err(io_err);
        let robosync_err = result.with_path(path.clone()).unwrap_err();
        assert!(matches!(robosync_err, RoboSyncError::NotFound { .. }));
    }

    #[test]
    fn test_error_display() {
        let path = PathBuf::from("/test/file.txt");
        let error = RoboSyncError::checksum_mismatch(path, "abc123", "def456");
        let display = format!("{}", error);
        assert!(display.contains("Checksum mismatch"));
        assert!(display.contains("/test/file.txt"));
        assert!(display.contains("abc123"));
        assert!(display.contains("def456"));
    }
}
