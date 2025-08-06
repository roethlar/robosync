#!/bin/bash

# RoboSync Working Benchmark Suite
# Clean, simple, actually works

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
ROBOSYNC="${ROBOSYNC_BIN:-$(pwd)/target/release/robosync}"
TEST_DIR="/tmp/robosync_benchmark_$$"
RESULTS_FILE="benchmark_results_$(date +%Y%m%d_%H%M%S).txt"

# Check if robosync exists
if [ ! -f "$ROBOSYNC" ]; then
    echo -e "${RED}Error: RoboSync binary not found at $ROBOSYNC${NC}"
    exit 1
fi

# Check if rsync exists
if ! command -v rsync &> /dev/null; then
    echo -e "${YELLOW}Warning: rsync not found, skipping comparison${NC}"
    COMPARE_RSYNC=false
else
    COMPARE_RSYNC=true
fi

echo "=== RoboSync Performance Benchmark ==="
echo "Binary: $ROBOSYNC"
echo "Test directory: $TEST_DIR"
echo "Results file: $RESULTS_FILE"
echo ""

# Create test directory
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Function to measure time in milliseconds
get_time_ms() {
    echo $(($(date +%s%N) / 1000000))
}

# Function to run benchmark
run_benchmark() {
    local name="$1"
    local setup_cmd="$2"
    local file_count="$3"
    local total_size="$4"
    
    echo "----------------------------------------"
    echo "Test: $name"
    echo "Files: $file_count, Total size: $total_size"
    
    # Setup test data
    rm -rf source dest_robosync dest_rsync
    mkdir -p source
    eval "$setup_cmd"
    
    # Test RoboSync
    local start=$(get_time_ms)
    "$ROBOSYNC" source dest_robosync > /dev/null 2>&1
    local end=$(get_time_ms)
    local robosync_time=$((end - start))
    echo -e "RoboSync: ${GREEN}${robosync_time}ms${NC}"
    
    # Test rsync if available
    if [ "$COMPARE_RSYNC" = true ]; then
        local start=$(get_time_ms)
        rsync -r source/ dest_rsync/ > /dev/null 2>&1
        local end=$(get_time_ms)
        local rsync_time=$((end - start))
        echo -e "rsync:    ${GREEN}${rsync_time}ms${NC}"
        
        # Calculate speedup
        if [ $rsync_time -gt 0 ]; then
            local speedup=$(echo "scale=2; $rsync_time / $robosync_time" | bc)
            if (( $(echo "$speedup > 1" | bc -l) )); then
                echo -e "Speedup:  ${GREEN}${speedup}x faster${NC}"
            else
                local slowdown=$(echo "scale=2; $robosync_time / $rsync_time" | bc)
                echo -e "Speedup:  ${RED}${slowdown}x slower${NC}"
            fi
        fi
        
        # Save to results
        echo "$name,$file_count,$total_size,$robosync_time,$rsync_time" >> "$RESULTS_FILE"
    else
        echo "$name,$file_count,$total_size,$robosync_time,N/A" >> "$RESULTS_FILE"
    fi
    
    # Verify files copied correctly
    local copied=$(find dest_robosync -type f | wc -l)
    if [ "$copied" -eq "$file_count" ]; then
        echo -e "Verify:   ${GREEN}✓ All files copied${NC}"
    else
        echo -e "Verify:   ${RED}✗ Expected $file_count files, got $copied${NC}"
    fi
    echo ""
}

# Initialize results file
echo "Test,Files,Size,RoboSync_ms,rsync_ms" > "$RESULTS_FILE"

# Test 1: Small files (1000 x 1KB) - The critical test
run_benchmark \
    "Small files (1000 x 1KB)" \
    "for i in {1..1000}; do echo 'test data' > source/file_\$i.txt; done" \
    1000 \
    "10KB"

# Test 2: Many small files (5000 x 1KB)  
run_benchmark \
    "Many small files (5000 x 1KB)" \
    "for i in {1..5000}; do echo 'test' > source/file_\$i.txt; done" \
    5000 \
    "25KB"

# Test 3: Medium files (100 x 100KB)
run_benchmark \
    "Medium files (100 x 100KB)" \
    "for i in {1..100}; do dd if=/dev/zero of=source/file_\$i.dat bs=100K count=1 2>/dev/null; done" \
    100 \
    "10MB"

# Test 4: Large files (10 x 10MB)
run_benchmark \
    "Large files (10 x 10MB)" \
    "for i in {1..10}; do dd if=/dev/zero of=source/file_\$i.dat bs=10M count=1 2>/dev/null; done" \
    10 \
    "100MB"

# Test 5: Mixed workload
run_benchmark \
    "Mixed workload" \
    "for i in {1..100}; do echo 'small' > source/small_\$i.txt; done; \
     for i in {1..10}; do dd if=/dev/zero of=source/medium_\$i.dat bs=100K count=1 2>/dev/null; done; \
     dd if=/dev/zero of=source/large.dat bs=10M count=1 2>/dev/null" \
    111 \
    "11MB"

# Test 6: Deep directory structure
run_benchmark \
    "Deep directory (10 levels, 100 files)" \
    "mkdir -p source/d1/d2/d3/d4/d5/d6/d7/d8/d9/d10; \
     for i in {1..10}; do \
         for j in {1..10}; do \
             echo 'test' > source/d1/d2/d3/d4/d5/d6/d7/d8/d9/d\$i/file_\$j.txt 2>/dev/null || true; \
         done; \
         mkdir -p source/d1/d2/d3/d4/d5/d6/d7/d8/d9/d\$i 2>/dev/null || true; \
     done" \
    100 \
    "1KB"

# Test 7: Single large file (100MB)
run_benchmark \
    "Single large file (100MB)" \
    "dd if=/dev/zero of=source/large.dat bs=100M count=1 2>/dev/null" \
    1 \
    "100MB"

# Summary
echo "========================================="
echo "Benchmark Complete!"
echo "Results saved to: $(pwd)/$RESULTS_FILE"
echo ""

if [ "$COMPARE_RSYNC" = true ]; then
    # Calculate overall performance
    echo "Summary by test type:"
    while IFS=',' read -r test files size robosync rsync_time; do
        if [ "$test" != "Test" ] && [ "$rsync_time" != "N/A" ]; then
            if [ $rsync_time -gt 0 ] && [ $robosync -gt 0 ]; then
                speedup=$(echo "scale=2; $rsync_time / $robosync" | bc)
                if (( $(echo "$speedup > 1" | bc -l) )); then
                    echo -e "  $test: ${GREEN}${speedup}x faster${NC}"
                else
                    slowdown=$(echo "scale=2; $robosync / $rsync_time" | bc)
                    echo -e "  $test: ${RED}${slowdown}x slower${NC}"
                fi
            fi
        fi
    done < "$RESULTS_FILE"
fi

# Cleanup option
echo ""
echo "Keep test directory? (y/n)"
read -t 5 -n 1 keep || keep="n"
if [ "$keep" != "y" ]; then
    cd /
    rm -rf "$TEST_DIR"
    echo "Test directory cleaned up"
else
    echo "Test directory preserved at: $TEST_DIR"
fi