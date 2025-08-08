//! macOS-specific Network Filesystem Detection and Optimization
//! 
//! This module provides detection and optimization for network filesystems on macOS:
//! - NFS (Network File System) detection and optimization  
//! - SMB/CIFS (Server Message Block) optimization
//! - AFP (Apple Filing Protocol) support
//! - Network latency and bandwidth detection
//! - Optimized copying strategies for network storage

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Network filesystem type
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NetworkFsType {
    NFS,
    SMB,
    AFP,
    FTP,
    WebDAV,
    Unknown,
}

/// Network filesystem information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkFsInfo {
    pub fs_type: NetworkFsType,
    pub mount_point: PathBuf,
    pub server_address: String,
    pub remote_path: String,
    pub protocol_version: String,
    pub mount_options: Vec<String>,
    pub is_encrypted: bool,
    pub supports_locking: bool,
    pub max_io_size: Option<u32>,
}

/// Network performance characteristics
#[derive(Debug, Clone)]
pub struct NetworkPerformance {
    pub latency_ms: f64,
    pub bandwidth_mbps: f64,
    pub packet_loss_percent: f64,
    pub optimal_chunk_size: usize,
    pub concurrent_connections: usize,
}

/// Network copy optimization strategy
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkCopyStrategy {
    /// Sequential copying for high-latency networks
    Sequential,
    /// Parallel chunk copying for high-bandwidth networks
    ParallelChunks,
    /// Large buffer copying for local high-speed networks
    LargeBuffer,
    /// Compressed transfer for slow networks
    Compressed,
    /// Direct network protocol optimization (e.g., SMB multichannel)
    ProtocolOptimized,
}

/// Network filesystem manager for macOS
pub struct MacOSNetworkFsManager {
    network_mounts: HashMap<PathBuf, NetworkFsInfo>,
    performance_cache: HashMap<String, NetworkPerformance>,
}

impl MacOSNetworkFsManager {
    /// Create new network filesystem manager
    pub fn new() -> Result<Self> {
        let mut manager = MacOSNetworkFsManager {
            network_mounts: HashMap::new(),
            performance_cache: HashMap::new(),
        };
        
        manager.detect_network_mounts()?;
        Ok(manager)
    }
    
    /// Detect network-mounted filesystems
    fn detect_network_mounts(&mut self) -> Result<()> {
        let output = Command::new("mount")
            .arg("-t")
            .arg("nfs,smbfs,afp,ftp,webdav")
            .output()
            .context("Failed to run mount command")?;
        
        let mount_info = String::from_utf8_lossy(&output.stdout);
        
        for line in mount_info.lines() {
            if let Some(network_info) = self.parse_network_mount_line(line)? {
                self.network_mounts.insert(network_info.mount_point.clone(), network_info);
            }
        }
        
        // Also check for network mounts in /Volumes
        self.scan_volumes_directory()?;
        
        println!("Detected {} network filesystem mounts", self.network_mounts.len());
        Ok(())
    }
    
    /// Parse network mount line from mount command
    fn parse_network_mount_line(&self, line: &str) -> Result<Option<NetworkFsInfo>> {
        // Examples:
        // server.local:/path on /Volumes/Share (nfs)
        // //user@server.local/share on /Volumes/Share (smbfs, nodev, nosuid, mounted by user)
        
        let parts: Vec<&str> = line.split(" on ").collect();
        if parts.len() != 2 {
            return Ok(None);
        }
        
        let server_part = parts[0];
        let mount_part = parts[1];
        
        // Parse mount point and options
        let mount_parts: Vec<&str> = mount_part.split(" (").collect();
        if mount_parts.len() < 2 {
            return Ok(None);
        }
        
        let mount_point = PathBuf::from(mount_parts[0]);
        let options_str = mount_parts[1].trim_end_matches(')');
        let options: Vec<String> = options_str.split(", ").map(|s| s.to_string()).collect();
        
        // Determine filesystem type
        let fs_type = if options.contains(&"nfs".to_string()) {
            NetworkFsType::NFS
        } else if options.contains(&"smbfs".to_string()) {
            NetworkFsType::SMB
        } else if options.contains(&"afp".to_string()) {
            NetworkFsType::AFP
        } else if options.contains(&"ftp".to_string()) {
            NetworkFsType::FTP
        } else if options.contains(&"webdav".to_string()) {
            NetworkFsType::WebDAV
        } else {
            NetworkFsType::Unknown
        };
        
        // Parse server and remote path
        let (server_address, remote_path) = self.parse_server_path(server_part, &fs_type)?;
        
        // Determine protocol features
        let is_encrypted = options.iter().any(|opt| 
            opt.contains("sec=krb5") || opt.contains("encrypted") || opt.contains("tls")
        );
        
        let supports_locking = !options.contains(&"nolock".to_string());
        
        // Get protocol version
        let protocol_version = self.extract_protocol_version(&options, &fs_type);
        
        // Get max I/O size
        let max_io_size = self.extract_max_io_size(&options);
        
        Ok(Some(NetworkFsInfo {
            fs_type,
            mount_point,
            server_address,
            remote_path,
            protocol_version,
            mount_options: options,
            is_encrypted,
            supports_locking,
            max_io_size,
        }))
    }
    
    /// Parse server address and remote path from mount source
    fn parse_server_path(&self, server_part: &str, fs_type: &NetworkFsType) -> Result<(String, String)> {
        match fs_type {
            NetworkFsType::NFS => {
                // Format: server:/path
                if let Some(colon_pos) = server_part.find(':') {
                    let server = server_part[..colon_pos].to_string();
                    let path = server_part[colon_pos + 1..].to_string();
                    Ok((server, path))
                } else {
                    Ok((server_part.to_string(), "/".to_string()))
                }
            }
            NetworkFsType::SMB => {
                // Format: //[user@]server/share or smb://server/share
                let cleaned = server_part.trim_start_matches("//").trim_start_matches("smb://");
                
                if let Some(slash_pos) = cleaned.find('/') {
                    let server_part = &cleaned[..slash_pos];
                    let share_path = &cleaned[slash_pos..];
                    
                    // Remove user@ prefix if present
                    let server = if let Some(at_pos) = server_part.find('@') {
                        server_part[at_pos + 1..].to_string()
                    } else {
                        server_part.to_string()
                    };
                    
                    Ok((server, share_path.to_string()))
                } else {
                    Ok((cleaned.to_string(), "/".to_string()))
                }
            }
            _ => {
                // Generic parsing for other protocols
                Ok((server_part.to_string(), "/".to_string()))
            }
        }
    }
    
    /// Extract protocol version from mount options
    fn extract_protocol_version(&self, options: &[String], fs_type: &NetworkFsType) -> String {
        match fs_type {
            NetworkFsType::NFS => {
                // Look for vers=X or nfsvers=X
                for option in options {
                    if option.starts_with("vers=") {
                        return option[5..].to_string();
                    }
                    if option.starts_with("nfsvers=") {
                        return option[8..].to_string();
                    }
                }
                "3".to_string() // Default NFS version
            }
            NetworkFsType::SMB => {
                // Look for SMB version in options
                for option in options {
                    if option.contains("smb") {
                        if option.contains("3.1.1") {
                            return "3.1.1".to_string();
                        } else if option.contains("3.0") {
                            return "3.0".to_string();
                        } else if option.contains("2.1") {
                            return "2.1".to_string();
                        } else if option.contains("2.0") {
                            return "2.0".to_string();
                        }
                    }
                }
                "2.1".to_string() // Default SMB version
            }
            _ => "unknown".to_string(),
        }
    }
    
    /// Extract maximum I/O size from mount options
    fn extract_max_io_size(&self, options: &[String]) -> Option<u32> {
        for option in options {
            if option.starts_with("rsize=") {
                if let Ok(size) = option[6..].parse::<u32>() {
                    return Some(size);
                }
            }
            if option.starts_with("wsize=") {
                if let Ok(size) = option[6..].parse::<u32>() {
                    return Some(size);
                }
            }
        }
        None
    }
    
    /// Scan /Volumes directory for additional network mounts
    fn scan_volumes_directory(&mut self) -> Result<()> {
        let volumes_path = Path::new("/Volumes");
        if !volumes_path.exists() {
            return Ok(());
        }
        
        let entries = std::fs::read_dir(volumes_path)?;
        
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_dir() {
                // Check if this volume is already detected
                if !self.network_mounts.contains_key(&path) {
                    // Try to detect if it's a network mount by checking mount info
                    if let Some(network_info) = self.detect_volume_network_info(&path)? {
                        self.network_mounts.insert(path, network_info);
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Detect network filesystem info for a volume in /Volumes
    fn detect_volume_network_info(&self, volume_path: &Path) -> Result<Option<NetworkFsInfo>> {
        // Use mount command to get specific info about this volume
        let output = Command::new("mount")
            .output()
            .context("Failed to run mount command")?;
        
        let mount_info = String::from_utf8_lossy(&output.stdout);
        
        for line in mount_info.lines() {
            if line.contains(&format!(" on {} ", volume_path.display())) {
                return self.parse_network_mount_line(line);
            }
        }
        
        Ok(None)
    }
    
    /// Check if a path is on a network filesystem
    pub fn is_network_path(&self, path: &Path) -> bool {
        self.get_network_info(path).is_some()
    }
    
    /// Get network filesystem info for a path
    pub fn get_network_info(&self, path: &Path) -> Option<&NetworkFsInfo> {
        // Find the longest matching mount point
        let mut best_match: Option<&NetworkFsInfo> = None;
        let mut best_match_len = 0;
        
        for network_info in self.network_mounts.values() {
            if path.starts_with(&network_info.mount_point) {
                let match_len = network_info.mount_point.components().count();
                if match_len > best_match_len {
                    best_match = Some(network_info);
                    best_match_len = match_len;
                }
            }
        }
        
        best_match
    }
    
    /// Measure network performance to a server
    pub fn measure_network_performance(&mut self, server: &str) -> Result<NetworkPerformance> {
        // Check cache first
        if let Some(cached) = self.performance_cache.get(server) {
            return Ok(cached.clone());
        }
        
        let performance = self.do_network_measurement(server)?;
        self.performance_cache.insert(server.to_string(), performance.clone());
        
        Ok(performance)
    }
    
    /// Perform actual network performance measurement
    fn do_network_measurement(&self, server: &str) -> Result<NetworkPerformance> {
        // Measure latency using ping
        let latency = self.measure_latency(server)?;
        
        // Estimate bandwidth (simplified)
        let bandwidth = self.estimate_bandwidth(server, latency)?;
        
        // Calculate optimal parameters based on measurements
        let optimal_chunk_size = self.calculate_optimal_chunk_size(latency, bandwidth);
        let concurrent_connections = self.calculate_optimal_concurrency(latency, bandwidth);
        
        Ok(NetworkPerformance {
            latency_ms: latency,
            bandwidth_mbps: bandwidth,
            packet_loss_percent: 0.0, // Would need more sophisticated measurement
            optimal_chunk_size,
            concurrent_connections,
        })
    }
    
    /// Measure network latency using ping
    fn measure_latency(&self, server: &str) -> Result<f64> {
        let output = Command::new("ping")
            .args(&["-c", "3", server])
            .output()
            .context("Failed to run ping command")?;
        
        if !output.status.success() {
            return Ok(100.0); // Default high latency if ping fails
        }
        
        let ping_output = String::from_utf8_lossy(&output.stdout);
        
        // Parse average latency from ping output
        for line in ping_output.lines() {
            if line.contains("avg") {
                // Look for format like: "round-trip min/avg/max/stddev = 1.234/5.678/9.012/1.234 ms"
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() == 2 {
                    let times: Vec<&str> = parts[1].trim().split('/').collect();
                    if times.len() >= 2 {
                        if let Ok(avg_time) = times[1].parse::<f64>() {
                            return Ok(avg_time);
                        }
                    }
                }
            }
        }
        
        Ok(50.0) // Default latency if parsing fails
    }
    
    /// Estimate bandwidth based on latency and network type
    fn estimate_bandwidth(&self, _server: &str, latency: f64) -> Result<f64> {
        // Simplified bandwidth estimation based on latency
        let bandwidth_mbps = if latency < 1.0 {
            1000.0 // Local network - assume gigabit
        } else if latency < 10.0 {
            100.0 // Fast LAN
        } else if latency < 50.0 {
            10.0 // Slower network or internet
        } else {
            1.0 // Slow connection
        };
        
        Ok(bandwidth_mbps)
    }
    
    /// Calculate optimal chunk size for network transfers
    fn calculate_optimal_chunk_size(&self, latency: f64, bandwidth: f64) -> usize {
        // Bandwidth-delay product calculation
        let bdp_bytes = (bandwidth * 1_000_000.0 / 8.0) * (latency / 1000.0);
        
        // Optimal chunk size should be 2-4x the bandwidth-delay product
        let optimal_size = (bdp_bytes * 3.0) as usize;
        
        // Clamp to reasonable bounds
        std::cmp::max(64 * 1024, std::cmp::min(optimal_size, 16 * 1024 * 1024))
    }
    
    /// Calculate optimal number of concurrent connections
    fn calculate_optimal_concurrency(&self, latency: f64, bandwidth: f64) -> usize {
        if latency < 5.0 && bandwidth > 100.0 {
            // Low latency, high bandwidth - use many connections
            8
        } else if latency < 20.0 && bandwidth > 10.0 {
            // Medium latency/bandwidth
            4
        } else {
            // High latency or low bandwidth - use fewer connections
            2
        }
    }
    
    /// Determine optimal copy strategy for network paths
    pub fn get_optimal_network_copy_strategy(
        &mut self,
        source: &Path,
        dest: &Path,
        file_size: u64,
    ) -> Result<NetworkCopyStrategy> {
        let source_network = self.get_network_info(source);
        let dest_network = self.get_network_info(dest);
        
        match (source_network, dest_network) {
            (Some(src_info), Some(dst_info)) => {
                // Both paths are on network - optimize for network-to-network copy
                if src_info.server_address == dst_info.server_address {
                    // Same server - use protocol-specific optimization
                    Ok(NetworkCopyStrategy::ProtocolOptimized)
                } else {
                    // Different servers - use parallel chunks
                    Ok(NetworkCopyStrategy::ParallelChunks)
                }
            }
            (Some(net_info), None) | (None, Some(net_info)) => {
                // One network, one local - optimize based on network performance
                // Get network info first to avoid borrowing issues
                let server_address = net_info.server_address.clone();
                let performance = self.measure_network_performance(&server_address)?;
                
                if performance.latency_ms > 50.0 || performance.bandwidth_mbps < 10.0 {
                    // High latency or low bandwidth - use compression
                    Ok(NetworkCopyStrategy::Compressed)
                } else if performance.bandwidth_mbps > 100.0 {
                    // High bandwidth - use large buffers
                    Ok(NetworkCopyStrategy::LargeBuffer)
                } else if file_size > 100 * 1024 * 1024 {
                    // Large files on medium networks - use parallel chunks
                    Ok(NetworkCopyStrategy::ParallelChunks)
                } else {
                    // Default to sequential for smaller files
                    Ok(NetworkCopyStrategy::Sequential)
                }
            }
            (None, None) => {
                // Both local - no network optimization needed
                Ok(NetworkCopyStrategy::Sequential)
            }
        }
    }
    
    /// Get all detected network mounts
    pub fn get_network_mounts(&self) -> &HashMap<PathBuf, NetworkFsInfo> {
        &self.network_mounts
    }
    
    /// Get cached performance data
    pub fn get_performance_cache(&self) -> &HashMap<String, NetworkPerformance> {
        &self.performance_cache
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_network_fs_manager_creation() {
        let manager = MacOSNetworkFsManager::new();
        assert!(manager.is_ok());
        
        let manager = manager.unwrap();
        println!("Network mounts found: {}", manager.get_network_mounts().len());
        
        for (mount_point, info) in manager.get_network_mounts() {
            println!("Network FS: {} -> {:?}", mount_point.display(), info);
        }
    }
    
    #[test]
    fn test_path_detection() {
        let manager = MacOSNetworkFsManager::new().unwrap();
        
        let test_paths = [
            "/",
            "/Users",
            "/Volumes",
            "/tmp",
        ];
        
        for path in &test_paths {
            let is_network = manager.is_network_path(Path::new(path));
            println!("Path {} is network: {}", path, is_network);
            
            if is_network {
                if let Some(info) = manager.get_network_info(Path::new(path)) {
                    println!("  -> Type: {:?}", info.fs_type);
                    println!("  -> Server: {}", info.server_address);
                    println!("  -> Remote path: {}", info.remote_path);
                    println!("  -> Encrypted: {}", info.is_encrypted);
                }
            }
        }
    }
    
    #[test]
    fn test_copy_strategy() {
        let mut manager = MacOSNetworkFsManager::new().unwrap();
        
        let test_path = Path::new("/tmp/test");
        let test_sizes = [1024, 1024 * 1024, 100 * 1024 * 1024];
        
        for size in &test_sizes {
            let strategy = manager.get_optimal_network_copy_strategy(test_path, test_path, *size);
            match strategy {
                Ok(s) => println!("Size {} bytes -> Strategy: {:?}", size, s),
                Err(e) => println!("Error determining strategy for {} bytes: {}", size, e),
            }
        }
    }
    
    #[test]
    fn test_server_path_parsing() {
        let manager = MacOSNetworkFsManager::new().unwrap();
        
        let test_cases = [
            ("server.local:/path/to/share", NetworkFsType::NFS),
            ("//user@server.local/share", NetworkFsType::SMB),
            ("smb://server.local/share", NetworkFsType::SMB),
        ];
        
        for (input, fs_type) in &test_cases {
            match manager.parse_server_path(input, fs_type) {
                Ok((server, path)) => {
                    println!("Input: {} -> Server: {}, Path: {}", input, server, path);
                }
                Err(e) => {
                    println!("Failed to parse {}: {}", input, e);
                }
            }
        }
    }
}