# Performance Benchmarks

This document describes the performance benchmarks available for RoboSync and the optimizations implemented.

## Running Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark suite
cargo bench --bench quick_performance
cargo bench --bench performance
```

## Available Benchmarks

### File List Generation (`file_scanning`)
- Tests directory scanning performance with different file counts (100, 500, 1000 files)
- Measures files scanned per second
- Tests the efficiency of WalkDir-based directory traversal

### Checksum Calculation (`checksum_performance`) 
- Tests Blake3 streaming checksum calculation with different file sizes
- File sizes: 10KB, 100KB, 1MB (50 files each)
- Measures throughput in bytes per second
- Validates the performance of parallel checksum computation

### Memory Efficiency (`memory_efficiency`)
- **Small files**: 500 files × 512B each (tests file batching optimization)
- **Large files**: 5 files × 2MB each (tests streaming I/O optimization)
- Compares memory usage patterns between small and large file processing

## Performance Optimizations Implemented

### 1. Streaming Checksum Calculation
- **Before**: Loading entire files into memory for checksum calculation
- **After**: Blake3 streaming hasher with 64KB buffers
- **Impact**: Constant memory usage regardless of file size

### 2. Parallel Checksum Computation
- **Before**: Sequential checksum calculation during directory scanning
- **After**: Parallel computation using Rayon after file collection
- **Impact**: Better CPU utilization, especially on multi-core systems

### 3. Streaming I/O for Large Files
- **Before**: Reading entire files into memory
- **After**: 256KB buffers for files >10MB, streaming copy operations
- **Impact**: Reduced memory footprint for large file operations

### 4. File Batching for Small Files
- **Before**: Individual processing of each small file
- **After**: Batching small files (<1MB) in groups of 10
- **Impact**: Reduced thread overhead and improved processing efficiency

### 5. Progress Bar Optimization
- **Implementation**: MultiProgress bars for concurrent operations
- **Phases**: Source scanning, destination scanning, comparison, and sync operations
- **Result**: Real-time progress visibility during all phases

## Expected Performance Characteristics

### File Scanning
- **Target**: ~2M files/second for directory traversal
- **Scales**: Linearly with file count
- **Bottleneck**: I/O subsystem and filesystem metadata access

### Checksum Calculation  
- **Target**: 10-50 GiB/s for Blake3 hashing (depends on CPU)
- **Scales**: With parallel workers and CPU cores
- **Bottleneck**: CPU computational capacity and memory bandwidth

### Memory Usage
- **Small files**: Constant memory usage through batching
- **Large files**: Bounded memory usage through streaming
- **Target**: <100MB memory usage regardless of dataset size

## Benchmark Results Interpretation

The benchmarks provide several key metrics:

1. **Throughput**: Files processed or bytes hashed per second
2. **Latency**: Time per operation
3. **Scalability**: How performance changes with dataset size
4. **Memory efficiency**: Constant vs. linear memory growth

Use these benchmarks to:
- Validate performance improvements after code changes
- Compare performance across different hardware configurations
- Identify performance regressions during development
- Guide further optimization efforts

## Hardware Considerations

Performance will vary based on:
- **CPU cores**: More cores improve parallel checksum calculation
- **Storage type**: SSDs significantly outperform HDDs for file scanning
- **Memory bandwidth**: Affects large file streaming performance
- **Filesystem**: ext4, NTFS, APFS have different metadata access speeds