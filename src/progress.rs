//! Progress reporting and statistics

use indicatif::{ProgressBar, ProgressStyle};
use std::time::{Duration, Instant};

/// Progress tracking for file synchronization
pub struct SyncProgress {
    total_files: u64,
    completed_files: u64,
    total_bytes: u64,
    transferred_bytes: u64,
    start_time: Instant,
    progress_bar: ProgressBar,
}

impl SyncProgress {
    pub fn new(total_files: u64, total_bytes: u64) -> Self {
        // Use file-based progress instead of byte-based for delta sync accuracy
        let progress_bar = ProgressBar::new(total_files);
        progress_bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:50.cyan/blue}] {pos}/{len} files ({per_sec}, {eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        
        Self {
            total_files,
            completed_files: 0,
            total_bytes,
            transferred_bytes: 0,
            start_time: Instant::now(),
            progress_bar,
        }
    }
    
    pub fn update_file_complete(&mut self, file_size: u64) {
        self.completed_files += 1;
        self.transferred_bytes += file_size;
        // Update progress bar based on file count, not bytes
        self.progress_bar.set_position(self.completed_files);
    }
    
    pub fn update_bytes_transferred(&mut self, bytes: u64) {
        self.transferred_bytes += bytes;
        // Don't update progress bar position here - use file count instead
    }
    
    pub fn finish(&self) {
        self.progress_bar.finish_with_message("Synchronization complete");
        
        let elapsed = self.start_time.elapsed();
        let rate = self.transferred_bytes as f64 / elapsed.as_secs_f64();
        
        println!();
        println!("Synchronization statistics:");
        println!("  Files processed: {}/{}", self.completed_files, self.total_files);
        println!("  Bytes transferred: {} bytes", self.transferred_bytes);
        println!("  Time elapsed: {:.2}s", elapsed.as_secs_f64());
        println!("  Transfer rate: {:.2} MB/s", rate / 1_000_000.0);
    }
}