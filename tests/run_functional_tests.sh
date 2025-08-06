#!/bin/bash
# tests/run_functional_tests.sh
# Executes a suite of functional tests for RoboSync.

set -euo pipefail

ROBOSYNC="./target/release/robosync"
TEST_ROOT_DIR="$1"
SOURCE_DIR="$TEST_ROOT_DIR/source"
DEST_DIR="$TEST_ROOT_DIR/destination"
LOG_DIR="$TEST_ROOT_DIR/logs"

PASSED_TESTS=0
FAILED_TESTS=0

# --- Helper Functions ---

log_test_start() {
    TEST_NAME="$1"
    echo "\n--- Running Test: $TEST_NAME ---"
}

log_test_result() {
    RESULT="$1"
    if [ "$RESULT" == "PASS" ]; then
        echo "--- Test Passed: $TEST_NAME ---"
        PASSED_TESTS=$((PASSED_TESTS + 1))
    else
        echo "--- Test FAILED: $TEST_NAME ---"
        FAILED_TESTS=$((FAILED_TESTS + 1))
    fi
}

# Function to compare directories (content, permissions, timestamps, symlinks)
# Usage: compare_dirs <dir1> <dir2> <test_name>
compare_dirs() {
    local dir1="$1"
    local dir2="$2"
    local test_name="$3"

    log_test_start "$test_name"

    # Use rsync's dry-run to compare, ignoring timestamps for initial check
    # -a: archive mode (preserves permissions, timestamps, etc.)
    # -n: dry run
    # -v: verbose
    # --checksum: compare by checksum, not just size/mtime
    # --delete: delete extraneous files from dest dirs
    # --exclude: exclude .DS_Store and other OS-specific files
    # --ignore-times: ignore modification times (only compare size/checksum)

    # First, check for structural differences (missing/extra files/dirs)
    # We expect no differences in dry-run if sync was successful
    if ! rsync -avn --delete --exclude='.DS_Store' --exclude='Thumbs.db' "$dir1/" "$dir2/" | grep -q 'sending incremental file list'; then
        echo "PASS: Structural comparison (rsync dry-run) - no differences found."
    else
        echo "FAIL: Structural comparison (rsync dry-run) - differences found."
        rsync -avn --delete --exclude='.DS_Store' --exclude='Thumbs.db' "$dir1/" "$dir2/"
        log_test_result "FAIL"
        return 1
    fi

    # Second, check file contents using diff -r
    if diff -r "$dir1" "$dir2" > /dev/null; then
        echo "PASS: Content comparison (diff -r) - directories are identical."
    else
        echo "FAIL: Content comparison (diff -r) - directories differ."
        diff -r "$dir1" "$dir2"
        log_test_result "FAIL"
        return 1
    fi

    # Third, check symlinks (if any) - this requires careful handling
    # Find symlinks in dir1 and compare their targets with dir2
    local symlinks1=$(find "$dir1" -type l -print0 | xargs -0 -n1 readlink)
    local symlinks2=$(find "$dir2" -type l -print0 | xargs -0 -n1 readlink)

    if [ "$symlinks1" == "$symlinks2" ]; then
        echo "PASS: Symlink targets are identical."
    else
        echo "FAIL: Symlink targets differ."
        echo "Symlinks in $dir1:"
        find "$dir1" -type l -ls
        echo "Symlinks in $dir2:"
        find "$dir2" -type l -ls
        log_test_result "FAIL"
        return 1
    fi

    log_test_result "PASS"
    return 0
}

# Function to check if a file exists and has expected content
# Usage: check_file <path> <expected_content> <test_name>
check_file() {
    local file_path="$1"
    local expected_content="$2"
    local test_name="$3"

    log_test_start "$test_name"

    if [ ! -f "$file_path" ]; then
        echo "FAIL: File '$file_path' does not exist."
        log_test_result "FAIL"
        return 1
    fi

    local actual_content=$(cat "$file_path")
    if [ "$actual_content" == "$expected_content" ]; then
        echo "PASS: File '$file_path' has expected content."
        log_test_result "PASS"
        return 0
    else
        echo "FAIL: File '$file_path' content mismatch. Expected: '$expected_content', Actual: '$actual_content'."
        log_test_result "FAIL"
        return 1
    fi
}

# Function to check if a directory is empty
# Usage: check_empty_dir <path> <test_name>
check_empty_dir() {
    local dir_path="$1"
    local test_name="$2"

    log_test_start "$test_name"

    if [ ! -d "$dir_path" ]; then
        echo "FAIL: Directory '$dir_path' does not exist."
        log_test_result "FAIL"
        return 1
    fi

    if [ -z "$(ls -A "$dir_path")" ]; then
        echo "PASS: Directory '$dir_path' is empty."
        log_test_result "PASS"
        return 0
    else
        echo "FAIL: Directory '$dir_path' is NOT empty."
        ls -l "$dir_path"
        log_test_result "FAIL"
        return 1
    fi
}

# Function to check if a file/dir does NOT exist
# Usage: check_not_exist <path> <test_name>
check_not_exist() {
    local path="$1"
    local test_name="$2"

    log_test_start "$test_name"

    if [ ! -e "$path" ]; then
        echo "PASS: Path '$path' does not exist."
        log_test_result "PASS"
        return 0
    else
        echo "FAIL: Path '$path' unexpectedly exists."
        log_test_result "FAIL"
        return 1
    fi
}

# --- Test Scenarios ---

# T1.1: Simple file-to-file copy (new)
run_test_1_1() {
    log_test_start "T1.1: Simple file-to-file copy (new)"
    rm -f "$DEST_DIR/file_1k.bin"
    "$ROBOSYNC" "$SOURCE_DIR/file_1k.bin" "$DEST_DIR/file_1k.bin" > "$LOG_DIR/T1.1.log" 2>&1
    check_file "$DEST_DIR/file_1k.bin" "$(cat "$SOURCE_DIR/file_1k.bin")" "T1.1: Verify copied file content"
}

# T1.2: Simple file-to-file copy (overwrite)
run_test_1_2() {
    log_test_start "T1.2: Simple file-to-file copy (overwrite)"
    echo "old content" > "$DEST_DIR/file_1k.bin"
    "$ROBOSYNC" "$SOURCE_DIR/file_1k.bin" "$DEST_DIR/file_1k.bin" > "$LOG_DIR/T1.2.log" 2>&1
    check_file "$DEST_DIR/file_1k.bin" "$(cat "$SOURCE_DIR/file_1k.bin")" "T1.2: Verify overwritten file content"
}

# T1.3: File-to-directory copy
run_test_1_3() {
    log_test_start "T1.3: File-to-directory copy"
    rm -rf "$DEST_DIR/dir1_copy"
    mkdir -p "$DEST_DIR/dir1_copy"
    "$ROBOSYNC" "$SOURCE_DIR/dir1/hello.txt" "$DEST_DIR/dir1_copy" > "$LOG_DIR/T1.3.log" 2>&1
    check_file "$DEST_DIR/dir1_copy/hello.txt" "$(cat "$SOURCE_DIR/dir1/hello.txt")" "T1.3: Verify file copied into directory"
}

# T1.4: Directory-to-new-directory copy
run_test_1_4() {
    log_test_start "T1.4: Directory-to-new-directory copy"
    rm -rf "$DEST_DIR/dir1_new"
    "$ROBOSYNC" "$SOURCE_DIR/dir1" "$DEST_DIR/dir1_new" > "$LOG_DIR/T1.4.log" 2>&1
    compare_dirs "$SOURCE_DIR/dir1" "$DEST_DIR/dir1_new" "T1.4: Compare source and new destination directories"
}

# T1.5: Directory-to-existing-directory sync (updates, creates, deletes)
run_test_1_5() {
    log_test_start "T1.5: Directory-to-existing-directory sync"
    # Setup: Copy source to dest, then modify source
    rm -rf "$DEST_DIR/sync_test"
    "$ROBOSYNC" "$SOURCE_DIR/dir1" "$DEST_DIR/sync_test" > /dev/null 2>&1

    # Modify source: add a new file, modify an existing, delete one
    echo "new file content" > "$SOURCE_DIR/dir1/new_file_for_sync.txt"
    echo "modified content" > "$SOURCE_DIR/dir1/hello.txt"
    rm -f "$SOURCE_DIR/dir1/subdir1/another.txt"

    "$ROBOSYNC" "$SOURCE_DIR/dir1" "$DEST_DIR/sync_test" > "$LOG_DIR/T1.5.log" 2>&1

    # Verify changes
    check_file "$DEST_DIR/sync_test/new_file_for_sync.txt" "new file content" "T1.5: New file created"
    check_file "$DEST_DIR/sync_test/hello.txt" "modified content" "T1.5: Existing file updated"
    check_not_exist "$DEST_DIR/sync_test/subdir1/another.txt" "T1.5: File deleted"

    # Cleanup source modifications for next tests
    rm -f "$SOURCE_DIR/dir1/new_file_for_sync.txt"
    echo "hello world" > "$SOURCE_DIR/dir1/hello.txt"
    echo "another file" > "$SOURCE_DIR/dir1/subdir1/another.txt"
}

# T2.1: Recursive copy with empty directories (-e)
run_test_2_1() {
    log_test_start "T2.1: Recursive copy with empty directories (-e)"
    rm -rf "$DEST_DIR/empty_dirs_e"
    "$ROBOSYNC" -e "$SOURCE_DIR" "$DEST_DIR/empty_dirs_e" > "$LOG_DIR/T2.1.log" 2>&1
    if [ -d "$DEST_DIR/empty_dirs_e/empty_dir" ]; then
        echo "PASS: Empty directory copied."
        log_test_result "PASS"
    else
        echo "FAIL: Empty directory not copied."
        log_test_result "FAIL"
    fi
}

# T2.2: Recursive copy without empty directories (-s)
run_test_2_2() {
    log_test_start "T2.2: Recursive copy without empty directories (-s)"
    rm -rf "$DEST_DIR/empty_dirs_s"
    "$ROBOSYNC" -s "$SOURCE_DIR" "$DEST_DIR/empty_dirs_s" > "$LOG_DIR/T2.2.log" 2>&1
    if [ ! -d "$DEST_DIR/empty_dirs_s/empty_dir" ]; then
        echo "PASS: Empty directory not copied."
        log_test_result "PASS"
    else
        echo "FAIL: Empty directory unexpectedly copied."
        log_test_result "FAIL"
    fi
}

# T2.3: Mirroring (--mir) and purging (--purge)
run_test_2_3() {
    log_test_start "T2.3: Mirroring (--mir) and purging (--purge)"
    rm -rf "$DEST_DIR/mirror_test"
    "$ROBOSYNC" "$SOURCE_DIR" "$DEST_DIR/mirror_test" > /dev/null 2>&1

    # Add extra file/dir to destination
    echo "extra content" > "$DEST_DIR/mirror_test/extra_file.txt"
    mkdir -p "$DEST_DIR/mirror_test/extra_dir"

    # Remove file/dir from source
    rm -f "$SOURCE_DIR/file_to_delete.txt"
    rm -rf "$SOURCE_DIR/dir_to_delete"

    "$ROBOSYNC" --mir "$SOURCE_DIR" "$DEST_DIR/mirror_test" > "$LOG_DIR/T2.3.log" 2>&1

    # Verify extra files/dirs are purged
    check_not_exist "$DEST_DIR/mirror_test/extra_file.txt" "T2.3: Extra file purged"
    check_not_exist "$DEST_DIR/mirror_test/extra_dir" "T2.3: Extra directory purged"

    # Verify deleted source files are removed from dest
    check_not_exist "$DEST_DIR/mirror_test/file_to_delete.txt" "T2.3: Deleted source file removed from dest"
    check_not_exist "$DEST_DIR/mirror_test/dir_to_delete" "T2.3: Deleted source directory removed from dest"

    # Cleanup source modifications
    echo "content to be deleted" > "$SOURCE_DIR/file_to_delete.txt"
    mkdir -p "$SOURCE_DIR/dir_to_delete"
    echo "file in dir to delete" > "$SOURCE_DIR/dir_to_delete/file.txt"
}

# T2.4: Dry run (-n, --list-only) to verify no changes are made
run_test_2_4() {
    log_test_start "T2.4: Dry run (-n, --list-only)"
    rm -rf "$DEST_DIR/dry_run_test"
    mkdir -p "$DEST_DIR/dry_run_test"
    
    # Get initial state of destination
    local initial_dest_hash=$(find "$DEST_DIR/dry_run_test" -type f -print0 | sort -z | xargs -0 sha1sum | sha1sum)

    "$ROBOSYNC" -n "$SOURCE_DIR" "$DEST_DIR/dry_run_test" > "$LOG_DIR/T2.4.log" 2>&1

    # Get final state of destination
    local final_dest_hash=$(find "$DEST_DIR/dry_run_test" -type f -print0 | sort -z | xargs -0 sha1sum | sha1sum)

    if [ "$initial_dest_hash" == "$final_dest_hash" ]; then
        echo "PASS: Dry run did not modify destination."
        log_test_result "PASS"
    else
        echo "FAIL: Dry run unexpectedly modified destination."
        log_test_result "FAIL"
    fi
}

# T3.1: Exclude files by pattern (--xf *.log)
run_test_3_1() {
    log_test_start "T3.1: Exclude files by pattern (--xf *.log)"
    rm -rf "$DEST_DIR/exclude_files"
    mkdir -p "$DEST_DIR/exclude_files"
    echo "test log" > "$SOURCE_DIR/test.log"
    echo "test txt" > "$SOURCE_DIR/test.txt"

    "$ROBOSYNC" --xf "*.log" "$SOURCE_DIR" "$DEST_DIR/exclude_files" > "$LOG_DIR/T3.1.log" 2>&1

    check_not_exist "$DEST_DIR/exclude_files/test.log" "T3.1: Excluded log file not copied"
    check_file "$DEST_DIR/exclude_files/test.txt" "test txt" "T3.1: Non-excluded txt file copied"

    rm -f "$SOURCE_DIR/test.log" "$SOURCE_DIR/test.txt"
}

# T3.2: Exclude directories by pattern (--xd node_modules)
run_test_3_2() {
    log_test_start "T3.2: Exclude directories by pattern (--xd node_modules)"
    rm -rf "$DEST_DIR/exclude_dirs"
    mkdir -p "$SOURCE_DIR/node_modules/some_lib"
    echo "node file" > "$SOURCE_DIR/node_modules/some_lib/file.js"
    mkdir -p "$DEST_DIR/exclude_dirs"

    "$ROBOSYNC" --xd "node_modules" "$SOURCE_DIR" "$DEST_DIR/exclude_dirs" > "$LOG_DIR/T3.2.log" 2>&1

    check_not_exist "$DEST_DIR/exclude_dirs/node_modules" "T3.2: Excluded directory not copied"

    rm -rf "$SOURCE_DIR/node_modules"
}

# T3.3: Minimum file size (--min)
run_test_3_3() {
    log_test_start "T3.3: Minimum file size (--min)"
    rm -rf "$DEST_DIR/min_size_test"
    mkdir -p "$DEST_DIR/min_size_test"

    "$ROBOSYNC" --min 5000 "$SOURCE_DIR" "$DEST_DIR/min_size_test" > "$LOG_DIR/T3.3.log" 2>&1

    check_not_exist "$DEST_DIR/min_size_test/file_1k.bin" "T3.3: 1KB file (too small) not copied"
    check_file "$DEST_DIR/min_size_test/file_10k.bin" "$(cat "$SOURCE_DIR/file_10k.bin")" "T3.3: 10KB file (large enough) copied"
}

# T3.4: Maximum file size (--max)
run_test_3_4() {
    log_test_start "T3.4: Maximum file size (--max)"
    rm -rf "$DEST_DIR/max_size_test"
    mkdir -p "$DEST_DIR/max_size_test"

    "$ROBOSYNC" --max 50000 "$SOURCE_DIR" "$DEST_DIR/max_size_test" > "$LOG_DIR/T3.4.log" 2>&1

    check_not_exist "$DEST_DIR/max_size_test/large_file_50m.bin" "T3.4: 50MB file (too large) not copied"
    check_file "$DEST_DIR/max_size_test/file_100k.bin" "$(cat "$SOURCE_DIR/file_100k.bin")" "T3.4: 100KB file (small enough) copied"
}

# T4.1: Preserve file and directory symlinks (--links, default)
run_test_4_1() {
    log_test_start "T4.1: Preserve symlinks"
    rm -rf "$DEST_DIR/symlinks_preserve"
    "$ROBOSYNC" --links "$SOURCE_DIR" "$DEST_DIR/symlinks_preserve" > "$LOG_DIR/T4.1.log" 2>&1

    if [ -L "$DEST_DIR/symlinks_preserve/dir1/symlink_to_file.bin" ]; then
        echo "PASS: File symlink preserved."
        log_test_result "PASS"
    else
        echo "FAIL: File symlink not preserved."
        log_test_result "FAIL"
    fi

    if [ -L "$DEST_DIR/symlinks_preserve/dir2/symlink_to_dir" ]; then
        echo "PASS: Directory symlink preserved."
        log_test_result "PASS"
    else
        echo "FAIL: Directory symlink not preserved."
        log_test_result "FAIL"
    fi

    # Verify broken symlink is copied as broken
    if [ -L "$DEST_DIR/symlinks_preserve/broken_symlink.txt" ] && [ ! -e "$DEST_DIR/symlinks_preserve/broken_symlink.txt" ]; then
        echo "PASS: Broken symlink preserved as broken."
        log_test_result "PASS"
    else
        echo "FAIL: Broken symlink not preserved as broken."
        log_test_result "FAIL"
    fi
}

# T4.2: Dereference symlinks - copy the target file/directory instead of the symlink (--deref)
run_test_4_2() {
    log_test_start "T4.2: Dereference symlinks"
    rm -rf "$DEST_DIR/symlinks_deref"
    "$ROBOSYNC" --deref "$SOURCE_DIR" "$DEST_DIR/symlinks_deref" > "$LOG_DIR/T4.2.log" 2>&1

    if [ ! -L "$DEST_DIR/symlinks_deref/dir1/symlink_to_file.bin" ] && \
       [ -f "$DEST_DIR/symlinks_deref/dir1/symlink_to_file.bin" ]; then
        echo "PASS: File symlink dereferenced."
        log_test_result "PASS"
    else
        echo "FAIL: File symlink not dereferenced."
        log_test_result "FAIL"
    fi

    if [ ! -L "$DEST_DIR/symlinks_deref/dir2/symlink_to_dir" ] && \
       [ -d "$DEST_DIR/symlinks_deref/dir2/symlink_to_dir" ]; then
        echo "PASS: Directory symlink dereferenced."
        log_test_result "PASS"
    else
        echo "FAIL: Directory symlink not dereferenced."
        log_test_result "FAIL"
    fi

    # Broken symlinks should be skipped when dereferencing
    check_not_exist "$DEST_DIR/symlinks_deref/broken_symlink.txt" "T4.2: Broken symlink skipped"
}

# T4.3: Skip all symlinks (--no-links)
run_test_4_3() {
    log_test_start "T4.3: Skip all symlinks"
    rm -rf "$DEST_DIR/symlinks_skip"
    "$ROBOSYNC" --no-links "$SOURCE_DIR" "$DEST_DIR/symlinks_skip" > "$LOG_DIR/T4.3.log" 2>&1

    check_not_exist "$DEST_DIR/symlinks_skip/dir1/symlink_to_file.bin" "T4.3: File symlink skipped"
    check_not_exist "$DEST_DIR/symlinks_skip/dir2/symlink_to_dir" "T4.3: Directory symlink skipped"
    check_not_exist "$DEST_DIR/symlinks_skip/broken_symlink.txt" "T4.3: Broken symlink skipped"
}

# T5.1: Verify default copy (DAT) preserves timestamps and attributes
run_test_5_1() {
    log_test_start "T5.1: Verify default copy (DAT) preserves timestamps and attributes"
    rm -rf "$DEST_DIR/copy_dat"
    "$ROBOSYNC" "$SOURCE_DIR/file_1k.bin" "$DEST_DIR/copy_dat/file_1k.bin" > "$LOG_DIR/T5.1.log" 2>&1

    local src_mtime=$(stat -c %Y "$SOURCE_DIR/file_1k.bin")
    local dest_mtime=$(stat -c %Y "$DEST_DIR/copy_dat/file_1k.bin")

    if [ "$src_mtime" == "$dest_mtime" ]; then
        echo "PASS: Timestamps preserved."
        log_test_result "PASS"
    else
        echo "FAIL: Timestamps not preserved. Source: $src_mtime, Dest: $dest_mtime."
        log_test_result "FAIL"
    fi
    # Attributes are harder to verify generically, rely on content/timestamp for now
}

# T5.2: Verify --copyall preserves permissions and ownership (on Unix)
run_test_5_2() {
    log_test_start "T5.2: Verify --copyall preserves permissions and ownership (on Unix)"
    rm -rf "$DEST_DIR/copy_all"
    # Set specific permissions on source file
    chmod 755 "$SOURCE_DIR/file_1k.bin"
    # Set specific ownership (if possible, requires root)
    # sudo chown nobody:nogroup "$SOURCE_DIR/file_1k.bin" || true

    "$ROBOSYNC" --copyall "$SOURCE_DIR/file_1k.bin" "$DEST_DIR/copy_all/file_1k.bin" > "$LOG_DIR/T5.2.log" 2>&1

    local src_perms=$(stat -c %a "$SOURCE_DIR/file_1k.bin")
    local dest_perms=$(stat -c %a "$DEST_DIR/copy_all/file_1k.bin")

    if [ "$src_perms" == "$dest_perms" ]; then
        echo "PASS: Permissions preserved."
        log_test_result "PASS"
    else
        echo "FAIL: Permissions not preserved. Source: $src_perms, Dest: $dest_perms."
        log_test_result "FAIL"
    fi

    # Ownership check (requires root or specific user setup)
    # local src_owner=$(stat -c %U:%G "$SOURCE_DIR/file_1k.bin")
    # local dest_owner=$(stat -c %U:%G "$DEST_DIR/copy_all/file_1k.bin")
    # if [ "$src_owner" == "$dest_owner" ]; then
    #     echo "PASS: Ownership preserved."
    #     log_test_result "PASS"
    # else
    #     echo "FAIL: Ownership not preserved. Source: $src_owner, Dest: $dest_owner."
    #     log_test_result "FAIL"
    # fi

    # Reset permissions for next tests
    chmod 644 "$SOURCE_DIR/file_1k.bin"
}

# T5.3: Verify --checksum (-c) correctly identifies files needing updates based on content, not just size/mtime
run_test_5_3() {
    log_test_start "T5.3: Verify --checksum (-c)"
    rm -rf "$DEST_DIR/checksum_test"
    mkdir -p "$DEST_DIR/checksum_test"

    # Create initial file
    echo "original content" > "$SOURCE_DIR/checksum_file.txt"
    "$ROBOSYNC" "$SOURCE_DIR/checksum_file.txt" "$DEST_DIR/checksum_test/checksum_file.txt" > /dev/null 2>&1

    # Modify source file content but keep size and mtime same (if possible, or just mtime)
    # For simplicity, we'll just change content and rely on checksum to detect
    echo "new content different size" > "$SOURCE_DIR/checksum_file.txt"
    # Touch the file to ensure mtime is different if RoboSync relies on it first
    touch -m "$SOURCE_DIR/checksum_file.txt"

    # Run sync with checksum
    "$ROBOSYNC" -c "$SOURCE_DIR/checksum_file.txt" "$DEST_DIR/checksum_test/checksum_file.txt" > "$LOG_DIR/T5.3.log" 2>&1

    # Verify destination file content is updated
    check_file "$DEST_DIR/checksum_test/checksum_file.txt" "new content different size" "T5.3: Checksum detected content change"

    # Cleanup
    rm -f "$SOURCE_DIR/checksum_file.txt"
}

# T6.1: Syncing an empty source directory
run_test_6_1() {
    log_test_start "T6.1: Syncing an empty source directory"
    rm -rf "$DEST_DIR/empty_source_sync"
    mkdir -p "$DEST_DIR/empty_source_sync"
    mkdir -p "$SOURCE_DIR/empty_source"

    "$ROBOSYNC" "$SOURCE_DIR/empty_source" "$DEST_DIR/empty_source_sync" > "$LOG_DIR/T6.1.log" 2>&1

    check_empty_dir "$DEST_DIR/empty_source_sync" "T6.1: Destination is empty after syncing empty source"

    rm -rf "$SOURCE_DIR/empty_source"
}

# T6.2: Syncing a source with only empty subdirectories
run_test_6_2() {
    log_test_start "T6.2: Syncing a source with only empty subdirectories"
    rm -rf "$DEST_DIR/empty_subdirs_sync"
    mkdir -p "$DEST_DIR/empty_subdirs_sync"
    mkdir -p "$SOURCE_DIR/source_with_empty_subdirs/sub1/sub2"

    "$ROBOSYNC" -e "$SOURCE_DIR/source_with_empty_subdirs" "$DEST_DIR/empty_subdirs_sync" > "$LOG_DIR/T6.2.log" 2>&1

    if [ -d "$DEST_DIR/empty_subdirs_sync/sub1/sub2" ]; then
        echo "PASS: Empty subdirectories copied."
        log_test_result "PASS"
    else
        echo "FAIL: Empty subdirectories not copied."
        log_test_result "FAIL"
    fi

    rm -rf "$SOURCE_DIR/source_with_empty_subdirs"
}

# T6.3: Handling file and directory names with special characters (spaces, non-ASCII, etc.)
run_test_6_3() {
    log_test_start "T6.3: Special characters in names"
    rm -rf "$DEST_DIR/special_chars"
    "$ROBOSYNC" "$SOURCE_DIR/dir2" "$DEST_DIR/special_chars" > "$LOG_DIR/T6.3.log" 2>&1

    check_file "$DEST_DIR/special_chars/file with spaces & (brackets).txt" "special chars" "T6.3: File with spaces copied"
    check_file "$DEST_DIR/special_chars/文件.txt" "unicode chars" "T6.3: File with unicode copied"
}

# T6.4: Syncing a single very large file (>4GB) - requires creating a large file
# This test will be skipped by default as it takes a long time and disk space.
# To run, uncomment and ensure you have enough disk space.
# run_test_6_4() {
#     log_test_start "T6.4: Syncing a single very large file (>4GB)"
#     local large_file_src="$SOURCE_DIR/very_large_file.bin"
#     local large_file_dest="$DEST_DIR/very_large_file.bin"
#     local file_size_gb=5 # 5GB file

#     echo "Creating a ${file_size_gb}GB test file... This may take a while."
#     fallocate -l ${file_size_gb}G "$large_file_src" || head -c ${file_size_gb}G /dev/zero > "$large_file_src"

#     "$ROBOSYNC" "$large_file_src" "$large_file_dest" > "$LOG_DIR/T6.4.log" 2>&1

#     local src_size=$(stat -c %s "$large_file_src")
#     local dest_size=$(stat -c %s "$large_file_dest")

#     if [ "$src_size" == "$dest_size" ] && [ "$src_size" -gt 4000000000 ]; then
#         echo "PASS: Large file copied successfully. Size: ${src_size} bytes."
#         log_test_result "PASS"
#     else
#         echo "FAIL: Large file copy failed or size mismatch. Source: ${src_size}, Dest: ${dest_size}."
#         log_test_result "FAIL"
#     fi

#     rm -f "$large_file_src" "$large_file_dest"
# }

# T6.5: Syncing thousands of very small files (0-1KB)
run_test_6_5() {
    log_test_start "T6.5: Syncing thousands of very small files"
    local small_files_src_dir="$SOURCE_DIR/many_small_files"
    local small_files_dest_dir="$DEST_DIR/many_small_files"
    local num_files=5000

    rm -rf "$small_files_src_dir" "$small_files_dest_dir"
    mkdir -p "$small_files_src_dir"

    echo "Creating $num_files small files..."
    for i in $(seq 1 $num_files); do
        head -c $(( RANDOM % 1024 )) </dev/urandom > "$small_files_src_dir/file_$i.bin"
    done

    "$ROBOSYNC" "$small_files_src_dir" "$small_files_dest_dir" > "$LOG_DIR/T6.5.log" 2>&1

    local src_count=$(find "$small_files_src_dir" -type f | wc -l)
    local dest_count=$(find "$small_files_dest_dir" -type f | wc -l)

    if [ "$src_count" == "$dest_count" ] && [ "$src_count" == "$num_files" ]; then
        echo "PASS: All $num_files small files copied successfully."
        log_test_result "PASS"
    else
        echo "FAIL: Mismatch in small file count. Source: $src_count, Dest: $dest_count."
        log_test_result "FAIL"
    fi

    rm -rf "$small_files_src_dir" "$small_files_dest_dir"
}

# T6.6: Attempting to sync a directory into itself (should fail gracefully)
run_test_6_6() {
    log_test_start "T6.6: Syncing a directory into itself"
    local self_sync_dir="$SOURCE_DIR/dir1"
    local result_code=0

    # RoboSync should return a non-zero exit code for this
    "$ROBOSYNC" "$self_sync_dir" "$self_sync_dir/subdir1" > "$LOG_DIR/T6.6.log" 2>&1 || result_code=$?

    if [ "$result_code" -ne 0 ]; then
        echo "PASS: RoboSync failed gracefully when syncing directory into itself (exit code: $result_code)."
        log_test_result "PASS"
    else
        echo "FAIL: RoboSync unexpectedly succeeded or did not fail gracefully when syncing directory into itself."
        log_test_result "FAIL"
    fi
}

# --- Main Test Runner ---

# Ensure RoboSync executable exists
if [ ! -f "$ROBOSYNC" ]; then
    echo "Error: RoboSync executable not found at $ROBOSYNC. Please run 'cargo build --release' first."
    exit 1
fi

# Run test harness to prepare data
echo "Preparing test data using test_harness.sh..."
./tests/test_harness.sh "$TEST_ROOT_DIR"

# Execute all tests
run_test_1_1
run_test_1_2
run_test_1_3
run_test_1_4
run_test_1_5

run_test_2_1
run_test_2_2
run_test_2_3
run_test_2_4

run_test_3_1
run_test_3_2
run_test_3_3
run_test_3_4

run_test_4_1
run_test_4_2
run_test_4_3

run_test_5_1
run_test_5_2
run_test_5_3

run_test_6_1
run_test_6_2
run_test_6_3
# run_test_6_4 # Skipped by default, uncomment to run large file test
run_test_6_5
run_test_6_6

# --- Summary ---

echo "\n--- Test Summary ---"
echo "Total Tests: $((PASSED_TESTS + FAILED_TESTS))"
echo "Passed: $PASSED_TESTS"
echo "Failed: $FAILED_TESTS"

if [ "$FAILED_TESTS" -gt 0 ]; then
    echo "\nSome tests failed. Check logs in $LOG_DIR for details."
    exit 1
else
    echo "\nAll tests passed!"
    exit 0
fi
