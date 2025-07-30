//! Synchronization statistics tracking

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Statistics for a synchronization operation
#[derive(Debug, Default)]
pub struct SyncStats {
    files_processed: AtomicU64,
    files_copied: AtomicU64,
    files_deleted: AtomicU64,
    bytes_transferred: AtomicU64,
    blocks_matched: AtomicU64,
    errors: AtomicU64,
    pub elapsed_time: Duration,
    pub warnings: Arc<Mutex<Vec<String>>>,
}

impl SyncStats {
    /// Create a new stats tracker
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Add bytes transferred
    pub fn add_bytes_transferred(&self, bytes: u64) {
        self.bytes_transferred.fetch_add(bytes, Ordering::Relaxed);
    }
    
    /// Set total bytes transferred
    pub fn set_bytes_transferred(&self, bytes: u64) {
        self.bytes_transferred.store(bytes, Ordering::Relaxed);
    }
    
    /// Get bytes transferred
    pub fn bytes_transferred(&self) -> u64 {
        self.bytes_transferred.load(Ordering::Relaxed)
    }
    
    /// Increment files processed
    pub fn increment_files_processed(&self) {
        self.files_processed.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Increment files copied
    pub fn increment_files_copied(&self) {
        self.files_copied.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Increment files deleted
    pub fn increment_files_deleted(&self) {
        self.files_deleted.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Increment errors
    pub fn increment_errors(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Add matched blocks count
    pub fn add_blocks_matched(&self, blocks: u64) {
        self.blocks_matched.fetch_add(blocks, Ordering::Relaxed);
    }
    
    /// Add a warning message
    pub fn add_warning(&self, warning: String) {
        if let Ok(mut warnings) = self.warnings.lock() {
            warnings.push(warning);
        }
    }
    
    /// Get all warnings
    pub fn get_warnings(&self) -> Vec<String> {
        self.warnings.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    
    /// Get files processed count
    pub fn files_processed(&self) -> u64 {
        self.files_processed.load(Ordering::Relaxed)
    }
    
    /// Get files copied count
    pub fn files_copied(&self) -> u64 {
        self.files_copied.load(Ordering::Relaxed)
    }
    
    /// Get files deleted count
    pub fn files_deleted(&self) -> u64 {
        self.files_deleted.load(Ordering::Relaxed)
    }
    
    /// Get error count
    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }
    
    /// Get matched blocks count
    pub fn blocks_matched(&self) -> u64 {
        self.blocks_matched.load(Ordering::Relaxed)
    }
}

impl Clone for SyncStats {
    fn clone(&self) -> Self {
        Self {
            files_processed: AtomicU64::new(self.files_processed()),
            files_copied: AtomicU64::new(self.files_copied()),
            files_deleted: AtomicU64::new(self.files_deleted()),
            bytes_transferred: AtomicU64::new(self.bytes_transferred()),
            blocks_matched: AtomicU64::new(self.blocks_matched()),
            errors: AtomicU64::new(self.errors()),
            elapsed_time: self.elapsed_time,
            warnings: Arc::new(Mutex::new(self.get_warnings())),
        }
    }
}