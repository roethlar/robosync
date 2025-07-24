//! Synchronization options and configuration

use crate::compression::CompressionConfig;

/// Synchronization options parsed from command line
#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub recursive: bool,
    pub purge: bool,
    pub mirror: bool,
    pub dry_run: bool,
    pub verbose: bool,
    pub no_progress: bool,
    pub move_files: bool,
    pub exclude_files: Vec<String>,
    pub exclude_dirs: Vec<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub copy_flags: String,
    pub log_file: Option<String>,
    pub compress: bool,
    pub compression_config: CompressionConfig,
    pub show_eta: bool,
    pub retry_count: u32,
    pub retry_wait: u32,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            recursive: false,
            purge: false,
            mirror: false,
            dry_run: false,
            verbose: false,
            no_progress: false,
            move_files: false,
            exclude_files: Vec::new(),
            exclude_dirs: Vec::new(),
            min_size: None,
            max_size: None,
            copy_flags: "DAT".to_string(),
            log_file: None,
            compress: false,
            compression_config: CompressionConfig::default(),
            show_eta: false,
            retry_count: 0,
            retry_wait: 30,
        }
    }
}