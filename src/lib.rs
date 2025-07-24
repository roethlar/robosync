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
pub mod file_list;
pub mod progress;
pub mod options;
pub mod metadata;
pub mod logging;
pub mod compression;
pub mod retry;
pub mod sync;
pub mod parallel_sync;

pub use sync::synchronize;
pub use parallel_sync::{ParallelSyncer, ParallelSyncConfig};
pub use algorithm::DeltaAlgorithm;
pub use checksum::ChecksumType;
pub use options::SyncOptions;
pub use retry::{RetryConfig, with_retry};