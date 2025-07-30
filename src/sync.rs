//! Main synchronization logic

use crate::algorithm::{DeltaAlgorithm, Match};
use crate::file_list::{compare_file_lists_with_roots, generate_file_list_with_options, FileOperation};
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
    println!("Starting synchronization...");
    println!("  Source: {}", source.display());
    println!("  Destination: {}", destination.display());

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
        sync_single_file(&source, &dest_file)?;
    } else if source_metadata.is_file() && (!destination.exists() || destination.is_file()) {
        // Single file to file (new file or existing file)
        sync_single_file(&source, &destination)?;
    } else if source_metadata.is_dir() {
        // Directory synchronization
        let default_options = SyncOptions::default();
        sync_directories(&source, &destination, &default_options)?;
    } else {
        return Err(anyhow::anyhow!("Invalid source/destination combination"));
    }

    println!("Synchronization completed successfully!");
    Ok(())
}

/// Synchronize files from source to destination with options
pub fn synchronize_with_options(
    source: PathBuf,
    destination: PathBuf,
    _threads: usize,
    mut options: SyncOptions,
) -> Result<()> {
    // Detect destination filesystem capabilities and adjust copy flags if needed
    let dest_parent = if destination.exists() {
        destination.clone()
    } else {
        destination.parent()
            .ok_or_else(|| anyhow::anyhow!("Destination has no parent directory"))?
            .to_path_buf()
    };
    
    if let Ok(capabilities) = crate::metadata::detect_filesystem_capabilities(&dest_parent) {
        // Debug output
        // Detected filesystem type and ownership support
        
        let original_flags = crate::metadata::CopyFlags::from_string(&options.copy_flags);
        let filtered_flags = crate::metadata::filter_copy_flags_for_filesystem(&original_flags, &capabilities);
        
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
                    println!("Warning: Network filesystem detected. The following copy flags may fail and have been disabled:");
                },
                crate::metadata::FilesystemType::Tmpfs => {
                    println!("Warning: Temporary filesystem detected. The following copy flags may fail and have been disabled:");
                },
                _ => {
                    println!("Warning: Filesystem limitations detected. The following copy flags have been disabled:");
                }
            }
            for flag in &filtered_out {
                println!("  - {}", flag);
            }
            println!("Consider using -copyflags DAT for cross-filesystem copies.");
        }
        
        // Update options with filtered flags
        let mut new_flags = String::new();
        if filtered_flags.data { new_flags.push('D'); }
        if filtered_flags.attributes { new_flags.push('A'); }
        if filtered_flags.timestamps { new_flags.push('T'); }
        if filtered_flags.security { new_flags.push('S'); }
        if filtered_flags.owner { new_flags.push('O'); }
        // U (auditing) is always filtered out
        
        options.copy_flags = new_flags;
    }

    if options.dry_run {
        println!("DRY RUN - would synchronize:");
        println!("  Source: {}", source.display());
        println!("  Destination: {}", destination.display());
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
        sync_single_file(&source, &dest_file)?;
    } else if source_metadata.is_file() && (!destination.exists() || destination.is_file()) {
        // Single file to file (new file or existing file)
        sync_single_file(&source, &destination)?;
    } else if source_metadata.is_dir() {
        // Directory synchronization
        sync_directories(&source, &destination, &options)?;
    } else {
        return Err(anyhow::anyhow!("Invalid source/destination combination"));
    }

    println!("Synchronization completed successfully!");
    Ok(())
}

/// Synchronize a single file using delta algorithm
fn sync_single_file(source: &Path, destination: &Path) -> Result<()> {
    println!(
        "Syncing file: {} -> {}",
        source.display(),
        destination.display()
    );

    let source_data = fs::read(source)
        .with_context(|| format!("Failed to read source file: {}", source.display()))?;

    if !destination.exists() {
        // Destination doesn't exist, just copy the file
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create parent directory: {}", parent.display())
            })?;
        }

        fs::write(destination, &source_data).with_context(|| {
            format!(
                "Failed to write destination file: {}",
                destination.display()
            )
        })?;

        println!("  Copied {} bytes (new file)", source_data.len());
        return Ok(());
    }

    // Destination exists, use delta algorithm
    let dest_data = fs::read(destination)
        .with_context(|| format!("Failed to read destination file: {}", destination.display()))?;

    let algorithm = DeltaAlgorithm::default();

    // Generate checksums for destination (target) blocks
    let checksums = algorithm
        .generate_checksums(&dest_data)
        .context("Failed to generate checksums for destination")?;

    // Find matches between source and destination
    let matches = algorithm
        .find_matches(&source_data, &checksums)
        .context("Failed to find matches")?;

    // Apply the delta to reconstruct the file
    let new_data = apply_delta(&dest_data, &matches)?;

    // Write the updated file
    fs::write(destination, &new_data)
        .with_context(|| format!("Failed to write updated file: {}", destination.display()))?;

    // Calculate transfer statistics
    let literal_bytes: usize = matches
        .iter()
        .filter_map(|m| match m {
            Match::Literal { data, .. } => Some(data.len()),
            _ => None,
        })
        .sum();

    let block_matches = matches
        .iter()
        .filter(|m| matches!(m, Match::Block { .. }))
        .count();

    println!("  Transferred {literal_bytes} bytes ({literal_bytes} literal, {block_matches} block matches)");

    Ok(())
}

/// Apply delta matches to reconstruct a file
fn apply_delta(dest_data: &[u8], matches: &[Match]) -> Result<Vec<u8>> {
    let mut result = Vec::new();

    for match_item in matches {
        match match_item {
            Match::Literal { data, .. } => {
                result.extend_from_slice(data);
            }
            Match::Block {
                target_offset,
                length,
                ..
            } => {
                let start = *target_offset as usize;
                let end = start + length;
                if end <= dest_data.len() {
                    result.extend_from_slice(&dest_data[start..end]);
                } else {
                    return Err(anyhow::anyhow!(
                        "Block match extends beyond destination data"
                    ));
                }
            }
        }
    }

    Ok(result)
}

/// Synchronize directories recursively
fn sync_directories(source: &Path, destination: &Path, options: &SyncOptions) -> Result<()> {
    println!(
        "Syncing directory: {} -> {}",
        source.display(),
        destination.display()
    );
    

    // Generate file lists
    let source_files = generate_file_list_with_options(source, options).context("Failed to generate source file list")?;

    let dest_files = if destination.exists() {
        generate_file_list_with_options(destination, options).context("Failed to generate destination file list")?
    } else {
        Vec::new()
    };

    // Compare file lists to determine operations
    let operations = compare_file_lists_with_roots(
        &source_files,
        &dest_files,
        source,
        destination,
        options,
    );

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
                sync_single_file(&path, &dest_path)?;
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
                    let target_path = if target.is_absolute() {
                        target.clone()
                    } else {
                        path.parent().unwrap_or(Path::new(".")).join(&target)
                    };

                    if target_path.is_dir() {
                        std::os::windows::fs::symlink_dir(&target, &dest_path).with_context(
                            || {
                                format!(
                                    "Failed to create directory symlink: {} -> {}",
                                    dest_path.display(),
                                    target.display()
                                )
                            },
                        )?;
                    } else {
                        std::os::windows::fs::symlink_file(&target, &dest_path).with_context(
                            || {
                                format!(
                                    "Failed to create file symlink: {} -> {}",
                                    dest_path.display(),
                                    target.display()
                                )
                            },
                        )?;
                    }
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
        // On Windows, we need to determine if the target is a directory or file
        // For relative paths, we need to resolve them relative to the symlink location
        let target_path = if target.is_absolute() {
            target.to_path_buf()
        } else if let Some(parent) = destination.parent() {
            parent.join(target)
        } else {
            target.to_path_buf()
        };

        if target_path.is_dir() {
            std::os::windows::fs::symlink_dir(target, destination).with_context(|| {
                format!(
                    "Failed to create directory symlink: {} -> {}",
                    destination.display(),
                    target.display()
                )
            })?;
        } else {
            std::os::windows::fs::symlink_file(target, destination).with_context(|| {
                format!(
                    "Failed to create file symlink: {} -> {}",
                    destination.display(),
                    target.display()
                )
            })?;
        }
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

        sync_single_file(&source, &dest)?;

        let dest_content = fs::read(&dest)?;
        assert_eq!(dest_content, b"Hello, World!");

        Ok(())
    }

    #[test]
    fn test_apply_delta() -> Result<()> {
        let dest_data = b"Hello, World!";
        let matches = vec![
            Match::Block {
                source_offset: 0,
                target_offset: 0,
                length: 5,
            }, // "Hello"
            Match::Literal {
                offset: 5,
                data: b" Rust".to_vec(),
                is_compressed: false,
            }, // " Rust"
            Match::Block {
                source_offset: 10,
                target_offset: 5,
                length: 8,
            }, // ", World!"
        ];

        let result = apply_delta(dest_data, &matches)?;
        assert_eq!(result, b"Hello Rust, World!");

        Ok(())
    }
}
