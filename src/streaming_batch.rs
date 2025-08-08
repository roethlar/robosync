//! Streaming batch mode for efficient small file transfers
//! 
//! This module implements the tar streaming approach to solve the small file
//! overhead problem by batching many small files into a single stream.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use anyhow::{Result, Context};
use tar::{Builder, Archive};
use walkdir::WalkDir;
use indicatif::{ProgressBar, ProgressStyle};
use crate::sync_stats::SyncStats;
use crate::options::SyncOptions;

/// Threshold for determining when to use batch mode
pub const BATCH_MODE_FILE_SIZE_THRESHOLD: u64 = 10_240; // 10KB
pub const BATCH_MODE_FILE_COUNT_THRESHOLD: usize = 15;  // Reduced from 100 to match sample size

/// Configuration for streaming batch mode
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// Maximum number of files per batch
    pub batch_size: usize,
    /// Whether to compress the tar stream
    pub compress: bool,
    /// File size threshold for batching
    pub size_threshold: u64,
}

impl Default for BatchConfig {
    fn default() -> Self {
        BatchConfig {
            batch_size: 10_000,
            compress: false,
            size_threshold: BATCH_MODE_FILE_SIZE_THRESHOLD,
        }
    }
}

/// Profile of a directory for strategy selection
#[derive(Debug)]
pub struct WorkloadProfile {
    pub file_count: usize,
    pub avg_file_size: u64,
    pub total_size: u64,
    pub sample_size: usize,
}

/// Sample a directory to determine if batch mode should be used
pub fn sample_directory(path: &Path, sample_size: usize) -> Result<WorkloadProfile> {
    let start = std::time::Instant::now();
    let mut file_count = 0;
    let mut total_size = 0u64;
    
    // Use read_dir for lazy iteration (don't scan entire directory)
    for entry in fs::read_dir(path)?.take(sample_size) {
        let entry = entry?;
        let metadata = entry.metadata()?;
        
        if metadata.is_file() {
            file_count += 1;
            total_size += metadata.len();
        }
    }
    
    // Ensure we don't divide by zero
    let avg_file_size = if file_count > 0 {
        total_size / file_count as u64
    } else {
        0
    };
    
    let elapsed = start.elapsed();
    if elapsed.as_millis() > 10 {
        eprintln!("Warning: Directory sampling took {}ms (target: <10ms)", elapsed.as_millis());
    }
    
    Ok(WorkloadProfile {
        file_count,
        avg_file_size,
        total_size,
        sample_size,
    })
}

/// Determine if batch mode should be used based on workload profile
pub fn should_use_batch_mode(profile: &WorkloadProfile, options: &SyncOptions) -> bool {
    // Check if batch mode is disabled
    if options.no_batch {
        return false;
    }
    
    // Use batch mode for many small files
    profile.file_count >= BATCH_MODE_FILE_COUNT_THRESHOLD 
        && profile.avg_file_size < BATCH_MODE_FILE_SIZE_THRESHOLD
}

/// Stream files through tar without creating intermediate archive
pub fn streaming_batch_transfer(
    source: &Path,
    dest: &Path,
    _config: &BatchConfig,
    stats: &SyncStats,
    options: &SyncOptions,
) -> Result<SyncStats> {
    // Debug logging removed - use progress bar instead
    
    // Create a channel for streaming with larger buffer to reduce blocking
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(64);
    
    // Progress bar for user feedback
    let progress = if options.show_progress {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} [{elapsed_precise}] {msg} {prefix}")
                .unwrap()
        );
        pb.set_message("Analyzing files for tar streaming...");
        Some(pb)
    } else {
        None
    };
    
    // Clone paths for thread usage
    let source_path = source.to_path_buf();
    let dest_path = dest.to_path_buf();
    let progress_clone = progress.clone();
    
    // Thread 1: Create tar stream from source files
    let packer_handle = thread::spawn(move || -> Result<(u64, u64)> {
        // Use a writer that sends chunks through the channel
        let mut stream_writer = ChannelWriter::new(tx);
        let mut file_count = 0u64;
        let mut total_bytes = 0u64;
        
        {
            let mut builder = Builder::new(&mut stream_writer);
            
            // Walk source directory and add files to tar
            for entry in WalkDir::new(&source_path)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.is_file() {
                    // Get relative path for tar entry
                    let relative = path.strip_prefix(&source_path)
                        .unwrap_or(path);
                    
                    // Get file size for stats
                    if let Ok(metadata) = path.metadata() {
                        total_bytes += metadata.len();
                    }
                    
                    // Add file to tar - this writes directly to the channel
                    builder.append_path_with_name(path, relative)
                        .with_context(|| format!("Failed to add {:?} to tar", path))?;
                    
                    file_count += 1;
                    
                    // Update progress every 100 files
                    if file_count % 100 == 0 {
                        if let Some(ref pb) = progress_clone {
                            pb.set_message(format!("Streaming {} files ({:.1} MB)...", 
                                                  file_count, 
                                                  total_bytes as f64 / 1_048_576.0));
                        }
                    }
                }
            }
            
            // Finish the tar archive properly
            builder.finish()?;
        }
        
        // Flush any remaining data
        stream_writer.flush()?;
        
        Ok((file_count, total_bytes))
    });
    
    // Thread 2: Extract tar stream at destination
    let unpacker_handle = thread::spawn(move || -> Result<()> {
        // Ensure destination exists
        fs::create_dir_all(&dest_path)?;
        
        // Create a reader from received chunks
        let reader = ChannelReader::new(rx);
        let mut archive = Archive::new(reader);
        
        // Extract files
        archive.unpack(&dest_path)
            .with_context(|| format!("Failed to extract tar to {:?}", dest_path))?;
        
        Ok(())
    });
    
    // Wait for both threads to complete
    let (file_count, total_bytes) = packer_handle.join()
        .map_err(|e| anyhow::anyhow!("Packer thread panicked: {:?}", e))??;
    
    unpacker_handle.join()
        .map_err(|e| anyhow::anyhow!("Unpacker thread panicked: {:?}", e))??;
    
    if let Some(pb) = progress {
        pb.finish_with_message(format!("Tar streaming complete: {} files", file_count));
    }
    
    // Update stats with actual transfer data
    for _ in 0..file_count {
        stats.increment_files_copied();
    }
    stats.add_bytes_transferred(total_bytes);
    
    // Log tar streaming summary if verbose
    if options.verbose >= 1 {
        println!("📦 Tar streaming completed: {} files ({:.2} MB) transferred", 
                 file_count, total_bytes as f64 / 1_048_576.0);
    }
    
    // Transfer completed - progress bar handles notification
    Ok(stats.clone())
}

/// Writer that sends data through a channel
pub struct ChannelWriter {
    tx: mpsc::SyncSender<Vec<u8>>,
    buffer: Vec<u8>,
    chunk_size: usize,
}

impl ChannelWriter {
    pub fn new(tx: mpsc::SyncSender<Vec<u8>>) -> Self {
        ChannelWriter {
            tx,
            buffer: Vec::with_capacity(262144), // 256KB buffer for better throughput
            chunk_size: 262144,
        }
    }
    
    fn send_buffer(&mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            // Use std::mem::take to avoid cloning - this is the key optimization!
            let data = std::mem::take(&mut self.buffer);
            self.tx.send(data)
                .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
            self.buffer = Vec::with_capacity(self.chunk_size);
        }
        Ok(())
    }
}

impl io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        
        // Send when buffer reaches chunk size
        if self.buffer.len() >= self.chunk_size {
            self.send_buffer()?;
        }
        
        Ok(buf.len())
    }
    
    fn flush(&mut self) -> io::Result<()> {
        self.send_buffer()
    }
}

/// Reader that consumes from a channel
pub struct ChannelReader {
    rx: mpsc::Receiver<Vec<u8>>,
    current_chunk: Vec<u8>,
    position: usize,
}

impl ChannelReader {
    pub fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        ChannelReader {
            rx,
            current_chunk: Vec::new(),
            position: 0,
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // If current chunk is exhausted, get next one
        if self.position >= self.current_chunk.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.current_chunk = chunk;
                    self.position = 0;
                }
                Err(_) => {
                    // Channel closed, no more data
                    return Ok(0);
                }
            }
        }
        
        // Read from current chunk
        let available = self.current_chunk.len() - self.position;
        let to_read = buf.len().min(available);
        
        if to_read > 0 {
            buf[..to_read].copy_from_slice(
                &self.current_chunk[self.position..self.position + to_read]
            );
            self.position += to_read;
        }
        
        Ok(to_read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs::File;
    use std::io::Write;
    
    #[test]
    fn test_workload_profiling() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();
        
        // Create some small files
        for i in 0..10 {
            let file_path = temp_path.join(format!("file_{}.txt", i));
            let mut file = File::create(file_path).unwrap();
            file.write_all(b"small content").unwrap();
        }
        
        let profile = sample_directory(temp_path, 100).unwrap();
        
        assert_eq!(profile.file_count, 10);
        assert!(profile.avg_file_size < 1000);
    }
    
    #[test]
    fn test_should_use_batch_mode() {
        let profile = WorkloadProfile {
            file_count: 200,
            avg_file_size: 5_000,
            total_size: 1_000_000,
            sample_size: 100,
        };
        
        let mut options = SyncOptions::default();
        options.no_batch = false;
        
        assert!(should_use_batch_mode(&profile, &options));
        
        // Test with batch disabled
        options.no_batch = true;
        assert!(!should_use_batch_mode(&profile, &options));
    }
    
    #[test]
    fn test_streaming_batch_transfer() {
        let source_dir = TempDir::new().unwrap();
        let dest_dir = TempDir::new().unwrap();
        
        // Create test files
        for i in 0..5 {
            let file_path = source_dir.path().join(format!("test_{}.txt", i));
            let mut file = File::create(file_path).unwrap();
            writeln!(file, "Test content {}", i).unwrap();
        }
        
        let config = BatchConfig::default();
        let stats = SyncStats::new();
        let options = SyncOptions::default();
        
        let _result = streaming_batch_transfer(
            source_dir.path(),
            dest_dir.path(),
            &config,
            &stats,
            &options,
        ).unwrap();
        
        // Verify files were transferred
        for i in 0..5 {
            let dest_file = dest_dir.path().join(format!("test_{}.txt", i));
            assert!(dest_file.exists());
            
            let content = fs::read_to_string(dest_file).unwrap();
            assert_eq!(content.trim(), format!("Test content {}", i));
        }
    }
}