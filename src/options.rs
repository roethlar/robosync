//! Synchronization options and configuration

use crate::compression::CompressionConfig;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

/// Symlink handling behavior
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub enum SymlinkBehavior {
    /// Preserve symlinks as symlinks (default, equivalent to --links)
    #[serde(rename = "preserve")]
    Preserve,
    /// Dereference symlinks - copy the target content (equivalent to --deref)
    #[serde(rename = "dereference")]
    Dereference,
    /// Skip symlinks entirely (equivalent to --no-links)
    #[serde(rename = "skip")]
    Skip,
}

impl Default for SymlinkBehavior {
    fn default() -> Self {
        SymlinkBehavior::Preserve
    }
}

/// Configuration loaded from .robosync.toml file
#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub threads: Option<usize>,
    pub retry_count: Option<u32>,
    pub retry_wait: Option<u32>,
    pub symlink_behavior: Option<SymlinkBehavior>,
    pub compress: Option<bool>,
    pub checksum: Option<bool>,
    pub purge: Option<bool>,
    pub move_files: Option<bool>,
    pub exclude_files: Option<Vec<String>>,
    pub exclude_dirs: Option<Vec<String>>,
    pub block_size: Option<usize>,
    pub small_file_threshold: Option<u64>,
    pub medium_file_threshold: Option<u64>,
    pub large_file_threshold: Option<u64>,
}

/// Load configuration from .robosync.toml file if it exists
pub fn load_config() -> Result<Option<Config>> {
    let mut config_paths = vec![
        PathBuf::from(".robosync.toml"),
        PathBuf::from("robosync.toml"),
    ];
    
    if let Some(config_dir) = dirs::config_dir() {
        config_paths.push(config_dir.join("robosync/robosync.toml"));
    }

    for path in &config_paths {
        if path.exists() {
            let contents = fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file: {}", path.display()))?;
            let config: Config = toml::from_str(&contents)
                .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
            return Ok(Some(config));
        }
    }

    Ok(None)
}

/// Synchronization options parsed from command line
#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub recursive: bool,
    pub purge: bool,
    pub mirror: bool,
    pub dry_run: bool,
    pub verbose: u8, // 0 = quiet, 1 = -v, 2 = -vv
    pub confirm: bool,
    pub show_progress: bool,
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
    pub checksum: bool,
    #[cfg(target_os = "linux")]
    pub linux_optimized: bool,
    pub forced_strategy: Option<String>,
    pub symlink_behavior: SymlinkBehavior,
    pub no_report_errors: bool,
    pub debug: bool,
    // shimmer_model_path removed - AI features moved to separate project
    
    // Configurable file size thresholds
    pub small_file_threshold: Option<u64>,
    pub medium_file_threshold: Option<u64>,
    pub large_file_threshold: Option<u64>,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            recursive: false,
            purge: false,
            mirror: false,
            dry_run: false,
            verbose: 0,
            confirm: false,
            show_progress: false,
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
            checksum: false,
            #[cfg(target_os = "linux")]
            linux_optimized: false,
            forced_strategy: None,
            symlink_behavior: SymlinkBehavior::Preserve,
            no_report_errors: false,
            debug: false,
            small_file_threshold: None,
            medium_file_threshold: None,
            large_file_threshold: None,
        }
    }
}