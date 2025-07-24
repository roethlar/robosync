//! Main synchronization logic

use anyhow::{Result, Context};
use std::path::{Path, PathBuf};
use std::fs;
use std::io::{self, Read, Write};
use crate::algorithm::{DeltaAlgorithm, Match};
use crate::file_list::{generate_file_list, FileInfo, FileOperation, compare_file_lists_with_roots};
use crate::progress::SyncProgress;
use crate::options::SyncOptions;

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
        fs::create_dir_all(&destination)
            .with_context(|| format!("Failed to create destination directory: {}", destination.display()))?;
    }
    
    if source.is_file() && destination.is_dir() {
        // Single file to directory
        let file_name = source.file_name()
            .ok_or_else(|| anyhow::anyhow!("Source file has no name"))?;
        let dest_file = destination.join(file_name);
        sync_single_file(&source, &dest_file)?;
    } else if source.is_file() && (!destination.exists() || destination.is_file()) {
        // Single file to file (new file or existing file)
        sync_single_file(&source, &destination)?;
    } else if source.is_dir() {
        // Directory synchronization
        sync_directories(&source, &destination)?;
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
    options: SyncOptions,
) -> Result<()> {
    if options.dry_run {
        println!("DRY RUN - would synchronize:");
        println!("  Source: {}", source.display());
        println!("  Destination: {}", destination.display());
        return Ok(());
    }
    
    // For now, just call the basic synchronize function
    // TODO: Implement full options support
    synchronize(source, destination, _threads, options.compress)
}

/// Synchronize a single file using delta algorithm
fn sync_single_file(source: &Path, destination: &Path) -> Result<()> {
    println!("Syncing file: {} -> {}", source.display(), destination.display());
    
    let source_data = fs::read(source)
        .with_context(|| format!("Failed to read source file: {}", source.display()))?;
    
    if !destination.exists() {
        // Destination doesn't exist, just copy the file
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parent directory: {}", parent.display()))?;
        }
        
        fs::write(destination, &source_data)
            .with_context(|| format!("Failed to write destination file: {}", destination.display()))?;
        
        println!("  Copied {} bytes (new file)", source_data.len());
        return Ok(());
    }
    
    // Destination exists, use delta algorithm
    let dest_data = fs::read(destination)
        .with_context(|| format!("Failed to read destination file: {}", destination.display()))?;
    
    let algorithm = DeltaAlgorithm::default();
    
    // Generate checksums for destination (target) blocks
    let checksums = algorithm.generate_checksums(&dest_data)
        .context("Failed to generate checksums for destination")?;
    
    // Find matches between source and destination
    let matches = algorithm.find_matches(&source_data, &checksums)
        .context("Failed to find matches")?;
    
    // Apply the delta to reconstruct the file
    let new_data = apply_delta(&dest_data, &matches)?;
    
    // Write the updated file
    fs::write(destination, &new_data)
        .with_context(|| format!("Failed to write updated file: {}", destination.display()))?;
    
    // Calculate transfer statistics
    let literal_bytes: usize = matches.iter()
        .filter_map(|m| match m {
            Match::Literal { data, .. } => Some(data.len()),
            _ => None,
        })
        .sum();
    
    let block_matches = matches.iter()
        .filter(|m| matches!(m, Match::Block { .. }))
        .count();
    
    println!("  Transferred {} bytes ({} literal, {} block matches)", 
             literal_bytes, literal_bytes, block_matches);
    
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

/// Synchronize directories recursively
fn sync_directories(source: &Path, destination: &Path) -> Result<()> {
    println!("Syncing directory: {} -> {}", source.display(), destination.display());
    
    // Generate file lists
    let source_files = generate_file_list(source)
        .context("Failed to generate source file list")?;
    
    let dest_files = if destination.exists() {
        generate_file_list(destination)
            .context("Failed to generate destination file list")?
    } else {
        Vec::new()
    };
    
    // Compare file lists to determine operations
    let operations = compare_file_lists_with_roots(&source_files, &dest_files, source, destination);
    
    let total_files = operations.len() as u64;
    let total_bytes: u64 = source_files.iter()
        .filter(|f| !f.is_directory)
        .map(|f| f.size)
        .sum();
    
    let mut progress = SyncProgress::new(total_files, total_bytes);
    
    // Execute operations
    for operation in operations {
        match operation {
            FileOperation::CreateDirectory { path } => {
                let dest_path = map_source_to_dest(&path, source, destination)?;
                fs::create_dir_all(&dest_path)
                    .with_context(|| format!("Failed to create directory: {}", dest_path.display()))?;
                progress.update_file_complete(0);
            }
            FileOperation::Create { path } => {
                let dest_path = map_source_to_dest(&path, source, destination)?;
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let file_size = fs::metadata(&path)?.len();
                fs::copy(&path, &dest_path)
                    .with_context(|| format!("Failed to copy file: {} -> {}", path.display(), dest_path.display()))?;
                progress.update_file_complete(file_size);
            }
            FileOperation::Update { path, use_delta: true } => {
                let dest_path = map_source_to_dest(&path, source, destination)?;
                sync_single_file(&path, &dest_path)?;
                let file_size = fs::metadata(&path)?.len();
                progress.update_file_complete(file_size);
            }
            FileOperation::Update { path, use_delta: false } => {
                let dest_path = map_source_to_dest(&path, source, destination)?;
                let file_size = fs::metadata(&path)?.len();
                fs::copy(&path, &dest_path)
                    .with_context(|| format!("Failed to copy file: {} -> {}", path.display(), dest_path.display()))?;
                progress.update_file_complete(file_size);
            }
            FileOperation::Delete { path } => {
                if path.is_file() {
                    fs::remove_file(&path)
                        .with_context(|| format!("Failed to delete file: {}", path.display()))?;
                } else if path.is_dir() {
                    fs::remove_dir_all(&path)
                        .with_context(|| format!("Failed to delete directory: {}", path.display()))?;
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
    let relative = source_file.strip_prefix(source_root)
        .with_context(|| format!("File {} is not under source root {}", source_file.display(), source_root.display()))?;
    Ok(dest_root.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

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
            Match::Block { source_offset: 0, target_offset: 0, length: 5 }, // "Hello"
            Match::Literal { offset: 5, data: b" Rust".to_vec() }, // " Rust"
            Match::Block { source_offset: 10, target_offset: 5, length: 8 }, // ", World!"
        ];
        
        let result = apply_delta(dest_data, &matches)?;
        assert_eq!(result, b"Hello Rust, World!");
        
        Ok(())
    }
}