use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use robosync::file_list::generate_file_list_with_options;
use robosync::options::SyncOptions;
use robosync::parallel_sync::{ParallelSyncer, ParallelSyncConfig};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Create a test directory with the specified number of files of given size
fn create_test_files(dir: &Path, num_files: usize, file_size: usize) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dir)?;
    
    let content = vec![b'A'; file_size];
    
    for i in 0..num_files {
        let file_path = dir.join(format!("file_{}.txt", i));
        fs::write(&file_path, &content)?;
    }
    
    Ok(())
}

/// Benchmark file list generation with different file counts
fn bench_file_list_generation(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();
    
    let mut group = c.benchmark_group("file_list_generation");
    
    for &num_files in &[10, 100, 1000] {
        let test_dir = base_path.join(format!("test_{}", num_files));
        create_test_files(&test_dir, num_files, 1024).unwrap(); // 1KB files
        
        group.throughput(Throughput::Elements(num_files as u64));
        group.bench_with_input(
            BenchmarkId::new("files", num_files),
            &num_files,
            |b, _| {
                let options = SyncOptions::default();
                b.iter(|| {
                    black_box(generate_file_list_with_options(&test_dir, &options).unwrap())
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark checksum calculation with different file sizes
fn bench_checksum_calculation(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();
    
    let mut group = c.benchmark_group("checksum_calculation");
    
    for &file_size in &[1024, 10_240, 102_400, 1_024_000] { // 1KB, 10KB, 100KB, 1MB
        let test_dir = base_path.join(format!("checksum_{}", file_size));
        create_test_files(&test_dir, 10, file_size).unwrap(); // 10 files of each size
        
        group.throughput(Throughput::Bytes((file_size * 10) as u64));
        group.bench_with_input(
            BenchmarkId::new("file_size", file_size),
            &file_size,
            |b, _| {
                let mut options = SyncOptions::default();
                options.checksum = true; // Enable checksum calculation
                
                b.iter(|| {
                    black_box(generate_file_list_with_options(&test_dir, &options).unwrap())
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark parallel vs sequential processing
fn bench_parallel_processing(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let source_dir = temp_dir.path().join("source");
    let dest_dir = temp_dir.path().join("dest");
    
    // Create test files
    create_test_files(&source_dir, 100, 10240).unwrap(); // 100 files of 10KB each
    
    let mut group = c.benchmark_group("parallel_processing");
    
    // Test different thread counts
    for &thread_count in &[1, 2, 4, 8] {
        group.bench_with_input(
            BenchmarkId::new("threads", thread_count),
            &thread_count,
            |b, &threads| {
                b.iter(|| {
                    // Clean destination directory
                    if dest_dir.exists() {
                        fs::remove_dir_all(&dest_dir).unwrap();
                    }
                    
                    let config = ParallelSyncConfig {
                        worker_threads: threads,
                        io_threads: threads,
                        block_size: 1024,
                        max_parallel_files: threads * 2,
                    };
                    
                    let syncer = ParallelSyncer::new(config);
                    let options = SyncOptions::default();
                    
                    black_box(syncer.synchronize_with_options(
                        source_dir.clone(),
                        dest_dir.clone(),
                        options
                    ).unwrap());
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark memory usage optimization - small vs large files
fn bench_memory_optimization(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();
    
    let mut group = c.benchmark_group("memory_optimization");
    
    // Test small files (should use batching)
    let small_files_dir = base_path.join("small_files");
    create_test_files(&small_files_dir, 1000, 512).unwrap(); // 1000 files of 512B each
    
    group.bench_function("small_files_batch", |b| {
        let mut options = SyncOptions::default();
        options.checksum = true;
        
        b.iter(|| {
            black_box(generate_file_list_with_options(&small_files_dir, &options).unwrap())
        });
    });
    
    // Test large files (should use streaming)
    let large_files_dir = base_path.join("large_files");
    create_test_files(&large_files_dir, 10, 1_048_576).unwrap(); // 10 files of 1MB each
    
    group.throughput(Throughput::Bytes(10 * 1_048_576));
    group.bench_function("large_files_streaming", |b| {
        let mut options = SyncOptions::default();
        options.checksum = true;
        
        b.iter(|| {
            black_box(generate_file_list_with_options(&large_files_dir, &options).unwrap())
        });
    });
    
    group.finish();
}

/// Benchmark directory scanning performance
fn bench_directory_scanning(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();
    
    let mut group = c.benchmark_group("directory_scanning");
    
    // Create nested directory structure
    for depth in &[1, 3, 5] {
        let test_dir = base_path.join(format!("depth_{}", depth));
        let mut current_dir = test_dir.clone();
        
        // Create nested directories
        for d in 0..*depth {
            current_dir = current_dir.join(format!("level_{}", d));
            fs::create_dir_all(&current_dir).unwrap();
            
            // Add some files at each level
            for i in 0..10 {
                let file_path = current_dir.join(format!("file_{}_{}.txt", d, i));
                fs::write(&file_path, b"test content").unwrap();
            }
        }
        
        group.bench_with_input(
            BenchmarkId::new("depth", depth),
            depth,
            |b, _| {
                let options = SyncOptions::default();
                b.iter(|| {
                    black_box(generate_file_list_with_options(&test_dir, &options).unwrap())
                });
            },
        );
    }
    
    group.finish();
}

criterion_group!(
    benches,
    bench_file_list_generation,
    bench_checksum_calculation,
    bench_parallel_processing,
    bench_memory_optimization,
    bench_directory_scanning
);
criterion_main!(benches);