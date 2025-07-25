//! Multithreaded synchronization implementation

use anyhow::{Result, Context};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicU64;
use rayon::prelude::*;
use std::fs;
use std::time::Instant;

use crate::algorithm::{DeltaAlgorithm, Match, BlockChecksum};
use crate::file_list::{generate_file_list_with_options, FileInfo, FileOperation, compare_file_lists};
use crate::progress::SyncProgress;
use crate::options::SyncOptions;

/// Configuration for multithreaded synchronization
#[derive(Debug, Clone)]
pub struct ParallelSyncConfig {
    /// Number of worker threads for file processing
    pub worker_threads: usize,
    /// Number of I/O threads for reading/writing
    pub io_threads: usize,
    /// Block size for delta algorithm
    pub block_size: usize,
    /// Maximum number of files to process in parallel
    pub max_parallel_files: usize,
}

impl Default for ParallelSyncConfig {
    fn default() -> Self {
        let num_cpus = std::thread::available_parallelism().unwrap().get();
        Self {
            worker_threads: num_cpus,
            io_threads: std::cmp::min(4, num_cpus),
            block_size: 1024,
            max_parallel_files: num_cpus * 2,
        }
    }
}

/// Multithreaded file synchronization engine
pub struct ParallelSyncer {
    config: ParallelSyncConfig,
}

impl ParallelSyncer {
    pub fn new(config: ParallelSyncConfig) -> Self {
        Self { config }
    }

    /// Synchronize files using multiple threads with options
    pub fn synchronize_with_options(
        &self,
        source: PathBuf,
        destination: PathBuf,
        options: SyncOptions,
    ) -> Result<SyncStats> {
        let start_time = Instant::now();
        
        println!("Starting parallel synchronization...");
        println!("  Source: {}", source.display());
        println!("  Destination: {}", destination.display());
        println!("  Threads: {}", self.config.worker_threads);

        // Create destination parent directory if needed, but don't create destination itself for file-to-file sync
        if source.is_dir() && !destination.exists() {
            fs::create_dir_all(&destination)
                .with_context(|| format!("Failed to create destination directory: {}", destination.display()))?;
            println!("Created destination directory: {}", destination.display());
        }

        if source.is_file() {
            // Single file sync
            self.sync_single_file(&source, &destination, &options)
        } else if source.is_dir() {
            // Directory sync
            self.sync_directories(&source, &destination, &options)
        } else {
            Err(anyhow::anyhow!("Invalid source: {}", source.display()))
        }
    }

    /// Synchronize a single file
    fn sync_single_file(
        &self,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        let dest_path = if destination.exists() && destination.is_dir() {
            let file_name = source.file_name()
                .ok_or_else(|| anyhow::anyhow!("Source file has no name"))?;
            destination.join(file_name)
        } else {
            destination.to_path_buf()
        };

        let stats = self.sync_file_pair(source, &dest_path)?;
        
        println!("Synchronization completed successfully!");
        println!("  Files processed: 1");
        println!("  Bytes transferred: {}", stats.bytes_transferred);
        println!("  Time elapsed: {:.2}s", stats.elapsed_time.as_secs_f64());
        
        Ok(stats)
    }

    /// Synchronize directories using parallel processing
    fn sync_directories(
        &self,
        source: &Path,
        destination: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        println!("Scanning source directory...");
        let source_files = generate_file_list_with_options(source, options)
            .context("Failed to generate source file list")?;
        println!("Found {} items in source", source_files.len());

        println!("Scanning destination directory...");
        let dest_files = if destination.exists() {
            let mut files = generate_file_list_with_options(destination, options)
                .context("Failed to generate destination file list")?;
            // Filter out the destination root directory to avoid deleting it
            files.retain(|f| f.path != *destination);
            println!("Found {} items in destination", files.len());
            files
        } else {
            println!("Destination does not exist, will create");
            Vec::new()
        };

        println!("Analyzing changes...");
        let mut operations = compare_file_lists(&source_files, &dest_files);
        
        // Add purge operations if mirror or purge mode is enabled
        if options.purge || options.mirror {
            let purge_ops = self.find_purge_operations(&source_files, &dest_files, source, destination)?;
            operations.extend(purge_ops);
        }
        
        if operations.is_empty() {
            println!("No changes needed.");
            return Ok(SyncStats::default());
        }

        // Count operations and calculate total bytes for operations that will transfer data
        let total_files = operations.len() as u64;
        let total_bytes: u64 = operations.iter()
            .filter_map(|op| match op {
                FileOperation::Create { path } | FileOperation::Update { path, .. } => {
                    source_files.iter()
                        .find(|f| f.path == *path && !f.is_directory)
                        .map(|f| f.size)
                }
                _ => None
            })
            .sum();

        println!("Processing {} operations, {} create operations, {} delete operations",
            operations.len(),
            operations.iter().filter(|op| matches!(op, FileOperation::Create { .. } | FileOperation::Update { .. })).count(),
            operations.iter().filter(|op| matches!(op, FileOperation::Delete { .. })).count()
        );

        // Show file list in verbose mode
        if options.verbose {
            println!("\nFile operations to be performed:");
            for operation in &operations {
                match operation {
                    FileOperation::Create { path } => {
                        if let Some(file_info) = source_files.iter().find(|f| f.path == *path) {
                            if file_info.is_directory {
                                println!("    New Dir                      {}", path.display());
                            } else {
                                println!("    New File        {:>12}  {}", file_info.size, path.display());
                            }
                        }
                    }
                    FileOperation::Update { path, use_delta } => {
                        if let Some(file_info) = source_files.iter().find(|f| f.path == *path) {
                            let method = if *use_delta { "Delta" } else { "Newer" };
                            println!("    {}           {:>12}  {}", method, file_info.size, path.display());
                        }
                    }
                    FileOperation::Delete { path } => {
                        if path.is_file() {
                            let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                            println!("    *EXTRA File     {:>12}  {}", file_size, path.display());
                        } else {
                            println!("    *EXTRA Dir                   {}", path.display());
                        }
                    }
                    FileOperation::CreateDirectory { path } => {
                        println!("    New Dir                      {}", path.display());
                    }
                }
            }
            println!();
        }

        // Only show progress if not in verbose mode (like RoboCopy)
        let progress = if options.verbose || options.no_progress {
            None
        } else {
            Some(Arc::new(Mutex::new(SyncProgress::new(total_files, total_bytes))))
        };
        let stats = Arc::new(SyncStats::new());

        // Set up Rayon thread pool for parallel processing
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.config.worker_threads)
            .build()
            .context("Failed to create thread pool")?;

        // Separate operations by type for optimal ordering
        let (dir_ops, file_ops): (Vec<_>, Vec<_>) = operations.into_iter()
            .partition(|op| matches!(op, FileOperation::CreateDirectory { .. }));
        
        // Separate delete operations to run last
        let (file_ops, delete_ops): (Vec<_>, Vec<_>) = file_ops.into_iter()
            .partition(|op| !matches!(op, FileOperation::Delete { .. }));

        // Create directories first (sequentially to avoid race conditions)
        for operation in dir_ops {
            self.execute_operation(operation, source, destination, &stats, options)?;
            if let Some(ref progress) = progress {
                if let Ok(mut p) = progress.lock() {
                    p.update_file_complete(0);
                }
            }
        }

        // Process files in parallel
        pool.install(|| {
            file_ops.par_iter()
                .try_for_each(|operation| -> Result<()> {
                    let file_stats = self.execute_operation(operation.clone(), source, destination, &stats, options)?;
                    
                    if let Some(ref progress) = progress {
                        if let Ok(mut p) = progress.lock() {
                            p.update_file_complete(file_stats.bytes_transferred);
                        }
                    }
                    
                    Ok(())
                })
        })?;

        // Process delete operations last (sequentially to avoid issues)
        for operation in delete_ops {
            self.execute_operation(operation, source, destination, &stats, options)?;
            if let Some(ref progress) = progress {
                if let Ok(mut p) = progress.lock() {
                    p.update_file_complete(0);
                }
            }
        }

        if let Some(ref progress) = progress {
            if let Ok(p) = progress.lock() {
                p.finish();
            }
        }

        let final_stats = Arc::try_unwrap(stats).unwrap();
        println!("Parallel synchronization completed successfully!");
        
        Ok(final_stats)
    }

    /// Execute a single file operation
    fn execute_operation(
        &self,
        operation: FileOperation,
        source_root: &Path,
        dest_root: &Path,
        stats: &Arc<SyncStats>,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        match operation {
            FileOperation::CreateDirectory { path } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;
                fs::create_dir_all(&dest_path)
                    .with_context(|| format!("Failed to create directory: {}", dest_path.display()))?;
                Ok(SyncStats::default())
            }
            FileOperation::Create { path } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let file_size = fs::metadata(&path)?.len();
                
                if options.verbose {
                    // Use RoboCopy-style output: "    New File                   123  filename"
                    println!("    New File        {:>12}  {}", file_size, dest_path.display());
                }
                
                fs::copy(&path, &dest_path)
                    .with_context(|| format!("Failed to copy file: {} -> {}", path.display(), dest_path.display()))?;
                
                stats.add_bytes_transferred(file_size);
                Ok(SyncStats { bytes_transferred: file_size, ..Default::default() })
            }
            FileOperation::Update { path, use_delta } => {
                let dest_path = self.map_source_to_dest(&path, source_root, dest_root)?;
                let file_size = fs::metadata(&path)?.len();
                
                if options.verbose {
                    if use_delta {
                        println!("    Newer           {:>12}  {}", file_size, dest_path.display());
                    } else {
                        println!("    Older           {:>12}  {}", file_size, dest_path.display());
                    }
                }
                
                if use_delta {
                    let file_stats = self.sync_file_pair(&path, &dest_path)?;
                    stats.add_bytes_transferred(file_stats.bytes_transferred);
                    Ok(file_stats)
                } else {
                    fs::copy(&path, &dest_path)
                        .with_context(|| format!("Failed to copy file: {} -> {}", path.display(), dest_path.display()))?;
                    stats.add_bytes_transferred(file_size);
                    Ok(SyncStats { bytes_transferred: file_size, ..Default::default() })
                }
            }
            FileOperation::Delete { path } => {
                if options.verbose {
                    if path.is_file() {
                        let file_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        println!("    *EXTRA File     {:>12}  {}", file_size, path.display());
                    } else {
                        println!("    *EXTRA Dir                   {}", path.display());
                    }
                }
                
                if path.is_file() {
                    fs::remove_file(&path)
                        .with_context(|| format!("Failed to delete file: {}", path.display()))?;
                } else if path.is_dir() {
                    fs::remove_dir_all(&path)
                        .with_context(|| format!("Failed to delete directory: {}", path.display()))?;
                }
                Ok(SyncStats::default())
            }
        }
    }

    /// Synchronize a single file pair using parallel block processing
    fn sync_file_pair(&self, source: &Path, destination: &Path) -> Result<SyncStats> {
        let source_data = fs::read(source)
            .with_context(|| format!("Failed to read source file: {}", source.display()))?;

        if !destination.exists() {
            // New file, just copy
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create parent directory: {}", parent.display()))?;
            }

            fs::write(destination, &source_data)
                .with_context(|| format!("Failed to write destination file: {}", destination.display()))?;

            return Ok(SyncStats {
                bytes_transferred: source_data.len() as u64,
                ..Default::default()
            });
        }

        // Existing file, use parallel delta algorithm
        let dest_data = fs::read(destination)
            .with_context(|| format!("Failed to read destination file: {}", destination.display()))?;

        let algorithm = DeltaAlgorithm::new(self.config.block_size);

        // Generate checksums in parallel
        let checksums = self.parallel_generate_checksums(&algorithm, &dest_data)?;

        // Find matches
        let matches = algorithm.find_matches(&source_data, &checksums)
            .context("Failed to find matches")?;

        // Apply delta to reconstruct file
        let new_data = self.apply_delta(&dest_data, &matches)?;

        // Write updated file
        fs::write(destination, &new_data)
            .with_context(|| format!("Failed to write updated file: {}", destination.display()))?;

        // Calculate transfer statistics
        let literal_bytes: u64 = matches.iter()
            .filter_map(|m| match m {
                Match::Literal { data, .. } => Some(data.len() as u64),
                _ => None,
            })
            .sum();

        Ok(SyncStats {
            bytes_transferred: literal_bytes,
            ..Default::default()
        })
    }

    /// Generate checksums in parallel using Rayon
    fn parallel_generate_checksums(
        &self,
        algorithm: &DeltaAlgorithm,
        data: &[u8],
    ) -> Result<Vec<BlockChecksum>> {
        let block_size = self.config.block_size;
        
        // Split data into chunks for parallel processing
        let chunks: Vec<(usize, &[u8])> = data
            .chunks(block_size)
            .enumerate()
            .collect();

        // Process chunks in parallel
        let checksums: Result<Vec<_>, _> = chunks
            .par_iter()
            .map(|(index, block)| {
                algorithm.generate_checksums(block)
                    .map(|mut block_checksums| {
                        // Adjust offsets for the chunk position
                        for checksum in &mut block_checksums {
                            checksum.offset = (index * block_size) as u64;
                        }
                        block_checksums
                    })
            })
            .collect();

        // Flatten results
        Ok(checksums?.into_iter().flatten().collect())
    }

    /// Apply delta matches to reconstruct a file
    fn apply_delta(&self, dest_data: &[u8], matches: &[Match]) -> Result<Vec<u8>> {
        let mut result = Vec::new();

        for match_item in matches {
            match match_item {
                Match::Literal { data, .. } => {
                    result.extend_from_slice(data);
                }
                Match::Block { target_offset, length, .. } => {
                    let start = *target_offset as usize;
                    let end = start + length;
                    if end <= dest_data.len() {
                        result.extend_from_slice(&dest_data[start..end]);
                    } else {
                        return Err(anyhow::anyhow!("Block match extends beyond destination data"));
                    }
                }
            }
        }

        Ok(result)
    }

    /// Map a source path to the corresponding destination path
    fn map_source_to_dest(&self, source_file: &Path, source_root: &Path, dest_root: &Path) -> Result<PathBuf> {
        let relative = source_file.strip_prefix(source_root)
            .with_context(|| format!("File {} is not under source root {}", source_file.display(), source_root.display()))?;
        Ok(dest_root.join(relative))
    }

    /// Find files/directories in destination that should be purged (deleted)
    fn find_purge_operations(
        &self,
        source_files: &[FileInfo],
        dest_files: &[FileInfo],
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<Vec<FileOperation>> {
        use std::collections::HashSet;
        
        // Create a set of all source file paths (relative to source root)
        let source_paths: HashSet<PathBuf> = source_files
            .iter()
            .filter_map(|f| f.path.strip_prefix(source_root).ok())
            .map(|p| p.to_path_buf())
            .collect();
        
        // Find destination files that don't exist in source
        let mut purge_ops = Vec::new();
        for dest_file in dest_files {
            if let Ok(relative_path) = dest_file.path.strip_prefix(dest_root) {
                if !source_paths.contains(relative_path) {
                    purge_ops.push(FileOperation::Delete {
                        path: dest_file.path.clone(),
                    });
                }
            }
        }
        
        // Sort purge operations: files first, then directories (deepest first)
        purge_ops.sort_by(|a, b| {
            if let (FileOperation::Delete { path: path_a }, FileOperation::Delete { path: path_b }) = (a, b) {
                // Files before directories
                match (path_a.is_file(), path_b.is_file()) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => {
                        // For directories, sort by depth (deepest first)
                        let depth_a = path_a.components().count();
                        let depth_b = path_b.components().count();
                        depth_b.cmp(&depth_a)
                    }
                }
            } else {
                std::cmp::Ordering::Equal
            }
        });
        
        Ok(purge_ops)
    }
}

/// Statistics for synchronization performance
#[derive(Debug, Default)]
pub struct SyncStats {
    pub files_processed: AtomicU64,
    pub bytes_transferred: u64,
    pub blocks_matched: AtomicU64,
    pub elapsed_time: std::time::Duration,
}

impl SyncStats {
    pub fn new() -> Self {
        Self {
            files_processed: AtomicU64::new(0),
            bytes_transferred: 0,
            blocks_matched: AtomicU64::new(0),
            elapsed_time: std::time::Duration::from_secs(0),
        }
    }

    pub fn add_bytes_transferred(&self, bytes: u64) {
        // Note: In a real implementation, we'd need atomic operations for bytes_transferred too
        // For now, this is a simplified version
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parallel_sync_config() {
        let config = ParallelSyncConfig::default();
        assert!(config.worker_threads > 0);
        assert!(config.io_threads > 0);
        assert!(config.block_size > 0);
    }

    #[test]
    fn test_parallel_syncer_creation() {
        let config = ParallelSyncConfig::default();
        let syncer = ParallelSyncer::new(config);
        assert_eq!(syncer.config.worker_threads, std::thread::available_parallelism().unwrap().get());
    }

    #[test]
    fn test_map_source_to_dest() -> Result<()> {
        let config = ParallelSyncConfig::default();
        let syncer = ParallelSyncer::new(config);
        
        let source_root = Path::new("/source");
        let dest_root = Path::new("/dest");
        let source_file = Path::new("/source/subdir/file.txt");
        
        let result = syncer.map_source_to_dest(source_file, source_root, dest_root)?;
        assert_eq!(result, Path::new("/dest/subdir/file.txt"));
        
        Ok(())
    }
}