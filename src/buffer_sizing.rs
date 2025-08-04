use std::cmp::{max, min};
use crate::options::SyncOptions;

pub struct BufferSizer {
    max_buffer_size: usize,
    min_buffer_size: usize,
    memory_fraction: f64,
}

impl BufferSizer {
    pub fn new(options: &SyncOptions) -> Self {
        let memory_fraction = options.buffer_memory_fraction.unwrap_or(0.1);
        // Default min/max buffer sizes, can be refined
        let min_buffer_size = 64 * 1024; // 64KB
        let max_buffer_size = 8 * 1024 * 1024; // 8MB

        BufferSizer {
            max_buffer_size,
            min_buffer_size,
            memory_fraction,
        }
    }

    pub fn calculate_buffer_size(&self, file_size: u64) -> usize {
        let available_memory = sys_info::mem_info().map_or(0, |mem| mem.avail * 1024); // avail in KB
        let max_allowed_by_memory = (available_memory as f64 * self.memory_fraction) as usize;

        let mut calculated_size = if file_size < 1 * 1024 * 1024 { // < 1MB
            64 * 1024 // 64KB
        } else if file_size < 100 * 1024 * 1024 { // 1MB - 100MB
            // Scale from 256KB to 1MB
            let scale_factor = (file_size as f64 - 1.0 * 1024.0 * 1024.0) / (99.0 * 1024.0 * 1024.0);
            (256.0 * 1024.0 + (768.0 * 1024.0 * scale_factor)) as usize
        } else { // > 100MB
            // Scale from 2MB to 8MB
            let scale_factor = (file_size as f64 - 100.0 * 1024.0 * 1024.0) / (900.0 * 1024.0 * 1024.0); // Assuming max file size of 1GB for scaling
            (2.0 * 1024.0 * 1024.0 + (6.0 * 1024.0 * 1024.0 * scale_factor)) as usize
        };
        calculated_size = min(max(calculated_size, self.min_buffer_size), self.max_buffer_size);

        // Cap by available memory
        if calculated_size > max_allowed_by_memory {
            eprintln!("Warning: Calculated buffer size ({}) capped by available memory ({}).", calculated_size, max_allowed_by_memory);
            calculated_size = max_allowed_by_memory;
        }

        // Round up to nearest power of 2
        round_up_to_power_of_two(calculated_size)
    }
}

fn round_up_to_power_of_two(size: usize) -> usize {
    if size == 0 { return 0; }
    let mut power_of_two = 1;
    while power_of_two < size { 
        power_of_two <<= 1; 
    }
    power_of_two
}
