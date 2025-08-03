//! Unified error logging system that respects verbosity levels
//!
//! This module provides centralized error logging that:
//! - Always saves errors to a report file (unless --no-report-errors)
//! - Shows errors on console with -v
//! - Shows all operations with -vv (no progress bar)
//! - Integrates with the existing ErrorReporter

use crate::error_report::{ErrorReportHandle, ErrorReporter};
use crate::options::SyncOptions;
use crate::sync_stats::SyncStats;
use anyhow::Result;
use chrono::Local;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Unified error logger that handles console output and file logging
pub struct ErrorLogger {
    options: SyncOptions,
    error_reporter: Option<ErrorReporter>,
    error_handle: Option<ErrorReportHandle>,
    operation_log: Arc<Mutex<Vec<String>>>,
}

impl ErrorLogger {
    /// Create a new error logger
    pub fn new(options: SyncOptions, source: &Path, destination: &Path) -> Self {
        let (error_reporter, error_handle) = if options.no_report_errors {
            (None, None)
        } else {
            let reporter = ErrorReporter::new(source, destination, &options);
            let handle = reporter.get_handle();
            (Some(reporter), Some(handle))
        };

        Self {
            options,
            error_reporter,
            error_handle,
            operation_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Get a handle for error logging that can be cloned and shared
    pub fn get_handle(&self) -> ErrorLogHandle {
        ErrorLogHandle {
            verbose: self.options.verbose,
            error_handle: self.error_handle.clone(),
            operation_log: Arc::clone(&self.operation_log),
        }
    }

    /// Log an error (saves to file and optionally prints to console)
    pub fn log_error(&self, path: &Path, message: &str, operation: &str) {
        // Always save to error report (unless disabled)
        if let Some(ref handle) = self.error_handle {
            let full_message = format!("{operation}: {message}");
            handle.add_error(path, &full_message);
        }

        // Never print errors to console during execution - they break the progress bar
        // Errors are always saved to the error report file

        // Log operation if verbose >= 2
        if self.options.verbose >= 2 {
            self.log_operation(&format!(
                "ERROR {}: {} - {}",
                operation,
                path.display(),
                message
            ));
        }
    }

    /// Log a warning (saves to file and optionally prints to console)
    pub fn log_warning(&self, path: &Path, message: &str, operation: &str) {
        // Always save to error report (unless disabled)
        if let Some(ref handle) = self.error_handle {
            let full_message = format!("{operation}: {message}");
            handle.add_warning(path, &full_message);
        }

        // Never print warnings to console during execution - they break the progress bar
        // Warnings are always saved to the error report file

        // Log operation if verbose >= 2
        if self.options.verbose >= 2 {
            self.log_operation(&format!(
                "WARNING {}: {} - {}",
                operation,
                path.display(),
                message
            ));
        }
    }

    /// Log a successful operation (only shown with -vv)
    pub fn log_success(&self, path: &Path, operation: &str) {
        if self.options.verbose >= 2 {
            let timestamp = Local::now().format("%H:%M:%S");
            println!("[{}] {} {}", timestamp, operation, path.display());
            self.log_operation(&format!("{}: {}", operation, path.display()));
        }
    }

    /// Log any operation (for -vv mode)
    pub fn log_operation(&self, message: &str) {
        if self.options.verbose >= 2 {
            if let Ok(mut log) = self.operation_log.lock() {
                log.push(format!("[{}] {}", Local::now().format("%H:%M:%S"), message));
            }
        }
    }

    /// Should we display progress bars?
    pub fn should_show_progress(&self) -> bool {
        // Show progress bar with --progress (unless -vv)
        self.options.verbose < 2 && self.options.show_progress
    }

    /// Write the error report file if needed
    pub fn finalize(&self) -> Result<Option<PathBuf>> {
        if let Some(ref reporter) = self.error_reporter {
            reporter.write_report()
        } else {
            Ok(None)
        }
    }

    /// Write the error report file with details from SyncStats
    pub fn finalize_with_stats(&self, stats: &SyncStats) -> Result<Option<PathBuf>> {
        if let Some(ref reporter) = self.error_reporter {
            // Add all error details from SyncStats to the error reporter
            for error_detail in stats.get_error_details() {
                reporter.add_error(
                    &error_detail.path,
                    &format!("{}: {}", error_detail.operation, error_detail.message),
                );
            }

            // Add all structured errors from SyncStats
            // Add all structured errors from SyncStats
            for structured_error in stats.get_structured_errors() {
                // Extract path from error if available
                let path = match &structured_error.error {
                    crate::error::RoboSyncError::Io { path: Some(p), .. } => p.clone(),
                    crate::error::RoboSyncError::Permission { path, .. } => path.clone(),
                    crate::error::RoboSyncError::NotFound { path } => path.clone(),
                    crate::error::RoboSyncError::SyncFailed {
                        source_path: Some(p),
                        ..
                    } => p.clone(),
                    crate::error::RoboSyncError::SyncFailed {
                        dest_path: Some(p), ..
                    } => p.clone(),
                    crate::error::RoboSyncError::DeltaFailed { file_path, .. } => file_path.clone(),
                    crate::error::RoboSyncError::ChecksumMismatch { path, .. } => path.clone(),
                    crate::error::RoboSyncError::PatternError { path: Some(p), .. } => p.clone(),
                    _ => PathBuf::from("unknown"),
                };

                reporter.add_error(
                    &path,
                    &format!("{}: {}", structured_error.context, structured_error.error),
                );
            }

            reporter.write_report()
        } else {
            Ok(None)
        }
    }

    /// Get error count
    pub fn error_count(&self) -> usize {
        self.error_reporter
            .as_ref()
            .map(|r| r.error_count())
            .unwrap_or(0)
    }

    /// Get warning count
    pub fn warning_count(&self) -> usize {
        self.error_reporter
            .as_ref()
            .map(|r| r.warning_count())
            .unwrap_or(0)
    }
}

/// Handle for error logging that can be cloned and shared across threads
#[derive(Clone)]
pub struct ErrorLogHandle {
    verbose: u8,
    error_handle: Option<ErrorReportHandle>,
    operation_log: Arc<Mutex<Vec<String>>>,
}

impl ErrorLogHandle {
    /// Log an error (saves to file and optionally prints to console)
    pub fn log_error(&self, path: &Path, message: &str, operation: &str) {
        // Always save to error report (unless disabled)
        if let Some(ref handle) = self.error_handle {
            let full_message = format!("{operation}: {message}");
            handle.add_error(path, &full_message);
        }

        // Never print errors to console during execution - they break the progress bar
        // Errors are always saved to the error report file

        // Log operation if verbose >= 2
        if self.verbose >= 2 {
            self.log_operation(&format!(
                "ERROR {}: {} - {}",
                operation,
                path.display(),
                message
            ));
        }
    }

    /// Log a warning (saves to file and optionally prints to console)
    pub fn log_warning(&self, path: &Path, message: &str, operation: &str) {
        // Always save to error report (unless disabled)
        if let Some(ref handle) = self.error_handle {
            let full_message = format!("{operation}: {message}");
            handle.add_warning(path, &full_message);
        }

        // Never print warnings to console during execution - they break the progress bar
        // Warnings are always saved to the error report file

        // Log operation if verbose >= 2
        if self.verbose >= 2 {
            self.log_operation(&format!(
                "WARNING {}: {} - {}",
                operation,
                path.display(),
                message
            ));
        }
    }

    /// Log a successful operation (only shown with -vv)
    pub fn log_success(&self, path: &Path, operation: &str) {
        if self.verbose >= 2 {
            let timestamp = Local::now().format("%H:%M:%S");
            println!("[{}] {} {}", timestamp, operation, path.display());
            self.log_operation(&format!("{}: {}", operation, path.display()));
        }
    }

    /// Log any operation (for -vv mode)
    fn log_operation(&self, message: &str) {
        if self.verbose >= 2 {
            if let Ok(mut log) = self.operation_log.lock() {
                log.push(format!("[{}] {}", Local::now().format("%H:%M:%S"), message));
            }
        }
    }
}
