//! RoboSync: Fast, parallel file synchronization with delta-transfer
//!
//! RoboSync combines the best of RoboCopy and rsync:
//! - RoboCopy-style command-line interface and options
//! - Rsync's efficient delta-transfer algorithm
//! - Modern multithreaded architecture with Rust
//! - Advanced features: compression, retry logic, filtering
//! - Cross-platform support (Windows, macOS, Linux)
//! - High performance with parallel I/O and BLAKE3 hashing

pub mod algorithm;
pub mod checksum;
pub mod compression;
pub mod file_list;
pub mod logging;
pub mod metadata;
pub mod options;
pub mod parallel_sync;
pub mod progress;
pub mod retry;
pub mod sync;

pub use algorithm::DeltaAlgorithm;
pub use checksum::ChecksumType;
pub use options::SyncOptions;
pub use parallel_sync::{ParallelSyncConfig, ParallelSyncer};
pub use retry::{with_retry, RetryConfig};
pub use sync::synchronize;
