// Global thread pool for RoboSync
// Eliminates ~27ms thread spawning overhead by reusing persistent pool

use once_cell::sync::Lazy;
use rayon::ThreadPool;
use std::sync::atomic::{AtomicBool, Ordering};

/// Global thread pool for all parallel operations
pub static GLOBAL_THREAD_POOL: Lazy<ThreadPool> = Lazy::new(|| {
    let num_threads = num_cpus::get();
    
    rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .thread_name(|i| format!("robosync-worker-{}", i))
        .panic_handler(|_| {
            // Log panic but don't crash the whole application
            eprintln!("Thread pool worker panicked, continuing...");
        })
        .build()
        .expect("Failed to create global thread pool")
});

/// Track if the thread pool has been initialized
static POOL_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize the global thread pool eagerly
/// Call this early in main() to avoid lazy initialization overhead
pub fn init_thread_pool() {
    // Force initialization of the lazy static
    Lazy::force(&GLOBAL_THREAD_POOL);
    POOL_INITIALIZED.store(true, Ordering::Relaxed);
    
    let num_threads = GLOBAL_THREAD_POOL.current_num_threads();
    // Use simple eprintln for now - can be replaced with proper logging later
    if std::env::var("ROBOSYNC_DEBUG").is_ok() {
        eprintln!("[THREAD_POOL] Initialized with {} threads", num_threads);
    }
}

/// Check if the thread pool has been initialized
pub fn is_initialized() -> bool {
    POOL_INITIALIZED.load(Ordering::Relaxed)
}

/// Execute a closure on the global thread pool
/// This is a convenience wrapper that ensures the pool is initialized
pub fn spawn<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    GLOBAL_THREAD_POOL.spawn(f);
}

/// Execute a closure on the global thread pool and wait for result
pub fn spawn_and_wait<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    GLOBAL_THREAD_POOL.install(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thread_pool_initialization() {
        init_thread_pool();
        assert!(is_initialized());
        
        // Verify we can spawn work
        let (tx, rx) = std::sync::mpsc::channel();
        spawn(move || {
            tx.send(42).unwrap();
        });
        
        let result = rx.recv().unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_spawn_and_wait() {
        init_thread_pool();
        
        let result = spawn_and_wait(|| {
            1 + 1
        });
        
        assert_eq!(result, 2);
    }
}