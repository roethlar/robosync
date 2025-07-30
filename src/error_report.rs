//! Error reporting module for saving detailed error logs

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
}

#[derive(Clone)]
struct ErrorEntry {
    timestamp: String,
    path: String,
    message: String,
}

impl ErrorReporter {
    /// Create a new error reporter
    pub fn new(source: &Path, destination: &Path) -> Self {
        // Create report filename with timestamp and sanitized paths
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let source_name = source.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .replace('/', "_");
        let dest_name = destination.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .replace('/', "_");
        
        let report_filename = format!(
            "robosync_errors_{}_{}__to__{}.log",
            timestamp, source_name, dest_name
        );
        
        let report_path = PathBuf::from(&report_filename);
        
        Self {
            report_file: Some(report_path),
            errors: Arc::new(Mutex::new(Vec::new())),
            warnings: Arc::new(Mutex::new(Vec::new())),
            error_count: Arc::new(Mutex::new(0)),
            warning_count: Arc::new(Mutex::new(0)),
        }
    }
    
    /// Get a handle for error reporting that can be cloned and shared
    pub fn get_handle(&self) -> ErrorReportHandle {
        ErrorReportHandle {
            errors: Arc::clone(&self.errors),
            warnings: Arc::clone(&self.warnings),
            error_count: Arc::clone(&self.error_count),
            warning_count: Arc::clone(&self.warning_count),
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
        *self.error_count.lock().unwrap_or_else(|_| panic!("Lock poisoned"))
    }
    
    /// Get warning count
    pub fn warning_count(&self) -> usize {
        *self.warning_count.lock().unwrap_or_else(|_| panic!("Lock poisoned"))
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
        writeln!(file, "Generated: {}", Local::now().format("%Y-%m-%d %H:%M:%S"))?;
        writeln!(file, "=")?;
        writeln!(file)?;
        
        // Write summary
        writeln!(file, "Summary:")?;
        writeln!(file, "  Errors: {}", error_count)?;
        writeln!(file, "  Warnings: {}", warning_count)?;
        writeln!(file)?;
        
        // Write errors
        if error_count > 0 {
            writeln!(file, "ERRORS:")?;
            writeln!(file, "-------")?;
            if let Ok(errors) = self.errors.lock() {
                for (i, error) in errors.iter().enumerate() {
                    writeln!(file, "\n{}. [{}]", i + 1, error.timestamp)?;
                    writeln!(file, "   File: {}", error.path)?;
                    writeln!(file, "   Error: {}", error.message)?;
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
                    writeln!(file, "\n{}. [{}]", i + 1, warning.timestamp)?;
                    writeln!(file, "   File: {}", warning.path)?;
                    writeln!(file, "   Warning: {}", warning.message)?;
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
}

impl ErrorReportHandle {
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
            // Only print count periodically to reduce noise
            if *count % 10 == 0 {
                eprintln!("  [{}] errors encountered (details will be saved to report)", count);
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
                eprintln!("  [{}] warnings encountered (details will be saved to report)", count);
            }
        }
    }
}