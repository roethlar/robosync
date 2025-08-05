# macOS Testing Plan for RoboSync 2.0.0

## Overview
This testing plan is designed for macclaude to validate RoboSync on macOS platforms, focusing on APFS, HFS+, and network filesystem performance.

## Prerequisites
1. macOS 10.15 (Catalina) or later
2. Rust toolchain installed (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
3. Xcode Command Line Tools (`xcode-select --install`)
4. At least 10GB free disk space
5. Access to both APFS and HFS+ volumes (if possible)

## Build Instructions
```bash
# Clone the repository
git clone https://github.com/yourusername/robosync.git
cd robosync

# Build in release mode
cargo build --release

# Verify the binary
./target/release/robosync --version
```

## Test Suite 1: Basic Functionality Tests

### 1.1 Single File Operations
```bash
# Create test data
mkdir -p ~/robosync_test/source
echo "Test content" > ~/robosync_test/source/test.txt

# Test file-to-file copy
./target/release/robosync ~/robosync_test/source/test.txt ~/robosync_test/dest.txt

# Test file-to-directory copy
mkdir -p ~/robosync_test/dest_dir
./target/release/robosync ~/robosync_test/source/test.txt ~/robosync_test/dest_dir/

# Verify
diff ~/robosync_test/source/test.txt ~/robosync_test/dest.txt
```

### 1.2 Directory Operations
```bash
# Create nested directory structure
mkdir -p ~/robosync_test/source/subdir1/subdir2
echo "File 1" > ~/robosync_test/source/file1.txt
echo "File 2" > ~/robosync_test/source/subdir1/file2.txt
echo "File 3" > ~/robosync_test/source/subdir1/subdir2/file3.txt

# Test recursive copy
./target/release/robosync ~/robosync_test/source ~/robosync_test/dest -s

# Test mirror mode
./target/release/robosync ~/robosync_test/source ~/robosync_test/dest --mir
```

## Test Suite 2: macOS-Specific Features

### 2.1 Extended Attributes (xattr)
```bash
# Create file with extended attributes
echo "xattr test" > ~/robosync_test/source/xattr_test.txt
xattr -w com.apple.metadata:test "test value" ~/robosync_test/source/xattr_test.txt
xattr -w user.comment "This is a comment" ~/robosync_test/source/xattr_test.txt

# Copy with attributes preservation
./target/release/robosync ~/robosync_test/source/xattr_test.txt ~/robosync_test/dest/ --copyall

# Verify attributes
xattr -l ~/robosync_test/dest/xattr_test.txt
```

### 2.2 Resource Forks
```bash
# Create file with resource fork (using Finder info)
touch ~/robosync_test/source/resource_test.txt
echo "resource data" > ~/robosync_test/source/resource_test.txt/..namedfork/rsrc

# Copy with resource fork preservation
./target/release/robosync ~/robosync_test/source/resource_test.txt ~/robosync_test/dest/

# Verify resource fork
ls -l ~/robosync_test/dest/resource_test.txt/..namedfork/rsrc
```

### 2.3 Symbolic Links
```bash
# Create various symlinks
ln -s test.txt ~/robosync_test/source/link.txt
ln -s subdir1 ~/robosync_test/source/dirlink
ln -s /usr/bin/ls ~/robosync_test/source/abs_link

# Test symlink handling
./target/release/robosync ~/robosync_test/source ~/robosync_test/dest_links --links

# Verify symlinks
ls -la ~/robosync_test/dest_links/
```

### 2.4 Application Bundles
```bash
# Copy an actual app bundle (use Calculator as test)
cp -R /System/Applications/Calculator.app ~/robosync_test/source/

# Test bundle preservation
./target/release/robosync ~/robosync_test/source/Calculator.app ~/robosync_test/dest/

# Verify bundle integrity
codesign -v ~/robosync_test/dest/Calculator.app
```

## Test Suite 3: APFS-Specific Features

### 3.1 APFS Clone (Reflink) Support
```bash
# Create test file on APFS volume
dd if=/dev/zero of=~/robosync_test/source/100mb.dat bs=1m count=100

# Get initial disk usage
df -h ~

# Test APFS clonefile
./target/release/robosync ~/robosync_test/source/100mb.dat ~/robosync_test/dest/100mb_clone.dat

# Verify space-efficient copy (should use minimal additional space)
df -h ~

# Verify clone relationship
ls -li ~/robosync_test/source/100mb.dat ~/robosync_test/dest/100mb_clone.dat
```

### 3.2 APFS Sparse Files
```bash
# Create sparse file
dd if=/dev/zero of=~/robosync_test/source/sparse.dat bs=1m seek=1000 count=1

# Check actual disk usage
du -h ~/robosync_test/source/sparse.dat
ls -lh ~/robosync_test/source/sparse.dat

# Copy sparse file
./target/release/robosync ~/robosync_test/source/sparse.dat ~/robosync_test/dest/

# Verify sparse file preserved
du -h ~/robosync_test/dest/sparse.dat
```

## Test Suite 4: Performance Benchmarks

### 4.1 Small Files Performance
```bash
# Create 5000 small files
mkdir -p ~/robosync_test/small_files
for i in {1..5000}; do
    size=$((RANDOM % 10 + 1))
    dd if=/dev/urandom of=~/robosync_test/small_files/file_$i.dat bs=1k count=$size 2>/dev/null
done

# Benchmark against rsync
time ./target/release/robosync ~/robosync_test/small_files ~/robosync_test/dest_small

# Clean and benchmark rsync
rm -rf ~/robosync_test/dest_small
time rsync -a ~/robosync_test/small_files/ ~/robosync_test/dest_small/
```

### 4.2 Large Files Performance
```bash
# Create large test files
dd if=/dev/zero of=~/robosync_test/large/1GB.dat bs=1m count=1024
dd if=/dev/zero of=~/robosync_test/large/500MB.dat bs=1m count=512

# Benchmark
time ./target/release/robosync ~/robosync_test/large ~/robosync_test/dest_large
```

### 4.3 Run Comprehensive Benchmark
```bash
# Use the provided benchmark script
chmod +x benchmark_vs_rsync_macos.sh
./benchmark_vs_rsync_macos.sh
```

## Test Suite 5: Network Filesystem Tests

### 5.1 SMB/CIFS Performance
```bash
# Mount SMB share (adjust server/share as needed)
mkdir -p ~/mnt/smb
mount -t smbfs //server/share ~/mnt/smb

# Test network copy
./target/release/robosync ~/robosync_test/source ~/mnt/smb/robosync_test

# Compare with rsync
time rsync -a ~/robosync_test/source/ ~/mnt/smb/robosync_test_rsync/
```

### 5.2 AFP Performance (if available)
```bash
# Mount AFP share
mkdir -p ~/mnt/afp
mount -t afp afp://server/share ~/mnt/afp

# Test AFP copy
./target/release/robosync ~/robosync_test/source ~/mnt/afp/robosync_test
```

## Test Suite 6: Error Handling and Edge Cases

### 6.1 Permission Errors
```bash
# Create protected directory
mkdir -p ~/robosync_test/protected
chmod 000 ~/robosync_test/protected

# Test error handling
./target/release/robosync ~/robosync_test/source ~/robosync_test/protected/

# Cleanup
chmod 755 ~/robosync_test/protected
```

### 6.2 Special Characters
```bash
# Create files with special characters
touch ~/robosync_test/source/"file with spaces.txt"
touch ~/robosync_test/source/"file:with:colons.txt"
touch ~/robosync_test/source/"file|with|pipes.txt"
touch ~/robosync_test/source/"file★with★unicode.txt"

# Test handling
./target/release/robosync ~/robosync_test/source ~/robosync_test/dest_special
```

### 6.3 Case Sensitivity
```bash
# Test case sensitivity handling
touch ~/robosync_test/source/CaseSensitive.txt
touch ~/robosync_test/source/casesensitive.txt

# Copy to case-insensitive destination (if available)
./target/release/robosync ~/robosync_test/source ~/robosync_test/dest_case
```

## Test Suite 7: Enterprise Features

### 7.1 Mission-Critical Mode
```bash
# Test enterprise mode with integrity verification
./target/release/robosync ~/robosync_test/source ~/robosync_test/dest_enterprise --enterprise

# Test with checksum verification
./target/release/robosync ~/robosync_test/source ~/robosync_test/dest_checksum -c
```

### 7.2 Spotlight and Finder Integration
```bash
# Create files with Spotlight metadata
echo "Searchable content" > ~/robosync_test/source/spotlight.txt
xattr -w com.apple.metadata:kMDItemWhereFroms "https://example.com" ~/robosync_test/source/spotlight.txt

# Copy and verify Spotlight metadata preserved
./target/release/robosync ~/robosync_test/source/spotlight.txt ~/robosync_test/dest/

# Check if Spotlight can find the copied file
mdfind -name spotlight.txt
```

## Expected Results

1. **Basic Operations**: All file and directory copies should complete successfully
2. **macOS Features**: Extended attributes, resource forks, and bundle structures should be preserved
3. **APFS Features**: Clone operations should be space-efficient, sparse files preserved
4. **Performance**: RoboSync should be competitive with or faster than rsync
5. **Network FS**: Should handle SMB/AFP gracefully with appropriate performance
6. **Error Handling**: Should provide clear error messages for permission/access issues

## Reporting

Please report:
1. Any test failures with exact error messages
2. Performance comparison results (RoboSync vs rsync)
3. APFS clonefile functionality status
4. Network filesystem performance characteristics
5. Any macOS-specific issues or incompatibilities
6. Overall assessment and recommendations

## Cleanup
```bash
# Remove test directories
rm -rf ~/robosync_test
rm -rf ~/mnt/smb/robosync_test*
rm -rf ~/mnt/afp/robosync_test*

# Unmount network shares if mounted
umount ~/mnt/smb 2>/dev/null
umount ~/mnt/afp 2>/dev/null
```