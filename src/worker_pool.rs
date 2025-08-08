//! Global persistent worker pool for RoboSync
//! 
//! This module implements a global thread pool that's initialized once at application
//! startup, eliminating the 27ms thread spawning overhead identified in performance analysis.
//! 
//! Uses Rayon for efficient work-stealing and zero-overhead task scheduling.

use std::sync::Once;
use rayon::{ThreadPool, ThreadPoolBuilder};
use anyhow::Result;

/// Global thread pool instance
static mut GLOBAL_POOL: Option<ThreadPool> = None;
static INIT: Once = Once::new();

/// Initialize the global thread pool
/// 
/// This should be called once at application startup. Subsequent calls are ignored.
/// 
/// # Arguments
/// * `num_threads` - Number of worker threads. If None, uses num_cpus * 1.5
pub fn initialize_global_pool(num_threads: Option<usize>) -> Result<()> {
    INIT.call_once(|| {
        let threads = num_threads.unwrap_or_else(|| {
            // Follow unified strategy: num_cpus * 1.5 for network optimization
            (num_cpus::get() as f32 * 1.5).round() as usize
        });
        
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("robosync-worker-{}", i))
            .build()
            .expect("Failed to create global thread pool");
        
        unsafe {
            GLOBAL_POOL = Some(pool);
        }
        
        // Don't show initialization message - it's internal implementation detail
    });
    
    Ok(())
}

/// Get a reference to the global thread pool
/// 
/// # Panics
/// Panics if the pool hasn't been initialized with `initialize_global_pool()`
pub fn global_pool() -> &'static ThreadPool {
    unsafe {
        GLOBAL_POOL.as_ref()
            .expect("Global thread pool not initialized. Call initialize_global_pool() first.")
    }
}

/// Check if the global pool is initialized
pub fn is_initialized() -> bool {
    unsafe { GLOBAL_POOL.is_some() }
}

/// Get the number of threads in the global pool
pub fn thread_count() -> usize {
    global_pool().current_num_threads()
}

/// Execute a parallel iterator operation using the global pool
pub fn execute<T, F, I>(iterable: I, op: F) 
where
    T: Send,
    F: Fn(T) + Sync + Send,
    I: rayon::iter::IntoParallelIterator<Item = T> + Send,
{
    global_pool().install(move || {
        use rayon::prelude::*;
        iterable.into_par_iter().for_each(op);
    });
}

/// Execute a collection of tasks in parallel using intelligent task batching
/// 
/// This implements the "16-32 files per task" batching strategy to reduce
/// synchronization overhead identified in the performance analysis.
pub fn execute_batched<T, F>(items: Vec<T>, batch_size: usize, op: F)
where
    T: Send + Sync,
    F: Fn(&[T]) + Sync + Send,
{
    global_pool().install(move || {
        use rayon::prelude::*;
        
        items
            .par_chunks(batch_size)
            .for_each(|batch| op(batch));
    });
}

/// Scope for running multiple parallel tasks
pub fn scope<'scope, OP, R>(op: OP) -> R
where
    OP: FnOnce(&rayon::Scope<'scope>) -> R + Send,
    R: Send,
{
    global_pool().scope(op)
}

/// Configuration for the global pool
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Number of worker threads
    pub num_threads: usize,
    /// Thread name prefix
    pub thread_prefix: String,
    /// Stack size for worker threads (optional)
    pub stack_size: Option<usize>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            num_threads: (num_cpus::get() as f32 * 1.5).round() as usize,
            thread_prefix: "robosync-worker".to_string(),
            stack_size: None,
        }
    }
}

impl PoolConfig {
    /// Create config optimized for network transfers
    pub fn for_network(bandwidth_gbps: f32) -> Self {
        // Rule from unified strategy: 1 worker per 1-2Gbps
        let network_threads = (bandwidth_gbps / 1.5).ceil() as usize;
        let cpu_threads = num_cpus::get();
        let optimal_threads = network_threads.max(cpu_threads).min(64); // Cap at 64
        
        Self {
            num_threads: optimal_threads,
            thread_prefix: "robosync-net".to_string(),
            stack_size: Some(2 * 1024 * 1024), // 2MB stack for network operations
        }
    }
    
    /// Create config optimized for local transfers
    pub fn for_local() -> Self {
        Self {
            num_threads: num_cpus::get(),
            thread_prefix: "robosync-local".to_string(),
            stack_size: Some(1024 * 1024), // 1MB stack for local operations
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_pool_config() {
        let config = PoolConfig::default();
        assert!(config.num_threads > 0);
        
        let net_config = PoolConfig::for_network(10.0); // 10GbE
        assert!(net_config.num_threads >= num_cpus::get());
        
        let local_config = PoolConfig::for_local();
        assert_eq!(local_config.num_threads, num_cpus::get());
    }
}