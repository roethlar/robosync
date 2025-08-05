#!/bin/bash
# test_harness.sh - Creates comprehensive test data for RoboSync testing

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test directory setup - accept argument or use default
TEST_DIR="${1:-/tmp/robosync_test}"
SOURCE_DIR="$TEST_DIR/source"
DEST_DIR="$TEST_DIR/destination"
LOG_DIR="$TEST_DIR/logs"

echo -e "${GREEN}RoboSync Test Harness${NC}"
echo "Creating test data in: $TEST_DIR"
echo "=================================="

# Clean up previous test data
if [ -d "$TEST_DIR" ]; then
    echo -e "${YELLOW}Cleaning existing test directory...${NC}"
    rm -rf "$TEST_DIR"
fi
mkdir -p "$SOURCE_DIR" "$DEST_DIR" "$LOG_DIR"

# Function to create random content
random_content() {
    local size=$1
    if command -v openssl >/dev/null 2>&1; then
        openssl rand -base64 $size 2>/dev/null | head -c $size
    else
        head -c $size /dev/urandom | base64 | head -c $size
    fi
}

echo -e "\n${GREEN}1. Creating basic file structure...${NC}"
# Basic files
echo "Hello, World!" > "$SOURCE_DIR/hello.txt"
echo "Simple text file" > "$SOURCE_DIR/simple.txt"
random_content 1024 > "$SOURCE_DIR/file_1k.bin"
random_content 10240 > "$SOURCE_DIR/file_10k.bin"
random_content 102400 > "$SOURCE_DIR/file_100k.bin"

# Nested directories
mkdir -p "$SOURCE_DIR/dir1/subdir1/subdir2"
mkdir -p "$SOURCE_DIR/dir2/subdir1"
mkdir -p "$SOURCE_DIR/empty_dir"
mkdir -p "$SOURCE_DIR/dir_with_symlink_target"
echo "File in dir1" > "$SOURCE_DIR/dir1/hello.txt"
echo "File in subdir1" > "$SOURCE_DIR/dir1/subdir1/another.txt"
echo "Deep file" > "$SOURCE_DIR/dir1/subdir1/subdir2/deep.txt"

echo -e "\n${GREEN}2. Creating files with special names...${NC}"
# Special characters in names
echo "special chars" > "$SOURCE_DIR/dir2/file with spaces & (brackets).txt"
echo "unicode chars" > "$SOURCE_DIR/dir2/文件.txt"
touch "$SOURCE_DIR/file_with_unicode_🚀.txt"
touch "$SOURCE_DIR/file[with]brackets.txt"
touch "$SOURCE_DIR/file(with)parens.txt"
touch "$SOURCE_DIR/file\$with\$dollar.txt"

echo -e "\n${GREEN}3. Creating symlinks...${NC}"
# Symlinks (Unix only)
if [[ "$OSTYPE" != "msys" && "$OSTYPE" != "win32" ]]; then
    ln -s "file_1k.bin" "$SOURCE_DIR/dir1/symlink_to_file.bin"
    ln -s "../dir1/subdir1" "$SOURCE_DIR/dir2/symlink_to_dir"
    ln -s "/etc/hosts" "$SOURCE_DIR/link_absolute"
    ln -s "non_existent_target.txt" "$SOURCE_DIR/broken_symlink.txt"
fi

echo -e "\n${GREEN}4. Creating files of various sizes...${NC}"
# Small files (many of them for performance testing)
mkdir -p "$SOURCE_DIR/small_files"
for i in {1..1000}; do
    size=$((RANDOM % 1024 + 1))
    random_content $size > "$SOURCE_DIR/small_files/small_$i.dat"
done

# Medium files
dd if=/dev/urandom of="$SOURCE_DIR/file_1m.bin" bs=1M count=1 2>/dev/null
dd if=/dev/urandom of="$SOURCE_DIR/file_5m.bin" bs=1M count=5 2>/dev/null
dd if=/dev/urandom of="$SOURCE_DIR/file_10m.bin" bs=1M count=10 2>/dev/null

# Large file
echo "Creating large file (50MB)..."
dd if=/dev/urandom of="$SOURCE_DIR/large_file_50m.bin" bs=1M count=50 2>/dev/null

# Very large file for specific tests (optional)
if [ "${CREATE_HUGE_FILE:-false}" = "true" ]; then
    echo "Creating very large file (1GB)..."
    dd if=/dev/zero of="$SOURCE_DIR/huge_file_1g.bin" bs=1M count=1024 2>/dev/null
fi

echo -e "\n${GREEN}5. Creating files with specific attributes...${NC}"
# Different timestamps
touch -t 202001010000 "$SOURCE_DIR/old_file.txt"
touch -t 203012312359 "$SOURCE_DIR/future_file.txt"

# Read-only file
echo "Read only content" > "$SOURCE_DIR/readonly.txt"
chmod 444 "$SOURCE_DIR/readonly.txt"

# Executable file
echo "#!/bin/bash" > "$SOURCE_DIR/script.sh"
echo "echo 'Test script'" >> "$SOURCE_DIR/script.sh"
chmod +x "$SOURCE_DIR/script.sh"

# Hidden files (Unix)
if [[ "$OSTYPE" != "msys" && "$OSTYPE" != "win32" ]]; then
    echo "Hidden content" > "$SOURCE_DIR/.hidden_file"
    mkdir -p "$SOURCE_DIR/.hidden_dir"
    echo "Hidden dir content" > "$SOURCE_DIR/.hidden_dir/secret.txt"
fi

echo -e "\n${GREEN}6. Creating sparse files (if supported)...${NC}"
# Sparse files
if command -v truncate >/dev/null 2>&1; then
    truncate -s 100M "$SOURCE_DIR/sparse_100m.dat"
    echo "data" | dd of="$SOURCE_DIR/sparse_100m.dat" bs=1 seek=50000000 conv=notrunc 2>/dev/null
fi

echo -e "\n${GREEN}7. Creating files for modification/deletion tests...${NC}"
# Files that will be modified
echo "original content" > "$SOURCE_DIR/file_to_modify.txt"
echo "content to be deleted" > "$SOURCE_DIR/file_to_delete.txt"

# Directory that will be deleted
mkdir -p "$SOURCE_DIR/dir_to_delete"
echo "file in dir to delete" > "$SOURCE_DIR/dir_to_delete/file.txt"

echo -e "\n${GREEN}8. Creating checksum test files...${NC}"
# Files for checksum testing
mkdir -p "$SOURCE_DIR/checksum_test"
echo -n "exact content 1" > "$SOURCE_DIR/checksum_test/file1.txt"
echo -n "exact content 2" > "$SOURCE_DIR/checksum_test/file2.txt"
echo -n "exact content 1" > "$SOURCE_DIR/checksum_test/duplicate.txt"  # Same as file1

echo -e "\n${GREEN}9. Creating edge case files...${NC}"
# Zero-byte file
touch "$SOURCE_DIR/empty_file.txt"

# File with only newlines
printf "\n\n\n\n\n" > "$SOURCE_DIR/newlines_only.txt"

# Binary file with null bytes
printf "\x00\x01\x02\x03\x04\x05" > "$SOURCE_DIR/binary_with_nulls.dat"

echo -e "\n${GREEN}10. Creating platform-specific test data...${NC}"
# Extended attributes (macOS/Linux)
if [[ "$OSTYPE" == "darwin"* ]]; then
    echo "macOS file" > "$SOURCE_DIR/macos_xattr.txt"
    xattr -w user.testattr "test value" "$SOURCE_DIR/macos_xattr.txt" 2>/dev/null || true
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    if command -v setfattr >/dev/null 2>&1; then
        echo "Linux file" > "$SOURCE_DIR/linux_xattr.txt"
        setfattr -n user.testattr -v "test value" "$SOURCE_DIR/linux_xattr.txt" 2>/dev/null || true
    fi
fi

echo -e "\n${GREEN}Test data creation complete!${NC}"
echo "=================================="
echo "Summary:"
echo "  Total files: $(find "$SOURCE_DIR" -type f | wc -l)"
echo "  Total directories: $(find "$SOURCE_DIR" -type d | wc -l)"
if [[ "$OSTYPE" != "msys" && "$OSTYPE" != "win32" ]]; then
    echo "  Total symlinks: $(find "$SOURCE_DIR" -type l 2>/dev/null | wc -l)"
fi
echo "  Total size: $(du -sh "$SOURCE_DIR" 2>/dev/null | cut -f1)"
echo ""
echo "Source: $SOURCE_DIR"
echo "Destination: $DEST_DIR"
echo "Logs: $LOG_DIR"
