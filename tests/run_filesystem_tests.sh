#!/bin/bash
# run_filesystem_tests.sh - Run RoboSync tests on different filesystems

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROBOSYNC="${ROBOSYNC:-$SCRIPT_DIR/../target/release/robosync}"
BASE_TEST_DIR="${BASE_TEST_DIR:-/mnt/robosync_test}"

# Filesystem mount points
declare -A FILESYSTEMS=(
    ["ext4"]="/mnt/robosync_test/ext4"
    ["xfs"]="/mnt/robosync_test/xfs"
    ["btrfs"]="/mnt/robosync_test/btrfs"
    ["ntfs"]="/mnt/robosync_test/ntfs"
    ["nfs"]="/mnt/robosync_test/nfs"
    ["smb"]="/mnt/robosync_test/smb"
    ["zfs"]="/mnt/robosync_test/zfs"
    ["apfs"]="/mnt/robosync_test/apfs"
    ["refs"]="/mnt/robosync_test/refs"
)

# Check if filesystem is available
check_filesystem() {
    local fs_type="$1"
    local mount_point="$2"
    
    if [ ! -d "$mount_point" ]; then
        return 1
    fi
    
    # Check if mounted
    if ! mount | grep -q "$mount_point"; then
        return 1
    fi
    
    # Check if writable
    if ! touch "$mount_point/.test" 2>/dev/null; then
        return 1
    fi
    rm -f "$mount_point/.test"
    
    return 0
}

# Get filesystem type
get_fs_type() {
    local mount_point="$1"
    
    if [[ "$OSTYPE" == "darwin"* ]]; then
        mount -t nofs | grep "$mount_point" | awk '{print $3}' | cut -d. -f1
    else
        df -T "$mount_point" 2>/dev/null | tail -1 | awk '{print $2}'
    fi
}

# Test reflink support
test_reflink_support() {
    local test_dir="$1"
    local test_file="$test_dir/.reflink_test"
    local test_copy="$test_dir/.reflink_copy"
    
    # Create test file
    dd if=/dev/zero of="$test_file" bs=1M count=10 2>/dev/null
    
    # Try to create reflink
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS clonefile
        if cp -c "$test_file" "$test_copy" 2>/dev/null; then
            rm -f "$test_file" "$test_copy"
            return 0
        fi
    else
        # Linux reflink
        if cp --reflink=always "$test_file" "$test_copy" 2>/dev/null; then
            rm -f "$test_file" "$test_copy"
            return 0
        fi
    fi
    
    rm -f "$test_file" "$test_copy"
    return 1
}

# Run performance benchmark on filesystem
run_performance_test() {
    local fs_name="$1"
    local test_dir="$2"
    local results_file="$3"
    
    echo -e "\n${BLUE}Running performance test on $fs_name...${NC}"
    
    # Create test data
    local src_dir="$test_dir/perf_src"
    local dst_dir="$test_dir/perf_dst"
    mkdir -p "$src_dir" "$dst_dir"
    
    # Small files test (1000 x 1KB)
    echo "Creating small files..."
    mkdir -p "$src_dir/small"
    for i in {1..1000}; do
        dd if=/dev/urandom of="$src_dir/small/file_$i.dat" bs=1K count=1 2>/dev/null
    done
    
    # Medium files test (100 x 1MB)
    echo "Creating medium files..."
    mkdir -p "$src_dir/medium"
    for i in {1..100}; do
        dd if=/dev/urandom of="$src_dir/medium/file_$i.dat" bs=1M count=1 2>/dev/null
    done
    
    # Large file test (1 x 100MB)
    echo "Creating large file..."
    dd if=/dev/urandom of="$src_dir/large.dat" bs=1M count=100 2>/dev/null
    
    # Run benchmarks
    echo "Benchmarking small files..."
    start_time=$(date +%s.%N)
    "$ROBOSYNC" "$src_dir/small" "$dst_dir/small" >/dev/null 2>&1
    end_time=$(date +%s.%N)
    small_time=$(echo "$end_time - $start_time" | bc)
    
    rm -rf "$dst_dir/small"
    
    echo "Benchmarking medium files..."
    start_time=$(date +%s.%N)
    "$ROBOSYNC" "$src_dir/medium" "$dst_dir/medium" >/dev/null 2>&1
    end_time=$(date +%s.%N)
    medium_time=$(echo "$end_time - $start_time" | bc)
    
    rm -rf "$dst_dir/medium"
    
    echo "Benchmarking large file..."
    start_time=$(date +%s.%N)
    "$ROBOSYNC" "$src_dir/large.dat" "$dst_dir/large.dat" >/dev/null 2>&1
    end_time=$(date +%s.%N)
    large_time=$(echo "$end_time - $start_time" | bc)
    
    # Calculate throughput
    small_mb=$(du -sm "$src_dir/small" | cut -f1)
    medium_mb=$(du -sm "$src_dir/medium" | cut -f1)
    large_mb=100
    
    small_throughput=$(echo "scale=2; $small_mb / $small_time" | bc)
    medium_throughput=$(echo "scale=2; $medium_mb / $medium_time" | bc)
    large_throughput=$(echo "scale=2; $large_mb / $large_time" | bc)
    
    # Save results
    echo "$fs_name,small,$small_time,$small_throughput" >> "$results_file"
    echo "$fs_name,medium,$medium_time,$medium_throughput" >> "$results_file"
    echo "$fs_name,large,$large_time,$large_throughput" >> "$results_file"
    
    echo -e "${GREEN}Performance test complete${NC}"
    echo "  Small files: ${small_time}s (${small_throughput} MB/s)"
    echo "  Medium files: ${medium_time}s (${medium_throughput} MB/s)"
    echo "  Large file: ${large_time}s (${large_throughput} MB/s)"
    
    # Cleanup
    rm -rf "$src_dir" "$dst_dir"
}

# Run functional tests on filesystem
run_functional_test() {
    local fs_name="$1"
    local test_dir="$2"
    
    echo -e "\n${BLUE}Running functional tests on $fs_name...${NC}"
    
    # Run comprehensive test suite
    if "$SCRIPT_DIR/run_comprehensive_tests.sh" "$test_dir"; then
        echo -e "${GREEN}Functional tests passed on $fs_name${NC}"
        return 0
    else
        echo -e "${RED}Functional tests failed on $fs_name${NC}"
        return 1
    fi
}

# Main execution
main() {
    echo -e "${BLUE}RoboSync Filesystem Test Suite${NC}"
    echo "=================================="
    echo "Binary: $ROBOSYNC"
    echo ""
    
    # Check if robosync exists
    if [ ! -x "$ROBOSYNC" ]; then
        echo -e "${RED}Error: RoboSync binary not found${NC}"
        echo "Please build with: cargo build --release"
        exit 1
    fi
    
    # Results file
    local results_file="filesystem_test_results_$(date +%Y%m%d_%H%M%S).csv"
    echo "filesystem,test_type,time_seconds,throughput_mbps" > "$results_file"
    
    # Summary
    local tested_count=0
    local passed_count=0
    declare -A test_results
    
    # Test each filesystem
    for fs_name in "${!FILESYSTEMS[@]}"; do
        mount_point="${FILESYSTEMS[$fs_name]}"
        
        echo -e "\n${YELLOW}Testing $fs_name filesystem...${NC}"
        
        if ! check_filesystem "$fs_name" "$mount_point"; then
            echo -e "${YELLOW}Skipping $fs_name: Not available${NC}"
            test_results[$fs_name]="SKIPPED"
            continue
        fi
        
        # Get actual filesystem type
        actual_fs=$(get_fs_type "$mount_point")
        echo "Mount point: $mount_point"
        echo "Filesystem type: $actual_fs"
        
        # Check reflink support
        if test_reflink_support "$mount_point"; then
            echo -e "${GREEN}Reflink support: YES${NC}"
        else
            echo "Reflink support: NO"
        fi
        
        tested_count=$((tested_count + 1))
        
        # Create test directory
        test_dir="$mount_point/robosync_test_$$"
        mkdir -p "$test_dir"
        
        # Run tests
        if run_functional_test "$fs_name" "$test_dir"; then
            run_performance_test "$fs_name" "$test_dir" "$results_file"
            test_results[$fs_name]="PASSED"
            passed_count=$((passed_count + 1))
        else
            test_results[$fs_name]="FAILED"
        fi
        
        # Cleanup
        rm -rf "$test_dir"
    done
    
    # Summary report
    echo -e "\n${BLUE}=================================="
    echo "Filesystem Test Summary"
    echo "==================================${NC}"
    
    for fs_name in "${!FILESYSTEMS[@]}"; do
        result="${test_results[$fs_name]:-NOT_TESTED}"
        case "$result" in
            "PASSED")
                echo -e "$fs_name: ${GREEN}$result${NC}"
                ;;
            "FAILED")
                echo -e "$fs_name: ${RED}$result${NC}"
                ;;
            "SKIPPED")
                echo -e "$fs_name: ${YELLOW}$result${NC}"
                ;;
            *)
                echo "$fs_name: $result"
                ;;
        esac
    done
    
    echo ""
    echo "Filesystems tested: $tested_count"
    echo "Tests passed: $passed_count"
    echo ""
    echo "Performance results saved to: $results_file"
    
    if [ $passed_count -eq $tested_count ] && [ $tested_count -gt 0 ]; then
        echo -e "\n${GREEN}All filesystem tests passed!${NC}"
        exit 0
    elif [ $tested_count -eq 0 ]; then
        echo -e "\n${RED}No filesystems available for testing${NC}"
        exit 1
    else
        echo -e "\n${RED}Some filesystem tests failed${NC}"
        exit 1
    fi
}

# Handle command line arguments
case "${1:-}" in
    --setup)
        echo "Setting up test filesystems..."
        echo "This requires root privileges and appropriate disk space."
        echo ""
        echo "Example setup commands:"
        echo "  # Create test images"
        echo "  dd if=/dev/zero of=/tmp/ext4.img bs=1M count=1024"
        echo "  dd if=/dev/zero of=/tmp/xfs.img bs=1M count=1024"
        echo "  dd if=/dev/zero of=/tmp/btrfs.img bs=1M count=1024"
        echo ""
        echo "  # Format filesystems"
        echo "  mkfs.ext4 /tmp/ext4.img"
        echo "  mkfs.xfs /tmp/xfs.img"
        echo "  mkfs.btrfs /tmp/btrfs.img"
        echo ""
        echo "  # Mount filesystems"
        echo "  mkdir -p /mnt/robosync_test/{ext4,xfs,btrfs}"
        echo "  mount -o loop /tmp/ext4.img /mnt/robosync_test/ext4"
        echo "  mount -o loop /tmp/xfs.img /mnt/robosync_test/xfs"
        echo "  mount -o loop /tmp/btrfs.img /mnt/robosync_test/btrfs"
        ;;
    --help)
        echo "Usage: $0 [--setup|--help]"
        echo ""
        echo "Run RoboSync tests on multiple filesystems."
        echo ""
        echo "Options:"
        echo "  --setup    Show filesystem setup instructions"
        echo "  --help     Show this help message"
        echo ""
        echo "Environment variables:"
        echo "  ROBOSYNC         Path to robosync binary"
        echo "  BASE_TEST_DIR    Base directory for filesystem mounts"
        ;;
    *)
        main
        ;;
esac