//! Native tool wrappers for rsync and robocopy
//!
//! This module provides a unified interface to native file copying tools,
//! handling their execution, progress parsing, and error handling.

use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;

use crate::progress::{SyncProgress, ToolType};
use crate::sync_stats::SyncStats;

/// Wrapper for native rsync command
pub struct RsyncWrapper {
    extra_args: Vec<String>,
    progress_manager: Option<Arc<SyncProgress>>,
}

impl RsyncWrapper {
    pub fn new(extra_args: Vec<String>) -> Self {
        Self {
            extra_args,
            progress_manager: None,
        }
    }

    pub fn with_progress(mut self, progress_manager: Arc<SyncProgress>) -> Self {
        self.progress_manager = Some(progress_manager);
        self
    }

    /// Execute rsync with the given source and destination
    pub fn execute(&self, source: &Path, destination: &Path) -> Result<SyncStats> {
        let mut cmd = Command::new("rsync");

        // Add our extra arguments
        for arg in &self.extra_args {
            cmd.arg(arg);
        }

        // Add progress flag if we have a progress tracker
        if self.progress_manager.is_some() {
            cmd.arg("--info=progress2");
        }

        // Add source and destination
        // Ensure trailing slash on source for consistent behavior
        let source_str = if source.is_dir() {
            format!("{}/", source.display())
        } else {
            source.display().to_string()
        };

        cmd.arg(&source_str);
        cmd.arg(destination);

        // Set up for output capture
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().context("Failed to execute rsync")?;

        let stats = Arc::new(SyncStats::default());

        // Spawn thread to parse progress output
        if let Some(stdout) = child.stdout.take() {
            if let Some(ref progress_manager) = self.progress_manager {
                let progress_manager = Arc::clone(progress_manager);
                thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            progress_manager.update_from_tool_output(&line, ToolType::Rsync);
                        }
                    }
                });
            } else {
                // No progress manager, just consume output
                thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for _ in reader.lines() {}
                });
            }
        }

        // Wait for completion
        let status = child.wait().context("Failed to wait for rsync")?;

        if !status.success() {
            bail!("rsync failed with status: {}", status);
        }

        // Return the stats
        Ok(Arc::try_unwrap(stats).unwrap_or_else(|arc| (*arc).clone()))
    }
}

/// Wrapper for native robocopy command (Windows)
#[cfg(target_os = "windows")]
pub struct RobocopyWrapper {
    extra_args: Vec<String>,
    progress_manager: Option<Arc<SyncProgress>>,
}

#[cfg(target_os = "windows")]
impl RobocopyWrapper {
    pub fn new(extra_args: Vec<String>) -> Self {
        Self {
            extra_args,
            progress_manager: None,
        }
    }

    pub fn with_progress(mut self, progress_manager: Arc<SyncProgress>) -> Self {
        self.progress_manager = Some(progress_manager);
        self
    }

    /// Execute robocopy with the given source and destination
    pub fn execute(&self, source: &Path, destination: &Path) -> Result<SyncStats> {
        let mut cmd = Command::new("robocopy");

        // Add source and destination
        cmd.arg(source);
        cmd.arg(destination);

        // Add our extra arguments
        for arg in &self.extra_args {
            cmd.arg(arg);
        }

        // Add progress flag
        cmd.arg("/NP"); // No progress in output (we'll track separately)
        cmd.arg("/BYTES"); // Show sizes in bytes

        // Set up for output capture
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().context("Failed to execute robocopy")?;

        let stats = Arc::new(SyncStats::default());
        let stats_clone = Arc::clone(&stats);

        // Spawn thread to parse output
        if let Some(stdout) = child.stdout.take() {
            if let Some(ref progress_manager) = self.progress_manager {
                let progress_manager = Arc::clone(progress_manager);
                thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            progress_manager.update_from_tool_output(&line, ToolType::Robocopy);
                            // Also update stats
                            Self::parse_file_line(&line, &stats_clone);
                        }
                    }
                });
            } else {
                // No progress manager, just parse for stats
                thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            Self::parse_file_line(&line, &stats_clone);
                        }
                    }
                });
            }
        }

        // Wait for completion
        let status = child.wait().context("Failed to wait for robocopy")?;

        // Robocopy exit codes:
        // 0 = No files copied
        // 1 = Files copied successfully
        // 2 = Extra files/dirs in destination (only with /MIR)
        // 4 = Mismatched files/dirs
        // 8 = Copy errors occurred
        // 16 = Fatal error

        match status.code() {
            Some(0) | Some(1) | Some(2) | Some(3) => {
                // Success cases (0-3 are all non-error states)
            }
            Some(code) if code >= 8 => {
                bail!("robocopy failed with error code: {}", code);
            }
            _ => {
                bail!("robocopy failed with unknown status");
            }
        }

        // Return the stats
        Ok(Arc::try_unwrap(stats).unwrap_or_else(|arc| (*arc).clone()))
    }

    /// Parse robocopy file copy lines
    fn parse_file_line(line: &str, stats: &Arc<SyncStats>) -> bool {
        // Robocopy output examples:
        // "      New File              123456    filename.txt"
        // "      Newer                 654321    updated.doc"

        if line.contains("New File") || line.contains("Newer") || line.contains("Older") {
            // Try to extract file size
            let parts: Vec<&str> = line.split_whitespace().collect();
            for part in &parts {
                if let Ok(size) = part.parse::<u64>() {
                    stats.add_bytes_transferred(size);
                    return true;
                }
            }
        }

        false
    }
}

/// Wrapper for executing native tools and capturing output
pub struct NativeToolExecutor {
    dry_run: bool,
}

impl NativeToolExecutor {
    pub fn new(dry_run: bool) -> Self {
        Self { dry_run }
    }

    /// Execute rsync with given options, with fallback
    #[cfg(unix)]
    pub fn run_rsync(
        &self,
        source: &Path,
        destination: &Path,
        args: Vec<String>,
        progress_manager: Option<Arc<SyncProgress>>,
    ) -> Result<SyncStats> {
        if self.dry_run {
            println!(
                "Would execute: rsync {} {} {}",
                args.join(" "),
                source.display(),
                destination.display()
            );
            return Ok(SyncStats::default());
        }

        // Check if rsync is available
        if !Self::is_rsync_available() {
            eprintln!("Warning: rsync not found, falling back to built-in implementation");
            return self.fallback_to_builtin(source, destination, progress_manager.clone());
        }

        let mut wrapper = RsyncWrapper::new(args);
        let progress_manager_clone = progress_manager.clone();
        if let Some(pm) = progress_manager {
            wrapper = wrapper.with_progress(pm);
        }

        match wrapper.execute(source, destination) {
            Ok(stats) => Ok(stats),
            Err(e) => {
                eprintln!(
                    "Warning: rsync failed ({e}), falling back to built-in implementation"
                );
                self.fallback_to_builtin(source, destination, progress_manager_clone)
            }
        }
    }

    /// Execute robocopy with given options, with fallback
    #[cfg(target_os = "windows")]
    pub fn run_robocopy(
        &self,
        source: &Path,
        destination: &Path,
        args: Vec<String>,
        progress_manager: Option<Arc<SyncProgress>>,
    ) -> Result<SyncStats> {
        if self.dry_run {
            println!(
                "Would execute: robocopy {} {} {}",
                source.display(),
                destination.display(),
                args.join(" ")
            );
            return Ok(SyncStats::default());
        }

        // Check if robocopy is available
        if !Self::is_robocopy_available() {
            eprintln!("Warning: robocopy not found, falling back to built-in implementation");
            return self.fallback_to_builtin(source, destination, progress_manager.clone());
        }

        let mut wrapper = RobocopyWrapper::new(args);
        let progress_manager_clone = progress_manager.clone();
        if let Some(pm) = progress_manager {
            wrapper = wrapper.with_progress(pm);
        }

        match wrapper.execute(source, destination) {
            Ok(stats) => Ok(stats),
            Err(e) => {
                eprintln!(
                    "Warning: robocopy failed ({e}), falling back to built-in implementation"
                );
                self.fallback_to_builtin(source, destination, progress_manager_clone)
            }
        }
    }

    /// Check if rsync is available
    #[cfg(unix)]
    pub fn is_rsync_available() -> bool {
        Command::new("rsync")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Check if robocopy is available
    #[cfg(target_os = "windows")]
    pub fn is_robocopy_available() -> bool {
        Command::new("robocopy")
            .arg("/?")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Fallback to built-in implementation when native tools are unavailable
    fn fallback_to_builtin(
        &self,
        source: &Path,
        destination: &Path,
        progress_manager: Option<Arc<SyncProgress>>,
    ) -> Result<SyncStats> {
        use crate::options::SyncOptions;
        use crate::parallel_sync::{ParallelSyncConfig, ParallelSyncer};

        println!("Using built-in RoboSync implementation...");

        // Create default options
        let options = SyncOptions {
            dry_run: self.dry_run,
            no_progress: progress_manager.is_none(),
            ..Default::default()
        };

        // Use our parallel sync implementation
        let config = ParallelSyncConfig::default();
        let syncer = ParallelSyncer::new(config);

        // Create a simple wrapper that uses our progress manager if available
        if let Some(pm) = progress_manager {
            // The parallel syncer will handle its own progress, but we can monitor
            pm.set_current_file("Running built-in sync implementation");
        }

        syncer.synchronize_with_options(source.to_path_buf(), destination.to_path_buf(), options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn test_rsync_available() {
        // This test might fail in minimal environments
        let available = NativeToolExecutor::is_rsync_available();
        println!("rsync available: {}", available);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_robocopy_available() {
        // Should always be true on Windows
        assert!(NativeToolExecutor::is_robocopy_available());
    }
}
