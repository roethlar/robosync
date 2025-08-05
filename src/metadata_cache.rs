use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use lazy_static::lazy_static;
use parking_lot::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MetadataType {
    Metadata,
    SymlinkMetadata,
}

struct CacheEntry {
    metadata: fs::Metadata,
    timestamp: Instant,
    meta_type: MetadataType,
}

pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub current_entries: u64,
    pub total_memory_bytes: u64,
}

struct InnerCache {
    cache: HashMap<PathBuf, CacheEntry>,
    hits: u64,
    misses: u64,
    evictions: u64,
    max_entries: usize,
    ttl: Duration,
}

impl InnerCache {
    fn new(max_entries: usize, ttl: Duration) -> Self {
        InnerCache {
            cache: HashMap::new(),
            hits: 0,
            misses: 0,
            evictions: 0,
            max_entries,
            ttl,
        }
    }

    fn get(&mut self, path: &Path, meta_type: MetadataType) -> Option<fs::Metadata> {
        if let Some(entry) = self.cache.get(path) {
            if entry.meta_type == meta_type && entry.timestamp.elapsed() < self.ttl {
                self.hits += 1;
                return Some(entry.metadata.clone());
            } else {
                // Expired or wrong type, evict
                self.cache.remove(path);
                self.evictions += 1;
            }
        }
        self.misses += 1;
        None
    }

    fn insert(&mut self, path: PathBuf, metadata: fs::Metadata, meta_type: MetadataType) {
        if self.cache.len() >= self.max_entries {
            // Simple eviction: remove oldest entry (not truly LRU, but simple)
            if let Some(oldest_path) = self.cache.keys().next().cloned() {
                self.cache.remove(&oldest_path);
                self.evictions += 1;
            }
        }
        self.cache.insert(
            path,
            CacheEntry {
                metadata,
                timestamp: Instant::now(),
                meta_type,
            },
        );
    }

    fn invalidate(&mut self, path: &Path) {
        self.cache.remove(path);
        // Invalidate children by prefix matching
        self.cache.retain(|k, _| !k.starts_with(path));
    }

    fn invalidate_all(&mut self) {
        self.cache.clear();
    }

    fn get_stats(&self) -> CacheStats {
        let mut total_memory_bytes = 0;
        for (path, entry) in &self.cache {
            total_memory_bytes += path.as_os_str().len(); // Estimate path memory
            // Add a rough estimate for metadata size (e.g., 100 bytes per entry)
            total_memory_bytes += 100;
        }

        CacheStats {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            current_entries: self.cache.len() as u64,
            total_memory_bytes: total_memory_bytes as u64,
        }
    }
}

lazy_static! {
    static ref METADATA_CACHE: RwLock<InnerCache> = RwLock::new(InnerCache::new(100_000, Duration::from_secs(30)));
}

pub struct MetadataCache;

impl MetadataCache {
    pub fn get_metadata(path: &Path) -> Result<fs::Metadata> {
        let mut cache = METADATA_CACHE.write();
        if let Some(metadata) = cache.get(path, MetadataType::Metadata) {
            return Ok(metadata);
        }

        let metadata = fs::metadata(path)?;
        cache.insert(path.to_path_buf(), metadata.clone(), MetadataType::Metadata);
        Ok(metadata)
    }

    pub fn get_symlink_metadata(path: &Path) -> Result<fs::Metadata> {
        let mut cache = METADATA_CACHE.write();
        if let Some(metadata) = cache.get(path, MetadataType::SymlinkMetadata) {
            return Ok(metadata);
        }

        let metadata = fs::symlink_metadata(path)?;
        cache.insert(path.to_path_buf(), metadata.clone(), MetadataType::SymlinkMetadata);
        Ok(metadata)
    }

    pub fn invalidate(path: &Path) {
        METADATA_CACHE.write().invalidate(path);
    }

    pub fn invalidate_all() {
        METADATA_CACHE.write().invalidate_all();
    }

    pub fn get_stats() -> CacheStats {
        METADATA_CACHE.read().get_stats()
    }

    pub fn configure(max_entries: usize, ttl_seconds: u64) {
        let mut cache = METADATA_CACHE.write();
        cache.max_entries = max_entries;
        cache.ttl = Duration::from_secs(ttl_seconds);
    }
}
