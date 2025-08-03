//! Linux-specific optimizations for copying thousands of small files

use anyhow::Result;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use io_uring::{opcode, types, IoUring};

/// Threshold for what we consider a "small file"
const SMALL_FILE_THRESHOLD: usize = 64 * 1024; // 64KB

/// Batch size for io_uring operations
pub const IO_URING_BATCH_SIZE: usize = 256;

/// Buffer pool for small file operations
pub struct SmallFileBuffer {
    buffers: Vec<Vec<u8>>,
    next_buffer: usize,
}

impl SmallFileBuffer {
    pub fn new(count: usize) -> Self {
        let mut buffers = Vec::with_capacity(count);
        for _ in 0..count {
            buffers.push(vec![0u8; SMALL_FILE_THRESHOLD]);
        }
        Self {
            buffers,
            next_buffer: 0,
        }
    }

    pub fn get_buffer(&mut self) -> &mut [u8] {
        let current = self.next_buffer;
        let total = self.buffers.len();
        self.next_buffer = (current + 1) % total;
        &mut self.buffers[current]
    }
}

/// Fast copy optimized for small files on Linux
#[cfg(target_os = "linux")]
pub fn copy_small_files_batch(files: &[(PathBuf, PathBuf)]) -> Result<u64> {
    let mut total_bytes = 0u64;
    let mut ring = IoUring::builder()
        .setup_sqpoll(1000) // Use kernel polling thread
        .build(IO_URING_BATCH_SIZE as u32)?;

    // Process files in batches
    for batch in files.chunks(IO_URING_BATCH_SIZE) {
        let batch_bytes = submit_batch_copy(&mut ring, batch)?;
        total_bytes += batch_bytes;
    }

    Ok(total_bytes)
}

#[cfg(target_os = "linux")]
fn submit_batch_copy(ring: &mut IoUring, files: &[(PathBuf, PathBuf)]) -> Result<u64> {
    use std::os::unix::io::AsRawFd;

    let mut total_bytes = 0u64;
    let mut file_handles = Vec::new();
    let mut buffers = SmallFileBuffer::new(files.len());

    // Open all files and submit read operations
    for (idx, (src, dst)) in files.iter().enumerate() {
        // Open source file
        let src_file = match fs::File::open(src) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Failed to open source {src:?}: {e}");
                continue;
            }
        };

        let metadata = match src_file.metadata() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to get metadata for {src:?}: {e}");
                continue;
            }
        };

        let file_size = metadata.len();
        if file_size > SMALL_FILE_THRESHOLD as u64 {
            // Fall back to regular copy for large files
            match fs::copy(src, dst) {
                Ok(bytes) => total_bytes += bytes,
                Err(e) => eprintln!("Failed to copy large file {src:?}: {e}"),
            }
            continue;
        }

        // Create parent directory if needed
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }

        // Open destination file
        let dst_file = match fs::File::create(dst) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Failed to create destination {dst:?}: {e}");
                continue;
            }
        };

        let src_fd = src_file.as_raw_fd();
        let dst_fd = dst_file.as_raw_fd();

        // Get a buffer for this file
        let buffer = buffers.get_buffer();
        let buffer_ptr = buffer.as_mut_ptr();

        // Submit read operation
        let read_op = opcode::Read::new(types::Fd(src_fd), buffer_ptr, file_size as u32)
            .offset(0)
            .build()
            .user_data(idx as u64 * 2); // Even numbers for reads

        unsafe {
            ring.submission()
                .push(&read_op)
                .map_err(|e| anyhow::anyhow!("Failed to submit read: {}", e))?;
        }

        file_handles.push((
            src_file,
            dst_file,
            file_size,
            dst_fd,
            buffer_ptr,
            metadata.mode(),
        ));
    }

    // Submit the batch
    ring.submit_and_wait(file_handles.len())
        .map_err(|e| anyhow::anyhow!("Failed to submit batch: {}", e))?;

    // Process completions and submit writes
    let mut completed_reads = Vec::new();
    for _ in 0..file_handles.len() {
        let cqe = ring.completion().next().expect("completion queue entry");
        let idx = (cqe.user_data() / 2) as usize;

        if cqe.result() < 0 {
            eprintln!("Read failed for file {}: {}", idx, cqe.result());
            continue;
        }

        completed_reads.push((idx, cqe.result() as u32));
    }

    // Count writes to submit
    let num_writes = completed_reads.len();

    // Submit write operations for successful reads
    for (idx, bytes_read) in completed_reads {
        if let Some((_, _, _, dst_fd, buffer_ptr, _)) = file_handles.get(idx) {
            let write_op = opcode::Write::new(types::Fd(*dst_fd), *buffer_ptr, bytes_read)
                .offset(0)
                .build()
                .user_data(idx as u64 * 2 + 1); // Odd numbers for writes

            unsafe {
                ring.submission()
                    .push(&write_op)
                    .map_err(|e| anyhow::anyhow!("Failed to submit write: {}", e))?;
            }
        }
    }

    // Submit the writes
    ring.submit_and_wait(num_writes)
        .map_err(|e| anyhow::anyhow!("Failed to submit writes: {}", e))?;

    // Process write completions
    for _ in 0..num_writes {
        let cqe = ring.completion().next().expect("completion queue entry");
        let idx = ((cqe.user_data() - 1) / 2) as usize;

        if cqe.result() < 0 {
            eprintln!("Write failed for file {}: {}", idx, cqe.result());
            continue;
        }

        total_bytes += cqe.result() as u64;

        // Set permissions on destination file
        if let Some((_, dst_file, _, _, _, mode)) = file_handles.get(idx) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = dst_file.set_permissions(fs::Permissions::from_mode(*mode));
            }
        }
    }

    Ok(total_bytes)
}

/// Memory-mapped copy for small files
pub fn mmap_copy_small_file(src: &Path, dst: &Path) -> Result<u64> {
    use memmap2::MmapOptions;

    let file = fs::File::open(src)?;
    let metadata = file.metadata()?;
    let len = metadata.len() as usize;

    if len > SMALL_FILE_THRESHOLD {
        // Fall back to regular copy for larger files
        return Ok(fs::copy(src, dst)?);
    }

    // Memory map the source file
    let mmap = unsafe { MmapOptions::new().map(&file)? };

    // Write to destination in one syscall
    fs::write(dst, &mmap[..])?;

    // Copy metadata
    let dst_file = fs::OpenOptions::new().write(true).open(dst)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        dst_file.set_permissions(fs::Permissions::from_mode(metadata.mode()))?;
    }

    Ok(len as u64)
}

/// Parallel directory scanner using jwalk
pub fn scan_directory_parallel(path: &Path) -> Result<Vec<PathBuf>> {
    use jwalk::WalkDir;
    use rayon::prelude::*;

    let entries: Vec<PathBuf> = WalkDir::new(path)
        .parallelism(jwalk::Parallelism::RayonNewPool(num_cpus::get()))
        .into_iter()
        .par_bridge()
        .filter_map(|entry| match entry {
            Ok(e) => {
                let file_type = e.file_type();
                if file_type.is_file() {
                    Some(e.path().to_owned())
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();

    Ok(entries)
}

/// Batch copy operation for multiple small files
pub fn batch_copy_files(operations: Vec<(PathBuf, PathBuf)>) -> Result<BatchCopyStats> {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    let total_files = operations.len();
    let files_copied = AtomicU64::new(0);
    let bytes_copied = AtomicU64::new(0);
    let start = std::time::Instant::now();

    // Group by file size for optimal handling
    let (small_files, large_files): (Vec<_>, Vec<_>) =
        operations.into_par_iter().partition(|(src, _)| {
            fs::metadata(src)
                .map(|m| m.len() < SMALL_FILE_THRESHOLD as u64)
                .unwrap_or(false)
        });

    // Process small files with memory mapping
    small_files.par_chunks(100).for_each(|chunk| {
        for (src, dst) in chunk {
            // Create parent directory if needed
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent);
            }

            match mmap_copy_small_file(src, dst) {
                Ok(bytes) => {
                    files_copied.fetch_add(1, Ordering::Relaxed);
                    bytes_copied.fetch_add(bytes, Ordering::Relaxed);
                }
                Err(e) => eprintln!("Error copying {src:?}: {e}"),
            }
        }
    });

    // Process large files with regular copy
    large_files.par_iter().for_each(|(src, dst)| {
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }

        match fs::copy(src, dst) {
            Ok(bytes) => {
                files_copied.fetch_add(1, Ordering::Relaxed);
                bytes_copied.fetch_add(bytes, Ordering::Relaxed);
            }
            Err(e) => eprintln!("Error copying {src:?}: {e}"),
        }
    });

    let elapsed = start.elapsed();
    Ok(BatchCopyStats {
        total_files,
        files_copied: files_copied.load(Ordering::Relaxed),
        bytes_copied: bytes_copied.load(Ordering::Relaxed),
        elapsed,
    })
}

#[derive(Debug)]
pub struct BatchCopyStats {
    pub total_files: usize,
    pub files_copied: u64,
    pub bytes_copied: u64,
    pub elapsed: std::time::Duration,
}

impl BatchCopyStats {
    pub fn files_per_second(&self) -> f64 {
        self.files_copied as f64 / self.elapsed.as_secs_f64()
    }

    pub fn throughput_mb_per_sec(&self) -> f64 {
        (self.bytes_copied as f64 / 1_000_000.0) / self.elapsed.as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_small_file_buffer() {
        let mut buffer = SmallFileBuffer::new(4);
        let buf1 = buffer.get_buffer();
        assert_eq!(buf1.len(), SMALL_FILE_THRESHOLD);
    }
}
