//! Main synchronization logic

use crate::file_list::{
    compare_file_lists_with_roots, generate_file_list_with_options, FileOperation,
};
use crate::logging::SyncLogger;
use crate::network_fs::{NetworkFsDetector, NetworkFsType};
use crate::options::SyncOptions;
use crate::progress::SyncProgress;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Synchronize files from source to destination
pub fn synchronize(
    source: PathBuf,
    destination: PathBuf,
    _threads: usize,
    _compress: bool,
) -> Result<()> {
    // Create a logger without file output for basic sync
    let logger = SyncLogger::new(None, false, 0)?;

    logger.log("Starting synchronization...");
    logger.log(&format!("  Source: {}", source.display()));
    logger.log(&format!("  Destination: {}", destination.display()));

    // Detect network filesystem types for optimization
    let mut fs_detector = NetworkFsDetector::new();
    let src_fs_info = fs_detector.detect_filesystem(&source);
    let dst_fs_info = fs_detector.detect_filesystem(&destination);

    // Log filesystem information
    if src_fs_info.fs_type != NetworkFsType::Local {
        logger.log(&format!("  Source filesystem: {:?} ({})", 
            src_fs_info.fs_type, src_fs_info.mount_point));
        if let Some(server) = &src_fs_info.server {
            logger.log(&format!("    Server: {}", server));
        }
    }
    
    if dst_fs_info.fs_type != NetworkFsType::Local {
        logger.log(&format!("  Destination filesystem: {:?} ({})", 
            dst_fs_info.fs_type, dst_fs_info.mount_point));
        if let Some(server) = &dst_fs_info.server {
            logger.log(&format!("    Server: {}", server));
        }
    }

    // Provide optimization recommendations
    if src_fs_info.fs_type != NetworkFsType::Local || dst_fs_info.fs_type != NetworkFsType::Local {
        logger.log("Network filesystem detected - optimization recommendations:");
        
        if src_fs_info.fs_type != NetworkFsType::Local {
            let recommendations = fs_detector.get_optimization_recommendations(&src_fs_info);
            for rec in recommendations {
                logger.log(&format!("  Source: {}", rec));
            }
        }
        
        if dst_fs_info.fs_type != NetworkFsType::Local {
            let recommendations = fs_detector.get_optimization_recommendations(&dst_fs_info);
            for rec in recommendations {
                logger.log(&format!("  Destination: {}", rec));
            }
        }
    }

    // Create destination if it doesn't exist
    if !destination.exists() {
        fs::create_dir_all(&destination).with_context(|| {
            format!(
                "Failed to create destination directory: {}",
                destination.display()
            )
        })?;
    }

    // Use symlink_metadata to handle symlinks properly
    let source_metadata = fs::symlink_metadata(&source)
        .with_context(|| format!("Failed to get source metadata: {}", source.display()))?;

    if source_metadata.is_symlink() {
        // Handle symlink copying
        let target = fs::read_link(&source)
            .with_context(|| format!("Failed to read symlink target: {}", source.display()))?;

        if destination.is_dir() {
            let file_name = source
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Source symlink has no name"))?;
            let dest_file = destination.join(file_name);
            create_symlink(&target, &dest_file)?;
        } else {
            create_symlink(&target, &destination)?;
        }
    } else if source_metadata.is_file() && destination.is_dir() {
        // Single file to directory
        let file_name = source
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Source file has no name"))?;
        let dest_file = destination.join(file_name);
        sync_single_file(&source, &dest_file, &logger)?;
    } else if source_metadata.is_file() && (!destination.exists() || destination.is_file()) {
        // Single file to file (new file or existing file)
        sync_single_file(&source, &destination, &logger)?;
    } else if source_metadata.is_dir() {
        // Directory synchronization
        let default_options = SyncOptions::default();
        sync_directories(&source, &destination, &default_options, &logger)?;
    } else {
        return Err(anyhow::anyhow!("Invalid source/destination combination"));
    }

    logger.log("Synchronization completed successfully!");
    logger.close();
    Ok(())
}

/// Synchronize files from source to destination with options
pub fn synchronize_with_options(
    source: PathBuf,
    destination: PathBuf,
    _threads: usize,
    mut options: SyncOptions,
) -> Result<()> {
    // Check if any filters are specified
    let has_filters = !options.exclude_files.is_empty() || 
                     !options.exclude_dirs.is_empty() || 
                     options.min_size.is_some() || 
                     options.max_size.is_some();
    
    // Fast path for small files scenario - bypass most overhead (only if no filters)
    if !options.purge && !has_filters && source.is_dir() && crate::small_file_optimizer::is_small_files_scenario(&source).unwrap_or(false) {
        if options.show_progress {
            println!("Fast path for small files detected");
        }
        let stats = crate::small_file_optimizer::sync_small_files_fast(&source, &destination)?;
        if options.show_progress {
            println!("Files copied: {}", stats.files_copied());
            println!("Bytes transferred: {}", stats.bytes_transferred());
            println!("Time: {:?}", stats.elapsed_time);
        }
        return Ok(());
    }

    // Create logger with optional log file
    let logger = SyncLogger::new(options.log_file.as_deref(), options.show_eta, options.verbose)?;

    // Detect destination filesystem capabilities and adjust copy flags if needed
    let dest_parent = if destination.exists() {
        destination.clone()
    } else {
        destination
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Destination has no parent directory"))?
            .to_path_buf()
    };

    if let Ok(capabilities) = crate::metadata::detect_filesystem_capabilities(&dest_parent) {
        // Debug output
        // Detected filesystem type and ownership support

        let original_flags = crate::metadata::CopyFlags::from_string(&options.copy_flags);
        let filtered_flags =
            crate::metadata::filter_copy_flags_for_filesystem(&original_flags, &capabilities);

        // Warn user if flags were filtered
        let mut filtered_out = Vec::new();
        if original_flags.owner && !filtered_flags.owner {
            filtered_out.push("Owner (O)");
        }
        if original_flags.security && !filtered_flags.security {
            filtered_out.push("Security/Permissions (S)");
        }
        if original_flags.attributes && !filtered_flags.attributes {
            filtered_out.push("Extended Attributes (A)");
        }
        if original_flags.timestamps && !filtered_flags.timestamps {
            filtered_out.push("Timestamps (T)");
        }

        if !filtered_out.is_empty() {
            match capabilities.filesystem_type {
                crate::metadata::FilesystemType::Network => {
                    logger.log(
                        "Warning: Network filesystem detected. The following copy flags may fail and have been disabled:"
                    );
                }
                crate::metadata::FilesystemType::Tmpfs => {
                    logger.log(
                        "Warning: Temporary filesystem detected. The following copy flags may fail and have been disabled:"
                    );
                }
                _ => {
                    logger.log(
                        "Warning: Filesystem limitations detected. The following copy flags have been disabled:"
                    );
                }
            }
            for flag in &filtered_out {
                logger.log(&format!("  - {flag}"));
            }
            logger.log("Consider using -copyflags DAT for cross-filesystem copies.");
        }

        // Update options with filtered flags
        let mut new_flags = String::new();
        if filtered_flags.data {
            new_flags.push('D');
        }
        if filtered_flags.attributes {
            new_flags.push('A');
        }
        if filtered_flags.timestamps {
            new_flags.push('T');
        }
        if filtered_flags.security {
            new_flags.push('S');
        }
        if filtered_flags.owner {
            new_flags.push('O');
        }
        // U (auditing) is always filtered out

        options.copy_flags = new_flags;
    }

    if options.dry_run {
        logger.log("DRY RUN - would synchronize:");
        logger.log(&format!("  Source: {}", source.display()));
        logger.log(&format!("  Destination: {}", destination.display()));
        logger.close();
        return Ok(());
    }

    // Handle different source/destination combinations
    let source_metadata = fs::metadata(&source)?;

    if source_metadata.is_file() && destination.is_dir() {
        // Single file to directory
        let file_name = source
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Source file has no name"))?;
        let dest_file = destination.join(file_name);
        sync_single_file(&source, &dest_file, &logger)?;
    } else if source_metadata.is_file() && (!destination.exists() || destination.is_file()) {
        // Single file to file (new file or existing file)
        sync_single_file(&source, &destination, &logger)?;
    } else if source_metadata.is_dir() {
        // Directory synchronization
        sync_directories(&source, &destination, &options, &logger)?;
    } else {
        return Err(anyhow::anyhow!("Invalid source/destination combination"));
    }

    logger.log("Synchronization completed successfully!");
    logger.close();
    Ok(())
}

/// Synchronize a single file using optimized copy paths
fn sync_single_file(source: &Path, destination: &Path, logger: &SyncLogger) -> Result<()> {
    logger.log(&format!(
        "Syncing file: {} -> {}",
        source.display(),
        destination.display()
    ));

    // Create parent directory if needed
    if let Some(parent) = destination.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create parent directory: {}", parent.display())
            })?;
        }
    }

    // Use the optimized copy function that supports reflinks, mmap, etc.
    use crate::metadata::{copy_file_with_metadata_and_reflink, CopyFlags};
    use crate::reflink::ReflinkOptions;
    use crate::sync_stats::SyncStats;
    
    let stats = SyncStats::default();
    let copy_flags = CopyFlags::default(); // DAT by default
    let reflink_options = ReflinkOptions::default(); // Auto mode
    
    match copy_file_with_metadata_and_reflink(
        source,
        destination,
        &copy_flags,
        &reflink_options,
        Some(&stats),
    ) {
        Ok(bytes_copied) => {
            logger.log(&format!("  Copied {} bytes", bytes_copied));
            logger.log(&format!("  Files copied: {}", stats.files_copied()));
            logger.log(&format!("  Total bytes transferred: {}", stats.bytes_transferred()));
            Ok(())
        }
        Err(e) => {
            logger.log(&format!("  Error copying file: {}", e));
            Err(e)
        }
    }
}



/// Synchronize directories recursively
fn sync_directories(
    source: &Path,
    destination: &Path,
    options: &SyncOptions,
    logger: &SyncLogger,
) -> Result<()> {
    logger.log(&format!(
        "Syncing directory: {} -> {}",
        source.display(),
        destination.display()
    ));

    // Generate file lists
    let source_files = generate_file_list_with_options(source, options)
        .context("Failed to generate source file list")?;

    let dest_files = if destination.exists() {
        generate_file_list_with_options(destination, options)
            .context("Failed to generate destination file list")?
    } else {
        Vec::new()
    };

    // Compare file lists to determine operations
    let operations =
        compare_file_lists_with_roots(&source_files, &dest_files, source, destination, options);

    let total_files = operations.len() as u64;
    let total_bytes: u64 = source_files
        .iter()
        .filter(|f| !f.is_directory)
        .map(|f| f.size)
        .sum();

    let mut progress = SyncProgress::new(total_files, total_bytes);

    // Execute operations
    for operation in operations {
        match operation {
            FileOperation::CreateDirectory { path } => {
                let dest_path = map_source_to_dest(&path, source, destination)?;
                fs::create_dir_all(&dest_path).with_context(|| {
                    format!("Failed to create directory: {}", dest_path.display())
                })?;
                progress.update_file_complete(0);
            }
            FileOperation::Create { path } => {
                let dest_path = map_source_to_dest(&path, source, destination)?;
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let file_size = fs::metadata(&path)?.len();
                fs::copy(&path, &dest_path).with_context(|| {
                    format!(
                        "Failed to copy file: {} -> {}",
                        path.display(),
                        dest_path.display()
                    )
                })?;
                progress.update_file_complete(file_size);
            }
            FileOperation::Update {
                path,
                use_delta: true,
            } => {
                let dest_path = map_source_to_dest(&path, source, destination)?;
                sync_single_file(&path, &dest_path, logger)?;
                let file_size = fs::metadata(&path)?.len();
                progress.update_file_complete(file_size);
            }
            FileOperation::Update {
                path,
                use_delta: false,
            } => {
                let dest_path = map_source_to_dest(&path, source, destination)?;
                let file_size = fs::metadata(&path)?.len();
                fs::copy(&path, &dest_path).with_context(|| {
                    format!(
                        "Failed to copy file: {} -> {}",
                        path.display(),
                        dest_path.display()
                    )
                })?;
                progress.update_file_complete(file_size);
            }
            FileOperation::Delete { path } => {
                // Use symlink_metadata to check if it's a symlink without following it
                let metadata = fs::symlink_metadata(&path)
                    .with_context(|| format!("Failed to get metadata for: {}", path.display()))?;

                if metadata.is_symlink() {
                    fs::remove_file(&path)
                        .with_context(|| format!("Failed to delete symlink: {}", path.display()))?;
                } else if metadata.is_file() {
                    fs::remove_file(&path)
                        .with_context(|| format!("Failed to delete file: {}", path.display()))?;
                } else if metadata.is_dir() {
                    fs::remove_dir_all(&path).with_context(|| {
                        format!("Failed to delete directory: {}", path.display())
                    })?;
                }
                progress.update_file_complete(0);
            }
            FileOperation::CreateSymlink { path, target } => {
                let dest_path = map_source_to_dest(&path, source, destination)?;
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)?;
                }

                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &dest_path).with_context(|| {
                    format!(
                        "Failed to create symlink: {} -> {}",
                        dest_path.display(),
                        target.display()
                    )
                })?;

                #[cfg(windows)]
                {
                    // On Windows, we need to check if the target is a directory or file
                    // to use the appropriate symlink function
                    let _target_path = if target.is_absolute() {
                        target.clone()
                    } else {
                        path.parent().unwrap_or(Path::new(".")).join(&target)
                    };

                    crate::windows_symlinks::create_symlink(&dest_path, &target)?;
                }

                progress.update_file_complete(0);
            }
            FileOperation::UpdateSymlink { path, target } => {
                let dest_path = map_source_to_dest(&path, source, destination)?;

                // Remove existing symlink
                fs::remove_file(&dest_path).with_context(|| {
                    format!("Failed to remove existing symlink: {}", dest_path.display())
                })?;

                // Create new symlink
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &dest_path).with_context(|| {
                    format!(
                        "Failed to update symlink: {} -> {}",
                        dest_path.display(),
                        target.display()
                    )
                })?;

                #[cfg(windows)]
                {
                    // On Windows, we need to check if the target is a directory or file
                    let target_path = if target.is_absolute() {
                        target.clone()
                    } else {
                        path.parent().unwrap_or(Path::new(".")).join(&target)
                    };

                    if target_path.is_dir() {
                        std::os::windows::fs::symlink_dir(&target, &dest_path).with_context(
                            || {
                                format!(
                                    "Failed to update directory symlink: {} -> {}",
                                    dest_path.display(),
                                    target.display()
                                )
                            },
                        )?;
                    } else {
                        std::os::windows::fs::symlink_file(&target, &dest_path).with_context(
                            || {
                                format!(
                                    "Failed to update file symlink: {} -> {}",
                                    dest_path.display(),
                                    target.display()
                                )
                            },
                        )?;
                    }
                }

                progress.update_file_complete(0);
            }
        }
    }

    progress.finish();
    Ok(())
}

/// Map a source path to the corresponding destination path
fn map_source_to_dest(source_file: &Path, source_root: &Path, dest_root: &Path) -> Result<PathBuf> {
    let relative = source_file.strip_prefix(source_root).with_context(|| {
        format!(
            "File {} is not under source root {}",
            source_file.display(),
            source_root.display()
        )
    })?;
    Ok(dest_root.join(relative))
}

/// Create a symlink at the destination pointing to the target
fn create_symlink(target: &Path, destination: &Path) -> Result<()> {
    println!(
        "Creating symlink: {} -> {}",
        destination.display(),
        target.display()
    );

    // Create parent directories if needed
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent directory: {}", parent.display()))?;
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(target, destination).with_context(|| {
        format!(
            "Failed to create symlink: {} -> {}",
            destination.display(),
            target.display()
        )
    })?;

    #[cfg(windows)]
    {
        // Use our comprehensive Windows symlink implementation
        crate::windows_symlinks::create_symlink(destination, target)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_sync_single_file_new() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let source = temp_dir.path().join("source.txt");
        let dest = temp_dir.path().join("dest.txt");

        fs::write(&source, b"Hello, World!")?;

        let logger = SyncLogger::new(None, false, 0)?;
        sync_single_file(&source, &dest, &logger)?;

        let dest_content = fs::read(&dest)?;
        assert_eq!(dest_content, b"Hello, World!");

        Ok(())
    }
}