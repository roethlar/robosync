#!/bin/bash
# benchmark_suite.sh - Comprehensive benchmark suite for RoboSync

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
ROBOSYNC="${ROBOSYNC:-./target/release/robosync}"
BENCHMARK_DIR="${BENCHMARK_DIR:-/tmp/robosync_benchmark}"
RESULTS_DIR="${RESULTS_DIR:-./benchmark_results}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Platform detection
PLATFORM="unknown"
NATIVE_TOOL=""
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    PLATFORM="linux"
    NATIVE_TOOL="rsync"
elif [[ "$OSTYPE" == "darwin"* ]]; then
    PLATFORM="macos"
    NATIVE_TOOL="rsync"
elif [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    PLATFORM="windows"
    NATIVE_TOOL="robocopy"
fi

# Results files
RESULTS_CSV="$RESULTS_DIR/benchmark_${PLATFORM}_${TIMESTAMP}.csv"
RESULTS_JSON="$RESULTS_DIR/benchmark_${PLATFORM}_${TIMESTAMP}.json"
RESULTS_SUMMARY="$RESULTS_DIR/benchmark_${PLATFORM}_${TIMESTAMP}_summary.txt"

# Ensure directories exist
mkdir -p "$RESULTS_DIR"
mkdir -p "$BENCHMARK_DIR"

# Initialize results files
echo "test_name,file_count,total_size_mb,tool,time_seconds,throughput_mbps,files_per_second" > "$RESULTS_CSV"
echo "{\"platform\": \"$PLATFORM\", \"timestamp\": \"$TIMESTAMP\", \"benchmarks\": [" > "$RESULTS_JSON"

# Function to format bytes to MB
bytes_to_mb() {
    echo "scale=2; $1 / 1048576" | bc
}

# Function to calculate throughput
calculate_throughput() {
    local size_mb=$1
    local time_seconds=$2
    echo "scale=2; $size_mb / $time_seconds" | bc
}

# Function to calculate files per second
calculate_fps() {
    local file_count=$1
    local time_seconds=$2
    echo "scale=2; $file_count / $time_seconds" | bc
}

# Function to run a benchmark
run_benchmark() {
    local test_name=$1
    local src_dir=$2
    local dst_dir=$3
    local tool=$4
    local tool_cmd=$5
    
    echo -e "\n${BLUE}Running $test_name with $tool...${NC}"
    
    # Clean destination
    rm -rf "$dst_dir"
    mkdir -p "$dst_dir"
    
    # Get file count and size
    local file_count=$(find "$src_dir" -type f | wc -l)
    local total_bytes=$(du -sb "$src_dir" | cut -f1)
    local total_mb=$(bytes_to_mb $total_bytes)
    
    # Clear caches if possible (Linux)
    if [[ "$PLATFORM" == "linux" ]] && command -v sync >/dev/null 2>&1; then
        sync && echo 3 | sudo tee /proc/sys/vm/drop_caches >/dev/null 2>&1 || true
    fi
    
    # Run benchmark
    local start_time=$(date +%s.%N)
    eval "$tool_cmd" >/dev/null 2>&1
    local end_time=$(date +%s.%N)
    
    # Calculate metrics
    local time_seconds=$(echo "$end_time - $start_time" | bc)
    local throughput=$(calculate_throughput $total_mb $time_seconds)
    local fps=$(calculate_fps $file_count $time_seconds)
    
    # Save results
    echo "$test_name,$file_count,$total_mb,$tool,$time_seconds,$throughput,$fps" >> "$RESULTS_CSV"
    
    # JSON entry
    if [ "$tool" != "$NATIVE_TOOL" ]; then
        echo "," >> "$RESULTS_JSON"
    fi
    echo -n "  {\"test\": \"$test_name\", \"tool\": \"$tool\", \"files\": $file_count, \"size_mb\": $total_mb, \"time_s\": $time_seconds, \"throughput_mbps\": $throughput, \"files_per_second\": $fps}" >> "$RESULTS_JSON"
    
    echo -e "${GREEN}✓${NC} $tool: ${time_seconds}s (${throughput} MB/s, ${fps} files/s)"
    
    # Return time for comparison
    echo "$time_seconds"
}

# Function to create test data
create_test_data() {
    local test_type=$1
    local test_dir="$BENCHMARK_DIR/$test_type"
    
    echo -e "\n${YELLOW}Creating $test_type test data...${NC}"
    rm -rf "$test_dir"
    mkdir -p "$test_dir"
    
    case "$test_type" in
        "small_files")
            # 5000 files, 1-10KB each
            for i in {1..5000}; do
                size=$((RANDOM % 10240 + 1024))
                dd if=/dev/urandom of="$test_dir/file_$i.dat" bs=1 count=$size 2>/dev/null
            done
            ;;
        
        "medium_files")
            # 500 files, 100KB-1MB each
            for i in {1..500}; do
                size=$((RANDOM % 924288 + 102400))
                dd if=/dev/urandom of="$test_dir/file_$i.dat" bs=1024 count=$((size/1024)) 2>/dev/null
            done
            ;;
        
        "large_files")
            # 50 files, 10-50MB each
            for i in {1..50}; do
                size=$((RANDOM % 41943040 + 10485760))
                dd if=/dev/urandom of="$test_dir/file_$i.dat" bs=1M count=$((size/1048576)) 2>/dev/null
            done
            ;;
        
        "mixed_workload")
            # Mix of all sizes
            mkdir -p "$test_dir/small" "$test_dir/medium" "$test_dir/large"
            # 1000 small files
            for i in {1..1000}; do
                size=$((RANDOM % 10240 + 1024))
                dd if=/dev/urandom of="$test_dir/small/file_$i.dat" bs=1 count=$size 2>/dev/null
            done
            # 100 medium files
            for i in {1..100}; do
                size=$((RANDOM % 924288 + 102400))
                dd if=/dev/urandom of="$test_dir/medium/file_$i.dat" bs=1024 count=$((size/1024)) 2>/dev/null
            done
            # 10 large files
            for i in {1..10}; do
                size=$((RANDOM % 41943040 + 10485760))
                dd if=/dev/urandom of="$test_dir/large/file_$i.dat" bs=1M count=$((size/1048576)) 2>/dev/null
            done
            ;;
        
        "deep_hierarchy")
            # Deep directory structure
            local path="$test_dir"
            for i in {1..10}; do
                path="$path/level_$i"
                mkdir -p "$path"
                for j in {1..10}; do
                    dd if=/dev/urandom of="$path/file_$j.dat" bs=1K count=10 2>/dev/null
                done
            done
            ;;
        
        "sparse_files")
            # Sparse files (if supported)
            for i in {1..10}; do
                if command -v truncate >/dev/null 2>&1; then
                    truncate -s 100M "$test_dir/sparse_$i.dat"
                    echo "data" | dd of="$test_dir/sparse_$i.dat" bs=1 seek=50000000 conv=notrunc 2>/dev/null
                else
                    # Fallback to regular files
                    dd if=/dev/zero of="$test_dir/sparse_$i.dat" bs=1M count=1 2>/dev/null
                fi
            done
            ;;
    esac
    
    echo -e "${GREEN}✓${NC} Created $test_type test data"
}

# Function to run comparison benchmark
run_comparison() {
    local test_name=$1
    local test_type=$2
    
    echo -e "\n${BLUE}=== $test_name ===${NC}"
    
    # Create test data
    create_test_data "$test_type"
    
    local src_dir="$BENCHMARK_DIR/$test_type"
    local dst_robosync="$BENCHMARK_DIR/${test_type}_robosync_dst"
    local dst_native="$BENCHMARK_DIR/${test_type}_native_dst"
    
    # Run RoboSync
    local robosync_cmd="$ROBOSYNC \"$src_dir\" \"$dst_robosync\" -s"
    local robosync_time=$(run_benchmark "$test_name" "$src_dir" "$dst_robosync" "robosync" "$robosync_cmd")
    
    # Run native tool
    local native_time=""
    if [[ "$PLATFORM" == "windows" ]]; then
        local native_cmd="robocopy \"$src_dir\" \"$dst_native\" /E /MT:16 /NFL /NDL /NJH /NJS"
        native_time=$(run_benchmark "$test_name" "$src_dir" "$dst_native" "robocopy" "$native_cmd")
    else
        local native_cmd="rsync -a \"$src_dir/\" \"$dst_native/\""
        native_time=$(run_benchmark "$test_name" "$src_dir" "$dst_native" "rsync" "$native_cmd")
    fi
    
    # Calculate speedup
    local speedup=$(echo "scale=2; $native_time / $robosync_time" | bc)
    echo -e "\n${GREEN}Speedup: ${speedup}x${NC} (RoboSync vs $NATIVE_TOOL)"
    
    # Add to summary
    echo "$test_name: RoboSync ${speedup}x faster than $NATIVE_TOOL" >> "$RESULTS_SUMMARY"
}

# Main benchmark execution
main() {
    echo -e "${BLUE}RoboSync Comprehensive Benchmark Suite${NC}"
    echo "======================================"
    echo "Platform: $PLATFORM"
    echo "Native tool: $NATIVE_TOOL"
    echo "Results directory: $RESULTS_DIR"
    echo ""
    
    # Check if RoboSync exists
    if [ ! -x "$ROBOSYNC" ]; then
        echo -e "${RED}Error: RoboSync binary not found at $ROBOSYNC${NC}"
        echo "Please build with: cargo build --release"
        exit 1
    fi
    
    # Check if native tool exists
    if ! command -v "$NATIVE_TOOL" >/dev/null 2>&1; then
        echo -e "${RED}Error: Native tool $NATIVE_TOOL not found${NC}"
        exit 1
    fi
    
    # Summary header
    echo "RoboSync Benchmark Results - $PLATFORM - $TIMESTAMP" > "$RESULTS_SUMMARY"
    echo "=================================================" >> "$RESULTS_SUMMARY"
    echo "" >> "$RESULTS_SUMMARY"
    
    # Run all benchmarks
    run_comparison "Small Files (5000 x 1-10KB)" "small_files"
    run_comparison "Medium Files (500 x 100KB-1MB)" "medium_files"
    run_comparison "Large Files (50 x 10-50MB)" "large_files"
    run_comparison "Mixed Workload" "mixed_workload"
    run_comparison "Deep Directory Hierarchy" "deep_hierarchy"
    
    # Platform-specific tests
    if [[ "$PLATFORM" != "windows" ]]; then
        run_comparison "Sparse Files" "sparse_files"
    fi
    
    # Close JSON file
    echo -e "\n]}" >> "$RESULTS_JSON"
    
    # Display summary
    echo -e "\n${BLUE}=== BENCHMARK SUMMARY ===${NC}"
    cat "$RESULTS_SUMMARY"
    
    echo -e "\n${GREEN}Benchmark complete!${NC}"
    echo "Results saved to:"
    echo "  - CSV: $RESULTS_CSV"
    echo "  - JSON: $RESULTS_JSON"
    echo "  - Summary: $RESULTS_SUMMARY"
    
    # Cleanup
    rm -rf "$BENCHMARK_DIR"
}

# Parse command line arguments
case "${1:-}" in
    --help)
        echo "Usage: $0 [--quick|--full]"
        echo ""
        echo "Run comprehensive benchmarks comparing RoboSync to native tools."
        echo ""
        echo "Options:"
        echo "  --quick    Run quick benchmark with smaller datasets"
        echo "  --full     Run full benchmark (default)"
        echo "  --help     Show this help message"
        echo ""
        echo "Environment variables:"
        echo "  ROBOSYNC        Path to robosync binary"
        echo "  BENCHMARK_DIR   Directory for test data"
        echo "  RESULTS_DIR     Directory for results"
        ;;
    --quick)
        # Modify test sizes for quick run
        echo "Running quick benchmark..."
        # TODO: Implement quick mode with smaller datasets
        main
        ;;
    *)
        main
        ;;
esac