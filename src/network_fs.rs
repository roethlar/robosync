//! Network filesystem detection and optimization
//!
//! This module detects different types of network filesystems (NFS, SMB, SSHFS)
//! and applies appropriate optimizations for each protocol type.

use std::path::Path;
use std::collections::HashMap;

/// Filesystem types (both network and local)
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkFsType {
    /// Network File System (NFS)
    NFS,
    /// Server Message Block (SMB/CIFS)
    SMB,
    /// SSH Filesystem (SSHFS)
    SSHFS,
    /// WebDAV
    WebDAV,
    /// ZFS (Zettabyte File System)
    ZFS,
    /// BTRFS (B-tree File System)
    BTRFS,
    /// XFS (SGI XFS)
    XFS,
    /// ext4 (Fourth Extended Filesystem)
    EXT4,
    /// NTFS (Windows)
    NTFS,
    /// APFS (Apple File System)
    APFS,
    /// Generic local filesystem
    Local,
    /// Unknown/undetected
    Unknown,
}

/// Network filesystem characteristics
#[derive(Debug, Clone)]
pub struct NetworkFsInfo {
    /// Filesystem type
    pub fs_type: NetworkFsType,
    /// Mount point
    pub mount_point: String,
    /// Server hostname/IP
    pub server: Option<String>,
    /// Protocol version
    pub version: Option<String>,
    /// Estimated latency (ms)
    pub latency_ms: Option<u32>,
    /// Estimated bandwidth (MB/s)
    pub bandwidth_mbps: Option<u32>,
    /// Supports large transfers
    pub supports_large_transfers: bool,
    /// Optimal buffer size
    pub optimal_buffer_size: usize,
}

/// Network filesystem detector
pub struct NetworkFsDetector {
    /// Cache of detected filesystems
    fs_cache: HashMap<String, NetworkFsInfo>,
}

impl NetworkFsDetector {
    /// Create new network filesystem detector
    pub fn new() -> Self {
        NetworkFsDetector {
            fs_cache: HashMap::new(),
        }
    }

    /// Detect filesystem type for a given path
    pub fn detect_filesystem(&mut self, path: &Path) -> NetworkFsInfo {
        let path_str = path.to_string_lossy().to_string();
        
        // Check cache first
        if let Some(cached) = self.fs_cache.get(&path_str) {
            return cached.clone();
        }

        let fs_info = self.detect_filesystem_internal(path);
        self.fs_cache.insert(path_str, fs_info.clone());
        fs_info
    }

    /// Internal filesystem detection logic
    fn detect_filesystem_internal(&self, path: &Path) -> NetworkFsInfo {
        #[cfg(target_os = "linux")]
        {
            self.detect_linux_filesystem(path)
        }
        
        #[cfg(target_os = "macos")]
        {
            self.detect_macos_filesystem(path)
        }
        
        #[cfg(target_os = "windows")]
        {
            self.detect_windows_filesystem(path)
        }
        
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            let _ = path; // Suppress unused warning
            NetworkFsInfo {
                fs_type: NetworkFsType::Unknown,
                mount_point: String::new(),
                server: None,
                version: None,
                latency_ms: None,
                bandwidth_mbps: None,
                supports_large_transfers: true,
                optimal_buffer_size: 64 * 1024,
            }
        }
    }

    #[cfg(target_os = "linux")]
    /// Detect filesystem on Linux using /proc/mounts
    fn detect_linux_filesystem(&self, path: &Path) -> NetworkFsInfo {
        // Read /proc/mounts to find filesystem type
        if let Ok(mounts_content) = std::fs::read_to_string("/proc/mounts") {
            let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            let path_str = canonical_path.to_string_lossy();
            
            // Collect all matching mounts first, then pick the best one
            let mut matching_mounts = Vec::new();
            
            for line in mounts_content.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let device = parts[0];
                    let mount_point = parts[1];
                    let fs_type = parts[2];
                    
                    // Check if path is under this mount point
                    if path_str.starts_with(mount_point) {
                        let detected_type = match fs_type {
                            "nfs" | "nfs4" => NetworkFsType::NFS,
                            "cifs" | "smb3" => NetworkFsType::SMB,
                            "fuse.sshfs" => NetworkFsType::SSHFS,
                            "davfs" => NetworkFsType::WebDAV,
                            "zfs" => NetworkFsType::ZFS,
                            "btrfs" => NetworkFsType::BTRFS,
                            "xfs" => NetworkFsType::XFS,
                            "ext4" => NetworkFsType::EXT4,
                            "ntfs" => NetworkFsType::NTFS,
                            _ => NetworkFsType::Local,
                        };
                        
                        let server = match detected_type {
                            NetworkFsType::NFS | NetworkFsType::SMB | NetworkFsType::SSHFS | NetworkFsType::WebDAV => {
                                Some(device.split(':').next().unwrap_or(device).to_string())
                            }
                            _ => if fs_type == "autofs" {
                                // For autofs, also extract server info if available
                                Some(device.to_string())
                            } else {
                                None
                            }
                        };
                        
                        matching_mounts.push((
                            mount_point.len(),
                            detected_type,
                            mount_point.to_string(),
                            server,
                            fs_type,
                        ));
                    }
                }
            }
            
            // Sort by mount point length (longest first) and prefer network types over local/autofs
            matching_mounts.sort_by(|a, b| {
                // First sort by mount point length (longest first)
                let len_cmp = b.0.cmp(&a.0);
                if len_cmp != std::cmp::Ordering::Equal {
                    return len_cmp;
                }
                
                // Then prefer specific filesystem types over generic local
                let fs_priority = |fs_type: &NetworkFsType| match fs_type {
                    NetworkFsType::NFS | NetworkFsType::SMB | NetworkFsType::SSHFS | NetworkFsType::WebDAV => 1,
                    NetworkFsType::ZFS | NetworkFsType::BTRFS | NetworkFsType::XFS | NetworkFsType::EXT4 | NetworkFsType::NTFS | NetworkFsType::APFS => 2,
                    NetworkFsType::Local => 3,
                    NetworkFsType::Unknown => 4,
                };
                fs_priority(&a.1).cmp(&fs_priority(&b.1))
            });
            
            let best_match = if let Some((_, detected_type, mount_point, server, fs_type)) = matching_mounts.first() {
                NetworkFsInfo {
                    fs_type: detected_type.clone(),
                    mount_point: mount_point.clone(),
                    server: server.clone(),
                    version: self.detect_protocol_version(detected_type, fs_type),
                    latency_ms: self.estimate_latency(detected_type),
                    bandwidth_mbps: self.estimate_bandwidth(detected_type),
                    supports_large_transfers: self.supports_large_transfers(detected_type),
                    optimal_buffer_size: self.get_optimal_buffer_size(detected_type),
                }
            } else {
                NetworkFsInfo {
                    fs_type: NetworkFsType::Local,
                    mount_point: "/".to_string(),
                    server: None,
                    version: None,
                    latency_ms: None,
                    bandwidth_mbps: None,
                    supports_large_transfers: true,
                    optimal_buffer_size: 64 * 1024,
                }
            };
            
            best_match
        } else {
            // Fallback: assume local filesystem
            NetworkFsInfo {
                fs_type: NetworkFsType::Local,
                mount_point: "/".to_string(),
                server: None,
                version: None,
                latency_ms: None,
                bandwidth_mbps: None,
                supports_large_transfers: true,
                optimal_buffer_size: 64 * 1024,
            }
        }
    }

    #[cfg(target_os = "macos")]
    /// Detect filesystem on macOS using mount command
    fn detect_macos_filesystem(&self, path: &Path) -> NetworkFsInfo {
        // Use mount command to detect filesystem type
        use std::process::Command;
        
        if let Ok(output) = Command::new("mount").output() {
            let mount_output = String::from_utf8_lossy(&output.stdout);
            let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            let path_str = canonical_path.to_string_lossy();
            
            let mut best_match = NetworkFsInfo {
                fs_type: NetworkFsType::Local,
                mount_point: "/".to_string(),
                server: None,
                version: None,
                latency_ms: None,
                bandwidth_mbps: None,
                supports_large_transfers: true,
                optimal_buffer_size: 64 * 1024,
            };
            
            let mut longest_match = 0;
            
            for line in mount_output.lines() {
                // Parse lines like: "server:/path on /mount/point (nfs, ...)"
                if let Some(on_pos) = line.find(" on ") {
                    let device_part = &line[..on_pos];
                    let rest = &line[on_pos + 4..];
                    
                    if let Some(paren_pos) = rest.find(" (") {
                        let mount_point = &rest[..paren_pos];
                        let fs_info = &rest[paren_pos + 2..];
                        
                        if path_str.starts_with(mount_point) && mount_point.len() > longest_match {
                            longest_match = mount_point.len();
                            
                            let detected_type = if fs_info.contains("nfs") {
                                NetworkFsType::NFS
                            } else if fs_info.contains("smbfs") || fs_info.contains("cifs") {
                                NetworkFsType::SMB
                            } else if fs_info.contains("osxfuse") && device_part.contains("sshfs") {
                                NetworkFsType::SSHFS
                            } else if fs_info.contains("webdav") {
                                NetworkFsType::WebDAV
                            } else {
                                NetworkFsType::Local
                            };
                            
                            let server = if detected_type != NetworkFsType::Local {
                                Some(device_part.split(':').next().unwrap_or(device_part).to_string())
                            } else {
                                None
                            };
                            
                            best_match = NetworkFsInfo {
                                fs_type: detected_type.clone(),
                                mount_point: mount_point.to_string(),
                                server,
                                version: self.detect_protocol_version(&detected_type, fs_info),
                                latency_ms: self.estimate_latency(&detected_type),
                                bandwidth_mbps: self.estimate_bandwidth(&detected_type),
                                supports_large_transfers: self.supports_large_transfers(&detected_type),
                                optimal_buffer_size: self.get_optimal_buffer_size(&detected_type),
                            };
                        }
                    }
                }
            }
            
            best_match
        } else {
            // Fallback: assume local filesystem
            NetworkFsInfo {
                fs_type: NetworkFsType::Local,
                mount_point: "/".to_string(),
                server: None,
                version: None,
                latency_ms: None,
                bandwidth_mbps: None,
                supports_large_transfers: true,
                optimal_buffer_size: 64 * 1024,
            }
        }
    }

    #[cfg(target_os = "windows")]
    /// Detect filesystem on Windows using drive type
    fn detect_windows_filesystem(&self, path: &Path) -> NetworkFsInfo {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        
        let path_wide: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        
        // Get drive type using Windows API
        let drive_type = unsafe {
            winapi::um::fileapi::GetDriveTypeW(path_wide.as_ptr())
        };
        
        match drive_type {
            winapi::um::winbase::DRIVE_REMOTE => {
                // Network drive - try to determine protocol
                // Check if it's a mapped network drive first
                let fs_type = if let Some(server) = self.get_mapped_drive_target(path) {
                    // This is a mapped network drive - assume SMB/CIFS
                    if server.starts_with("\\\\") {
                        NetworkFsType::SMB
                    } else {
                        NetworkFsType::SMB // Default to SMB for Windows network drives
                    }
                } else if self.is_smb_path(path) {
                    NetworkFsType::SMB
                } else {
                    NetworkFsType::Unknown
                };
                
                NetworkFsInfo {
                    fs_type: fs_type.clone(),
                    mount_point: path.to_string_lossy().to_string(),
                    server: self.extract_server_from_path(path),
                    version: self.detect_protocol_version(&fs_type, ""),
                    latency_ms: self.estimate_latency(&fs_type),
                    bandwidth_mbps: self.estimate_bandwidth(&fs_type),
                    supports_large_transfers: self.supports_large_transfers(&fs_type),
                    optimal_buffer_size: self.get_optimal_buffer_size(&fs_type),
                }
            }
            _ => {
                // Local drive
                NetworkFsInfo {
                    fs_type: NetworkFsType::Local,
                    mount_point: path.to_string_lossy().to_string(),
                    server: None,
                    version: None,
                    latency_ms: None,
                    bandwidth_mbps: None,
                    supports_large_transfers: true,
                    optimal_buffer_size: 64 * 1024,
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn is_smb_path(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        path_str.starts_with("\\\\") || path_str.starts_with("//")
    }

    #[cfg(target_os = "windows")]
    fn extract_server_from_path(&self, path: &Path) -> Option<String> {
        let path_str = path.to_string_lossy();
        if path_str.starts_with("\\\\") {
            path_str.get(2..)
                .and_then(|s| s.split('\\').next())
                .map(|s| s.to_string())
        } else {
            None
        }
    }
    
    #[cfg(target_os = "windows")]
    fn get_mapped_drive_target(&self, path: &Path) -> Option<String> {
        use std::process::Command;
        
        // Extract drive letter (H:, etc.)
        let path_str = path.to_string_lossy();
        if let Some(drive_letter) = path_str.chars().next() {
            if path_str.len() >= 2 && path_str.chars().nth(1) == Some(':') {
                let drive = format!("{}:", drive_letter);
                
                // Use 'net use' to check if this drive is mapped
                if let Ok(output) = Command::new("net")
                    .args(&["use", &drive])
                    .output() 
                {
                    let output_str = String::from_utf8_lossy(&output.stdout);
                    
                    // Parse the network path from net use output
                    for line in output_str.lines() {
                        if line.contains("\\\\") {
                            // Extract the UNC path
                            if let Some(start) = line.find("\\\\") {
                                if let Some(end) = line[start..].find(char::is_whitespace) {
                                    return Some(line[start..start+end].to_string());
                                } else {
                                    // If no whitespace found, take the rest of the line
                                    return Some(line[start..].trim().to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Detect protocol version from filesystem info
    fn detect_protocol_version(&self, fs_type: &NetworkFsType, fs_info: &str) -> Option<String> {
        match fs_type {
            NetworkFsType::NFS => {
                if fs_info.contains("nfs4") {
                    Some("4.0".to_string())
                } else if fs_info.contains("nfs3") {
                    Some("3.0".to_string())
                } else {
                    Some("3.0".to_string()) // Default assumption
                }
            }
            NetworkFsType::SMB => {
                if fs_info.contains("3.0") {
                    Some("3.0".to_string())
                } else if fs_info.contains("2.1") {
                    Some("2.1".to_string())
                } else {
                    Some("2.0".to_string()) // Default assumption
                }
            }
            _ => None,
        }
    }

    /// Estimate network latency based on filesystem type
    fn estimate_latency(&self, fs_type: &NetworkFsType) -> Option<u32> {
        match fs_type {
            NetworkFsType::NFS => Some(2), // Typically low-latency LAN
            NetworkFsType::SMB => Some(5), // Can be higher due to protocol overhead
            NetworkFsType::SSHFS => Some(10), // SSH overhead
            NetworkFsType::WebDAV => Some(50), // HTTP overhead
            NetworkFsType::ZFS => Some(0), // Local filesystem
            NetworkFsType::BTRFS => Some(0), // Local filesystem
            NetworkFsType::XFS => Some(0), // Local filesystem
            NetworkFsType::EXT4 => Some(0), // Local filesystem
            NetworkFsType::NTFS => Some(0), // Local filesystem
            NetworkFsType::APFS => Some(0), // Local filesystem
            NetworkFsType::Local => Some(0),
            NetworkFsType::Unknown => None,
        }
    }

    /// Estimate bandwidth based on filesystem type
    fn estimate_bandwidth(&self, fs_type: &NetworkFsType) -> Option<u32> {
        match fs_type {
            NetworkFsType::NFS => Some(1000), // Gigabit LAN
            NetworkFsType::SMB => Some(800), // SMB overhead
            NetworkFsType::SSHFS => Some(100), // SSH encryption overhead
            NetworkFsType::WebDAV => Some(50), // HTTP overhead
            NetworkFsType::ZFS => None, // Varies greatly by hardware
            NetworkFsType::BTRFS => None, // Varies greatly by hardware
            NetworkFsType::XFS => None, // Varies greatly by hardware
            NetworkFsType::EXT4 => None, // Varies greatly by hardware
            NetworkFsType::NTFS => None, // Varies greatly by hardware
            NetworkFsType::APFS => None, // Varies greatly by hardware
            NetworkFsType::Local => None, // Varies greatly
            NetworkFsType::Unknown => None,
        }
    }

    /// Check if filesystem supports large transfers
    fn supports_large_transfers(&self, fs_type: &NetworkFsType) -> bool {
        match fs_type {
            NetworkFsType::NFS => true,
            NetworkFsType::SMB => true,
            NetworkFsType::SSHFS => false, // Limited by SSH
            NetworkFsType::WebDAV => false, // HTTP limitations
            NetworkFsType::ZFS => true,
            NetworkFsType::BTRFS => true,
            NetworkFsType::XFS => true,
            NetworkFsType::EXT4 => true,
            NetworkFsType::NTFS => true,
            NetworkFsType::APFS => true,
            NetworkFsType::Local => true,
            NetworkFsType::Unknown => true,
        }
    }

    /// Get optimal buffer size for filesystem type (research-backed for 2025 systems)
    fn get_optimal_buffer_size(&self, fs_type: &NetworkFsType) -> usize {
        match fs_type {
            // Network filesystems: 4-8MB to hide latency while avoiding excessive memory
            NetworkFsType::NFS => 4 * 1024 * 1024, // 4MB - optimal for NFS based on testing
            NetworkFsType::SMB => 4 * 1024 * 1024, // 4MB - SMB3 performs well at this size
            NetworkFsType::SSHFS => 1024 * 1024, // 1MB - SSH crypto overhead limits benefits
            NetworkFsType::WebDAV => 2 * 1024 * 1024, // 2MB - HTTP chunking considerations
            
            // Local filesystems: 2-4MB based on research showing diminishing returns above 4MB
            NetworkFsType::ZFS => 1024 * 1024, // 1MB - match typical recordsize (128KB-1MB)
            NetworkFsType::BTRFS => 2 * 1024 * 1024, // 2MB - optimal for CoW operations
            NetworkFsType::XFS => 2 * 1024 * 1024, // 2MB - extent-based optimizations work well
            NetworkFsType::EXT4 => 2 * 1024 * 1024, // 2MB - research shows peak at 2-4MB
            NetworkFsType::NTFS => 2 * 1024 * 1024, // 2MB - Windows performs well at this size
            NetworkFsType::APFS => 2 * 1024 * 1024, // 2MB - optimized for SSD access patterns
            NetworkFsType::Local => 2 * 1024 * 1024, // 2MB - safe default for unknown local FS
            NetworkFsType::Unknown => 1024 * 1024, // 1MB - conservative for unknown filesystem
        }
    }

    /// Get optimization recommendations for filesystem type
    pub fn get_optimization_recommendations(&self, fs_info: &NetworkFsInfo) -> Vec<String> {
        let mut recommendations = Vec::new();
        
        match fs_info.fs_type {
            NetworkFsType::NFS => {
                recommendations.push("Using 4MB buffers - research-optimal for NFS throughput".to_string());
                recommendations.push("Enable parallel transfers if supported".to_string());
                recommendations.push("Consider disabling checksums for local network".to_string());
            }
            NetworkFsType::SMB => {
                recommendations.push("Using 4MB buffers - optimal balance for SMB3 performance".to_string());
                recommendations.push("Enable SMB multichannel if available".to_string());
                recommendations.push("Consider compression for slow links".to_string());
            }
            NetworkFsType::SSHFS => {
                recommendations.push("Using 1MB buffers - balanced for SSH crypto overhead".to_string());
                recommendations.push("Enable compression for better performance".to_string());
                recommendations.push("Limited parallelism due to SSH channel constraints".to_string());
            }
            NetworkFsType::WebDAV => {
                recommendations.push("Using 2MB buffers for HTTP chunked transfers".to_string());
                recommendations.push("Enable compression if supported".to_string());
                recommendations.push("Implement retry logic for HTTP errors".to_string());
            }
            NetworkFsType::ZFS => {
                recommendations.push("Using 1MB buffers matching typical ZFS recordsize".to_string());
                recommendations.push("Enable reflink for copy-on-write".to_string());
                recommendations.push("Consider ZFS compression settings".to_string());
            }
            NetworkFsType::BTRFS => {
                recommendations.push("Using 2MB buffers - optimal for BTRFS CoW operations".to_string());
                recommendations.push("Enable reflink for copy-on-write".to_string());
                recommendations.push("Consider BTRFS compression".to_string());
            }
            NetworkFsType::XFS => {
                recommendations.push("Using 2MB buffers - research-optimal for XFS extents".to_string());
                recommendations.push("Enable extent-based copying".to_string());
                recommendations.push("Optimized for large file performance".to_string());
            }
            NetworkFsType::EXT4 => {
                recommendations.push("Using 2MB buffers - peak performance for ext4".to_string());
                recommendations.push("Enable extent-based copying".to_string());
                recommendations.push("Standard Linux filesystem optimizations".to_string());
            }
            NetworkFsType::NTFS => {
                recommendations.push("Using 2MB buffers - optimal for modern NTFS".to_string());
                recommendations.push("Handle Windows file attributes".to_string());
                recommendations.push("Consider NTFS compression".to_string());
            }
            NetworkFsType::APFS => {
                recommendations.push("Using 2MB buffers - optimized for APFS/SSD patterns".to_string());
                recommendations.push("Enable reflink for copy-on-write".to_string());
                recommendations.push("Handle Apple extended attributes".to_string());
            }
            NetworkFsType::Local => {
                recommendations.push("Use standard local filesystem optimizations".to_string());
                recommendations.push("Enable reflink if supported".to_string());
            }
            NetworkFsType::Unknown => {
                recommendations.push("Use conservative settings for unknown filesystem".to_string());
            }
        }
        
        recommendations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_network_fs_detector_creation() {
        let detector = NetworkFsDetector::new();
        assert!(detector.fs_cache.is_empty());
    }

    #[test]
    fn test_filesystem_detection() {
        let mut detector = NetworkFsDetector::new();
        let path = PathBuf::from("/");
        
        let fs_info = detector.detect_filesystem(&path);
        
        // Should detect some filesystem type
        assert!(matches!(fs_info.fs_type, 
            NetworkFsType::Local | NetworkFsType::Unknown | NetworkFsType::NFS | NetworkFsType::SMB |
            NetworkFsType::ZFS | NetworkFsType::BTRFS | NetworkFsType::XFS | NetworkFsType::EXT4));
        assert!(!fs_info.mount_point.is_empty());
    }

    #[test]
    fn test_optimization_recommendations() {
        let detector = NetworkFsDetector::new();
        
        let nfs_info = NetworkFsInfo {
            fs_type: NetworkFsType::NFS,
            mount_point: "/mnt/nfs".to_string(),
            server: Some("server.local".to_string()),
            version: Some("4.0".to_string()),
            latency_ms: Some(2),
            bandwidth_mbps: Some(1000),
            supports_large_transfers: true,
            optimal_buffer_size: 1024 * 1024,
        };
        
        let recommendations = detector.get_optimization_recommendations(&nfs_info);
        assert!(!recommendations.is_empty());
        assert!(recommendations.iter().any(|r| r.contains("buffer")));
    }

    #[test]
    fn test_buffer_size_recommendations() {
        let detector = NetworkFsDetector::new();
        
        // Network filesystems
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::NFS), 4 * 1024 * 1024);
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::SMB), 4 * 1024 * 1024);
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::SSHFS), 1024 * 1024);
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::WebDAV), 2 * 1024 * 1024);
        
        // Local filesystems
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::ZFS), 1024 * 1024);
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::BTRFS), 2 * 1024 * 1024);
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::XFS), 2 * 1024 * 1024);
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::EXT4), 2 * 1024 * 1024);
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::NTFS), 2 * 1024 * 1024);
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::APFS), 2 * 1024 * 1024);
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::Local), 2 * 1024 * 1024);
        assert_eq!(detector.get_optimal_buffer_size(&NetworkFsType::Unknown), 1024 * 1024);
    }
}