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
pub mod compression;
pub mod error;
pub mod file_list;
pub mod logging;
pub mod metadata;
pub mod native_tools;
pub mod options;
pub mod parallel_sync;
pub mod platform_api;
pub mod progress;
pub mod retry;
pub mod strategy;
pub mod sync;
pub mod sync_stats;
pub mod mixed_strategy;
pub mod fast_file_list;
// Core file synchronization modules only

#[cfg(target_os = "linux")]
pub mod linux_fast_copy;

#[cfg(target_os = "linux")]
pub mod linux_parallel_sync;

pub use algorithm::DeltaAlgorithm;
pub use checksum::ChecksumType;
pub use error::{RoboSyncError, Result};
pub use options::SyncOptions;
pub use parallel_sync::{ParallelSyncConfig, ParallelSyncer};
pub use retry::{with_retry, RetryConfig};
pub use sync::synchronize;
pub use sync_stats::SyncStats;
