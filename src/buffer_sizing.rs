use std::cmp::{max, min};
use parking_lot::Mutex;
use crate::options::SyncOptions;
use crate::network_fs::{NetworkFsInfo, NetworkFsType};

#[cfg(windows)]
use winapi::um::sysinfoapi::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

pub struct BufferSizer {
    max_buffer_size: usize,
    min_buffer_size: usize,
    memory_fraction: f64,
    cached_available_memory: Mutex<Option<u64>>,
}

impl Clone for BufferSizer {
    fn clone(&self) -> Self {
        BufferSizer {
            max_buffer_size: self.max_buffer_size,
            min_buffer_size: self.min_buffer_size,
            memory_fraction: self.memory_fraction,
            cached_available_memory: Mutex::new(None), // Start fresh for cloned instance
        }
    }
}

impl BufferSizer {
    pub fn new(options: &SyncOptions) -> Self {
        let memory_fraction = options.buffer_memory_fraction.unwrap_or(0.1);
        // Optimized for 10GbE sustained transfers - Phase 1 Hybrid Dam optimization
        let min_buffer_size = 1024 * 1024; // 1MB - eliminates small buffer overhead
        let max_buffer_size = 16 * 1024 * 1024; // 16MB - enables high bandwidth utilization

        BufferSizer {
            max_buffer_size,
            min_buffer_size,
            memory_fraction,
            cached_available_memory: Mutex::new(None),
        }
    }

    /// Get available memory using Windows API (more accurate than sys_info on Windows)
    #[cfg(windows)]
    fn get_available_memory_windows() -> Result<u64, String> {
        unsafe {
            let mut mem_status: MEMORYSTATUSEX = std::mem::zeroed();
            mem_status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            
            if GlobalMemoryStatusEx(&mut mem_status) != 0 {
                Ok(mem_status.ullAvailPhys)
            } else {
                Err("GlobalMemoryStatusEx failed".to_string())
            }
        }
    }

    /// Get available memory using sys_info (fallback for non-Windows platforms)
    #[cfg(not(windows))]
    fn get_available_memory_sys_info() -> Result<u64, String> {
        match sys_info::mem_info() {
            Ok(mem) => {
                if mem.avail == 0 {
                    // Fallback: use 25% of total memory if available memory is 0
                    Ok(mem.total * 1024 / 4)
                } else {
                    Ok(mem.avail * 1024) // avail in KB, convert to bytes
                }
            },
            Err(e) => Err(format!("Failed to get memory info: {}", e))
        }
    }

    pub fn calculate_buffer_size(&self, file_size: u64) -> usize {
        self.calculate_buffer_size_with_fs(file_size, None)
    }

    pub fn calculate_buffer_size_with_fs(&self, file_size: u64, fs_info: Option<&NetworkFsInfo>) -> usize {
        let available_memory = {
            let mut cached = self.cached_available_memory.lock();
            if let Some(cached_mem) = *cached {
                cached_mem
            } else {
                let detected_memory = {
                    #[cfg(windows)]
                    {
                        match Self::get_available_memory_windows() {
                            Ok(memory) => {
                                if memory == 0 {
                                    eprintln!("Warning: Windows API reported 0 available memory, using 2GB fallback");
                                    2 * 1024 * 1024 * 1024  // 2GB fallback
                                } else {
                                    memory
                                }
                            },
                            Err(e) => {
                                eprintln!("Warning: Windows API failed ({}), trying sys_info fallback", e);
                                match sys_info::mem_info() {
                                    Ok(mem) => {
                                        if mem.avail == 0 {
                                            eprintln!("  sys_info also reports 0, using 25% of total memory ({:.1} GB)",
                                                     (mem.total as f64 / 1024.0 / 1024.0) * 0.25);
                                            mem.total * 1024 / 4
                                        } else {
                                            mem.avail * 1024
                                        }
                                    },
                                    Err(_) => {
                                        eprintln!("  sys_info also failed, using 2GB fallback");
                                        2 * 1024 * 1024 * 1024
                                    }
                                }
                            }
                        }
                    }
                    #[cfg(not(windows))]
                    {
                        match Self::get_available_memory_sys_info() {
                            Ok(memory) => memory,
                            Err(e) => {
                                eprintln!("Warning: Failed to get memory info ({}), using 2GB fallback", e);
                                2 * 1024 * 1024 * 1024  // 2GB fallback
                            }
                        }
                    }
                };
                *cached = Some(detected_memory);
                detected_memory
            }
        };
        let max_allowed_by_memory = (available_memory as f64 * self.memory_fraction) as usize;

        // Use filesystem-specific optimal buffer size if available
        let base_size = if let Some(fs_info) = fs_info {
            fs_info.optimal_buffer_size
        } else {
            // Hybrid Dam optimized buffer calculation - target large dataset transfers
            if file_size < 10 * 1024 * 1024 { // < 10MB (small/medium files)
                2 * 1024 * 1024 // 2MB - sufficient for network efficiency
            } else if file_size < 100 * 1024 * 1024 { // 10MB - 100MB
                // Scale from 4MB to 8MB for medium-large files
                let scale_factor = (file_size as f64 - 10.0 * 1024.0 * 1024.0) / (90.0 * 1024.0 * 1024.0);
                (4.0 * 1024.0 * 1024.0 + (4.0 * 1024.0 * 1024.0 * scale_factor)) as usize
            } else { // > 100MB (large files - Hybrid Dam "Slicer" component)
                // Scale from 8MB to 16MB for maximum throughput
                let scale_factor = (file_size as f64 - 100.0 * 1024.0 * 1024.0) / (900.0 * 1024.0 * 1024.0);
                (8.0 * 1024.0 * 1024.0 + (8.0 * 1024.0 * 1024.0 * scale_factor)).min(16.0 * 1024.0 * 1024.0) as usize
            }
        };

        // Apply network filesystem adjustments
        let mut calculated_size = if let Some(fs_info) = fs_info {
            match fs_info.fs_type {
                NetworkFsType::NFS => {
                    // NFS benefits from larger buffers for throughput
                    max(base_size, 2 * 1024 * 1024) // At least 2MB
                }
                NetworkFsType::SMB => {
                    // SMB has protocol limitations - modern SMB3 handles larger buffers
                    min(base_size, 4 * 1024 * 1024) // Cap at 4MB for SMB efficiency
                }
                NetworkFsType::SSHFS => {
                    // SSHFS has significant overhead - but still benefit from some buffering
                    min(base_size, 1024 * 1024) // Cap at 1MB (our new minimum)
                }
                NetworkFsType::WebDAV => {
                    // WebDAV has HTTP overhead - but modern HTTP/2 handles larger frames
                    min(base_size, 2 * 1024 * 1024) // Cap at 2MB
                }
                NetworkFsType::ZFS => {
                    // ZFS handles large blocks well - excellent for large dataset transfers
                    max(base_size, 4 * 1024 * 1024) // At least 4MB
                }
                NetworkFsType::BTRFS => {
                    // BTRFS good with moderate to large buffers
                    max(base_size, 2 * 1024 * 1024) // At least 2MB
                }
                NetworkFsType::XFS => {
                    // XFS excellent for large files - great for our use case
                    max(base_size, 4 * 1024 * 1024) // At least 4MB
                }
                NetworkFsType::EXT4 => {
                    // ext4 standard performance - modern ext4 handles large buffers well
                    max(base_size, 2 * 1024 * 1024) // At least 2MB
                }
                NetworkFsType::NTFS => {
                    // NTFS moderate performance - Windows can handle larger buffers
                    max(base_size, 2 * 1024 * 1024) // At least 2MB
                }
                NetworkFsType::APFS => {
                    // APFS modern filesystem - excellent for large transfers
                    max(base_size, 4 * 1024 * 1024) // At least 4MB
                }
                NetworkFsType::Local => base_size,
                NetworkFsType::Unknown => base_size,
            }
        } else {
            base_size
        };
        calculated_size = min(max(calculated_size, self.min_buffer_size), self.max_buffer_size);

        // Cap by available memory
        if calculated_size > max_allowed_by_memory {
            eprintln!("Warning: Calculated buffer size ({}) capped by available memory ({}).", calculated_size, max_allowed_by_memory);
            calculated_size = max_allowed_by_memory;
        }

        // Round up to nearest power of 2
        calculated_size.next_power_of_two()
    }
}
