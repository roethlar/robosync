//! Progress reporting and statistics

use indicatif::{ProgressBar, ProgressStyle};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

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
    completed_files: u64,
    #[allow(dead_code)]
    total_bytes: u64,
    transferred_bytes: AtomicU64,
    start_time: Instant,
    progress_bar: Option<ProgressBar>,
}

impl SyncProgress {
    pub fn new(total_files: u64, total_bytes: u64) -> Self {
        // Use file-based progress instead of byte-based for delta sync accuracy
        let progress_bar = ProgressBar::new(total_files);
        progress_bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:50.cyan/blue}] {pos}/{len} files | {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );

        Self {
            total_files,
            completed_files: 0,
            total_bytes,
            transferred_bytes: AtomicU64::new(0),
            start_time: Instant::now(),
            progress_bar: Some(progress_bar),
        }
    }

    /// Create with an optional pre-created progress bar (for MultiProgress integration)
    pub fn new_with_progress_bar(
        total_files: u64,
        total_bytes: u64,
        progress_bar: Option<ProgressBar>,
    ) -> Self {
        Self {
            total_files,
            completed_files: 0,
            total_bytes,
            transferred_bytes: AtomicU64::new(0),
            start_time: Instant::now(),
            progress_bar,
        }
    }

    pub fn update_file_complete(&mut self, file_size: u64) {
        self.completed_files += 1;
        self.transferred_bytes
            .fetch_add(file_size, Ordering::Relaxed);

        // Update progress bar with throughput
        if let Some(ref pb) = self.progress_bar {
            pb.set_position(self.completed_files);

            // Calculate and display throughput
            let elapsed = self.start_time.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                let bytes_total = self.transferred_bytes.load(Ordering::Relaxed);
                let throughput = (bytes_total as f64 / elapsed) as u64;
                pb.set_message(format!("{}/s", indicatif::HumanBytes(throughput)));
            }
        }
    }

    #[allow(dead_code)]
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
                self.completed_files, self.total_files
            );
            println!("  Bytes transferred: {transferred_bytes} bytes");
            println!("  Time elapsed: {:.2}s", elapsed.as_secs_f64());
            println!("  Transfer rate: {:.2} MB/s", rate / 1_000_000.0);
        }
    }
}
