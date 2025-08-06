#!/bin/bash
# Post-Fix Comprehensive Validation Suite for RoboSync 2.0.0
# Run this IMMEDIATELY after fixing macOS performance regressions
# Usage: ./post_fix_validation_suite.sh [macos|linux|windows]

set -e

PLATFORM=${1:-$(uname -s | tr '[:upper:]' '[:lower:]')}
TEST_DIR="/tmp/robosync_post_fix_$(date +%s)"
ROBOSYNC_BIN="${ROBOSYNC_BIN:-./target/release/robosync}"
RESULTS_FILE="post_fix_results_${PLATFORM}_$(date +%Y%m%d_%H%M%S).csv"

echo "=== RoboSync 2.0.0 Post-Fix Validation Suite ==="
echo "Platform: $PLATFORM"
echo "Test Directory: $TEST_DIR"
echo "RoboSync Binary: $ROBOSYNC_BIN"
echo "Results File: $RESULTS_FILE"
echo

# Performance targets based on previous baseline issues
declare -A PERFORMANCE_TARGETS
PERFORMANCE_TARGETS["small_files_min_speedup"]=1.0    # At least equal to rsync
PERFORMANCE_TARGETS["medium_files_min_speedup"]=1.0   # CRITICAL: Must not be slower than rsync
PERFORMANCE_TARGETS["large_files_min_speedup"]=2.0    # Should be 2x+ faster than rsync
PERFORMANCE_TARGETS["mixed_workload_min_speedup"]=1.0  # CRITICAL: Must not be slower than rsync

# Check if RoboSync binary exists
if [[ ! -f "$ROBOSYNC_BIN" ]]; then
    echo "❌ RoboSync binary not found at: $ROBOSYNC_BIN"
    echo "Build with: cargo build --release"
    exit 1
fi

# Create test environment
mkdir -p "$TEST_DIR"/{source,dest,temp}
cd "$TEST_DIR"

echo "📁 Creating comprehensive test data..."

# Test 1: Small files (exactly what failed before)
echo "  Creating small files test (5000 files)..."
mkdir -p source/small_files
for i in {1..5000}; do
    echo "small file content $i" > "source/small_files/small_$i.txt"
done

# Test 2: Medium files (THE CRITICAL REGRESSION - 1-16MB range)
echo "  Creating medium files test (500 files, 1-16MB range)..."
mkdir -p source/medium_files
for i in {1..500}; do
    # Create files in the 1-16MB range that were 6x slower
    size_kb=$((1024 + (i * 30)))  # 1MB to 15MB range
    dd if=/dev/zero of="source/medium_files/medium_$i.bin" bs=1K count=$size_kb 2>/dev/null
done

# Test 3: Large files (should be fast)
echo "  Creating large files test (50 files, 10-50MB)..."
mkdir -p source/large_files
for i in {1..50}; do
    size_mb=$((10 + (i % 40)))  # 10-50MB range
    dd if=/dev/zero of="source/large_files/large_$i.bin" bs=1M count=$size_mb 2>/dev/null
done

# Test 4: Mixed workload (THE OTHER CRITICAL REGRESSION)
echo "  Creating mixed workload test..."
mkdir -p source/mixed_workload/{small,medium,large}
# Small files
for i in {1..1000}; do
    echo "mixed small $i" > "source/mixed_workload/small/file_$i.txt"
done
# Medium files
for i in {1..100}; do
    dd if=/dev/zero of="source/mixed_workload/medium/file_$i.bin" bs=1M count=5 2>/dev/null
done
# Large files
for i in {1..20}; do
    dd if=/dev/zero of="source/mixed_workload/large/file_$i.bin" bs=1M count=25 2>/dev/null
done

# Test 5: Ultra-fast copy scenarios
echo "  Creating ultra-fast copy test..."
mkdir -p source/ultra_fast_test
for i in {1..100}; do
    echo "ultra fast content $i" > "source/ultra_fast_test/file_$i.txt"
done

# Platform-specific test data
case "$PLATFORM" in
    "macos")
        echo "🍎 Creating macOS-specific tests..."
        # APFS reflink test
        echo "reflink test content" > source/apfs_reflink_test.bin
        dd if=/dev/zero of=source/apfs_reflink_test.bin bs=1M count=10 2>/dev/null
        
        # Extended attributes test
        echo "xattr test" > source/xattr_test.txt
        xattr -w user.test "test value" source/xattr_test.txt 2>/dev/null || true
        ;;
    "linux")
        echo "🐧 Creating Linux-specific tests..."
        # Sparse file test
        dd if=/dev/zero of=source/sparse_test.bin bs=1M count=0 seek=100 2>/dev/null
        ;;
    "windows")
        echo "🪟 Creating Windows-specific tests..."
        # ADS test (if available)
        echo "main content" > source/ads_test.txt
        ;;
esac

# Initialize results CSV
echo "test_name,robosync_time,rsync_time,robocopy_time,speedup,status,files_count,total_size_mb,notes" > "$RESULTS_FILE"

run_performance_test() {
    local test_name="$1"
    local source_path="$2" 
    local dest_robosync="dest_robosync_$test_name"
    local dest_rsync="dest_rsync_$test_name"
    local min_speedup="${PERFORMANCE_TARGETS[${test_name}_min_speedup]:-1.0}"
    
    echo
    echo "🚀 Performance Test: $test_name"
    echo "   Source: $source_path"
    echo "   Minimum Required Speedup: ${min_speedup}x"
    
    # Clean destinations
    rm -rf "$dest_robosync" "$dest_rsync"
    
    # Count files and calculate size
    local file_count=$(find "$source_path" -type f | wc -l)
    local total_size_bytes=$(find "$source_path" -type f -exec stat -c%s {} + 2>/dev/null | awk '{sum+=$1} END {print sum}' || echo "0")
    local total_size_mb=$((total_size_bytes / 1024 / 1024))
    
    echo "   Files: $file_count, Size: ${total_size_mb}MB"
    
    # Test RoboSync
    echo "   Testing RoboSync..."
    local robosync_start=$(date +%s.%N)
    if "$ROBOSYNC_BIN" "$source_path" "$dest_robosync" -v >/dev/null 2>&1; then
        local robosync_end=$(date +%s.%N)
        local robosync_time=$(echo "$robosync_end - $robosync_start" | bc -l 2>/dev/null || echo "0")
        echo "   ✅ RoboSync: ${robosync_time}s"
    else
        echo "   ❌ RoboSync FAILED"
        echo "$test_name,FAILED,N/A,N/A,0,FAIL,$file_count,$total_size_mb,RoboSync execution failed" >> "$RESULTS_FILE"
        return 1
    fi
    
    # Test rsync (if available)
    local rsync_time="N/A"
    local speedup="N/A"
    local status="UNKNOWN"
    
    if command -v rsync >/dev/null 2>&1; then
        echo "   Testing rsync..."
        local rsync_start=$(date +%s.%N)
        if rsync -av "$source_path/" "$dest_rsync/" >/dev/null 2>&1; then
            local rsync_end=$(date +%s.%N)
            rsync_time=$(echo "$rsync_end - $rsync_start" | bc -l 2>/dev/null || echo "0")
            echo "   ✅ rsync: ${rsync_time}s"
            
            # Calculate speedup
            if command -v bc >/dev/null 2>&1 && [[ "$robosync_time" != "0" ]]; then
                speedup=$(echo "scale=2; $rsync_time / $robosync_time" | bc -l 2>/dev/null || echo "0")
                echo "   📈 Speedup: ${speedup}x"
                
                # Check if meets minimum requirement
                if (( $(echo "$speedup >= $min_speedup" | bc -l) )); then
                    status="PASS"
                    echo "   ✅ PERFORMANCE TARGET MET (${speedup}x >= ${min_speedup}x)"
                else
                    status="FAIL"
                    echo "   ❌ PERFORMANCE TARGET MISSED (${speedup}x < ${min_speedup}x)"
                fi
            fi
        else
            echo "   ❌ rsync failed"
            rsync_time="FAILED"
        fi
    else
        echo "   ⚠️  rsync not available"
        rsync_time="N/A"
    fi
    
    # Verify file integrity
    local robosync_count=$(find "$dest_robosync" -type f | wc -l)
    if [[ "$robosync_count" -eq "$file_count" ]]; then
        echo "   ✅ File count verification passed"
    else
        echo "   ❌ File count mismatch: expected $file_count, got $robosync_count"
        status="FAIL"
    fi
    
    # Save results
    echo "$test_name,$robosync_time,$rsync_time,N/A,$speedup,$status,$file_count,$total_size_mb," >> "$RESULTS_FILE"
    
    # Return success/failure for test suite
    [[ "$status" == "PASS" || "$status" == "UNKNOWN" ]]
}

# Run critical performance tests
echo
echo "🧪 Running CRITICAL performance regression tests..."

TESTS_PASSED=0
TESTS_FAILED=0

# Test the exact scenarios that were failing
echo "=== CRITICAL REGRESSION TESTS ==="

# Medium files test - WAS 6x SLOWER
if run_performance_test "medium_files" "source/medium_files"; then
    ((TESTS_PASSED++))
    echo "✅ Medium files regression test PASSED"
else
    ((TESTS_FAILED++))
    echo "❌ Medium files regression test FAILED - BLOCKING ISSUE"
fi

# Mixed workload test - WAS 7.5x SLOWER  
if run_performance_test "mixed_workload" "source/mixed_workload"; then
    ((TESTS_PASSED++))
    echo "✅ Mixed workload regression test PASSED"
else
    ((TESTS_FAILED++))
    echo "❌ Mixed workload regression test FAILED - BLOCKING ISSUE"
fi

echo
echo "=== BASELINE PERFORMANCE TESTS ==="

# Small files - should be equal or better
if run_performance_test "small_files" "source/small_files"; then
    ((TESTS_PASSED++))
    echo "✅ Small files test PASSED"
else
    ((TESTS_FAILED++))
    echo "❌ Small files test FAILED"
fi

# Large files - should be significantly faster
if run_performance_test "large_files" "source/large_files"; then
    ((TESTS_PASSED++))
    echo "✅ Large files test PASSED"
else
    ((TESTS_FAILED++))
    echo "❌ Large files test FAILED"
fi

# Ultra-fast copy mode test
echo
echo "🚀 Testing ultra-fast copy mode..."
rm -rf dest_ultra_fast
start_time=$(date +%s.%N)
if "$ROBOSYNC_BIN" source/ultra_fast_test dest_ultra_fast -v | grep -q "Ultra-fast copy mode detected"; then
    end_time=$(date +%s.%N)
    duration=$(echo "$end_time - $start_time" | bc -l 2>/dev/null || echo "N/A")
    echo "✅ Ultra-fast copy mode triggered in ${duration}s"
    ((TESTS_PASSED++))
else
    echo "❌ Ultra-fast copy mode NOT triggered"
    ((TESTS_FAILED++))
fi

# Platform-specific functionality tests
case "$PLATFORM" in
    "macos")
        echo
        echo "🍎 macOS-specific functionality tests..."
        
        # APFS reflink test with stats verification
        echo "Testing APFS reflink and stats reporting..."
        rm -rf dest_reflink_test
        if output=$("$ROBOSYNC_BIN" source/apfs_reflink_test.bin dest_reflink_test.bin -v 2>&1); then
            echo "$output"
            # Check if reflink stats are reported correctly
            if echo "$output" | grep -E "Reflinks succeeded: [1-9]|clonefile.*success" >/dev/null; then
                echo "✅ Reflink stats reporting FIXED"
                ((TESTS_PASSED++))
            else
                echo "❌ Reflink stats still reporting incorrectly"
                ((TESTS_FAILED++))
            fi
        else
            echo "❌ APFS reflink test failed"
            ((TESTS_FAILED++))
        fi
        ;;
esac

# Final results summary
echo
echo "📊 FINAL TEST RESULTS SUMMARY"
echo "============================="
echo "Tests Passed: $TESTS_PASSED"
echo "Tests Failed: $TESTS_FAILED"
echo "Total Tests: $((TESTS_PASSED + TESTS_FAILED))"
echo "Results saved to: $RESULTS_FILE"
echo

# Critical assessment
if [[ $TESTS_FAILED -eq 0 ]]; then
    echo "🎉 ALL TESTS PASSED - PERFORMANCE REGRESSIONS RESOLVED!"
    echo "✅ RoboSync 2.0.0 ready for release validation"
    echo
    echo "Performance Summary:"
    cat "$RESULTS_FILE" | column -t -s','
    
    exit_code=0
else
    echo "❌ $TESTS_FAILED TESTS FAILED - RELEASE STILL BLOCKED"
    echo
    echo "Failed Tests Details:"
    grep "FAIL" "$RESULTS_FILE" | column -t -s','
    echo
    echo "🚨 DO NOT PROCEED WITH RELEASE UNTIL ALL TESTS PASS"
    
    exit_code=1
fi

echo
echo "📁 Test data preserved at: $TEST_DIR"
echo "🧹 Clean up with: rm -rf $TEST_DIR"

exit $exit_code