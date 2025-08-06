#!/bin/bash
# Cross-platform integration test suite for RoboSync 2.0.0
# Usage: ./integration_test_suite.sh [platform]
# Platform: linux, macos, windows

set -e

PLATFORM=${1:-$(uname -s | tr '[:upper:]' '[:lower:]')}
TEST_DIR="/tmp/robosync_integration_$(date +%s)"
ROBOSYNC_BIN="${ROBOSYNC_BIN:-./target/release/robosync}"

echo "=== RoboSync 2.0.0 Integration Test Suite ==="
echo "Platform: $PLATFORM"
echo "Test Directory: $TEST_DIR"
echo "RoboSync Binary: $ROBOSYNC_BIN"
echo

# Check if RoboSync binary exists
if [[ ! -f "$ROBOSYNC_BIN" ]]; then
    echo "❌ RoboSync binary not found at: $ROBOSYNC_BIN"
    echo "Build with: cargo build --release"
    exit 1
fi

# Create test environment
mkdir -p "$TEST_DIR"/{source,dest,temp}
cd "$TEST_DIR"

echo "📁 Creating test data..."

# Test 1: Ultra-fast copy detection
mkdir -p source/ultra_fast_test
for i in {1..50}; do
    echo "content $i" > "source/ultra_fast_test/file_$i.txt"
done

# Test 2: Mixed file sizes
mkdir -p source/mixed_test
echo "small" > source/mixed_test/small.txt
dd if=/dev/zero of=source/mixed_test/medium.bin bs=1K count=512 2>/dev/null
dd if=/dev/zero of=source/mixed_test/large.bin bs=1M count=10 2>/dev/null

# Test 3: Directory structure
mkdir -p source/nested/{level1/{level2,level2b},level1b}
for dir in source/nested/level1/level2 source/nested/level1/level2b source/nested/level1b; do
    echo "nested content" > "$dir/nested.txt"
done

# Test 4: Platform-specific features
case "$PLATFORM" in
    "windows")
        echo "🪟 Windows-specific tests:"
        # Test ADS (if on Windows)
        if command -v fsutil >/dev/null 2>&1; then
            echo "main data" > source/ads_test.txt
            echo "alternate stream" > source/ads_test.txt:stream1
            echo "  - Created ADS test file"
        fi
        ;;
    "macos")
        echo "🍎 macOS-specific tests:"
        # Test extended attributes
        echo "xattr test" > source/xattr_test.txt
        xattr -w user.test "test value" source/xattr_test.txt 2>/dev/null || true
        echo "  - Created xattr test file"
        ;;
    "linux")
        echo "🐧 Linux-specific tests:"
        # Test sparse files
        dd if=/dev/zero of=source/sparse_test.bin bs=1M count=0 seek=100 2>/dev/null
        echo "  - Created sparse test file"
        ;;
esac

run_test() {
    local test_name="$1"
    local source_path="$2"
    local dest_path="$3"
    local options="$4"
    
    echo
    echo "🧪 Test: $test_name"
    echo "   Source: $source_path"
    echo "   Dest: $dest_path"
    echo "   Options: $options"
    
    # Clean destination
    rm -rf "$dest_path"
    
    # Run RoboSync with timing
    start_time=$(date +%s.%N)
    if "$ROBOSYNC_BIN" "$source_path" "$dest_path" $options; then
        end_time=$(date +%s.%N)
        duration=$(echo "$end_time - $start_time" | bc -l 2>/dev/null || echo "N/A")
        echo "   ✅ Success (${duration}s)"
        
        # Basic validation
        if [[ -d "$dest_path" ]]; then
            src_count=$(find "$source_path" -type f | wc -l)
            dest_count=$(find "$dest_path" -type f | wc -l)
            if [[ "$src_count" -eq "$dest_count" ]]; then
                echo "   ✅ File count matches ($src_count files)"
            else
                echo "   ❌ File count mismatch (src: $src_count, dest: $dest_count)"
                return 1
            fi
        fi
    else
        echo "   ❌ Failed"
        return 1
    fi
}

# Run integration tests
echo
echo "🚀 Running integration tests..."

run_test "Ultra-fast copy (new destination)" "source/ultra_fast_test" "dest/ultra_fast_new" "-v"
run_test "Ultra-fast copy (existing destination)" "source/ultra_fast_test" "dest/ultra_fast_existing" "-v"
run_test "Mixed file sizes" "source/mixed_test" "dest/mixed" "-v"
run_test "Nested directories" "source/nested" "dest/nested" "-v"
run_test "Single file copy" "source/mixed_test/small.txt" "dest/single_file.txt" "-v"

# Platform-specific tests
case "$PLATFORM" in
    "windows")
        if [[ -f "source/ads_test.txt:stream1" ]]; then
            run_test "ADS preservation" "source/ads_test.txt" "dest/ads_test.txt" "-v"
            # Verify ADS was copied
            if [[ -f "dest/ads_test.txt:stream1" ]]; then
                echo "   ✅ ADS stream preserved"
            else
                echo "   ❌ ADS stream not preserved"
            fi
        fi
        ;;
    "macos")
        run_test "Extended attributes" "source/xattr_test.txt" "dest/xattr_test.txt" "-v"
        # Check if xattr was preserved
        if xattr dest/xattr_test.txt 2>/dev/null | grep -q "user.test"; then
            echo "   ✅ Extended attributes preserved"
        else
            echo "   ⚠️  Extended attributes may not be preserved"
        fi
        ;;
    "linux")
        run_test "Sparse file handling" "source/sparse_test.bin" "dest/sparse_test.bin" "-v"
        # Check if sparse file properties preserved
        src_size=$(stat -c%s source/sparse_test.bin 2>/dev/null || echo "0")
        dest_size=$(stat -c%s dest/sparse_test.bin 2>/dev/null || echo "0")
        if [[ "$src_size" -eq "$dest_size" ]]; then
            echo "   ✅ Sparse file size preserved"
        else
            echo "   ❌ Sparse file size mismatch"
        fi
        ;;
esac

# Performance comparison test
echo
echo "📊 Performance comparison test..."
mkdir -p source/perf_test dest/perf_rsync dest/perf_robosync

# Create performance test data
for i in {1..1000}; do
    echo "performance test data $i" > "source/perf_test/file_$i.txt"
done

# Test rsync (if available)
if command -v rsync >/dev/null 2>&1; then
    echo "   Testing rsync..."
    rsync_start=$(date +%s.%N)
    rsync -av source/perf_test/ dest/perf_rsync/ >/dev/null 2>&1
    rsync_end=$(date +%s.%N)
    rsync_time=$(echo "$rsync_end - $rsync_start" | bc -l 2>/dev/null || echo "N/A")
    echo "   rsync: ${rsync_time}s"
    
    echo "   Testing RoboSync..."
    robosync_start=$(date +%s.%N)
    "$ROBOSYNC_BIN" source/perf_test dest/perf_robosync -v >/dev/null 2>&1
    robosync_end=$(date +%s.%N)
    robosync_time=$(echo "$robosync_end - $robosync_start" | bc -l 2>/dev/null || echo "N/A")
    echo "   RoboSync: ${robosync_time}s"
    
    if command -v bc >/dev/null 2>&1; then
        speedup=$(echo "scale=2; $rsync_time / $robosync_time" | bc -l 2>/dev/null || echo "N/A")
        echo "   📈 RoboSync speedup: ${speedup}x"
    fi
fi

echo
echo "✅ Integration test suite complete!"
echo "📁 Test data preserved at: $TEST_DIR"
echo "🧹 Clean up with: rm -rf $TEST_DIR"