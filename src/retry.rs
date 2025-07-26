//! Retry logic for handling transient failures

use anyhow::{Result, Context};
use std::time::Duration;
use std::thread;
use crate::logging::SyncLogger;

/// Retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Number of retry attempts (0 means no retries)
    pub max_retries: u32,
    /// Wait time between retries in seconds
    pub wait_seconds: u32,
}

impl RetryConfig {
    pub fn new(max_retries: u32, wait_seconds: u32) -> Self {
        Self {
            max_retries,
            wait_seconds,
        }
    }
    
    pub fn should_retry(&self) -> bool {
        self.max_retries > 0
    }
}

/// Execute an operation with retry logic
pub fn with_retry<F, T>(
    operation: F,
    config: &RetryConfig,
    description: &str,
    mut logger: Option<&mut SyncLogger>,
) -> Result<T>
where
    F: Fn() -> Result<T>,
{
    let mut last_error = None;
    
    for attempt in 0..=config.max_retries {
        match operation() {
            Ok(result) => {
                if attempt > 0 {
                    if let Some(ref mut log) = logger {
                        log.log(&format!("    {description} succeeded after {attempt} retries"));
                    }
                }
                return Ok(result);
            }
            Err(e) => {
                last_error = Some(e);
                
                if attempt < config.max_retries {
                    if let Some(ref mut log) = logger {
                        log.log(&format!(
                            "    {} failed (attempt {}/{}): {}. Retrying in {} seconds...",
                            description,
                            attempt + 1,
                            config.max_retries + 1,
                            last_error.as_ref().unwrap(),
                            config.wait_seconds
                        ));
                    }
                    
                    thread::sleep(Duration::from_secs(config.wait_seconds as u64));
                }
            }
        }
    }
    
    // All retries exhausted
    Err(last_error.unwrap())
        .with_context(|| format!("{} failed after {} retries", description, config.max_retries))
}

/// Check if an error is retryable
pub fn is_retryable_error(error: &anyhow::Error) -> bool {
    // Check the error chain for specific error types
    let error_string = error.to_string().to_lowercase();
    
    // File system errors that are typically transient
    if error_string.contains("permission denied") ||
       error_string.contains("access is denied") ||
       error_string.contains("sharing violation") ||
       error_string.contains("resource temporarily unavailable") ||
       error_string.contains("too many open files") ||
       error_string.contains("device or resource busy") {
        return true;
    }
    
    // Network errors (for future remote sync support)
    if error_string.contains("connection refused") ||
       error_string.contains("connection reset") ||
       error_string.contains("timeout") ||
       error_string.contains("network unreachable") {
        return true;
    }
    
    // Check if it's an I/O error
    if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
        matches!(io_error.kind(), 
            std::io::ErrorKind::PermissionDenied |
            std::io::ErrorKind::WouldBlock |
            std::io::ErrorKind::TimedOut |
            std::io::ErrorKind::Interrupted
        )
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    
    #[test]
    fn test_retry_success_first_attempt() {
        let config = RetryConfig::new(3, 1);
        let result = with_retry(
            || Ok(42),
            &config,
            "test operation",
            None,
        );
        assert_eq!(result.unwrap(), 42);
    }
    
    #[test]
    fn test_retry_success_after_failures() {
        let config = RetryConfig::new(3, 0); // 0 second wait for tests
        let attempt_count = AtomicU32::new(0);
        
        let result = with_retry(
            || {
                let count = attempt_count.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    Err(anyhow::anyhow!("Temporary failure"))
                } else {
                    Ok(42)
                }
            },
            &config,
            "test operation",
            None,
        );
        
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt_count.load(Ordering::SeqCst), 3);
    }
    
    #[test]
    fn test_retry_all_failures() {
        let config = RetryConfig::new(2, 0); // 0 second wait for tests
        let result: Result<i32> = with_retry(
            || Err(anyhow::anyhow!("Permanent failure")),
            &config,
            "test operation",
            None,
        );
        
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("failed after 2 retries"));
    }
    
    #[test]
    fn test_is_retryable_error() {
        // Retryable errors
        assert!(is_retryable_error(&anyhow::anyhow!("Permission denied")));
        assert!(is_retryable_error(&anyhow::anyhow!("Access is denied")));
        assert!(is_retryable_error(&anyhow::anyhow!("Resource temporarily unavailable")));
        
        // Non-retryable errors
        assert!(!is_retryable_error(&anyhow::anyhow!("File not found")));
        assert!(!is_retryable_error(&anyhow::anyhow!("Invalid argument")));
    }
}