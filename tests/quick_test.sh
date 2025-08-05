#!/bin/bash
# quick_test.sh - Quick smoke test for RoboSync

set -e

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

ROBOSYNC="${ROBOSYNC:-./target/release/robosync}"
TEST_DIR="/tmp/robosync_quick_test"

echo "RoboSync Quick Test"
echo "==================="

# Check binary
if [ ! -x "$ROBOSYNC" ]; then
    echo -e "${RED}Error: RoboSync binary not found${NC}"
    echo "Building RoboSync..."
    cargo build --release
fi

# Clean test directory
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/src" "$TEST_DIR/dst"

# Create test file
echo "Hello, RoboSync!" > "$TEST_DIR/src/test.txt"

# Test 1: Simple file copy
echo -n "Test 1: Simple file copy... "
if "$ROBOSYNC" "$TEST_DIR/src/test.txt" "$TEST_DIR/dst/test.txt" >/dev/null 2>&1; then
    if [ -f "$TEST_DIR/dst/test.txt" ] && [ "$(cat "$TEST_DIR/dst/test.txt")" = "Hello, RoboSync!" ]; then
        echo -e "${GREEN}PASS${NC}"
    else
        echo -e "${RED}FAIL${NC} - File content mismatch"
        exit 1
    fi
else
    echo -e "${RED}FAIL${NC} - Copy command failed"
    exit 1
fi

# Test 2: Directory copy
echo -n "Test 2: Directory copy... "
mkdir -p "$TEST_DIR/src/subdir"
echo "Subdir file" > "$TEST_DIR/src/subdir/file.txt"
if "$ROBOSYNC" "$TEST_DIR/src" "$TEST_DIR/dst2" >/dev/null 2>&1; then
    if [ -f "$TEST_DIR/dst2/test.txt" ] && [ -f "$TEST_DIR/dst2/subdir/file.txt" ]; then
        echo -e "${GREEN}PASS${NC}"
    else
        echo -e "${RED}FAIL${NC} - Directory structure not copied"
        exit 1
    fi
else
    echo -e "${RED}FAIL${NC} - Directory copy failed"
    exit 1
fi

# Test 3: Progress output
echo -n "Test 3: Progress display... "
if "$ROBOSYNC" "$TEST_DIR/src" "$TEST_DIR/dst3" --progress 2>&1 | grep -qE "files|bytes|complete"; then
    echo -e "${GREEN}PASS${NC}"
else
    echo -e "${RED}FAIL${NC} - No progress output"
fi

# Test 4: Dry run
echo -n "Test 4: Dry run... "
if "$ROBOSYNC" "$TEST_DIR/src/test.txt" "$TEST_DIR/dst4/test.txt" -n >/dev/null 2>&1; then
    if [ ! -f "$TEST_DIR/dst4/test.txt" ]; then
        echo -e "${GREEN}PASS${NC}"
    else
        echo -e "${RED}FAIL${NC} - Dry run created files"
        exit 1
    fi
else
    echo -e "${RED}FAIL${NC} - Dry run command failed"
    exit 1
fi

# Cleanup
rm -rf "$TEST_DIR"

echo -e "\n${GREEN}All tests passed!${NC}"
echo "RoboSync is working correctly."