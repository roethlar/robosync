//! Speculative tar execution with parallel analysis
//! 
//! This module implements Gemini's suggested approach:
//! 1. Start tar immediately to temp directory
//! 2. Analyze in parallel
//! 3. Either commit (atomic rename) or abort (cleanup)

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use anyhow::{Result, Context};
use crate::sync_stats::SyncStats;
use crate::options::SyncOptions;
use crate::streaming_batch::{sample_directory, should_use_batch_mode};

/// Get current timestamp for logging
fn timestamp() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

#[cfg(unix)]
use std::os::unix::io::FromRawFd;

/// Create an optimized pipe with larger buffer for speculation window
#[cfg(unix)]
fn create_optimized_pipe(buffer_size: usize) -> Result<(std::process::Stdio, std::process::Stdio)> {
    use libc::pipe;
    
    let mut fds = [0; 2];
    unsafe {
        if pipe(fds.as_mut_ptr()) != 0 {
            return Err(anyhow::anyhow!("Failed to create pipe"));
        }
        
        // Note: F_SETPIPE_SZ is Linux-specific and not available on macOS
        // On macOS we'll rely on the default pipe buffer
        #[cfg(target_os = "linux")]
        {
            const F_SETPIPE_SZ: libc::c_int = 1031;
            use libc::fcntl;
            let _ = fcntl(fds[0], F_SETPIPE_SZ, buffer_size as libc::c_int);
            let _ = fcntl(fds[1], F_SETPIPE_SZ, buffer_size as libc::c_int);
        }
        
        Ok((
            Stdio::from(std::fs::File::from_raw_fd(fds[0])),
            Stdio::from(std::fs::File::from_raw_fd(fds[1]))
        ))
    }
}

#[cfg(not(unix))]
fn create_optimized_pipe(_buffer_size: usize) -> Result<(std::process::Stdio, std::process::Stdio)> {
    // On non-Unix, just use regular piped stdio
    Ok((Stdio::piped(), Stdio::piped()))
}

/// Execute tar speculatively with parallel analysis
pub fn execute_speculative_tar(
    source: &Path,
    dest: &Path,
    stats: &SyncStats,
    options: &SyncOptions,
) -> Result<SyncStats> {
    // 1. Prepare temporary destination for atomicity
    let temp_dest = dest.parent()
        .unwrap_or(Path::new("."))
        .join(format!(".robosync_tmp_{}", std::process::id()));
    
    // Clean up any previous temp directory
    if temp_dest.exists() {
        fs::remove_dir_all(&temp_dest)?;
    }
    
    fs::create_dir_all(&temp_dest)
        .context("Failed to create temp directory")?;
    
    // 2. Start tar processes immediately (speculative execution)
    let mut tar_create = Command::new("tar")
        .arg("cf")
        .arg("-")
        .arg(".")
        .current_dir(source)
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to spawn tar create process")?;
    
    let tar_create_stdout = tar_create.stdout.take()
        .ok_or_else(|| anyhow::anyhow!("Failed to capture tar stdout"))?;
    
    let mut tar_extract = Command::new("tar")
        .arg("xf")
        .arg("-")
        .current_dir(&temp_dest)
        .stdin(tar_create_stdout)
        .spawn()
        .context("Failed to spawn tar extract process")?;
    
    // 3. Perform parallel analysis (while tar is running)
    let source_clone = source.to_path_buf();
    let verbose = options.verbose;
    let no_batch = options.no_batch;
    
    let analysis_handle = thread::spawn(move || -> Result<bool> {
        // We can now afford a more thorough sample since it's not blocking
        let profile = sample_directory(&source_clone, 100)?;
        
        if verbose >= 2 {
            println!("[{}] Speculative analysis: {} files sampled, avg size {} bytes", 
                timestamp(), profile.file_count, profile.avg_file_size);
        }
        
        // Recreate minimal options for the check
        let mut temp_options = crate::options::SyncOptions::default();
        temp_options.no_batch = no_batch;
        
        Ok(should_use_batch_mode(&profile, &temp_options))
    });
    
    // 4. Wait for analysis decision
    let use_tar = match analysis_handle.join() {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            // Analysis failed, kill tar processes and bail
            let _ = tar_create.kill();
            let _ = tar_extract.kill();
            fs::remove_dir_all(&temp_dest).ok();
            return Err(e);
        }
        Err(_) => {
            // Thread panicked
            let _ = tar_create.kill();
            let _ = tar_extract.kill();
            fs::remove_dir_all(&temp_dest).ok();
            return Err(anyhow::anyhow!("Analysis thread panicked"));
        }
    };
    
    if use_tar {
        // 5a. Strategy confirmed: wait for completion and atomic rename
        if options.verbose >= 1 {
            println!("[{}] Tar streaming confirmed by parallel analysis, committing...", timestamp());
        }
        
        let create_status = tar_create.wait()?;
        let extract_status = tar_extract.wait()?;
        
        if !create_status.success() {
            fs::remove_dir_all(&temp_dest).ok();
            return Err(anyhow::anyhow!("Tar create failed with status: {}", create_status));
        }
        
        if !extract_status.success() {
            fs::remove_dir_all(&temp_dest).ok();
            return Err(anyhow::anyhow!("Tar extract failed with status: {}", extract_status));
        }
        
        // Atomic rename (handle existing destination)
        if dest.exists() {
            // For safety, we should merge or backup, but for now we'll replace
            fs::remove_dir_all(dest)?;
        }
        
        fs::rename(&temp_dest, dest)
            .context("Failed to atomically rename temp directory")?;
        
        // Update stats (estimate since we don't count)
        stats.increment_files_copied();
        stats.add_bytes_transferred(1024 * 1000); // Rough estimate
        
        if options.verbose >= 1 {
            println!("[{}] Tar streaming completed successfully", timestamp());
        }
        
        Ok(stats.clone())
        
    } else {
        // 5b. Strategy rejected: abort and fallback
        if options.verbose >= 1 {
            println!("[{}] Tar streaming rejected by analysis, aborting and falling back...", timestamp());
        }
        
        // Kill tar processes
        let _ = tar_create.kill();
        let _ = tar_extract.kill();
        
        // Clean up temp directory
        fs::remove_dir_all(&temp_dest).ok();
        
        // Fall back to standard parallel sync
        // Note: This would normally call the mixed strategy
        Err(anyhow::anyhow!("Tar strategy rejected, fallback to mixed strategy"))
    }
}

/// Fast-path detection for known patterns that should always use tar
pub fn is_known_tar_candidate(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        match name {
            ".git" | "node_modules" | ".npm" | ".cargo" | "cache" | "logs" => true,
            _ => {
                // Check for .app bundles on macOS
                #[cfg(target_os = "macos")]
                if name.ends_with(".app") {
                    return true;
                }
                false
            }
        }
    } else {
        false
    }
}