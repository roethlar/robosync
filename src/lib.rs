//! RoboSync: High-performance file synchronization with intelligent strategy selection
//!
//! RoboSync combines the best of RoboCopy and rsync with modern performance:
//! - RoboCopy-style command-line interface and options
//! - Rsync's efficient delta-transfer algorithm
//! - Intelligent concurrent mixed processing for optimal performance
//! - Modern multithreaded architecture with Rust
//! - Advanced features: compression, retry logic, filtering
//! - Cross-platform support (Windows, macOS, Linux, BSD)
//! - High performance with parallel I/O and BLAKE3 hashing

pub mod algorithm;
pub mod checksum;
pub mod color_output;
pub mod compression;
pub mod error;
pub mod error_logger;
pub mod error_report;
pub mod fast_file_list;
pub mod file_list;
pub mod formatted_display;
pub mod logging;
pub mod metadata;
pub mod metadata_utils;
pub mod mixed_strategy;
pub mod native_tools;
pub mod operation_utils;
pub mod options;
pub mod parallel_sync;
pub mod platform_api;
pub mod progress;
pub mod retry;
pub mod strategy;
pub mod streaming_delta;
pub mod sync;
pub mod sync_stats;
pub mod filesystem_info;
pub mod reflink;
pub mod buffer_sizing;
pub mod parallel_dirs;
pub mod metadata_cache;
pub mod integrity;
pub mod safe_ops;
pub mod mission_critical;
pub mod streaming_batch;
// Core file synchronization modules only

#[cfg(target_os = "linux")]
pub mod linux_fast_copy;

#[cfg(target_os = "linux")]
pub mod linux_parallel_sync;

#[cfg(target_os = "linux")]
pub mod fast_batch_copy;
pub mod small_file_optimizer;
pub mod ultra_fast_copy;

#[cfg(target_os = "linux")]
pub mod io_uring;

#[cfg(target_os = "linux")]
pub mod extent_copy;

pub mod network_fs;

#[cfg(target_os = "windows")]
pub mod windows_symlinks;

#[cfg(target_os = "windows")]
pub mod windows_fast_enum;

#[cfg(target_os = "macos")]
pub mod macos_mmap;

#[cfg(target_os = "macos")]
pub mod macos_zfs;

#[cfg(target_os = "macos")]
pub mod macos_apfs;

#[cfg(target_os = "macos")]
pub mod macos_network_fs;

#[cfg(target_os = "macos")]
pub mod macos_benchmarks;

pub use algorithm::DeltaAlgorithm;
pub use checksum::ChecksumType;
pub use error::{Result, RoboSyncError};
pub use options::SyncOptions;
pub use parallel_sync::{ParallelSyncConfig, ParallelSyncer};
pub use retry::{with_retry, RetryConfig};
pub use sync::synchronize;
pub use sync_stats::SyncStats;
