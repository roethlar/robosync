#!/bin/bash

# RoboSync Small Files Regression Test
# This tests THE core problem RoboSync was meant to solve:
# "rsync's lack of parallel transfers for small files"

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Configuration
ROBOSYNC="${ROBOSYNC_BIN:-$(pwd)/target/release/robosync}"
TEST_DIR="/tmp/robosync_small_files_$$"

echo -e "${BLUE}=== RoboSync Small Files Regression Test ===${NC}"
echo "Testing THE core problem RoboSync was built to solve:"
echo "Parallel transfers for small files should be FASTER than rsync"
echo ""

# Check prerequisites
if [ ! -f "$ROBOSYNC" ]; then
    echo -e "${RED}Error: RoboSync binary not found at $ROBOSYNC${NC}"
    exit 1
fi

if ! command -v rsync &> /dev/null; then
    echo -e "${RED}Error: rsync not found. This test requires rsync for comparison.${NC}"
    exit 1
fi

# Setup
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Test scenarios - exactly what RoboSync should excel at
run_small_files_test() {
    local test_name="$1"
    local file_count="$2"
    local file_size="$3"
    local setup_cmd="$4"
    
    echo -e "\n${YELLOW}Test: $test_name${NC}"
    echo "Creating $file_count files of ~$file_size each..."
    
    # Clean and setup
    rm -rf source dest_robosync dest_rsync
    mkdir -p source
    eval "$setup_cmd"
    
    # Measure RoboSync
    echo -n "RoboSync: "
    local start=$(date +%s%N)
    "$ROBOSYNC" source dest_robosync 2>/dev/null
    local end=$(date +%s%N)
    local robosync_ns=$((end - start))
    local robosync_ms=$((robosync_ns / 1000000))
    echo -e "${GREEN}${robosync_ms}ms${NC}"
    
    # Measure rsync
    echo -n "rsync:    "
    local start=$(date +%s%N)
    rsync -r source/ dest_rsync/ 2>/dev/null
    local end=$(date +%s%N)
    local rsync_ns=$((end - start))
    local rsync_ms=$((rsync_ns / 1000000))
    echo -e "${GREEN}${rsync_ms}ms${NC}"
    
    # Calculate result
    if [ $robosync_ms -lt $rsync_ms ]; then
        local speedup=$(echo "scale=2; $rsync_ms / $robosync_ms" | bc)
        echo -e "Result:   ${GREEN}✓ RoboSync is ${speedup}x FASTER${NC}"
        return 0
    else
        local slowdown=$(echo "scale=2; $robosync_ms / $rsync_ms" | bc)
        echo -e "Result:   ${RED}✗ RoboSync is ${slowdown}x SLOWER${NC}"
        return 1
    fi
}

echo -e "${BLUE}Starting small files regression tests...${NC}"

PASS=0
FAIL=0

# Test 1: 1000 small files (1KB each) - Basic parallel test
if run_small_files_test \
    "1000 files × 1KB" \
    1000 \
    "1KB" \
    "for i in {1..1000}; do echo 'small file content' > source/file_\$i.txt; done"; then
    PASS=$((PASS + 1))
else
    FAIL=$((FAIL + 1))
fi

# Test 2: 5000 tiny files - Maximum parallelism benefit
if run_small_files_test \
    "5000 files × 100 bytes" \
    5000 \
    "100B" \
    "for i in {1..5000}; do echo 'tiny' > source/file_\$i.txt; done"; then
    PASS=$((PASS + 1))
else
    FAIL=$((FAIL + 1))
fi

# Test 3: 10000 minimal files - Stress test
if run_small_files_test \
    "10000 files × 10 bytes" \
    10000 \
    "10B" \
    "for i in {1..10000}; do echo 'x' > source/file_\$i.txt; done"; then
    PASS=$((PASS + 1))
else
    FAIL=$((FAIL + 1))
fi

# Test 4: Deep directory with small files
echo -e "\n${YELLOW}Test: Deep directory structure with small files${NC}"
echo "Creating nested directories with small files..."

rm -rf source dest_robosync dest_rsync
mkdir -p source
for i in {1..10}; do
    mkdir -p source/dir$i
    for j in {1..100}; do
        echo "small" > source/dir$i/file_$j.txt
    done
done

echo -n "RoboSync: "
start=$(date +%s%N)
"$ROBOSYNC" -r source dest_robosync 2>/dev/null
end=$(date +%s%N)
robosync_ns=$((end - start))
robosync_ms=$((robosync_ns / 1000000))
echo -e "${GREEN}${robosync_ms}ms${NC}"

echo -n "rsync:    "
start=$(date +%s%N)
rsync -r source/ dest_rsync/ 2>/dev/null
end=$(date +%s%N)
rsync_ns=$((end - start))
rsync_ms=$((rsync_ns / 1000000))
echo -e "${GREEN}${rsync_ms}ms${NC}"

if [ $robosync_ms -lt $rsync_ms ]; then
    speedup=$(echo "scale=2; $rsync_ms / $robosync_ms" | bc)
    echo -e "Result:   ${GREEN}✓ RoboSync is ${speedup}x FASTER${NC}"
    PASS=$((PASS + 1))
else
    slowdown=$(echo "scale=2; $robosync_ms / $rsync_ms" | bc)
    echo -e "Result:   ${RED}✗ RoboSync is ${slowdown}x SLOWER${NC}"
    FAIL=$((FAIL + 1))
fi

# Final summary
echo -e "\n${BLUE}=========================================${NC}"
echo -e "${BLUE}Small Files Regression Test Summary:${NC}"
echo -e "  Passed: ${GREEN}$PASS${NC}"
echo -e "  Failed: ${RED}$FAIL${NC}"

if [ $FAIL -eq 0 ]; then
    echo -e "\n${GREEN}✓ SUCCESS: RoboSync is FASTER than rsync for small files!${NC}"
    echo "The core mission is accomplished!"
    EXIT_CODE=0
else
    echo -e "\n${RED}✗ FAILURE: RoboSync is SLOWER than rsync for small files${NC}"
    echo -e "${RED}This is a critical regression - RoboSync has failed its primary purpose${NC}"
    echo ""
    echo "RoboSync was specifically built to solve rsync's lack of parallel"
    echo "transfers for small files. If it's slower, it has no reason to exist."
    EXIT_CODE=1
fi

# Cleanup
cd /
rm -rf "$TEST_DIR"

exit $EXIT_CODE