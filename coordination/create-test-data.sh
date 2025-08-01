#!/bin/bash
# Cross-platform test data creation script
# Creates identical test data for performance comparison

echo "Creating standardized test data..."

# Clean up any existing test data
rm -rf perf_test test_src test_dst* perf_dst* rsync_dst robosync_dst cp_dst

# Create directory structure
mkdir -p perf_test/{small,medium,large}
mkdir -p test_src/{small,medium,large}

# Function to create small files
create_small_files() {
    echo "Creating 10,000 small files (1KB each)..."
    for i in $(seq 1 10000); do
        echo "This is test file number $i with some padding content to reach approximately 1KB in size. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum. Adding more text to ensure we reach close to 1KB. The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs. How vexingly quick daft zebras jump! Bright vixens jump; dozy fowl quack." > perf_test/small/file_$i.txt
        
        # Progress indicator
        if [ $((i % 1000)) -eq 0 ]; then
            echo "  Created $i/10000 small files..."
        fi
    done
}

# Function to create medium files
create_medium_files() {
    echo "Creating 100 medium files (10MB each)..."
    for i in $(seq 1 100); do
        # Use dd with urandom for consistent cross-platform behavior
        dd if=/dev/urandom of=perf_test/medium/file_$i.bin bs=1024 count=10240 2>/dev/null
        echo "  Created medium file $i/100"
    done
}

# Function to create large files
create_large_files() {
    echo "Creating 5 large files (200MB each)..."
    for i in $(seq 1 5); do
        dd if=/dev/urandom of=perf_test/large/file_$i.bin bs=1024 count=204800 2>/dev/null
        echo "  Created large file $i/5"
    done
}

# Create test files for basic operations
echo "Creating basic test structure..."
echo "test content" > test_src/small/1kb.txt
dd if=/dev/urandom of=test_src/medium/5mb.bin bs=1024 count=5120 2>/dev/null
dd if=/dev/urandom of=test_src/large/150mb.bin bs=1024 count=153600 2>/dev/null

# Create performance test data
create_small_files
create_medium_files
create_large_files

# Calculate total size
echo ""
echo "Test data creation complete!"
echo "Basic test data size: $(du -sh test_src 2>/dev/null | cut -f1)"
echo "Performance test data size: $(du -sh perf_test 2>/dev/null | cut -f1)"
echo ""
echo "File counts:"
echo "  Small files: $(find perf_test/small -type f | wc -l)"
echo "  Medium files: $(find perf_test/medium -type f | wc -l)"
echo "  Large files: $(find perf_test/large -type f | wc -l)"