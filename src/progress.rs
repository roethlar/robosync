//! Progress reporting and statistics

use indicatif::{ProgressBar, ProgressStyle};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Trait for progress tracking across different implementations
pub trait ProgressTracker: Send + Sync {
    /// Update progress percentage (0-100)
    fn update_percentage(&self, percentage: u64);

    /// Update file count
    fn update_file_count(&self, count: u64);

    /// Update bytes transferred
    fn update_bytes(&self, bytes: u64);

    /// Finish progress tracking
    fn finish(&self);
}

/// Progress tracking for file synchronization
pub struct SyncProgress {
    total_files: u64,
    completed_files: AtomicU64,
    _total_bytes: u64,
    transferred_bytes: AtomicU64,
    start_time: Instant,
    progress_bar: Option<ProgressBar>,
    silent_mode: AtomicBool,
    update_interval: Duration,
    last_update: Mutex<Instant>,
    current_file: Mutex<String>,
}

impl SyncProgress {
    pub fn new(total_files: u64, total_bytes: u64) -> Self {
        // Use file-based progress instead of byte-based for delta sync accuracy
        let progress_bar = ProgressBar::new(total_files);
        progress_bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:50.cyan/blue}] {pos}/{len} files | {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("#>-"),
        );

        // Enable spinner animation - updates every 100ms
        progress_bar.enable_steady_tick(std::time::Duration::from_millis(100));

        Self {
            total_files,
            completed_files: AtomicU64::new(0),
            _total_bytes: total_bytes,
            transferred_bytes: AtomicU64::new(0),
            start_time: Instant::now(),
            progress_bar: Some(progress_bar),
            silent_mode: AtomicBool::new(false),
            update_interval: Duration::from_millis(500),
            last_update: Mutex::new(Instant::now()),
            current_file: Mutex::new(String::new()),
        }
    }

    /// Create a new progress tracker in silent mode (text-only updates)
    pub fn new_silent(total_files: u64, total_bytes: u64) -> Self {
        Self {
            total_files,
            completed_files: AtomicU64::new(0),
            _total_bytes: total_bytes,
            transferred_bytes: AtomicU64::new(0),
            start_time: Instant::now(),
            progress_bar: None,
            silent_mode: AtomicBool::new(true),
            update_interval: Duration::from_secs(2),
            last_update: Mutex::new(Instant::now()),
            current_file: Mutex::new(String::new()),
        }
    }

    /// Set silent mode (disables progress bars, enables text updates)
    pub fn set_silent(&self, silent: bool) {
        self.silent_mode.store(silent, Ordering::Relaxed);
        if silent {
            if let Some(ref pb) = self.progress_bar {
                pb.abandon();
            }
        }
    }

    /// Set update interval for silent mode
    pub fn set_update_interval(&mut self, interval: Duration) {
        self.update_interval = interval;
    }

    /// Create with an optional pre-created progress bar (for MultiProgress integration)
    pub fn new_with_progress_bar(
        total_files: u64,
        total_bytes: u64,
        progress_bar: Option<ProgressBar>,
    ) -> Self {
        let is_silent = progress_bar.is_none();
        Self {
            total_files,
            completed_files: AtomicU64::new(0),
            _total_bytes: total_bytes,
            transferred_bytes: AtomicU64::new(0),
            start_time: Instant::now(),
            progress_bar,
            silent_mode: AtomicBool::new(is_silent),
            update_interval: Duration::from_secs(2),
            last_update: Mutex::new(Instant::now()),
            current_file: Mutex::new(String::new()),
        }
    }

    pub fn update_file_complete(&mut self, file_size: u64) {
        let completed = self.completed_files.fetch_add(1, Ordering::Relaxed) + 1;
        self.transferred_bytes
            .fetch_add(file_size, Ordering::Relaxed);

        if self.silent_mode.load(Ordering::Relaxed) {
            self.maybe_print_text_update();
        } else {
            // Update progress bar with throughput
            if let Some(ref pb) = self.progress_bar {
                pb.set_position(completed);

                // Calculate and display throughput
                let elapsed = self.start_time.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    let bytes_total = self.transferred_bytes.load(Ordering::Relaxed);
                    let throughput = (bytes_total as f64 / elapsed) as u64;
                    pb.set_message(format!("{}/s", indicatif::HumanBytes(throughput)));
                }
            }
        }
    }

    pub fn update_bytes_transferred(&mut self, bytes: u64) {
        self.transferred_bytes.fetch_add(bytes, Ordering::Relaxed);

        // Update throughput display
        if let Some(ref pb) = self.progress_bar {
            let elapsed = self.start_time.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                let bytes_total = self.transferred_bytes.load(Ordering::Relaxed);
                let throughput = (bytes_total as f64 / elapsed) as u64;
                pb.set_message(format!("{}/s", indicatif::HumanBytes(throughput)));
            }
        }
    }

    pub fn finish(&self) {
        if self.silent_mode.load(Ordering::Relaxed) {
            // Final text summary for silent mode
            let elapsed = self.start_time.elapsed();
            let bytes = self.transferred_bytes.load(Ordering::Relaxed);
            let elapsed_secs = elapsed.as_secs_f64();
            let throughput = if elapsed_secs > 0.1 {
                (bytes as f64 / elapsed_secs) as u64
            } else {
                0
            };

            println!(
                "\nCompleted: {} files, {} in {:.1}s ({}/s)",
                self.completed_files.load(Ordering::Relaxed),
                format_bytes(bytes),
                elapsed.as_secs_f32(),
                format_bytes(throughput)
            );
        } else {
            if let Some(ref pb) = self.progress_bar {
                pb.finish_with_message("Synchronization complete");
            }

            let elapsed = self.start_time.elapsed();
            let transferred_bytes = self.transferred_bytes.load(Ordering::Relaxed);
            let rate = transferred_bytes as f64 / elapsed.as_secs_f64();

            // Only show statistics if we don't have a progress bar (to avoid duplication with logger)
            if self.progress_bar.is_none() {
                println!();
                println!("Synchronization statistics:");
                println!(
                    "  Files processed: {}/{}",
                    self.completed_files.load(Ordering::Relaxed),
                    self.total_files
                );
                println!("  Bytes transferred: {transferred_bytes} bytes");
                println!("  Time elapsed: {:.2}s", elapsed.as_secs_f64());
                println!("  Transfer rate: {:.2} MB/s", rate / 1_000_000.0);
            }
        }
    }

    /// Set current file being processed
    pub fn set_current_file(&self, file: &str) {
        if let Ok(mut current) = self.current_file.lock() {
            *current = file.to_string();
        }

        if self.silent_mode.load(Ordering::Relaxed) {
            // In silent mode, just track it
        } else if let Some(ref pb) = self.progress_bar {
            pb.set_message(format!("Processing: {file}"));
        }
    }

    /// Update progress from tool output (basic parsing)
    pub fn update_from_tool_output(&self, line: &str, tool_type: ToolType) {
        match tool_type {
            ToolType::Rsync => self.parse_rsync_output(line),
            ToolType::Robocopy => self.parse_robocopy_output(line),
        }
    }

    /// Parse rsync output for progress updates
    fn parse_rsync_output(&self, line: &str) {
        // Basic rsync progress parsing - simplified from unified_progress
        if line.contains("to-chk=") {
            if let Some(pos) = line.find("to-chk=") {
                let remaining = &line[pos + 7..];
                if let Some(slash) = remaining.find('/') {
                    if let (Ok(left), Ok(total)) = (
                        remaining[..slash].parse::<u64>(),
                        remaining[slash + 1..]
                            .split_whitespace()
                            .next()
                            .unwrap_or("0")
                            .parse::<u64>(),
                    ) {
                        let completed = total.saturating_sub(left);
                        if let Some(ref pb) = self.progress_bar {
                            pb.set_position(completed);
                        }
                    }
                }
            }
        }

        // Track current file
        if !line.starts_with(' ') && line.contains('/') {
            self.set_current_file(line.trim());
        }
    }

    /// Parse robocopy output for progress updates
    fn parse_robocopy_output(&self, line: &str) {
        // Basic robocopy progress parsing - simplified from unified_progress
        if line.trim().ends_with('%') {
            if let Ok(percent) = line.trim().trim_end_matches('%').parse::<u64>() {
                if let Some(ref pb) = self.progress_bar {
                    let position = (self.total_files * percent) / 100;
                    pb.set_position(position);
                }
            }
        }
    }

    /// Print text update in silent mode (from SimpleProgress)
    fn maybe_print_text_update(&self) {
        let now = Instant::now();
        let should_update = {
            if let Ok(last) = self.last_update.lock() {
                now.duration_since(*last) >= self.update_interval
            } else {
                false
            }
        };

        if should_update {
            if let Ok(mut last) = self.last_update.lock() {
                *last = now;
            }
            self.print_text_update();
        }
    }

    /// Print current progress as text
    fn print_text_update(&self) {
        let elapsed = self.start_time.elapsed();
        let bytes = self.transferred_bytes.load(Ordering::Relaxed);

        let elapsed_secs = elapsed.as_secs_f64();
        let throughput = if elapsed_secs > 0.1 {
            (bytes as f64 / elapsed_secs) as u64
        } else {
            0
        };

        print!(
            "\r{}/{} files | {} | {}/s | {:.1}s",
            self.completed_files.load(Ordering::Relaxed),
            if self.total_files > 0 {
                self.total_files.to_string()
            } else {
                "?".to_string()
            },
            format_bytes(bytes),
            format_bytes(throughput),
            elapsed.as_secs_f32()
        );

        let _ = io::stdout().flush();
    }

    /// Get current statistics
    pub fn get_stats(&self) -> (u64, u64, f64) {
        let files = self.completed_files.load(Ordering::Relaxed);
        let bytes = self.transferred_bytes.load(Ordering::Relaxed);
        let elapsed = self.start_time.elapsed().as_secs_f64();
        (files, bytes, elapsed)
    }

    /// Add bytes transferred (for Arc usage)
    pub fn add_bytes(&self, bytes: u64) {
        self.transferred_bytes.fetch_add(bytes, Ordering::Relaxed);
        if self.silent_mode.load(Ordering::Relaxed) {
            self.maybe_print_text_update();
        }
    }

    /// Add completed file (for Arc usage)
    pub fn add_file(&self) {
        self.completed_files.fetch_add(1, Ordering::Relaxed);
        if self.silent_mode.load(Ordering::Relaxed) {
            self.maybe_print_text_update();
        }
    }

    /// Force a progress update (for compatibility with SimpleProgress)
    pub fn print_update(&self) {
        if self.silent_mode.load(Ordering::Relaxed) {
            self.print_text_update();
        }
        // For visual progress bar mode, progress is updated automatically
    }

    /// Create a progress tracker for platform APIs (simplified from UnifiedProgressManager)
    pub fn create_tracker(self: Arc<Self>) -> Arc<SyncProgressTracker> {
        Arc::new(SyncProgressTracker {
            sync_progress: self,
        })
    }
}

/// Progress tracker implementation for platform API compatibility
pub struct SyncProgressTracker {
    sync_progress: Arc<SyncProgress>,
}

impl ProgressTracker for SyncProgressTracker {
    fn update_percentage(&self, _percentage: u64) {
        // Individual file progress - not needed for our consolidated system
    }

    fn update_file_count(&self, _count: u64) {
        // File count updates handled by add_file
    }

    fn update_bytes(&self, bytes: u64) {
        self.sync_progress.add_bytes(bytes);
    }

    fn finish(&self) {
        // Individual file completion - not needed here
    }
}

/// Tool types for output parsing
#[derive(Debug, Clone, Copy)]
pub enum ToolType {
    Rsync,
    Robocopy,
}

/// Format bytes in human-readable form (from SimpleProgress)
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", size as u64, UNITS[unit_idx])
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}
