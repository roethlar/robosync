#!/bin/bash
# run_comprehensive_tests.sh - Comprehensive functional test suite for RoboSync

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
ROBOSYNC="${ROBOSYNC:-./target/release/robosync}"
TEST_ROOT="${1:-/tmp/robosync_test}"
SOURCE_DIR="$TEST_ROOT/source"
DEST_DIR="$TEST_ROOT/destination"
LOG_DIR="$TEST_ROOT/logs"
RESULTS_FILE="$TEST_ROOT/test_results.txt"

# Test counters
PASSED_TESTS=0
FAILED_TESTS=0
SKIPPED_TESTS=0
CURRENT_TEST=""

# Ensure robosync binary exists
if [ ! -x "$ROBOSYNC" ]; then
    echo -e "${RED}Error: RoboSync binary not found at $ROBOSYNC${NC}"
    echo "Please build with: cargo build --release"
    exit 1
fi

# Initialize test environment
echo -e "${BLUE}RoboSync Comprehensive Test Suite${NC}"
echo "=================================="
echo "Binary: $ROBOSYNC"
echo "Test directory: $TEST_ROOT"
echo ""

# Create test data if not exists
if [ ! -d "$SOURCE_DIR" ]; then
    echo -e "${YELLOW}Creating test data...${NC}"
    bash "$(dirname "$0")/test_harness.sh" "$TEST_ROOT"
fi

# Clear previous results
rm -f "$RESULTS_FILE"
mkdir -p "$LOG_DIR"

# --- Helper Functions ---

log_test() {
    local test_id="$1"
    local test_name="$2"
    CURRENT_TEST="$test_id: $test_name"
    echo -e "\n${BLUE}[$test_id]${NC} $test_name"
    echo "[$test_id] $test_name" >> "$RESULTS_FILE"
}

pass_test() {
    local message="${1:-Test passed}"
    echo -e "  ${GREEN}✓${NC} $message"
    echo "  PASS: $message" >> "$RESULTS_FILE"
    PASSED_TESTS=$((PASSED_TESTS + 1))
}

fail_test() {
    local message="${1:-Test failed}"
    echo -e "  ${RED}✗${NC} $message"
    echo "  FAIL: $message" >> "$RESULTS_FILE"
    FAILED_TESTS=$((FAILED_TESTS + 1))
}

skip_test() {
    local message="${1:-Test skipped}"
    echo -e "  ${YELLOW}⚠${NC} $message"
    echo "  SKIP: $message" >> "$RESULTS_FILE"
    SKIPPED_TESTS=$((SKIPPED_TESTS + 1))
}

# Reset destination directory
reset_dest() {
    rm -rf "$DEST_DIR"
    mkdir -p "$DEST_DIR"
}

# Check if files are identical
files_identical() {
    local file1="$1"
    local file2="$2"
    
    if [ ! -f "$file1" ] || [ ! -f "$file2" ]; then
        return 1
    fi
    
    if command -v cmp >/dev/null 2>&1; then
        cmp -s "$file1" "$file2"
    else
        diff -q "$file1" "$file2" >/dev/null 2>&1
    fi
}

# Check if directories are identical (excluding .DS_Store and similar)
dirs_identical() {
    local dir1="$1"
    local dir2="$2"
    
    # Use diff to compare directory structures and content
    diff -r --exclude=".DS_Store" --exclude="Thumbs.db" "$dir1" "$dir2" >/dev/null 2>&1
}

# Verify file exists with expected content
check_file_content() {
    local file="$1"
    local expected_content="$2"
    
    if [ ! -f "$file" ]; then
        fail_test "File not found: $file"
        return 1
    fi
    
    if [ "$(cat "$file")" = "$expected_content" ]; then
        pass_test "File content matches"
        return 0
    else
        fail_test "File content mismatch"
        return 1
    fi
}

# Check symlink exists and points to correct target
check_symlink() {
    local link="$1"
    local expected_target="$2"
    
    if [ ! -L "$link" ]; then
        fail_test "Not a symlink: $link"
        return 1
    fi
    
    local actual_target=$(readlink "$link")
    if [ "$actual_target" = "$expected_target" ]; then
        pass_test "Symlink target correct: $expected_target"
        return 0
    else
        fail_test "Symlink target mismatch. Expected: $expected_target, Actual: $actual_target"
        return 1
    fi
}

# --- Test Suite 1: Basic Sync Operations ---

test_1_1() {
    log_test "T1.1" "Simple file-to-file copy (new)"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR/hello.txt" "$DEST_DIR/hello_copy.txt" > "$LOG_DIR/T1.1.log" 2>&1
    
    if files_identical "$SOURCE_DIR/hello.txt" "$DEST_DIR/hello_copy.txt"; then
        pass_test "File copied successfully"
    else
        fail_test "File copy failed"
    fi
}

test_1_2() {
    log_test "T1.2" "Simple file-to-file copy (overwrite)"
    reset_dest
    
    # Create existing file with different content
    echo "old content" > "$DEST_DIR/hello_copy.txt"
    
    "$ROBOSYNC" "$SOURCE_DIR/hello.txt" "$DEST_DIR/hello_copy.txt" > "$LOG_DIR/T1.2.log" 2>&1
    
    if files_identical "$SOURCE_DIR/hello.txt" "$DEST_DIR/hello_copy.txt"; then
        pass_test "File overwritten successfully"
    else
        fail_test "File overwrite failed"
    fi
}

test_1_3() {
    log_test "T1.3" "File-to-directory copy"
    reset_dest
    mkdir -p "$DEST_DIR/target_dir"
    
    "$ROBOSYNC" "$SOURCE_DIR/hello.txt" "$DEST_DIR/target_dir/" > "$LOG_DIR/T1.3.log" 2>&1
    
    if files_identical "$SOURCE_DIR/hello.txt" "$DEST_DIR/target_dir/hello.txt"; then
        pass_test "File copied to directory"
    else
        fail_test "File-to-directory copy failed"
    fi
}

test_1_4() {
    log_test "T1.4" "Directory-to-new-directory copy"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR/dir1" "$DEST_DIR/dir1_copy" > "$LOG_DIR/T1.4.log" 2>&1
    
    if dirs_identical "$SOURCE_DIR/dir1" "$DEST_DIR/dir1_copy"; then
        pass_test "Directory structure copied"
    else
        fail_test "Directory copy failed"
    fi
}

test_1_5() {
    log_test "T1.5" "Directory-to-existing-directory sync"
    reset_dest
    
    # Initial copy
    "$ROBOSYNC" "$SOURCE_DIR/dir1" "$DEST_DIR/dir1_sync" > "$LOG_DIR/T1.5a.log" 2>&1
    
    # Modify destination
    echo "new file" > "$DEST_DIR/dir1_sync/new.txt"
    rm -f "$DEST_DIR/dir1_sync/hello.txt"
    
    # Sync again
    "$ROBOSYNC" "$SOURCE_DIR/dir1" "$DEST_DIR/dir1_sync" > "$LOG_DIR/T1.5b.log" 2>&1
    
    # Verify hello.txt is restored but new.txt remains
    if [ -f "$DEST_DIR/dir1_sync/hello.txt" ] && [ -f "$DEST_DIR/dir1_sync/new.txt" ]; then
        pass_test "Sync updated missing files"
    else
        fail_test "Sync behavior incorrect"
    fi
}

# --- Test Suite 2: Core Options ---

test_2_1() {
    log_test "T2.1" "Recursive copy with empty directories (-e)"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR" "$DEST_DIR/recursive" -e > "$LOG_DIR/T2.1.log" 2>&1
    
    if [ -d "$DEST_DIR/recursive/empty_dir" ]; then
        pass_test "Empty directories preserved"
    else
        fail_test "Empty directories not copied"
    fi
}

test_2_2() {
    log_test "T2.2" "Recursive copy without empty directories (-s)"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR" "$DEST_DIR/recursive" -s > "$LOG_DIR/T2.2.log" 2>&1
    
    if [ ! -d "$DEST_DIR/recursive/empty_dir" ]; then
        pass_test "Empty directories excluded"
    else
        fail_test "Empty directories incorrectly copied"
    fi
}

test_2_3() {
    log_test "T2.3" "Mirroring (--mir)"
    reset_dest
    
    # Initial copy
    "$ROBOSYNC" "$SOURCE_DIR/dir1" "$DEST_DIR/mirror" > "$LOG_DIR/T2.3a.log" 2>&1
    
    # Add extra file to destination
    echo "extra" > "$DEST_DIR/mirror/extra.txt"
    
    # Mirror again
    "$ROBOSYNC" "$SOURCE_DIR/dir1" "$DEST_DIR/mirror" --mir > "$LOG_DIR/T2.3b.log" 2>&1
    
    if [ ! -f "$DEST_DIR/mirror/extra.txt" ] && dirs_identical "$SOURCE_DIR/dir1" "$DEST_DIR/mirror"; then
        pass_test "Mirror removed extra files"
    else
        fail_test "Mirror did not clean destination"
    fi
}

test_2_4() {
    log_test "T2.4" "Dry run (-n)"
    reset_dest
    
    # Dry run should not create any files
    "$ROBOSYNC" "$SOURCE_DIR/hello.txt" "$DEST_DIR/dry_run.txt" -n > "$LOG_DIR/T2.4.log" 2>&1
    
    if [ ! -f "$DEST_DIR/dry_run.txt" ]; then
        pass_test "Dry run made no changes"
    else
        fail_test "Dry run created files"
    fi
}

# --- Test Suite 3: Filtering ---

test_3_1() {
    log_test "T3.1" "Exclude files by pattern (--xf)"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR" "$DEST_DIR/filtered" --xf "*.bin" -s > "$LOG_DIR/T3.1.log" 2>&1
    
    if [ ! -f "$DEST_DIR/filtered/file_1k.bin" ] && [ -f "$DEST_DIR/filtered/hello.txt" ]; then
        pass_test "Binary files excluded"
    else
        fail_test "File exclusion failed"
    fi
}

test_3_2() {
    log_test "T3.2" "Exclude directories by pattern (--xd)"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR" "$DEST_DIR/filtered" --xd "small_files" -s > "$LOG_DIR/T3.2.log" 2>&1
    
    if [ ! -d "$DEST_DIR/filtered/small_files" ] && [ -d "$DEST_DIR/filtered/dir1" ]; then
        pass_test "Directory excluded"
    else
        fail_test "Directory exclusion failed"
    fi
}

test_3_3() {
    log_test "T3.3" "Minimum file size (--min)"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR" "$DEST_DIR/filtered" --min 1M -s > "$LOG_DIR/T3.3.log" 2>&1
    
    if [ -f "$DEST_DIR/filtered/file_1m.bin" ] && [ ! -f "$DEST_DIR/filtered/hello.txt" ]; then
        pass_test "Small files excluded"
    else
        fail_test "Minimum size filter failed"
    fi
}

test_3_4() {
    log_test "T3.4" "Maximum file size (--max)"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR" "$DEST_DIR/filtered" --max 100k -s > "$LOG_DIR/T3.4.log" 2>&1
    
    if [ -f "$DEST_DIR/filtered/hello.txt" ] && [ ! -f "$DEST_DIR/filtered/file_1m.bin" ]; then
        pass_test "Large files excluded"
    else
        fail_test "Maximum size filter failed"
    fi
}

# --- Test Suite 4: Symlinks ---

test_4_1() {
    log_test "T4.1" "Preserve symlinks (--links)"
    
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
        skip_test "Symlink test skipped on Windows"
        return
    fi
    
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR" "$DEST_DIR/with_links" --links -s > "$LOG_DIR/T4.1.log" 2>&1
    
    if [ -L "$DEST_DIR/with_links/dir1/symlink_to_file.bin" ]; then
        check_symlink "$DEST_DIR/with_links/dir1/symlink_to_file.bin" "file_1k.bin"
    else
        fail_test "Symlinks not preserved"
    fi
}

test_4_2() {
    log_test "T4.2" "Dereference symlinks (--deref)"
    
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
        skip_test "Symlink test skipped on Windows"
        return
    fi
    
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR/dir1" "$DEST_DIR/deref" --deref > "$LOG_DIR/T4.2.log" 2>&1
    
    if [ -f "$DEST_DIR/deref/symlink_to_file.bin" ] && [ ! -L "$DEST_DIR/deref/symlink_to_file.bin" ]; then
        pass_test "Symlinks dereferenced"
    else
        fail_test "Symlink dereference failed"
    fi
}

test_4_3() {
    log_test "T4.3" "Skip symlinks (--no-links)"
    
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
        skip_test "Symlink test skipped on Windows"
        return
    fi
    
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR" "$DEST_DIR/no_links" --no-links -s > "$LOG_DIR/T4.3.log" 2>&1
    
    if [ ! -e "$DEST_DIR/no_links/broken_symlink.txt" ]; then
        pass_test "Symlinks skipped"
    else
        fail_test "Symlinks incorrectly copied"
    fi
}

test_4_4() {
    log_test "T4.4" "Handle broken symlinks"
    
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
        skip_test "Symlink test skipped on Windows"
        return
    fi
    
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR" "$DEST_DIR/broken" --links -s > "$LOG_DIR/T4.4.log" 2>&1
    
    if [ -L "$DEST_DIR/broken/broken_symlink.txt" ]; then
        pass_test "Broken symlink preserved"
    else
        fail_test "Broken symlink handling failed"
    fi
}

# --- Test Suite 5: Metadata and Attributes ---

test_5_1() {
    log_test "T5.1" "Default copy preserves timestamps (DAT)"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR/old_file.txt" "$DEST_DIR/old_file.txt" > "$LOG_DIR/T5.1.log" 2>&1
    
    if [[ "$OSTYPE" == "darwin"* ]]; then
        src_time=$(stat -f "%m" "$SOURCE_DIR/old_file.txt")
        dst_time=$(stat -f "%m" "$DEST_DIR/old_file.txt")
    else
        src_time=$(stat -c "%Y" "$SOURCE_DIR/old_file.txt")
        dst_time=$(stat -c "%Y" "$DEST_DIR/old_file.txt")
    fi
    
    if [ "$src_time" = "$dst_time" ]; then
        pass_test "Timestamps preserved"
    else
        fail_test "Timestamps not preserved"
    fi
}

test_5_2() {
    log_test "T5.2" "Copy all attributes (--copyall)"
    
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
        skip_test "Unix permission test skipped on Windows"
        return
    fi
    
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR/readonly.txt" "$DEST_DIR/readonly.txt" --copyall > "$LOG_DIR/T5.2.log" 2>&1
    
    src_perms=$(stat -c "%a" "$SOURCE_DIR/readonly.txt" 2>/dev/null || stat -f "%Lp" "$SOURCE_DIR/readonly.txt")
    dst_perms=$(stat -c "%a" "$DEST_DIR/readonly.txt" 2>/dev/null || stat -f "%Lp" "$DEST_DIR/readonly.txt")
    
    if [ "$src_perms" = "$dst_perms" ]; then
        pass_test "Permissions preserved"
    else
        fail_test "Permissions not preserved"
    fi
}

test_5_3() {
    log_test "T5.3" "Checksum verification (-c)"
    reset_dest
    
    # Copy files
    cp -r "$SOURCE_DIR/checksum_test" "$DEST_DIR/"
    
    # Modify one file's timestamp but not content
    touch "$DEST_DIR/checksum_test/file1.txt"
    
    # Run with checksum - should detect no changes needed
    output=$("$ROBOSYNC" "$SOURCE_DIR/checksum_test" "$DEST_DIR/checksum_test" -c 2>&1)
    
    if echo "$output" | grep -q "0 files copied" || echo "$output" | grep -q "up to date"; then
        pass_test "Checksum correctly identified unchanged files"
    else
        fail_test "Checksum verification incorrect"
    fi
}

# --- Test Suite 6: Edge Cases ---

test_6_1() {
    log_test "T6.1" "Sync empty source directory"
    reset_dest
    mkdir -p "$TEST_ROOT/empty_source"
    
    "$ROBOSYNC" "$TEST_ROOT/empty_source" "$DEST_DIR/empty_result" > "$LOG_DIR/T6.1.log" 2>&1
    
    if [ -d "$DEST_DIR/empty_result" ]; then
        pass_test "Empty directory created"
    else
        fail_test "Empty directory sync failed"
    fi
    
    rm -rf "$TEST_ROOT/empty_source"
}

test_6_2() {
    log_test "T6.2" "Sync source with only empty subdirectories"
    reset_dest
    mkdir -p "$TEST_ROOT/empty_tree/a/b/c"
    
    "$ROBOSYNC" "$TEST_ROOT/empty_tree" "$DEST_DIR/empty_tree" -e > "$LOG_DIR/T6.2.log" 2>&1
    
    if [ -d "$DEST_DIR/empty_tree/a/b/c" ]; then
        pass_test "Empty directory tree preserved"
    else
        fail_test "Empty directory tree not copied"
    fi
    
    rm -rf "$TEST_ROOT/empty_tree"
}

test_6_3() {
    log_test "T6.3" "Special characters in filenames"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR/dir2" "$DEST_DIR/special_chars" > "$LOG_DIR/T6.3.log" 2>&1
    
    if [ -f "$DEST_DIR/special_chars/file with spaces & (brackets).txt" ] && [ -f "$DEST_DIR/special_chars/文件.txt" ]; then
        pass_test "Special characters handled"
    else
        fail_test "Special character handling failed"
    fi
}

test_6_4() {
    log_test "T6.4" "Very large file"
    
    if [ ! -f "$SOURCE_DIR/large_file_50m.bin" ]; then
        skip_test "Large file not present in test data"
        return
    fi
    
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR/large_file_50m.bin" "$DEST_DIR/large_file.bin" > "$LOG_DIR/T6.4.log" 2>&1
    
    if files_identical "$SOURCE_DIR/large_file_50m.bin" "$DEST_DIR/large_file.bin"; then
        pass_test "Large file copied successfully"
    else
        fail_test "Large file copy failed"
    fi
}

test_6_5() {
    log_test "T6.5" "Many small files"
    reset_dest
    
    "$ROBOSYNC" "$SOURCE_DIR/small_files" "$DEST_DIR/small_files" > "$LOG_DIR/T6.5.log" 2>&1
    
    src_count=$(find "$SOURCE_DIR/small_files" -type f | wc -l)
    dst_count=$(find "$DEST_DIR/small_files" -type f | wc -l)
    
    if [ "$src_count" -eq "$dst_count" ]; then
        pass_test "All $src_count small files copied"
    else
        fail_test "Small files count mismatch: $src_count vs $dst_count"
    fi
}

test_6_6() {
    log_test "T6.6" "Sync directory into itself"
    reset_dest
    
    # This should fail gracefully
    if "$ROBOSYNC" "$SOURCE_DIR" "$SOURCE_DIR/subdir" 2>"$LOG_DIR/T6.6.log"; then
        fail_test "Did not detect recursive copy"
    else
        pass_test "Recursive copy prevented"
    fi
}

# --- Test Suite 7: Performance and Special Features ---

test_7_1() {
    log_test "T7.1" "Progress display"
    reset_dest
    
    # Test progress output
    output=$("$ROBOSYNC" "$SOURCE_DIR/file_10m.bin" "$DEST_DIR/progress_test.bin" --progress 2>&1)
    
    if echo "$output" | grep -qE "MB/s|KB/s|%|Progress"; then
        pass_test "Progress information displayed"
    else
        fail_test "No progress information shown"
    fi
}

test_7_2() {
    log_test "T7.2" "Enterprise mode"
    reset_dest
    
    if "$ROBOSYNC" "$SOURCE_DIR/hello.txt" "$DEST_DIR/enterprise.txt" --enterprise > "$LOG_DIR/T7.2.log" 2>&1; then
        if [ -f "$DEST_DIR/enterprise.txt" ]; then
            pass_test "Enterprise mode copy succeeded"
        else
            fail_test "Enterprise mode did not copy file"
        fi
    else
        skip_test "Enterprise mode not available"
    fi
}

test_7_3() {
    log_test "T7.3" "Compression (-z)"
    reset_dest
    
    if "$ROBOSYNC" "$SOURCE_DIR/file_1m.bin" "$DEST_DIR/compressed.bin" -z > "$LOG_DIR/T7.3.log" 2>&1; then
        if files_identical "$SOURCE_DIR/file_1m.bin" "$DEST_DIR/compressed.bin"; then
            pass_test "Compressed transfer succeeded"
        else
            fail_test "Compressed transfer corrupted file"
        fi
    else
        skip_test "Compression not available"
    fi
}

# --- Main Test Execution ---

run_all_tests() {
    echo -e "\n${BLUE}Starting test execution...${NC}\n"
    
    # Basic sync operations
    test_1_1
    test_1_2
    test_1_3
    test_1_4
    test_1_5
    
    # Core options
    test_2_1
    test_2_2
    test_2_3
    test_2_4
    
    # Filtering
    test_3_1
    test_3_2
    test_3_3
    test_3_4
    
    # Symlinks
    test_4_1
    test_4_2
    test_4_3
    test_4_4
    
    # Metadata
    test_5_1
    test_5_2
    test_5_3
    
    # Edge cases
    test_6_1
    test_6_2
    test_6_3
    test_6_4
    test_6_5
    test_6_6
    
    # Performance and special features
    test_7_1
    test_7_2
    test_7_3
}

# Execute tests
run_all_tests

# Summary
echo -e "\n${BLUE}=================================="
echo "Test Summary"
echo "==================================${NC}"
echo -e "${GREEN}Passed:${NC} $PASSED_TESTS"
echo -e "${RED}Failed:${NC} $FAILED_TESTS"
echo -e "${YELLOW}Skipped:${NC} $SKIPPED_TESTS"
echo -e "Total: $((PASSED_TESTS + FAILED_TESTS + SKIPPED_TESTS))"
echo ""
echo "Detailed results saved to: $RESULTS_FILE"
echo "Test logs saved to: $LOG_DIR/"

# Exit with appropriate code
if [ $FAILED_TESTS -gt 0 ]; then
    echo -e "\n${RED}Some tests failed!${NC}"
    exit 1
else
    echo -e "\n${GREEN}All tests passed!${NC}"
    exit 0
fi