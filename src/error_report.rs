//! Error reporting module for saving detailed error logs

use crate::options::SyncOptions;
use anyhow::Result;
use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Error reporter that collects errors and warnings during sync
pub struct ErrorReporter {
    report_file: Option<PathBuf>,
    errors: Arc<Mutex<Vec<ErrorEntry>>>,
    warnings: Arc<Mutex<Vec<ErrorEntry>>>,
    error_count: Arc<Mutex<usize>>,
    warning_count: Arc<Mutex<usize>>,
    verbose: u8,
    show_progress: bool,
}

#[derive(Clone)]
struct ErrorEntry {
    timestamp: String,
    path: String,
    message: String,
}

impl ErrorReporter {
    /// Create a new error reporter
    pub fn new(_source: &Path, _destination: &Path, options: &SyncOptions) -> Self {
        // Create report filename
        let report_path = if let Some(ref log_file) = options.log_file {
            // If --log is specified, create error file based on that name
            let log_path = PathBuf::from(log_file);
            let stem = log_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("log");
            let extension = log_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("log");
            PathBuf::from(format!("{stem}_errors.{extension}"))
        } else {
            // Otherwise use timestamp-based name
            let timestamp = Local::now().format("%Y%m%d_%H%M%S");
            PathBuf::from(format!("{timestamp}_robosync_errors.log"))
        };

        let report_file = if options.no_report_errors {
            None
        } else {
            Some(report_path)
        };

        Self {
            report_file,
            errors: Arc::new(Mutex::new(Vec::new())),
            warnings: Arc::new(Mutex::new(Vec::new())),
            error_count: Arc::new(Mutex::new(0)),
            warning_count: Arc::new(Mutex::new(0)),
            verbose: options.verbose,
            show_progress: options.show_progress,
        }
    }

    /// Get a handle for error reporting that can be cloned and shared
    pub fn get_handle(&self) -> ErrorReportHandle {
        ErrorReportHandle {
            errors: Arc::clone(&self.errors),
            warnings: Arc::clone(&self.warnings),
            error_count: Arc::clone(&self.error_count),
            warning_count: Arc::clone(&self.warning_count),
            verbose: self.verbose,
            show_progress: self.show_progress,
        }
    }

    /// Add an error
    pub fn add_error(&self, path: &Path, message: &str) {
        let entry = ErrorEntry {
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            path: path.display().to_string(),
            message: message.to_string(),
        };

        if let Ok(mut errors) = self.errors.lock() {
            errors.push(entry);
        }

        if let Ok(mut count) = self.error_count.lock() {
            *count += 1;
        }
    }

    /// Add a warning
    pub fn add_warning(&self, path: &Path, message: &str) {
        let entry = ErrorEntry {
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            path: path.display().to_string(),
            message: message.to_string(),
        };

        if let Ok(mut warnings) = self.warnings.lock() {
            warnings.push(entry);
        }

        if let Ok(mut count) = self.warning_count.lock() {
            *count += 1;
        }
    }

    /// Get error count
    pub fn error_count(&self) -> usize {
        *self.error_count.lock().unwrap_or_else(|e| {
            eprintln!("Warning: Error report lock poisoned: {}", e);
            std::process::exit(1);
        })
    }

    /// Get warning count
    pub fn warning_count(&self) -> usize {
        *self.warning_count.lock().unwrap_or_else(|e| {
            eprintln!("Warning: Error report lock poisoned: {}", e);
            std::process::exit(1);
        })
    }

    /// Write the error report to file if there were any errors or warnings
    pub fn write_report(&self) -> Result<Option<PathBuf>> {
        let error_count = self.error_count();
        let warning_count = self.warning_count();

        if error_count == 0 && warning_count == 0 {
            return Ok(None);
        }

        let report_path = match &self.report_file {
            Some(path) => path,
            None => return Ok(None),
        };

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(report_path)?;

        // Write header
        writeln!(file, "RoboSync Error Report")?;
        writeln!(
            file,
            "Generated: {}",
            Local::now().format("%Y-%m-%d %H:%M:%S")
        )?;
        writeln!(file, "{}=", "=".repeat(78))?;
        writeln!(file)?;

        // Write summary
        writeln!(file, "Summary:")?;
        writeln!(file, "  Errors: {error_count}")?;
        writeln!(file, "  Warnings: {warning_count}")?;
        writeln!(file)?;

        // Write errors
        if error_count > 0 {
            writeln!(file, "ERRORS:")?;
            writeln!(file, "-------")?;
            if let Ok(errors) = self.errors.lock() {
                for (i, error) in errors.iter().enumerate() {
                    writeln!(file, "\n[{}] Error #{}", error.timestamp, i + 1)?;
                    writeln!(file, "  File: {}", error.path)?;
                    writeln!(file, "  Details: {}", error.message)?;
                }
            }
            writeln!(file)?;
        }

        // Write warnings
        if warning_count > 0 {
            writeln!(file, "WARNINGS:")?;
            writeln!(file, "---------")?;
            if let Ok(warnings) = self.warnings.lock() {
                for (i, warning) in warnings.iter().enumerate() {
                    writeln!(file, "\n[{}] Warning #{}", warning.timestamp, i + 1)?;
                    writeln!(file, "  File: {}", warning.path)?;
                    writeln!(file, "  Details: {}", warning.message)?;
                }
            }
        }

        file.flush()?;
        Ok(Some(report_path.clone()))
    }
}

/// Handle for error reporting that can be cloned and shared across threads
#[derive(Clone)]
pub struct ErrorReportHandle {
    errors: Arc<Mutex<Vec<ErrorEntry>>>,
    warnings: Arc<Mutex<Vec<ErrorEntry>>>,
    error_count: Arc<Mutex<usize>>,
    warning_count: Arc<Mutex<usize>>,
    verbose: u8,
    show_progress: bool,
}

impl ErrorReportHandle {
    /// Add an error
    pub fn add_error(&self, path: &Path, message: &str) {
        self.add_error_with_operation(path, message, "sync");
    }

    /// Add an error with operation context
    pub fn add_error_with_operation(&self, path: &Path, message: &str, operation: &str) {
        let entry = ErrorEntry {
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            path: path.display().to_string(),
            message: format!("{operation}: {message}"),
        };

        if let Ok(mut errors) = self.errors.lock() {
            errors.push(entry);
        }

        if let Ok(mut count) = self.error_count.lock() {
            *count += 1;

            // Print to console if verbose >= 1
            if self.verbose >= 1 {
                eprintln!(
                    "[{}] Error {}: {} - {}",
                    Local::now().format("%H:%M:%S"),
                    operation,
                    path.display(),
                    message
                );
            } else if self.show_progress && *count % 10 == 0 {
                // Only print count periodically if not verbose and progress is shown
                eprintln!("  [{count}] errors encountered (details will be saved to report)");
            }
        }
    }

    /// Add a warning
    pub fn add_warning(&self, path: &Path, message: &str) {
        let entry = ErrorEntry {
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            path: path.display().to_string(),
            message: message.to_string(),
        };

        if let Ok(mut warnings) = self.warnings.lock() {
            warnings.push(entry);
        }

        if let Ok(mut count) = self.warning_count.lock() {
            *count += 1;
            // Only print count periodically to reduce noise
            if *count % 10 == 0 {
                eprintln!("  [{count}] warnings encountered (details will be saved to report)");
            }
        }
    }

    /// Log a successful operation (only shown with -vv)
    pub fn log_success(&self, path: &Path, operation: &str) {
        if self.verbose >= 2 {
            let timestamp = Local::now().format("%H:%M:%S");
            println!("[{}] {} {}", timestamp, operation, path.display());
        }
    }

    /// Check if we should show progress bars
    pub fn should_show_progress(&self) -> bool {
        // Show progress bar with --progress (unless -vv)
        self.verbose < 2 && self.show_progress
    }
}
