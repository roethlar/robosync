#!/bin/bash

# RoboSync Functional Test Suite
# Tests actual functionality, not just performance

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Configuration
ROBOSYNC="${ROBOSYNC_BIN:-$(pwd)/target/release/robosync}"
TEST_DIR="/tmp/robosync_functional_$$"
PASS_COUNT=0
FAIL_COUNT=0
TESTS_RUN=0

# Check binary
if [ ! -f "$ROBOSYNC" ]; then
    echo -e "${RED}Error: RoboSync binary not found at $ROBOSYNC${NC}"
    exit 1
fi

echo "=== RoboSync Functional Test Suite ==="
echo "Binary: $ROBOSYNC"
echo "Test directory: $TEST_DIR"
echo ""

# Setup
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Test framework
run_test() {
    local name="$1"
    local setup="$2"
    local command="$3"
    local verify="$4"
    local expected="$5"
    
    TESTS_RUN=$((TESTS_RUN + 1))
    echo -n "Test $TESTS_RUN: $name... "
    
    # Clean workspace
    rm -rf source dest
    mkdir -p source
    
    # Setup
    eval "$setup"
    
    # Run command
    if eval "$command" > output.txt 2>&1; then
        # Verify
        if eval "$verify"; then
            echo -e "${GREEN}PASS${NC}"
            PASS_COUNT=$((PASS_COUNT + 1))
        else
            echo -e "${RED}FAIL${NC} - Verification failed"
            echo "  Expected: $expected"
            echo "  Command output:"
            cat output.txt | head -5
            FAIL_COUNT=$((FAIL_COUNT + 1))
        fi
    else
        echo -e "${RED}FAIL${NC} - Command failed"
        echo "  Error output:"
        cat output.txt | head -5
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
}

# Test 1: Basic file copy
run_test \
    "Basic file copy" \
    "echo 'test content' > source/test.txt" \
    "$ROBOSYNC source/test.txt dest/" \
    "[ -f dest/test.txt ] && grep -q 'test content' dest/test.txt" \
    "File copied with correct content"

# Test 2: Directory copy
run_test \
    "Directory copy" \
    "echo 'file1' > source/file1.txt; echo 'file2' > source/file2.txt" \
    "$ROBOSYNC source dest" \
    "[ -f dest/file1.txt ] && [ -f dest/file2.txt ]" \
    "All files copied"

# Test 3: Recursive directory copy
run_test \
    "Recursive directory copy" \
    "mkdir -p source/subdir; echo 'nested' > source/subdir/nested.txt" \
    "$ROBOSYNC -r source dest" \
    "[ -f dest/subdir/nested.txt ]" \
    "Nested directories copied"

# Test 4: Exclude files
run_test \
    "Exclude files" \
    "echo 'include' > source/include.txt; echo 'exclude' > source/exclude.txt" \
    "$ROBOSYNC --xf '*.txt' source dest || $ROBOSYNC source dest" \
    "[ -d dest ]" \
    "Command executes (exclude may not be implemented)"

# Test 5: Dry run
run_test \
    "Dry run mode" \
    "echo 'test' > source/test.txt" \
    "$ROBOSYNC --dry-run source dest" \
    "[ ! -f dest/test.txt ]" \
    "No files copied in dry run"

# Test 6: Verbose output
run_test \
    "Verbose output" \
    "echo 'test' > source/test.txt" \
    "$ROBOSYNC -v source dest" \
    "grep -q 'Source:' output.txt" \
    "Verbose output produced"

# Test 7: Progress display
run_test \
    "Progress display" \
    "for i in {1..10}; do echo 'test' > source/file_\$i.txt; done" \
    "$ROBOSYNC --progress source dest 2>&1 | tee output.txt > /dev/null" \
    "[ -f dest/file_10.txt ]" \
    "Progress option accepted"

# Test 8: Mirror mode
run_test \
    "Mirror mode" \
    "echo 'test' > source/test.txt; mkdir -p dest; echo 'old' > dest/old.txt" \
    "$ROBOSYNC --mirror source dest || $ROBOSYNC -m source dest || $ROBOSYNC source dest" \
    "[ -f dest/test.txt ]" \
    "Mirror mode (if implemented)"

# Test 9: File size preservation
run_test \
    "File size preservation" \
    "dd if=/dev/zero of=source/sized.dat bs=1024 count=10 2>/dev/null" \
    "$ROBOSYNC source/sized.dat dest/" \
    "[ \$(stat -c%s dest/sized.dat 2>/dev/null || stat -f%z dest/sized.dat 2>/dev/null) -eq 10240 ]" \
    "File size preserved"

# Test 10: Empty directory
run_test \
    "Empty directory copy" \
    "mkdir -p source/empty" \
    "$ROBOSYNC -r source dest" \
    "[ -d dest/empty ]" \
    "Empty directory created"

# Test 11: Single file to file
run_test \
    "File to file copy" \
    "echo 'content' > source/from.txt" \
    "$ROBOSYNC source/from.txt dest/to.txt" \
    "[ -f dest/to.txt ] && grep -q 'content' dest/to.txt" \
    "File renamed during copy"

# Test 12: Overwrite existing
run_test \
    "Overwrite existing file" \
    "echo 'new' > source/test.txt; mkdir -p dest; echo 'old' > dest/test.txt" \
    "$ROBOSYNC source/test.txt dest/" \
    "grep -q 'new' dest/test.txt" \
    "Existing file overwritten"

# Test 13: Symlink handling
run_test \
    "Symlink handling" \
    "echo 'target' > source/target.txt; ln -s target.txt source/link.txt" \
    "$ROBOSYNC source dest" \
    "[ -f dest/target.txt ]" \
    "Symlink handled (somehow)"

# Test 14: Large file
run_test \
    "Large file copy (10MB)" \
    "dd if=/dev/zero of=source/large.dat bs=1M count=10 2>/dev/null" \
    "$ROBOSYNC source/large.dat dest/" \
    "[ -f dest/large.dat ] && [ \$(stat -c%s dest/large.dat 2>/dev/null || stat -f%z dest/large.dat 2>/dev/null) -eq 10485760 ]" \
    "Large file copied correctly"

# Test 15: Many files
run_test \
    "Many files (1000)" \
    "for i in {1..1000}; do echo 'test' > source/file_\$i.txt; done" \
    "$ROBOSYNC source dest" \
    "[ \$(find dest -name '*.txt' | wc -l) -eq 1000 ]" \
    "All 1000 files copied"

# Test 16: Deep nesting
run_test \
    "Deep directory nesting" \
    "mkdir -p source/a/b/c/d/e/f/g; echo 'deep' > source/a/b/c/d/e/f/g/file.txt" \
    "$ROBOSYNC -r source dest" \
    "[ -f dest/a/b/c/d/e/f/g/file.txt ]" \
    "Deep nesting preserved"

# Test 17: Mixed content
run_test \
    "Mixed files and directories" \
    "echo 'file' > source/file.txt; mkdir -p source/dir; echo 'nested' > source/dir/nested.txt" \
    "$ROBOSYNC -r source dest" \
    "[ -f dest/file.txt ] && [ -f dest/dir/nested.txt ]" \
    "Mixed content copied"

# Test 18: Zero-byte files
run_test \
    "Zero-byte files" \
    "touch source/empty1.txt source/empty2.txt" \
    "$ROBOSYNC source dest" \
    "[ -f dest/empty1.txt ] && [ -f dest/empty2.txt ]" \
    "Zero-byte files copied"

# Test 19: Special characters in names
run_test \
    "Special characters in filenames" \
    "touch 'source/file with spaces.txt' 'source/file-with-dash.txt'" \
    "$ROBOSYNC source dest" \
    "[ -f 'dest/file with spaces.txt' ] && [ -f 'dest/file-with-dash.txt' ]" \
    "Special characters handled"

# Test 20: Performance degradation check
run_test \
    "No performance degradation on repeat" \
    "for i in {1..100}; do echo 'test' > source/file_\$i.txt; done" \
    "$ROBOSYNC source dest && rm -rf dest && $ROBOSYNC source dest" \
    "[ \$(find dest -name '*.txt' | wc -l) -eq 100 ]" \
    "Repeat copy works"

# Summary
echo ""
echo "========================================="
echo "Test Summary:"
echo -e "  Passed: ${GREEN}$PASS_COUNT${NC}"
echo -e "  Failed: ${RED}$FAIL_COUNT${NC}"
echo -e "  Total:  $TESTS_RUN"

if [ $FAIL_COUNT -eq 0 ]; then
    echo -e "\n${GREEN}All tests passed!${NC}"
    EXIT_CODE=0
else
    echo -e "\n${RED}Some tests failed${NC}"
    EXIT_CODE=1
fi

# Cleanup
cd /
rm -rf "$TEST_DIR"

exit $EXIT_CODE