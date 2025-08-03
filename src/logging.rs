//! Logging and progress reporting functionality

use anyhow::Result;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Logger that can write to both console and file
pub struct SyncLogger {
    log_file: Option<Arc<Mutex<BufWriter<File>>>>,
    start_time: Instant,
    total_files: u64,
    completed_files: u64,
    total_bytes: u64,
    transferred_bytes: u64,
    show_eta: bool,
}

impl SyncLogger {
    /// Create a new logger with optional log file
    pub fn new(log_file_path: Option<&str>, show_eta: bool) -> Result<Self> {
        let log_file = if let Some(path) = log_file_path {
            let file = File::create(path)?;
            Some(Arc::new(Mutex::new(BufWriter::new(file))))
        } else {
            None
        };

        Ok(Self {
            log_file,
            start_time: Instant::now(),
            total_files: 0,
            completed_files: 0,
            total_bytes: 0,
            transferred_bytes: 0,
            show_eta,
        })
    }

    /// Initialize progress tracking with total counts
    pub fn initialize_progress(&mut self, total_files: u64, total_bytes: u64) {
        self.total_files = total_files;
        self.total_bytes = total_bytes;
        self.completed_files = 0;
        self.transferred_bytes = 0;
    }

    /// Log a message to both console and file (if configured)
    pub fn log(&self, message: &str) {
        // Always print to console
        println!("{message}");

        // Also write to log file if configured
        if let Some(ref log_file) = self.log_file {
            if let Ok(mut writer) = log_file.lock() {
                let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
                let _ = writeln!(writer, "[{timestamp}] {message}");
                // Flush immediately to ensure log is written
                let _ = writer.flush();
            }
        }
    }

    /// Log an error message
    pub fn log_error(&self, error: &str) {
        // Don't print to stderr - errors are collected and shown at the end
        if let Some(ref log_file) = self.log_file {
            if let Ok(mut writer) = log_file.lock() {
                let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
                let _ = writeln!(writer, "[{timestamp}] ERROR: {error}");
                let _ = writer.flush();
            }
        }
    }

    /// Update progress and optionally show ETA
    pub fn update_progress(&mut self, files_delta: u64, bytes_delta: u64) {
        self.completed_files += files_delta;
        self.transferred_bytes += bytes_delta;

        if self.show_eta && self.total_files > 0 {
            let progress_message = self.generate_progress_message();
            self.log(&progress_message);
        }
    }
    
    /// Log current file being processed
    pub fn log_file_operation(&self, operation: &str, path: &str) {
        if let Some(ref log_file) = self.log_file {
            if let Ok(mut writer) = log_file.lock() {
                let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
                let _ = writeln!(writer, "[{timestamp}] {operation}: {path}");
                let _ = writer.flush();
            }
        }
    }

    /// Generate a progress message with ETA
    fn generate_progress_message(&self) -> String {
        let elapsed = self.start_time.elapsed();
        let elapsed_secs = elapsed.as_secs_f64();

        // Calculate progress percentages
        let file_progress = if self.total_files > 0 {
            (self.completed_files as f64 / self.total_files as f64) * 100.0
        } else {
            0.0
        };

        let byte_progress = if self.total_bytes > 0 {
            (self.transferred_bytes as f64 / self.total_bytes as f64) * 100.0
        } else {
            0.0
        };

        // Use byte progress for ETA calculation as it's more accurate
        let progress_ratio = if self.total_bytes > 0 {
            self.transferred_bytes as f64 / self.total_bytes as f64
        } else if self.total_files > 0 {
            self.completed_files as f64 / self.total_files as f64
        } else {
            0.0
        };

        // Calculate ETA
        let eta_str = if progress_ratio > 0.01 && elapsed_secs > 1.0 {
            let estimated_total_time = elapsed_secs / progress_ratio;
            let remaining_time = estimated_total_time - elapsed_secs;

            if remaining_time > 0.0 {
                format_duration(Duration::from_secs_f64(remaining_time))
            } else {
                "Almost done".to_string()
            }
        } else {
            "Calculating...".to_string()
        };

        // Calculate transfer rate
        let rate_str = if elapsed_secs > 0.0 {
            let bytes_per_sec = self.transferred_bytes as f64 / elapsed_secs;
            format_transfer_rate(bytes_per_sec)
        } else {
            "0.00 MB/s".to_string()
        };

        format!(
            "Progress: Files {}/{} ({:.1}%), Bytes {:.1}%, Rate: {}, ETA: {}",
            self.completed_files, self.total_files, file_progress, byte_progress, rate_str, eta_str
        )
    }

    /// Log the final summary using SyncStats
    pub fn log_summary(&self, stats: &crate::sync_stats::SyncStats) {
        let elapsed = self.start_time.elapsed();
        let elapsed_secs = elapsed.as_secs_f64();

        let rate_str = if elapsed_secs > 0.0 {
            let bytes_per_sec = stats.bytes_transferred() as f64 / elapsed_secs;
            format_transfer_rate(bytes_per_sec)
        } else {
            "N/A".to_string()
        };

        let operation_summary = if stats.files_deleted() > 0 {
            format!(
                "{} files copied, {} files deleted",
                stats.files_copied(),
                stats.files_deleted()
            )
        } else {
            format!("{} files", stats.files_copied())
        };

        let summary = format!(
            "Parallel synchronization completed successfully! {}, {} bytes transferred in {:.2}s ({})",
            operation_summary,
            stats.bytes_transferred(),
            elapsed_secs,
            rate_str
        );

        self.log(&summary);

        // Display any warnings that were collected during sync
        if let Ok(warnings) = stats.warnings.lock() {
            if !warnings.is_empty() {
                // Deduplicate warnings
                let mut unique_warnings = std::collections::HashSet::new();
                for warning in warnings.iter() {
                    unique_warnings.insert(warning.clone());
                }

                // Display unique warnings
                for warning in unique_warnings {
                    self.log(&warning);
                }
            }
        }
    }

    /// Flush and close the log file
    pub fn close(&self) {
        if let Some(ref log_file) = self.log_file {
            if let Ok(mut writer) = log_file.lock() {
                let _ = writer.flush();
            }
        }
    }
}

/// Format a duration as human-readable string
fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

/// Format transfer rate as human-readable string
fn format_transfer_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_000_000_000.0 {
        format!("{:.2} GB/s", bytes_per_sec / 1_000_000_000.0)
    } else if bytes_per_sec >= 1_000_000.0 {
        format!("{:.2} MB/s", bytes_per_sec / 1_000_000.0)
    } else if bytes_per_sec >= 1_000.0 {
        format!("{:.2} KB/s", bytes_per_sec / 1_000.0)
    } else {
        format!("{bytes_per_sec:.0} B/s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m 1s");
    }

    #[test]
    fn test_format_transfer_rate() {
        assert_eq!(format_transfer_rate(500.0), "500 B/s");
        assert_eq!(format_transfer_rate(1500.0), "1.50 KB/s");
        assert_eq!(format_transfer_rate(1500000.0), "1.50 MB/s");
        assert_eq!(format_transfer_rate(1500000000.0), "1.50 GB/s");
    }

    #[test]
    fn test_logger_creation() -> Result<()> {
        let mut logger = SyncLogger::new(None, true)?;
        logger.initialize_progress(100, 1000000);
        assert_eq!(logger.total_files, 100);
        assert_eq!(logger.total_bytes, 1000000);
        assert!(logger.show_eta);
        Ok(())
    }
}
