// Unified streaming walker for on-the-fly file discovery and processing
// Eliminates the startup latency from exhaustive upfront directory scanning

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use walkdir::WalkDir;

use crate::file_list::FileOperation;
use crate::options::SyncOptions;
use crate::sync_stats::SyncStats;

/// Operation with cached metadata to avoid duplicate stat calls
pub struct OperationWithSize {
    pub operation: FileOperation,
    pub size: u64,  // File size in bytes (0 for directories)
    pub warning_encountered: bool, // True if this operation triggered a warning (access denied, etc.)
}

/// A streaming walker that discovers files on-the-fly and sends them for immediate processing
pub struct StreamingWalker {
    source: PathBuf,
    destination: PathBuf,
    options: SyncOptions,
}

impl StreamingWalker {
    /// Create a new streaming walker
    pub fn new(source: PathBuf, destination: PathBuf, options: SyncOptions) -> Self {
        Self {
            source,
            destination,
            options,
        }
    }

    /// Walk directories and stream operations through a channel
    /// This allows the Hybrid Dam to start processing immediately without waiting
    pub fn walk_and_stream<F>(&self, mut process_fn: F) -> Result<SyncStats>
    where
        F: FnMut(FileOperation) -> Result<SyncStats> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let source = self.source.clone();
        let destination = self.destination.clone();
        let options = self.options.clone();

        // Spawn walker thread to discover files
        let walker_thread = thread::spawn(move || -> Result<()> {
            Self::discover_files_to_channel(source, destination, options, tx)
        });

        // Process files as they're discovered
        let mut total_stats = SyncStats::default();
        for operation in rx {
            match process_fn(operation) {
                Ok(stats) => {
                    // Would need to merge stats here, but SyncStats doesn't have a merge method
                    // This is part of why the streaming walker needs more architectural work
                    total_stats.add_bytes_transferred(stats.bytes_transferred());
                    for _ in 0..stats.files_copied() {
                        total_stats.increment_files_copied();
                    }
                    for _ in 0..stats.files_deleted() {
                        total_stats.increment_files_deleted();
                    }
                    for _ in 0..stats.errors() {
                        total_stats.increment_errors();
                    }
                }
                Err(e) => {
                    if self.options.verbose >= 1 {
                        eprintln!("Error processing operation: {}", e);
                    }
                    total_stats.increment_errors();
                }
            }
        }

        // Wait for walker to complete, handle panic gracefully
        match walker_thread.join() {
            Ok(result) => {
                // Only log errors, don't propagate them - we've already processed what we could
                if let Err(e) = result {
                    if self.options.verbose >= 1 {
                        eprintln!("Warning: Walker encountered errors: {}", e);
                    }
                    total_stats.increment_errors();
                }
            }
            Err(_) => {
                if self.options.verbose >= 1 {
                    eprintln!("Warning: Walker thread panicked, but synchronization continued with discovered files");
                }
                total_stats.increment_errors();
            }
        }

        Ok(total_stats)
    }

    /// Discover files and send operations through the channel (public for direct use)
    pub fn discover_files_to_channel(
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
        tx: mpsc::Sender<FileOperation>,
    ) -> Result<()> {
        // Create a channel that includes size to avoid duplicate metadata calls
        let (size_tx, size_rx) = mpsc::channel::<OperationWithSize>();
        
        // Spawn a thread to convert OperationWithSize back to FileOperation for compatibility
        thread::spawn(move || {
            for op_with_size in size_rx {
                if tx.send(op_with_size.operation).is_err() {
                    break;
                }
            }
        });
        
        Self::discover_files_with_size_to_channel(source, destination, options, size_tx)
    }
    
    /// Discover files and send operations WITH SIZE through the channel to avoid duplicate metadata calls
    pub fn discover_files_with_size_to_channel(
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
        tx: mpsc::Sender<OperationWithSize>,
    ) -> Result<()> {
        // For single file copy
        if source.is_file() {
            let dest_path = if destination.exists() && destination.is_dir() {
                destination.join(source.file_name().unwrap())
            } else {
                destination.clone()
            };

            let size = source.metadata().map(|m| m.len()).unwrap_or(0);
            tx.send(OperationWithSize {
                operation: FileOperation::Create {
                    path: source.clone(),  // SOURCE path, not destination!
                },
                size,
                warning_encountered: false,
            }).ok();

            return Ok(());
        }

        // Walk source directory
        let walker = WalkDir::new(&source)
            .follow_links(options.symlink_behavior == crate::options::SymlinkBehavior::Dereference)
            .same_file_system(false);

        for entry in walker {
            // Handle access denied and other I/O errors gracefully
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    // Log warning but continue processing other files
                    if options.verbose >= 1 {
                        eprintln!("Warning: Skipping inaccessible path: {}", err);
                    }
                    // Send a warning notification to the main thread
                    let _ = tx.send(OperationWithSize {
                        operation: FileOperation::Create {
                            path: PathBuf::from("__WARNING__"), // Placeholder path
                        },
                        size: 0,
                        warning_encountered: true,
                    });
                    continue;
                }
            };
            let path = entry.path();

            // Skip the root directory itself
            if path == source {
                continue;
            }

            // Apply exclusion filters
            if let Some(name) = path.file_name() {
                let name_str = name.to_string_lossy();
                
                // Check excluded directories
                if entry.file_type().is_dir() {
                    if options.exclude_dirs.iter().any(|pattern| {
                        name_str.contains(pattern)
                    }) {
                        continue;
                    }
                }

                // Check excluded files
                if entry.file_type().is_file() {
                    if options.exclude_files.iter().any(|pattern| {
                        name_str.contains(pattern)
                    }) {
                        continue;
                    }
                }
            }

            // Apply size filters and get file size for later use
            // CRITICAL: We get metadata here and must pass the size forward to avoid duplicate calls!
            let mut file_size = 0u64;
            if entry.file_type().is_file() {
                // NOTE: entry.metadata() uses WalkDir's cached metadata - it's FAST!
                // But we MUST pass this size forward to avoid calling std::fs::metadata again later
                if let Ok(metadata) = entry.metadata() {
                    file_size = metadata.len();
                    if let Some(min) = options.min_size {
                        if file_size < min {
                            continue;
                        }
                    }
                    if let Some(max) = options.max_size {
                        if file_size > max {
                            continue;
                        }
                    }
                }
            }

            // Calculate destination path
            let rel_path = match path.strip_prefix(&source) {
                Ok(p) => p,
                Err(err) => {
                    if options.verbose >= 1 {
                        eprintln!("Warning: Skipping file with invalid path: {} ({})", path.display(), err);
                    }
                    // Send a warning notification to the main thread
                    let _ = tx.send(OperationWithSize {
                        operation: FileOperation::Create {
                            path: PathBuf::from("__WARNING__"), // Placeholder path
                        },
                        size: 0,
                        warning_encountered: true,
                    });
                    continue;
                }
            };
            let dest_path = destination.join(rel_path);

            // Determine operation type
            let (operation, size) = if entry.file_type().is_dir() {
                (FileOperation::CreateDirectory {
                    path: path.to_path_buf(),  // SOURCE path, not destination!
                }, 0u64)
            } else if entry.file_type().is_file() {
                // Check if destination exists and needs updating
                let op = if dest_path.exists() {
                    let needs_update = if options.checksum {
                        // Would need to compute checksums here
                        true
                    } else {
                        // Compare modification times
                        if let (Ok(src_meta), Ok(dst_meta)) = (path.metadata(), dest_path.metadata()) {
                            // Handle modification time access errors
                            match (src_meta.modified(), dst_meta.modified()) {
                                (Ok(src_time), Ok(dst_time)) => src_time > dst_time,
                                _ => true // If we can't read times, assume update needed
                            }
                        } else {
                            true
                        }
                    };

                    if needs_update {
                        FileOperation::Update {
                            path: path.to_path_buf(),  // SOURCE path, not destination!
                            use_delta: false, // Will be determined by strategy
                        }
                    } else {
                        continue; // Skip unchanged files
                    }
                } else {
                    FileOperation::Create {
                        path: path.to_path_buf(),  // SOURCE path, not destination!
                    }
                };
                (op, file_size)  // Use the file_size we already got from entry.metadata()!
            } else {
                continue; // Skip special files for now
            };

            // Send operation WITH SIZE for immediate processing
            if tx.send(OperationWithSize { operation, size, warning_encountered: false }).is_err() {
                // Receiver dropped, stop walking
                break;
            }
        }

        // Handle purge operations if needed
        if options.purge || options.mirror {
            Self::find_deletions_with_size(&destination, &source, &options, &tx)?;
        }

        Ok(())
    }

    /// Find files to delete in mirror/purge mode (compatibility wrapper)
    fn find_deletions(
        destination: &Path,
        source: &Path,
        _options: &SyncOptions,
        tx: &mpsc::Sender<FileOperation>,
    ) -> Result<()> {
        // Create a channel for OperationWithSize
        let (size_tx, size_rx) = mpsc::channel::<OperationWithSize>();
        
        // Spawn thread to convert back to FileOperation
        let tx_clone = tx.clone();
        thread::spawn(move || {
            for op_with_size in size_rx {
                if tx_clone.send(op_with_size.operation).is_err() {
                    break;
                }
            }
        });
        
        Self::find_deletions_with_size(destination, source, _options, &size_tx)
    }
    
    /// Find files to delete in mirror/purge mode (with size)
    fn find_deletions_with_size(
        destination: &Path,
        source: &Path,
        _options: &SyncOptions,
        tx: &mpsc::Sender<OperationWithSize>,
    ) -> Result<()> {
        let walker = WalkDir::new(destination)
            .follow_links(false)
            .same_file_system(false);

        for entry in walker {
            let entry = entry?;
            let dest_path = entry.path();

            // Skip the root directory
            if dest_path == destination {
                continue;
            }

            // Calculate corresponding source path
            let rel_path = dest_path.strip_prefix(destination)?;
            let src_path = source.join(rel_path);

            // If source doesn't exist, mark for deletion
            if !src_path.exists() {
                let size = dest_path.metadata().map(|m| m.len()).unwrap_or(0);
                let operation = OperationWithSize {
                    operation: FileOperation::Delete {
                        path: dest_path.to_path_buf(),
                    },
                    size,
                    warning_encountered: false,
                };

                if tx.send(operation).is_err() {
                    break;
                }
            }
        }

        Ok(())
    }
}