//! ZFS Send/Receive integration detection for macOS
//! 
//! This module detects ZFS datasets and provides integration with native ZFS
//! send/receive operations for optimal performance on macOS systems running ZFS.
//! 
//! ZFS on macOS can be installed via:
//! - OpenZFS on OS X (O3X) 
//! - Homebrew: brew install openzfs
//! - MacZFS project

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// ZFS dataset information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZfsDataset {
    pub name: String,
    pub mountpoint: PathBuf,
    pub filesystem_type: String,
    pub used: u64,
    pub available: u64,
    pub compression: String,
    pub dedup: String,
    pub checksum: String,
    pub readonly: bool,
    pub snapshots_enabled: bool,
}

/// ZFS send/receive operation details
#[derive(Debug, Clone)]
pub struct ZfsOperation {
    pub source_dataset: String,
    pub dest_dataset: String,
    pub operation_type: ZfsOperationType,
    pub estimated_size: u64,
    pub compression_ratio: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ZfsOperationType {
    InitialSend,
    IncrementalSend,
    ReceiveStream,
}

/// ZFS detection and integration manager for macOS
pub struct MacOSZfsManager {
    zfs_available: bool,
    zfs_version: Option<String>,
    datasets: HashMap<PathBuf, ZfsDataset>,
}

impl MacOSZfsManager {
    /// Create new ZFS manager with detection
    pub fn new() -> Result<Self> {
        let mut manager = MacOSZfsManager {
            zfs_available: false,
            zfs_version: None,
            datasets: HashMap::new(),
        };
        
        manager.detect_zfs_installation()?;
        if manager.zfs_available {
            manager.scan_datasets()?;
        }
        
        Ok(manager)
    }
    
    /// Detect if ZFS is installed and available on macOS
    fn detect_zfs_installation(&mut self) -> Result<()> {
        // Check for ZFS command availability
        let zfs_check = Command::new("zfs")
            .arg("version")
            .output();
            
        match zfs_check {
            Ok(output) if output.status.success() => {
                let version_info = String::from_utf8_lossy(&output.stdout);
                self.zfs_version = Some(version_info.trim().to_string());
                self.zfs_available = true;
                
                println!("ZFS detected: {}", version_info.trim());
            }
            Ok(_) => {
                // Command exists but failed - ZFS might be installed but not loaded
                self.check_zfs_kernel_module()?;
            }
            Err(_) => {
                // ZFS command not found
                self.zfs_available = false;
            }
        }
        
        Ok(())
    }
    
    /// Check if ZFS kernel extension is loaded on macOS
    fn check_zfs_kernel_module(&mut self) -> Result<()> {
        // Check if ZFS kernel extension is loaded
        let kextstat = Command::new("kextstat")
            .arg("-b")
            .arg("org.openzfsonosx.zfs")
            .output();
            
        match kextstat {
            Ok(output) if output.status.success() => {
                let kext_info = String::from_utf8_lossy(&output.stdout);
                if !kext_info.trim().is_empty() {
                    self.zfs_available = true;
                    self.zfs_version = Some("ZFS kernel module loaded".to_string());
                    println!("ZFS kernel extension loaded");
                } else {
                    println!("ZFS kernel extension not loaded");
                }
            }
            _ => {
                // Can't determine kernel module status
                self.zfs_available = false;
            }
        }
        
        Ok(())
    }
    
    /// Scan for available ZFS datasets
    fn scan_datasets(&mut self) -> Result<()> {
        let output = Command::new("zfs")
            .args(&["list", "-H", "-o", "name,mountpoint,type,used,avail,compression,dedup,checksum,readonly"])
            .output()
            .context("Failed to list ZFS datasets")?;
            
        if !output.status.success() {
            return Err(anyhow::anyhow!("ZFS list command failed"));
        }
        
        let datasets_info = String::from_utf8_lossy(&output.stdout);
        for line in datasets_info.lines() {
            if let Ok(dataset) = self.parse_dataset_line(line) {
                self.datasets.insert(dataset.mountpoint.clone(), dataset);
            }
        }
        
        println!("Found {} ZFS datasets", self.datasets.len());
        Ok(())
    }
    
    /// Parse a single dataset line from zfs list output
    fn parse_dataset_line(&self, line: &str) -> Result<ZfsDataset> {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 9 {
            return Err(anyhow::anyhow!("Invalid dataset line format"));
        }
        
        let used = self.parse_size_value(parts[3])?;
        let available = self.parse_size_value(parts[4])?;
        
        Ok(ZfsDataset {
            name: parts[0].to_string(),
            mountpoint: PathBuf::from(parts[1]),
            filesystem_type: parts[2].to_string(),
            used,
            available,
            compression: parts[5].to_string(),
            dedup: parts[6].to_string(),
            checksum: parts[7].to_string(),
            readonly: parts[8] == "on",
            snapshots_enabled: true, // Check this separately if needed
        })
    }
    
    /// Parse ZFS size values (e.g., "1.2G", "500M", "4.5K")
    fn parse_size_value(&self, size_str: &str) -> Result<u64> {
        if size_str == "-" {
            return Ok(0);
        }
        
        let size_str = size_str.trim();
        let (numeric_part, unit) = if let Some(last_char) = size_str.chars().last() {
            if last_char.is_alphabetic() {
                (&size_str[..size_str.len()-1], last_char)
            } else {
                (size_str, 'B')
            }
        } else {
            (size_str, 'B')
        };
        
        let numeric_value: f64 = numeric_part.parse()
            .context("Invalid numeric value in size")?;
            
        let multiplier = match unit {
            'K' => 1024,
            'M' => 1024 * 1024,
            'G' => 1024 * 1024 * 1024,
            'T' => 1024_u64.pow(4),
            'P' => 1024_u64.pow(5),
            _ => 1,
        };
        
        Ok((numeric_value * multiplier as f64) as u64)
    }
    
    /// Check if a path is on a ZFS dataset
    pub fn is_zfs_path(&self, path: &Path) -> bool {
        if !self.zfs_available {
            return false;
        }
        
        // Find the longest matching mountpoint
        let mut best_match: Option<&ZfsDataset> = None;
        let mut best_match_len = 0;
        
        for dataset in self.datasets.values() {
            if path.starts_with(&dataset.mountpoint) {
                let match_len = dataset.mountpoint.components().count();
                if match_len > best_match_len {
                    best_match = Some(dataset);
                    best_match_len = match_len;
                }
            }
        }
        
        best_match.is_some()
    }
    
    /// Get ZFS dataset for a given path
    pub fn get_dataset_for_path(&self, path: &Path) -> Option<&ZfsDataset> {
        if !self.zfs_available {
            return None;
        }
        
        // Find the longest matching mountpoint
        let mut best_match: Option<&ZfsDataset> = None;
        let mut best_match_len = 0;
        
        for dataset in self.datasets.values() {
            if path.starts_with(&dataset.mountpoint) {
                let match_len = dataset.mountpoint.components().count();
                if match_len > best_match_len {
                    best_match = Some(dataset);
                    best_match_len = match_len;
                }
            }
        }
        
        best_match
    }
    
    /// Check if ZFS send/receive would be beneficial for a copy operation
    pub fn should_use_zfs_transfer(&self, source: &Path, dest: &Path, size: u64) -> bool {
        if !self.zfs_available || size < 100 * 1024 * 1024 {
            return false; // Only beneficial for files > 100MB
        }
        
        let source_dataset = self.get_dataset_for_path(source);
        let dest_dataset = self.get_dataset_for_path(dest);
        
        match (source_dataset, dest_dataset) {
            (Some(src), Some(dst)) => {
                // Both on ZFS - check if they're different datasets
                src.name != dst.name && !src.readonly && !dst.readonly
            }
            _ => false, // At least one path not on ZFS
        }
    }
    
    /// Estimate ZFS send operation size and compression benefit
    pub fn estimate_zfs_operation(&self, source: &Path, dest: &Path) -> Option<ZfsOperation> {
        let source_dataset = self.get_dataset_for_path(source)?;
        let dest_dataset = self.get_dataset_for_path(dest)?;
        
        // Get actual file/directory size
        let estimated_size = self.get_path_size(source).unwrap_or(0);
        
        // Estimate compression ratio based on dataset settings
        let compression_ratio = match source_dataset.compression.as_str() {
            "off" => 1.0,
            "lz4" => 1.5,
            "gzip" => 2.0,
            "zstd" => 2.2,
            _ => 1.3, // Conservative estimate
        };
        
        Some(ZfsOperation {
            source_dataset: source_dataset.name.clone(),
            dest_dataset: dest_dataset.name.clone(),
            operation_type: ZfsOperationType::InitialSend,
            estimated_size,
            compression_ratio,
        })
    }
    
    /// Get size of a path (file or directory)
    fn get_path_size(&self, path: &Path) -> Result<u64> {
        let output = Command::new("du")
            .args(&["-sb", path.to_str().unwrap()])
            .output()
            .context("Failed to get path size")?;
            
        if !output.status.success() {
            return Err(anyhow::anyhow!("du command failed"));
        }
        
        let size_info = String::from_utf8_lossy(&output.stdout);
        let size_str = size_info.split_whitespace().next()
            .ok_or_else(|| anyhow::anyhow!("Invalid du output"))?;
            
        size_str.parse::<u64>()
            .context("Failed to parse size from du output")
    }
    
    /// Execute ZFS send operation (dry run by default)
    pub fn execute_zfs_send(
        &self, 
        operation: &ZfsOperation,
        dry_run: bool,
    ) -> Result<u64> {
        if !self.zfs_available {
            return Err(anyhow::anyhow!("ZFS not available"));
        }
        
        let mut cmd = Command::new("zfs");
        cmd.arg("send");
        
        if dry_run {
            cmd.arg("-n"); // Dry run mode
        }
        
        cmd.arg(&operation.source_dataset);
        
        let output = cmd.output()
            .context("Failed to execute ZFS send command")?;
            
        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("ZFS send failed: {}", error_msg));
        }
        
        // Parse output to get actual transfer size
        let output_info = String::from_utf8_lossy(&output.stdout);
        if dry_run {
            // In dry run mode, ZFS reports the size that would be sent
            self.parse_zfs_send_size(&output_info)
        } else {
            Ok(operation.estimated_size)
        }
    }
    
    /// Parse ZFS send output to extract transfer size
    fn parse_zfs_send_size(&self, _output: &str) -> Result<u64> {
        // ZFS send dry run output format varies, implement parsing logic
        // For now, return estimated size
        Ok(0) // Placeholder - implement actual parsing
    }
    
    /// Get all available ZFS datasets
    pub fn get_datasets(&self) -> &HashMap<PathBuf, ZfsDataset> {
        &self.datasets
    }
    
    /// Check if ZFS is available on this system
    pub fn is_zfs_available(&self) -> bool {
        self.zfs_available
    }
    
    /// Get ZFS version information
    pub fn get_zfs_version(&self) -> Option<&String> {
        self.zfs_version.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    
    #[test]
    fn test_zfs_manager_creation() {
        let manager = MacOSZfsManager::new();
        assert!(manager.is_ok());
        
        let manager = manager.unwrap();
        println!("ZFS available: {}", manager.is_zfs_available());
        if let Some(version) = manager.get_zfs_version() {
            println!("ZFS version: {}", version);
        }
    }
    
    #[test]
    fn test_size_parsing() {
        let manager = MacOSZfsManager {
            zfs_available: false,
            zfs_version: None,
            datasets: HashMap::new(),
        };
        
        assert_eq!(manager.parse_size_value("1024").unwrap(), 1024);
        assert_eq!(manager.parse_size_value("1K").unwrap(), 1024);
        assert_eq!(manager.parse_size_value("1M").unwrap(), 1024 * 1024);
        assert_eq!(manager.parse_size_value("1.5G").unwrap(), (1.5 * 1024.0 * 1024.0 * 1024.0) as u64);
        assert_eq!(manager.parse_size_value("-").unwrap(), 0);
    }
    
    #[test]
    fn test_path_detection() {
        let manager = MacOSZfsManager::new().unwrap();
        
        // Test with some common paths
        let test_paths = [
            "/tmp",
            "/Users",
            "/Applications",
            "/Volumes",
        ];
        
        for path in &test_paths {
            let is_zfs = manager.is_zfs_path(Path::new(path));
            println!("Path {} is ZFS: {}", path, is_zfs);
        }
    }
}