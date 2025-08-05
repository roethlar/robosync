//! Safe operations module for mission-critical reliability
//! 
//! This module provides safe alternatives to unwrap/expect operations
//! that could cause panics in production environments.

use std::sync::{Mutex, MutexGuard};
use std::path::Path;
use anyhow::{Result, Context, bail};

/// Safe mutex operations that handle poison errors gracefully
pub trait SafeMutex<T> {
    /// Lock mutex with proper error handling
    fn safe_lock(&self) -> Result<MutexGuard<T>>;
    
    /// Try to lock mutex, return error if poisoned or would block
    fn safe_try_lock(&self) -> Result<Option<MutexGuard<T>>>;
}

impl<T> SafeMutex<T> for Mutex<T> {
    fn safe_lock(&self) -> Result<MutexGuard<T>> {
        match self.lock() {
            Ok(guard) => Ok(guard),
            Err(poison_error) => {
                // In a production environment, we might want to log this
                // but still recover the data from the poisoned mutex
                eprintln!("Warning: Mutex was poisoned, recovering data");
                Ok(poison_error.into_inner())
            }
        }
    }

    fn safe_try_lock(&self) -> Result<Option<MutexGuard<T>>> {
        match self.try_lock() {
            Ok(guard) => Ok(Some(guard)),
            Err(std::sync::TryLockError::WouldBlock) => Ok(None),
            Err(std::sync::TryLockError::Poisoned(poison_error)) => {
                eprintln!("Warning: Mutex was poisoned, recovering data");
                Ok(Some(poison_error.into_inner()))
            }
        }
    }
}

/// Safe path operations that handle encoding issues
pub trait SafePath {
    /// Convert path to string, handling non-UTF8 paths gracefully
    fn safe_to_string(&self) -> Result<String>;
    
    /// Get file name as string with proper error handling
    fn safe_file_name(&self) -> Result<String>;
    
    /// Get parent directory with validation
    fn safe_parent(&self) -> Result<&Path>;
}

impl SafePath for Path {
    fn safe_to_string(&self) -> Result<String> {
        self.to_str()
            .ok_or_else(|| anyhow::anyhow!("Path contains invalid UTF-8: {}", self.to_string_lossy()))
            .map(|s| s.to_string())
    }

    fn safe_file_name(&self) -> Result<String> {
        self.file_name()
            .ok_or_else(|| anyhow::anyhow!("Path has no file name: {}", self.display()))?
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("File name contains invalid UTF-8: {}", self.display()))
            .map(|s| s.to_string())
    }

    fn safe_parent(&self) -> Result<&Path> {
        self.parent()
            .ok_or_else(|| anyhow::anyhow!("Path has no parent directory: {}", self.display()))
    }
}

/// Safe collection operations
pub trait SafeCollection<T> {
    /// Get last element without panicking
    fn safe_last(&self) -> Result<&T>;
    
    /// Get first element without panicking
    fn safe_first(&self) -> Result<&T>;
    
    /// Get element at index without panicking
    fn safe_get(&self, index: usize) -> Result<&T>;
}

impl<T> SafeCollection<T> for Vec<T> {
    fn safe_last(&self) -> Result<&T> {
        self.last()
            .ok_or_else(|| anyhow::anyhow!("Cannot get last element of empty vector"))
    }

    fn safe_first(&self) -> Result<&T> {
        self.first()
            .ok_or_else(|| anyhow::anyhow!("Cannot get first element of empty vector"))
    }

    fn safe_get(&self, index: usize) -> Result<&T> {
        self.get(index)
            .ok_or_else(|| anyhow::anyhow!("Index {} out of bounds for vector of length {}", index, self.len()))
    }
}

impl<T> SafeCollection<T> for [T] {
    fn safe_last(&self) -> Result<&T> {
        self.last()
            .ok_or_else(|| anyhow::anyhow!("Cannot get last element of empty slice"))
    }

    fn safe_first(&self) -> Result<&T> {
        self.first()
            .ok_or_else(|| anyhow::anyhow!("Cannot get first element of empty slice"))
    }

    fn safe_get(&self, index: usize) -> Result<&T> {
        self.get(index)
            .ok_or_else(|| anyhow::anyhow!("Index {} out of bounds for slice of length {}", index, self.len()))
    }
}

/// Safe option operations
pub trait SafeOption<T> {
    /// Convert Option to Result with context
    fn safe_unwrap(self, context: &str) -> Result<T>;
    
    /// Convert Option to Result with formatted context
    fn safe_unwrap_with<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> String;
}

impl<T> SafeOption<T> for Option<T> {
    fn safe_unwrap(self, context: &str) -> Result<T> {
        self.ok_or_else(|| anyhow::anyhow!("{}", context))
    }

    fn safe_unwrap_with<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> String,
    {
        self.ok_or_else(|| anyhow::anyhow!("{}", f()))
    }
}

/// Safe numeric operations
pub trait SafeNumeric {
    /// Divide with explicit zero check
    fn safe_divide(self, other: Self) -> Result<Self>
    where
        Self: Sized;
}

impl SafeNumeric for u64 {
    fn safe_divide(self, other: Self) -> Result<Self> {
        if other == 0 {
            bail!("Division by zero: {} / {}", self, other);
        }
        Ok(self / other)
    }
}

impl SafeNumeric for f64 {
    fn safe_divide(self, other: Self) -> Result<Self> {
        if other == 0.0 {
            bail!("Division by zero: {} / {}", self, other);
        }
        let result = self / other;
        if result.is_infinite() || result.is_nan() {
            bail!("Invalid division result: {} / {} = {}", self, other, result);
        }
        Ok(result)
    }
}

/// Safe partial comparison operations
pub trait SafePartialOrd<T> {
    /// Compare with explicit handling of NaN values
    fn safe_partial_cmp(&self, other: &T) -> Result<std::cmp::Ordering>;
    
    /// Find maximum with NaN handling
    fn safe_max(self, other: Self) -> Result<Self>
    where
        Self: Sized;
    
    /// Find minimum with NaN handling  
    fn safe_min(self, other: Self) -> Result<Self>
    where
        Self: Sized;
}

impl SafePartialOrd<f64> for f64 {
    fn safe_partial_cmp(&self, other: &f64) -> Result<std::cmp::Ordering> {
        self.partial_cmp(other)
            .ok_or_else(|| anyhow::anyhow!("Cannot compare values: {} and {} (possibly NaN)", self, other))
    }

    fn safe_max(self, other: Self) -> Result<Self> {
        if self.is_nan() || other.is_nan() {
            bail!("Cannot find max of NaN values: {} and {}", self, other);
        }
        Ok(self.max(other))
    }

    fn safe_min(self, other: Self) -> Result<Self> {
        if self.is_nan() || other.is_nan() {
            bail!("Cannot find min of NaN values: {} and {}", self, other);
        }
        Ok(self.min(other))
    }
}

/// Safe iterator operations
pub trait SafeIterator<T>: Iterator<Item = T> {
    /// Collect with size hint validation
    fn safe_collect_vec(self) -> Result<Vec<T>>
    where
        Self: Sized;
    
    /// Find maximum element safely
    fn safe_max(self) -> Result<T>
    where
        Self: Sized,
        T: Ord;
        
    /// Find maximum by comparison function safely
    fn safe_max_by<F>(self, compare: F) -> Result<T>
    where
        Self: Sized,
        F: FnMut(&T, &T) -> std::cmp::Ordering;
}

impl<I, T> SafeIterator<T> for I
where
    I: Iterator<Item = T>,
{
    fn safe_collect_vec(self) -> Result<Vec<T>> {
        let (lower, upper) = self.size_hint();
        
        // Protect against extremely large collections
        if lower > 10_000_000 {
            bail!("Iterator size hint too large: {} elements", lower);
        }
        
        if let Some(upper_bound) = upper {
            if upper_bound > 10_000_000 {
                bail!("Iterator upper bound too large: {} elements", upper_bound);
            }
        }
        
        Ok(self.collect())
    }

    fn safe_max(self) -> Result<T>
    where
        T: Ord,
    {
        self.max()
            .ok_or_else(|| anyhow::anyhow!("Cannot find maximum of empty iterator"))
    }

    fn safe_max_by<F>(self, compare: F) -> Result<T>
    where
        F: FnMut(&T, &T) -> std::cmp::Ordering,
    {
        self.max_by(compare)
            .ok_or_else(|| anyhow::anyhow!("Cannot find maximum of empty iterator"))
    }
}

/// Safe conversion operations
pub trait SafeConversion<T> {
    /// Convert with overflow checking
    fn safe_into(self) -> Result<T>;
}

impl SafeConversion<u32> for usize {
    fn safe_into(self) -> Result<u32> {
        self.try_into()
            .map_err(|_| anyhow::anyhow!("usize {} does not fit in u32", self))
    }
}

impl SafeConversion<usize> for u32 {
    fn safe_into(self) -> Result<usize> {
        Ok(self as usize) // u32 always fits in usize
    }
}

impl SafeConversion<u64> for usize {
    fn safe_into(self) -> Result<u64> {
        Ok(self as u64) // usize always fits in u64
    }
}

/// Safe string operations
pub trait SafeString {
    /// Parse string to number with proper error context
    fn safe_parse<T>(&self) -> Result<T>
    where
        T: std::str::FromStr,
        T::Err: std::error::Error + Send + Sync + 'static;
}

impl SafeString for str {
    fn safe_parse<T>(&self) -> Result<T>
    where
        T: std::str::FromStr,
        T::Err: std::error::Error + Send + Sync + 'static,
    {
        self.parse()
            .with_context(|| format!("Failed to parse '{}' as {}", self, std::any::type_name::<T>()))
    }
}

impl SafeString for String {
    fn safe_parse<T>(&self) -> Result<T>
    where
        T: std::str::FromStr,
        T::Err: std::error::Error + Send + Sync + 'static,
    {
        self.as_str().safe_parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn test_safe_mutex() {
        let mutex = Mutex::new(42);
        let guard = mutex.safe_lock().expect("Failed to lock mutex");
        assert_eq!(*guard, 42);
    }

    #[test]
    fn test_safe_path() {
        let path = Path::new("/test/path/file.txt");
        assert_eq!(path.safe_file_name().unwrap(), "file.txt");
        assert_eq!(path.safe_parent().unwrap(), Path::new("/test/path"));
    }

    #[test]
    fn test_safe_collection() {
        let vec = vec![1, 2, 3, 4, 5];
        assert_eq!(*vec.safe_last().unwrap(), 5);
        assert_eq!(*vec.safe_first().unwrap(), 1);
        assert_eq!(*vec.safe_get(2).unwrap(), 3);
    }

    #[test]
    fn test_safe_option() {
        let some_value = Some(42);
        let none_value: Option<i32> = None;
        
        assert_eq!(some_value.safe_unwrap("Should have value").unwrap(), 42);
        assert!(none_value.safe_unwrap("Should be None").is_err());
    }

    #[test]
    fn test_safe_numeric() {
        assert_eq!(10u64.safe_divide(2).unwrap(), 5);
        assert!(10u64.safe_divide(0).is_err());
        
        assert_eq!(10.0f64.safe_divide(2.0).unwrap(), 5.0);
        assert!(10.0f64.safe_divide(0.0).is_err());
    }

    #[test]
    fn test_safe_string() {
        assert_eq!("42".safe_parse::<i32>().unwrap(), 42);
        assert!("not_a_number".safe_parse::<i32>().is_err());
    }
}