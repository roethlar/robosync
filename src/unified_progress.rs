//! Unified progress tracking across all copy strategies
//!
//! This module provides a unified progress interface that works with:
//! - Native tools (rsync, robocopy) by parsing their output
//! - Platform APIs with callbacks
//! - Our custom implementations

use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::progress::ProgressTracker;

/// Unified progress manager that handles all strategies
pub struct UnifiedProgressManager {
    multi_progress: MultiProgress,
    main_bar: ProgressBar,
    status_bar: ProgressBar,
    total_files: AtomicU64,
    completed_files: AtomicU64,
    total_bytes: AtomicU64,
    transferred_bytes: AtomicU64,
    start_time: Instant,
    current_file: Arc<Mutex<String>>,
}

impl UnifiedProgressManager {
    pub fn new(estimated_files: u64, estimated_bytes: u64) -> Self {
        let multi_progress = MultiProgress::new();
        
        // Main progress bar for overall progress
        let main_bar = multi_progress.add(ProgressBar::new(100));
        main_bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:50.cyan/blue}] {pos}% | {msg}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏ "),
        );
        main_bar.set_message("Initializing...");
        
        // Status bar for current operation
        let status_bar = multi_progress.add(ProgressBar::new_spinner());
        status_bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.yellow} {msg}")
                .unwrap(),
        );
        
        Self {
            multi_progress,
            main_bar,
            status_bar,
            total_files: AtomicU64::new(estimated_files),
            completed_files: AtomicU64::new(0),
            total_bytes: AtomicU64::new(estimated_bytes),
            transferred_bytes: AtomicU64::new(0),
            start_time: Instant::now(),
            current_file: Arc::new(Mutex::new(String::new())),
        }
    }
    
    /// Update progress from native tool output
    pub fn update_from_rsync_output(&self, line: &str) {
        // Parse rsync progress output
        // Example: "xfr#1, to-chk=0/1"
        if line.contains("to-chk=") {
            if let Some(pos) = line.find("to-chk=") {
                let remaining = &line[pos + 7..];
                if let Some(slash) = remaining.find('/') {
                    if let (Ok(left), Ok(total)) = (
                        remaining[..slash].parse::<u64>(),
                        remaining[slash + 1..].split_whitespace().next()
                            .unwrap_or("0")
                            .parse::<u64>()
                    ) {
                        let completed = total.saturating_sub(left);
                        self.update_file_count(completed);
                    }
                }
            }
        }
        
        // Parse file being transferred
        if !line.starts_with(' ') && line.contains('/') {
            if let Ok(mut current) = self.current_file.lock() {
                *current = line.trim().to_string();
                self.status_bar.set_message(format!("Copying: {}", line.trim()));
            }
        }
    }
    
    /// Update progress from robocopy output
    pub fn update_from_robocopy_output(&self, line: &str) {
        // Parse robocopy progress
        // Example: "100%"
        if line.trim().ends_with('%') {
            if let Ok(percent) = line.trim().trim_end_matches('%').parse::<u64>() {
                self.update_percentage(percent);
            }
        }
        
        // Parse file count
        // Example: "Files :        10        10         0         0         0         0"
        if line.trim_start().starts_with("Files :") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                if let Ok(total) = parts[2].parse::<u64>() {
                    self.total_files.store(total, Ordering::Relaxed);
                }
            }
        }
    }
    
    /// Update file count
    pub fn update_file_count(&self, completed: u64) {
        self.completed_files.store(completed, Ordering::Relaxed);
        self.update_main_progress();
    }
    
    /// Update bytes transferred  
    pub fn update_bytes(&self, bytes: u64) {
        self.transferred_bytes.store(bytes, Ordering::Relaxed);
        self.update_main_progress();
    }
    
    /// Add to bytes transferred
    pub fn add_bytes(&self, bytes: u64) {
        self.transferred_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.update_main_progress();
    }
    
    /// Update percentage directly
    pub fn update_percentage(&self, percentage: u64) {
        self.main_bar.set_position(percentage.min(100));
        self.update_throughput();
    }
    
    /// Update current file being processed
    pub fn set_current_file(&self, file: &str) {
        if let Ok(mut current) = self.current_file.lock() {
            *current = file.to_string();
        }
        self.status_bar.set_message(format!("Processing: {}", file));
    }
    
    /// Get current statistics
    pub fn get_stats(&self) -> (u64, u64, f64) {
        let files = self.completed_files.load(Ordering::Relaxed);
        let bytes = self.transferred_bytes.load(Ordering::Relaxed);
        let elapsed = self.start_time.elapsed().as_secs_f64();
        (files, bytes, elapsed)
    }
    
    /// Update main progress bar
    fn update_main_progress(&self) {
        let total_files = self.total_files.load(Ordering::Relaxed);
        let completed_files = self.completed_files.load(Ordering::Relaxed);
        
        if total_files > 0 {
            let percentage = (completed_files * 100) / total_files;
            self.main_bar.set_position(percentage.min(100));
        }
        
        self.update_throughput();
    }
    
    /// Update throughput display
    fn update_throughput(&self) {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            let bytes = self.transferred_bytes.load(Ordering::Relaxed);
            let throughput = (bytes as f64 / elapsed) as u64;
            let files = self.completed_files.load(Ordering::Relaxed);
            let total_files = self.total_files.load(Ordering::Relaxed);
            
            self.main_bar.set_message(format!(
                "{}/{} files | {} | {}/s",
                files,
                total_files,
                indicatif::HumanBytes(bytes),
                indicatif::HumanBytes(throughput)
            ));
        }
    }
    
    /// Finish progress tracking
    pub fn finish(&self, success: bool) {
        let elapsed = self.start_time.elapsed();
        let files = self.completed_files.load(Ordering::Relaxed);
        let bytes = self.transferred_bytes.load(Ordering::Relaxed);
        let throughput = if elapsed.as_secs_f64() > 0.0 {
            (bytes as f64 / elapsed.as_secs_f64()) as u64
        } else {
            0
        };
        
        if success {
            self.main_bar.finish_with_message(format!(
                "✓ Complete: {} files, {} in {:.1}s ({}/s)",
                files,
                indicatif::HumanBytes(bytes),
                elapsed.as_secs_f64(),
                indicatif::HumanBytes(throughput)
            ));
            self.status_bar.finish_and_clear();
        } else {
            self.main_bar.abandon_with_message("✗ Operation cancelled or failed");
            self.status_bar.abandon();
        }
    }
    
    /// Create a subprocess progress tracker for platform APIs
    pub fn create_tracker(self: &Arc<Self>) -> Arc<UnifiedProgressTrackerImpl> {
        Arc::new(UnifiedProgressTrackerImpl {
            manager: Arc::clone(self),
        })
    }
}

/// Progress tracker implementation for use with platform APIs
pub struct UnifiedProgressTrackerImpl {
    manager: Arc<UnifiedProgressManager>,
}

impl ProgressTracker for UnifiedProgressTrackerImpl {
    fn update_percentage(&self, percentage: u64) {
        // Don't update the main progress bar percentage - that's calculated from files/bytes
        // This is for individual file progress which we can ignore for now
    }
    
    fn update_file_count(&self, count: u64) {
        self.manager.update_file_count(count);
    }
    
    fn update_bytes(&self, bytes: u64) {
        self.manager.update_bytes(bytes);
    }
    
    fn finish(&self) {
        // Don't finish the whole manager, just update status
        self.manager.status_bar.set_message("File complete");
    }
}

/// Helper to parse progress from a process output line by line
pub struct ProgressParser {
    manager: Arc<UnifiedProgressManager>,
    tool_type: ToolType,
}

#[derive(Debug, Clone, Copy)]
pub enum ToolType {
    Rsync,
    Robocopy,
}

impl ProgressParser {
    pub fn new(manager: Arc<UnifiedProgressManager>, tool_type: ToolType) -> Self {
        Self { manager, tool_type }
    }
    
    /// Parse a line of output from the tool
    pub fn parse_line(&self, line: &str) {
        match self.tool_type {
            ToolType::Rsync => self.manager.update_from_rsync_output(line),
            ToolType::Robocopy => self.manager.update_from_robocopy_output(line),
        }
    }
}