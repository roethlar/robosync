use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use robosync::file_list::generate_file_list_with_options;
use robosync::options::SyncOptions;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Create a test directory with the specified number of files of given size
fn create_test_files(dir: &Path, num_files: usize, file_size: usize) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dir)?;
    
    let content = vec![b'A'; file_size];
    
    for i in 0..num_files {
        let file_path = dir.join(format!("file_{i}.txt"));
        fs::write(&file_path, &content)?;
    }
    
    Ok(())
}

/// Benchmark file list generation with different file counts (without checksum)
fn bench_file_scanning(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();
    
    let mut group = c.benchmark_group("file_scanning");
    
    for &num_files in &[100, 500, 1000] {
        let test_dir = base_path.join(format!("scan_{num_files}"));
        create_test_files(&test_dir, num_files, 1024).unwrap(); // 1KB files
        
        group.throughput(Throughput::Elements(num_files as u64));
        group.bench_with_input(
            BenchmarkId::new("files", num_files),
            &num_files,
            |b, _| {
                let options = SyncOptions::default(); // No checksum
                b.iter(|| {
                    black_box(generate_file_list_with_options(&test_dir, &options).unwrap())
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark checksum calculation performance
fn bench_checksum_performance(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();
    
    let mut group = c.benchmark_group("checksum_performance");
    
    for &file_size in &[10_240, 102_400, 1_024_000] { // 10KB, 100KB, 1MB
        let test_dir = base_path.join(format!("checksum_{file_size}"));
        create_test_files(&test_dir, 50, file_size).unwrap(); // 50 files of each size
        
        group.throughput(Throughput::Bytes((file_size * 50) as u64));
        group.bench_with_input(
            BenchmarkId::new("size", file_size),
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

/// Benchmark memory efficiency - small files vs large files
fn bench_memory_efficiency(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();
    
    let mut group = c.benchmark_group("memory_efficiency");
    
    // Test small files (should use batching)
    let small_files_dir = base_path.join("small_files");
    create_test_files(&small_files_dir, 500, 512).unwrap(); // 500 files of 512B each
    
    group.throughput(Throughput::Bytes(500 * 512));
    group.bench_function("small_files_500x512B", |b| {
        let mut options = SyncOptions::default();
        options.checksum = true;
        
        b.iter(|| {
            black_box(generate_file_list_with_options(&small_files_dir, &options).unwrap())
        });
    });
    
    // Test large files (should use streaming)
    let large_files_dir = base_path.join("large_files");
    create_test_files(&large_files_dir, 5, 2_097_152).unwrap(); // 5 files of 2MB each
    
    group.throughput(Throughput::Bytes(5 * 2_097_152));
    group.bench_function("large_files_5x2MB", |b| {
        let mut options = SyncOptions::default();
        options.checksum = true;
        
        b.iter(|| {
            black_box(generate_file_list_with_options(&large_files_dir, &options).unwrap())
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_file_scanning,
    bench_checksum_performance,
    bench_memory_efficiency
);
criterion_main!(benches);