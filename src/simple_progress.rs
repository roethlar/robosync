//! Simple progress reporting focused on performance
//!
//! This module provides lightweight progress reporting without the overhead
//! of progress bars, focusing on periodic status updates.

use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, Duration};
use std::io::{self, Write};

/// Simple progress reporter that prints periodic updates
pub struct SimpleProgress {
    total_files: AtomicU64,
    completed_files: AtomicU64,
    total_bytes: AtomicU64,
    transferred_bytes: AtomicU64,
    start_time: Instant,
    last_update: Mutex<Instant>,
    update_interval: Duration,
    silent: AtomicBool,
}

impl SimpleProgress {
    pub fn new(estimated_files: u64, estimated_bytes: u64) -> Arc<Self> {
        Arc::new(Self {
            total_files: AtomicU64::new(estimated_files),
            completed_files: AtomicU64::new(0),
            total_bytes: AtomicU64::new(estimated_bytes),
            transferred_bytes: AtomicU64::new(0),
            start_time: Instant::now(),
            last_update: Mutex::new(Instant::now()),
            update_interval: Duration::from_secs(2), // Update every 2 seconds
            silent: AtomicBool::new(false),
        })
    }
    
    /// Set silent mode (no output)
    pub fn set_silent(&self, silent: bool) {
        self.silent.store(silent, Ordering::Relaxed);
    }
    
    /// Update file count
    pub fn add_file(&self) {
        self.completed_files.fetch_add(1, Ordering::Relaxed);
        self.maybe_print_update();
    }
    
    /// Add bytes transferred
    pub fn add_bytes(&self, bytes: u64) {
        self.transferred_bytes.fetch_add(bytes, Ordering::Relaxed);
    }
    
    /// Force a progress update
    pub fn print_update(&self) {
        if self.silent.load(Ordering::Relaxed) {
            return;
        }
        
        let elapsed = self.start_time.elapsed();
        let files = self.completed_files.load(Ordering::Relaxed);
        let total_files = self.total_files.load(Ordering::Relaxed);
        let bytes = self.transferred_bytes.load(Ordering::Relaxed);
        
        let throughput = if elapsed.as_secs() > 0 {
            bytes / elapsed.as_secs()
        } else {
            0
        };
        
        // Simple status line
        print!("\r{}/{} files | {} | {}/s | {:.1}s",
            files,
            if total_files > 0 { total_files.to_string() } else { "?".to_string() },
            format_bytes(bytes),
            format_bytes(throughput),
            elapsed.as_secs_f32()
        );
        
        let _ = io::stdout().flush();
    }
    
    /// Check if we should print an update
    fn maybe_print_update(&self) {
        let now = Instant::now();
        let should_update = {
            let last = self.last_update.lock().unwrap();
            now.duration_since(*last) >= self.update_interval
        };
        
        if should_update {
            *self.last_update.lock().unwrap() = now;
            self.print_update();
        }
    }
    
    /// Print final summary
    pub fn finish(&self) {
        if self.silent.load(Ordering::Relaxed) {
            return;
        }
        
        let elapsed = self.start_time.elapsed();
        let files = self.completed_files.load(Ordering::Relaxed);
        let bytes = self.transferred_bytes.load(Ordering::Relaxed);
        
        let throughput = if elapsed.as_secs() > 0 {
            bytes / elapsed.as_secs()
        } else {
            bytes
        };
        
        println!("\nCompleted: {} files, {} in {:.1}s ({}/s)",
            files,
            format_bytes(bytes),
            elapsed.as_secs_f32(),
            format_bytes(throughput)
        );
    }
}

/// Format bytes in human-readable form
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