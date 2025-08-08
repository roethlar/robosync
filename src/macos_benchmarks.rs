//! macOS Performance Benchmarking Suite
//! 
//! Comprehensive benchmarking for RoboSync on macOS including:
//! - Apple Silicon vs Intel performance comparison
//! - APFS vs other filesystem performance
//! - Memory-mapped IO vs standard IO benchmarks
//! - Network filesystem performance analysis
//! - Reflink/clonefile optimization measurement
//! - Multi-threaded performance scaling

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use rayon::prelude::*;

#[cfg(target_os = "macos")]
use crate::macos_mmap::MacOSMemoryMapper;
#[cfg(target_os = "macos")]
use crate::macos_apfs::MacOSApfsManager;

/// Benchmark test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkConfig {
    pub test_name: String,
    pub file_sizes: Vec<u64>,
    pub thread_counts: Vec<usize>,
    pub iterations: usize,
    pub warmup_iterations: usize,
    pub test_directory: PathBuf,
}

/// Individual benchmark result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub test_name: String,
    pub file_size: u64,
    pub thread_count: usize,
    pub duration_ms: f64,
    pub throughput_mbps: f64,
    pub files_per_second: f64,
    pub cpu_usage_percent: f64,
    pub memory_usage_mb: f64,
    pub success_count: u64,
    pub error_count: u64,
}

/// Complete benchmark suite results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSuite {
    pub system_info: SystemInfo,
    pub results: Vec<BenchmarkResult>,
    pub summary: BenchmarkSummary,
    pub timestamp: String,
}

/// System information for benchmark context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub cpu_model: String,
    pub cpu_cores: usize,
    pub memory_gb: f64,
    pub is_apple_silicon: bool,
    pub macos_version: String,
    pub filesystem_type: String,
    pub apfs_features: Vec<String>,
}

/// Benchmark summary statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSummary {
    pub best_throughput_mbps: f64,
    pub worst_throughput_mbps: f64,
    pub average_throughput_mbps: f64,
    pub optimal_thread_count: usize,
    pub optimal_file_size: u64,
    pub recommended_strategy: String,
}

/// Performance counters for detailed analysis
struct PerformanceCounters {
    bytes_processed: AtomicU64,
    files_processed: AtomicU64,
    errors_encountered: AtomicU64,
    start_time: Instant,
}

impl PerformanceCounters {
    fn new() -> Self {
        PerformanceCounters {
            bytes_processed: AtomicU64::new(0),
            files_processed: AtomicU64::new(0),
            errors_encountered: AtomicU64::new(0),
            start_time: Instant::now(),
        }
    }
    
    fn add_bytes(&self, bytes: u64) {
        self.bytes_processed.fetch_add(bytes, Ordering::Relaxed);
    }
    
    fn add_file(&self) {
        self.files_processed.fetch_add(1, Ordering::Relaxed);
    }
    
    fn add_error(&self) {
        self.errors_encountered.fetch_add(1, Ordering::Relaxed);
    }
    
    fn get_throughput_mbps(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let bytes = self.bytes_processed.load(Ordering::Relaxed) as f64;
        if elapsed > 0.0 {
            (bytes / (1024.0 * 1024.0)) / elapsed
        } else {
            0.0
        }
    }
    
    fn get_files_per_second(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let files = self.files_processed.load(Ordering::Relaxed) as f64;
        if elapsed > 0.0 {
            files / elapsed
        } else {
            0.0
        }
    }
}

/// macOS Performance Benchmarking Suite
pub struct MacOSBenchmarkSuite {
    system_info: SystemInfo,
    config: BenchmarkConfig,
    results: Vec<BenchmarkResult>,
}

impl MacOSBenchmarkSuite {
    /// Create new benchmark suite
    pub fn new(config: BenchmarkConfig) -> Result<Self> {
        let system_info = Self::detect_system_info()?;
        
        Ok(MacOSBenchmarkSuite {
            system_info,
            config,
            results: Vec::new(),
        })
    }
    
    /// Detect system information
    fn detect_system_info() -> Result<SystemInfo> {
        use std::process::Command;
        
        // Get CPU model
        let cpu_output = Command::new("sysctl")
            .args(&["-n", "machdep.cpu.brand_string"])
            .output()
            .context("Failed to get CPU info")?;
        let cpu_model = String::from_utf8_lossy(&cpu_output.stdout).trim().to_string();
        
        // Get CPU core count
        let core_output = Command::new("sysctl")
            .args(&["-n", "hw.physicalcpu"])
            .output()
            .context("Failed to get CPU core count")?;
        let cpu_cores: usize = String::from_utf8_lossy(&core_output.stdout)
            .trim()
            .parse()
            .unwrap_or(1);
        
        // Get memory size
        let mem_output = Command::new("sysctl")
            .args(&["-n", "hw.memsize"])
            .output()
            .context("Failed to get memory size")?;
        let memory_bytes: u64 = String::from_utf8_lossy(&mem_output.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        let memory_gb = memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        
        // Detect Apple Silicon
        let is_apple_silicon = cpu_model.contains("Apple") && 
                              (cpu_model.contains("M1") || cpu_model.contains("M2") || cpu_model.contains("M3"));
        
        // Get macOS version
        let version_output = Command::new("sw_vers")
            .args(&["-productVersion"])
            .output()
            .context("Failed to get macOS version")?;
        let macos_version = String::from_utf8_lossy(&version_output.stdout).trim().to_string();
        
        // Detect filesystem type of current directory
        let fs_output = Command::new("df")
            .args(&["-T", "."])
            .output()
            .context("Failed to get filesystem type")?;
        let fs_info = String::from_utf8_lossy(&fs_output.stdout);
        let filesystem_type = if fs_info.contains("apfs") {
            "APFS".to_string()
        } else if fs_info.contains("hfs") {
            "HFS+".to_string()
        } else {
            "Unknown".to_string()
        };
        
        // Get APFS features if applicable
        let apfs_features = if filesystem_type == "APFS" {
            vec![
                "Clone Files".to_string(),
                "Snapshots".to_string(),
                "Compression".to_string(),
                "Encryption".to_string(),
            ]
        } else {
            vec![]
        };
        
        Ok(SystemInfo {
            cpu_model,
            cpu_cores,
            memory_gb,
            is_apple_silicon,
            macos_version,
            filesystem_type,
            apfs_features,
        })
    }
    
    /// Run complete benchmark suite
    pub fn run_all_benchmarks(&mut self) -> Result<BenchmarkSuite> {
        println!("🚀 Starting macOS RoboSync Benchmark Suite");
        println!("==========================================");
        self.print_system_info();
        
        // Ensure test directory exists
        std::fs::create_dir_all(&self.config.test_directory)?;
        
        // Run individual benchmark tests
        self.benchmark_sequential_copy()?;
        self.benchmark_parallel_copy()?;
        self.benchmark_memory_mapped_copy()?;
        self.benchmark_apfs_clonefile()?;
        self.benchmark_filesystem_comparison()?;
        self.benchmark_thread_scaling()?;
        
        // Generate summary
        let summary = self.generate_summary();
        
        let suite = BenchmarkSuite {
            system_info: self.system_info.clone(),
            results: self.results.clone(),
            summary,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        
        Ok(suite)
    }
    
    /// Print system information
    fn print_system_info(&self) {
        println!("System Information:");
        println!("  CPU: {}", self.system_info.cpu_model);
        println!("  Cores: {}", self.system_info.cpu_cores);
        println!("  Memory: {:.1} GB", self.system_info.memory_gb);
        println!("  Apple Silicon: {}", self.system_info.is_apple_silicon);
        println!("  macOS: {}", self.system_info.macos_version);
        println!("  Filesystem: {}", self.system_info.filesystem_type);
        println!();
    }
    
    /// Benchmark sequential file copying
    fn benchmark_sequential_copy(&mut self) -> Result<()> {
        println!("📋 Sequential Copy Benchmark");
        println!("----------------------------");
        
        for &file_size in &self.config.file_sizes {
            let test_result = self.run_sequential_copy_test(file_size)?;
            self.results.push(test_result);
            
            println!("  {} MB: {:.1} MB/s", 
                    file_size / (1024 * 1024), 
                    self.results.last().unwrap().throughput_mbps);
        }
        
        println!();
        Ok(())
    }
    
    /// Run single sequential copy test
    fn run_sequential_copy_test(&self, file_size: u64) -> Result<BenchmarkResult> {
        let counters = Arc::new(PerformanceCounters::new());
        let test_dir = self.config.test_directory.join("sequential");
        std::fs::create_dir_all(&test_dir)?;
        
        // Create test file
        let source_path = test_dir.join("source.bin");
        let dest_path = test_dir.join("dest.bin");
        self.create_test_file(&source_path, file_size)?;
        
        let start_time = Instant::now();
        
        // Run test iterations
        for _ in 0..self.config.iterations {
            if dest_path.exists() {
                std::fs::remove_file(&dest_path)?;
            }
            
            match std::fs::copy(&source_path, &dest_path) {
                Ok(bytes) => {
                    counters.add_bytes(bytes);
                    counters.add_file();
                }
                Err(_) => {
                    counters.add_error();
                }
            }
        }
        
        let duration = start_time.elapsed();
        
        // Cleanup
        let _ = std::fs::remove_file(&source_path);
        let _ = std::fs::remove_file(&dest_path);
        
        Ok(BenchmarkResult {
            test_name: "Sequential Copy".to_string(),
            file_size,
            thread_count: 1,
            duration_ms: duration.as_millis() as f64,
            throughput_mbps: counters.get_throughput_mbps(),
            files_per_second: counters.get_files_per_second(),
            cpu_usage_percent: 0.0, // Would need system monitoring
            memory_usage_mb: 0.0,   // Would need system monitoring
            success_count: counters.files_processed.load(Ordering::Relaxed),
            error_count: counters.errors_encountered.load(Ordering::Relaxed),
        })
    }
    
    /// Benchmark parallel file copying
    fn benchmark_parallel_copy(&mut self) -> Result<()> {
        println!("🔀 Parallel Copy Benchmark");
        println!("--------------------------");
        
        for &thread_count in &self.config.thread_counts {
            for &file_size in &self.config.file_sizes {
                let test_result = self.run_parallel_copy_test(file_size, thread_count)?;
                self.results.push(test_result);
                
                println!("  {} threads, {} MB: {:.1} MB/s", 
                        thread_count,
                        file_size / (1024 * 1024), 
                        self.results.last().unwrap().throughput_mbps);
            }
        }
        
        println!();
        Ok(())
    }
    
    /// Run parallel copy test
    fn run_parallel_copy_test(&self, file_size: u64, thread_count: usize) -> Result<BenchmarkResult> {
        let counters = Arc::new(PerformanceCounters::new());
        let test_dir = self.config.test_directory.join("parallel");
        std::fs::create_dir_all(&test_dir)?;
        
        // Create multiple test files
        let mut source_files = Vec::new();
        for i in 0..thread_count {
            let source_path = test_dir.join(format!("source_{}.bin", i));
            self.create_test_file(&source_path, file_size)?;
            source_files.push(source_path);
        }
        
        let start_time = Instant::now();
        
        // Configure thread pool
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(thread_count)
            .build()?;
        
        // Run parallel test
        pool.install(|| {
            source_files.par_iter().enumerate().for_each(|(i, source_path)| {
                let dest_path = test_dir.join(format!("dest_{}.bin", i));
                
                for _ in 0..self.config.iterations {
                    if dest_path.exists() {
                        let _ = std::fs::remove_file(&dest_path);
                    }
                    
                    match std::fs::copy(source_path, &dest_path) {
                        Ok(bytes) => {
                            counters.add_bytes(bytes);
                            counters.add_file();
                        }
                        Err(_) => {
                            counters.add_error();
                        }
                    }
                }
                
                let _ = std::fs::remove_file(&dest_path);
            });
        });
        
        let duration = start_time.elapsed();
        
        // Cleanup
        for source_path in source_files {
            let _ = std::fs::remove_file(&source_path);
        }
        
        Ok(BenchmarkResult {
            test_name: "Parallel Copy".to_string(),
            file_size,
            thread_count,
            duration_ms: duration.as_millis() as f64,
            throughput_mbps: counters.get_throughput_mbps(),
            files_per_second: counters.get_files_per_second(),
            cpu_usage_percent: 0.0,
            memory_usage_mb: 0.0,
            success_count: counters.files_processed.load(Ordering::Relaxed),
            error_count: counters.errors_encountered.load(Ordering::Relaxed),
        })
    }
    
    /// Benchmark memory-mapped IO
    fn benchmark_memory_mapped_copy(&mut self) -> Result<()> {
        println!("🗺️  Memory-Mapped IO Benchmark");
        println!("-----------------------------");
        
        #[cfg(target_os = "macos")]
        {
            let mapper = MacOSMemoryMapper::new()?;
            
            for &file_size in &self.config.file_sizes {
                if mapper.should_use_mmap(file_size) {
                    let test_result = self.run_mmap_copy_test(file_size)?;
                    self.results.push(test_result);
                    
                    println!("  {} MB: {:.1} MB/s", 
                            file_size / (1024 * 1024), 
                            self.results.last().unwrap().throughput_mbps);
                }
            }
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            println!("  Memory-mapped IO not available on this platform");
        }
        
        println!();
        Ok(())
    }
    
    /// Run memory-mapped copy test
    #[cfg(target_os = "macos")]
    fn run_mmap_copy_test(&self, file_size: u64) -> Result<BenchmarkResult> {
        let mapper = MacOSMemoryMapper::new()?;
        let counters = Arc::new(PerformanceCounters::new());
        let test_dir = self.config.test_directory.join("mmap");
        std::fs::create_dir_all(&test_dir)?;
        
        let source_path = test_dir.join("source.bin");
        let dest_path = test_dir.join("dest.bin");
        self.create_test_file(&source_path, file_size)?;
        
        let start_time = Instant::now();
        
        for _ in 0..self.config.iterations {
            if dest_path.exists() {
                std::fs::remove_file(&dest_path)?;
            }
            
            match mapper.copy_file_mmap(&source_path, &dest_path) {
                Ok(bytes) => {
                    counters.add_bytes(bytes);
                    counters.add_file();
                }
                Err(_) => {
                    counters.add_error();
                }
            }
        }
        
        let duration = start_time.elapsed();
        
        // Cleanup
        let _ = std::fs::remove_file(&source_path);
        let _ = std::fs::remove_file(&dest_path);
        
        Ok(BenchmarkResult {
            test_name: "Memory-Mapped IO".to_string(),
            file_size,
            thread_count: 1,
            duration_ms: duration.as_millis() as f64,
            throughput_mbps: counters.get_throughput_mbps(),
            files_per_second: counters.get_files_per_second(),
            cpu_usage_percent: 0.0,
            memory_usage_mb: 0.0,
            success_count: counters.files_processed.load(Ordering::Relaxed),
            error_count: counters.errors_encountered.load(Ordering::Relaxed),
        })
    }
    
    /// Benchmark APFS clonefile operations
    fn benchmark_apfs_clonefile(&mut self) -> Result<()> {
        println!("🍎 APFS Clonefile Benchmark");
        println!("---------------------------");
        
        #[cfg(target_os = "macos")]
        {
            if let Ok(apfs_manager) = MacOSApfsManager::new() {
                if apfs_manager.is_clonefile_available() {
                    for &file_size in &self.config.file_sizes {
                        let test_result = self.run_clonefile_test(file_size)?;
                        self.results.push(test_result);
                        
                        println!("  {} MB: {:.1} MB/s", 
                                file_size / (1024 * 1024), 
                                self.results.last().unwrap().throughput_mbps);
                    }
                } else {
                    println!("  clonefile not available on this system");
                }
            } else {
                println!("  APFS not detected");
            }
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            println!("  APFS clonefile not available on this platform");
        }
        
        println!();
        Ok(())
    }
    
    /// Run clonefile test
    #[cfg(target_os = "macos")]
    fn run_clonefile_test(&self, file_size: u64) -> Result<BenchmarkResult> {
        let apfs_manager = MacOSApfsManager::new()?;
        let counters = Arc::new(PerformanceCounters::new());
        let test_dir = self.config.test_directory.join("clonefile");
        std::fs::create_dir_all(&test_dir)?;
        
        let source_path = test_dir.join("source.bin");
        let dest_path = test_dir.join("dest.bin");
        self.create_test_file(&source_path, file_size)?;
        
        let start_time = Instant::now();
        
        for _ in 0..self.config.iterations {
            if dest_path.exists() {
                std::fs::remove_file(&dest_path)?;
            }
            
            match apfs_manager.clone_file(&source_path, &dest_path) {
                Ok(bytes) => {
                    counters.add_bytes(bytes);
                    counters.add_file();
                }
                Err(_) => {
                    counters.add_error();
                }
            }
        }
        
        let duration = start_time.elapsed();
        
        // Cleanup
        let _ = std::fs::remove_file(&source_path);
        let _ = std::fs::remove_file(&dest_path);
        
        Ok(BenchmarkResult {
            test_name: "APFS Clonefile".to_string(),
            file_size,
            thread_count: 1,
            duration_ms: duration.as_millis() as f64,
            throughput_mbps: counters.get_throughput_mbps(),
            files_per_second: counters.get_files_per_second(),
            cpu_usage_percent: 0.0,
            memory_usage_mb: 0.0,
            success_count: counters.files_processed.load(Ordering::Relaxed),
            error_count: counters.errors_encountered.load(Ordering::Relaxed),
        })
    }
    
    /// Benchmark different filesystem performance
    fn benchmark_filesystem_comparison(&mut self) -> Result<()> {
        println!("💽 Filesystem Comparison Benchmark");
        println!("---------------------------------");
        
        // This would test performance on different mounted filesystems
        // For now, just report current filesystem performance
        println!("  Current filesystem: {}", self.system_info.filesystem_type);
        
        Ok(())
    }
    
    /// Benchmark thread scaling performance
    fn benchmark_thread_scaling(&mut self) -> Result<()> {
        println!("⚡ Thread Scaling Benchmark");
        println!("--------------------------");
        
        let test_file_size = 10 * 1024 * 1024; // 10MB
        
        for &thread_count in &self.config.thread_counts {
            let test_result = self.run_thread_scaling_test(test_file_size, thread_count)?;
            self.results.push(test_result);
            
            println!("  {} threads: {:.1} MB/s", 
                    thread_count, 
                    self.results.last().unwrap().throughput_mbps);
        }
        
        println!();
        Ok(())
    }
    
    /// Run thread scaling test
    fn run_thread_scaling_test(&self, file_size: u64, thread_count: usize) -> Result<BenchmarkResult> {
        // This is similar to parallel copy test but focuses on scaling
        self.run_parallel_copy_test(file_size, thread_count)
    }
    
    /// Create a test file with specified size
    fn create_test_file(&self, path: &Path, size: u64) -> Result<()> {
        let mut file = File::create(path)?;
        
        // Write test data in chunks to avoid excessive memory usage
        let chunk_size = std::cmp::min(size, 1024 * 1024) as usize; // 1MB chunks
        let chunk_data = vec![0xAB; chunk_size];
        let mut remaining = size;
        
        while remaining > 0 {
            let write_size = std::cmp::min(remaining, chunk_size as u64) as usize;
            file.write_all(&chunk_data[..write_size])?;
            remaining -= write_size as u64;
        }
        
        file.flush()?;
        Ok(())
    }
    
    /// Generate benchmark summary
    fn generate_summary(&self) -> BenchmarkSummary {
        if self.results.is_empty() {
            return BenchmarkSummary {
                best_throughput_mbps: 0.0,
                worst_throughput_mbps: 0.0,
                average_throughput_mbps: 0.0,
                optimal_thread_count: 1,
                optimal_file_size: 1024 * 1024,
                recommended_strategy: "Sequential".to_string(),
            };
        }
        
        let throughputs: Vec<f64> = self.results.iter().map(|r| r.throughput_mbps).collect();
        
        let best_throughput = throughputs.iter().fold(0.0f64, |acc, &x| acc.max(x));
        let worst_throughput = throughputs.iter().fold(f64::INFINITY, |acc, &x| acc.min(x));
        let average_throughput = throughputs.iter().sum::<f64>() / throughputs.len() as f64;
        
        // Find optimal configurations
        let best_result = self.results.iter()
            .max_by(|a, b| a.throughput_mbps.partial_cmp(&b.throughput_mbps).unwrap())
            .unwrap();
        
        let recommended_strategy = if best_result.test_name.contains("Clonefile") {
            "APFS Clonefile for same-volume copies".to_string()
        } else if best_result.test_name.contains("Memory-Mapped") {
            "Memory-Mapped IO for large files".to_string()
        } else if best_result.test_name.contains("Parallel") {
            format!("Parallel copy with {} threads", best_result.thread_count)
        } else {
            "Sequential copy".to_string()
        };
        
        BenchmarkSummary {
            best_throughput_mbps: best_throughput,
            worst_throughput_mbps: worst_throughput,
            average_throughput_mbps: average_throughput,
            optimal_thread_count: best_result.thread_count,
            optimal_file_size: best_result.file_size,
            recommended_strategy,
        }
    }
    
    /// Save results to JSON file
    pub fn save_results(&self, suite: &BenchmarkSuite, output_path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(suite)?;
        std::fs::write(output_path, json)?;
        Ok(())
    }
    
    /// Print results summary
    pub fn print_results_summary(&self, suite: &BenchmarkSuite) {
        println!("📊 Benchmark Results Summary");
        println!("============================");
        println!("System: {} on {}", suite.system_info.cpu_model, suite.system_info.macos_version);
        println!("Filesystem: {}", suite.system_info.filesystem_type);
        println!();
        
        println!("Performance Results:");
        println!("  Best Throughput: {:.1} MB/s", suite.summary.best_throughput_mbps);
        println!("  Worst Throughput: {:.1} MB/s", suite.summary.worst_throughput_mbps);
        println!("  Average Throughput: {:.1} MB/s", suite.summary.average_throughput_mbps);
        println!();
        
        println!("Optimal Configuration:");
        println!("  Thread Count: {}", suite.summary.optimal_thread_count);
        println!("  File Size: {} MB", suite.summary.optimal_file_size / (1024 * 1024));
        println!("  Strategy: {}", suite.summary.recommended_strategy);
        println!();
        
        println!("Test Results by Category:");
        let mut by_category: HashMap<String, Vec<&BenchmarkResult>> = HashMap::new();
        for result in &suite.results {
            by_category.entry(result.test_name.clone()).or_insert_with(Vec::new).push(result);
        }
        
        for (category, results) in by_category {
            let avg_throughput: f64 = results.iter().map(|r| r.throughput_mbps).sum::<f64>() / results.len() as f64;
            println!("  {}: {:.1} MB/s average", category, avg_throughput);
        }
    }
}

/// Default benchmark configuration for macOS
impl Default for BenchmarkConfig {
    fn default() -> Self {
        BenchmarkConfig {
            test_name: "macOS RoboSync Benchmark".to_string(),
            file_sizes: vec![
                1024 * 1024,        // 1MB
                10 * 1024 * 1024,   // 10MB  
                100 * 1024 * 1024,  // 100MB
                500 * 1024 * 1024,  // 500MB
            ],
            thread_counts: vec![1, 2, 4, 8, 16],
            iterations: 3,
            warmup_iterations: 1,
            test_directory: PathBuf::from("/tmp/robosync_benchmark"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    
    #[test]
    fn test_benchmark_suite_creation() {
        let config = BenchmarkConfig::default();
        let suite = MacOSBenchmarkSuite::new(config);
        assert!(suite.is_ok());
        
        let suite = suite.unwrap();
        println!("System: {:?}", suite.system_info);
    }
    
    #[test]
    fn test_system_info_detection() {
        let system_info = MacOSBenchmarkSuite::detect_system_info();
        assert!(system_info.is_ok());
        
        let info = system_info.unwrap();
        println!("CPU: {}", info.cpu_model);
        println!("Cores: {}", info.cpu_cores);
        println!("Memory: {:.1} GB", info.memory_gb);
        println!("Apple Silicon: {}", info.is_apple_silicon);
        println!("macOS: {}", info.macos_version);
        println!("Filesystem: {}", info.filesystem_type);
    }
    
    #[test]
    fn test_file_creation() {
        let temp_dir = tempdir().unwrap();
        let config = BenchmarkConfig::default();
        let suite = MacOSBenchmarkSuite::new(config).unwrap();
        
        let test_file = temp_dir.path().join("test.bin");
        let result = suite.create_test_file(&test_file, 1024);
        assert!(result.is_ok());
        
        let metadata = std::fs::metadata(&test_file).unwrap();
        assert_eq!(metadata.len(), 1024);
    }
}