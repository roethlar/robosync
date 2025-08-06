# Windows Testing Plan for RoboSync 2.0.0

## Overview
This testing plan is designed for winclaude to validate RoboSync on Windows platforms, including NTFS and ReFS filesystems.

## Prerequisites
1. Windows 10/11 or Windows Server 2019+
2. Rust toolchain installed (`rustup`)
3. PowerShell with Administrator privileges (for some tests)
4. At least 10GB free disk space
5. Access to both NTFS and ReFS volumes (if possible)

## Build Instructions
```powershell
# Clone the repository
git clone https://github.com/yourusername/robosync.git
cd robosync

# Build in release mode
cargo build --release

# Verify the binary
.\target\release\robosync.exe --version
```

## Test Suite 1: Basic Functionality Tests

### 1.1 Single File Operations
```powershell
# Create test data
mkdir C:\temp\robosync_test\source
echo "Test content" > C:\temp\robosync_test\source\test.txt

# Test file-to-file copy
.\target\release\robosync.exe C:\temp\robosync_test\source\test.txt C:\temp\robosync_test\dest.txt

# Test file-to-directory copy
mkdir C:\temp\robosync_test\dest_dir
.\target\release\robosync.exe C:\temp\robosync_test\source\test.txt C:\temp\robosync_test\dest_dir\

# Verify
fc C:\temp\robosync_test\source\test.txt C:\temp\robosync_test\dest.txt
```

### 1.2 Directory Operations
```powershell
# Create nested directory structure
mkdir C:\temp\robosync_test\source\subdir1\subdir2
echo "File 1" > C:\temp\robosync_test\source\file1.txt
echo "File 2" > C:\temp\robosync_test\source\subdir1\file2.txt
echo "File 3" > C:\temp\robosync_test\source\subdir1\subdir2\file3.txt

# Test recursive copy
.\target\release\robosync.exe C:\temp\robosync_test\source C:\temp\robosync_test\dest -s

# Test mirror mode
.\target\release\robosync.exe C:\temp\robosync_test\source C:\temp\robosync_test\dest --mir
```

## Test Suite 2: Windows-Specific Features

### 2.1 NTFS Attributes and Permissions
```powershell
# Create file with specific attributes
$testFile = "C:\temp\robosync_test\source\attributes.txt"
echo "Attribute test" > $testFile
attrib +R +H +S $testFile

# Copy with attributes preservation
.\target\release\robosync.exe $testFile C:\temp\robosync_test\dest\attributes.txt --copyall

# Verify attributes
attrib C:\temp\robosync_test\dest\attributes.txt
```

### 2.2 Alternate Data Streams (ADS)
```powershell
# Create file with ADS
echo "Main content" > C:\temp\robosync_test\source\ads_test.txt
echo "Hidden stream" > C:\temp\robosync_test\source\ads_test.txt:hidden

# Copy with ADS preservation
.\target\release\robosync.exe C:\temp\robosync_test\source\ads_test.txt C:\temp\robosync_test\dest\

# Verify ADS
Get-Content C:\temp\robosync_test\dest\ads_test.txt:hidden
```

### 2.3 Symbolic Links and Junctions
```powershell
# Run as Administrator
# Create symlinks and junctions
mklink C:\temp\robosync_test\source\link.txt C:\temp\robosync_test\source\test.txt
mklink /D C:\temp\robosync_test\source\dirlink C:\temp\robosync_test\source\subdir1
mklink /J C:\temp\robosync_test\source\junction C:\temp\robosync_test\source\subdir1

# Test symlink handling
.\target\release\robosync.exe C:\temp\robosync_test\source C:\temp\robosync_test\dest_links --links
```

## Test Suite 3: Performance Benchmarks

### 3.1 Small Files Performance
```powershell
# Create 5000 small files
1..5000 | ForEach-Object {
    $size = Get-Random -Minimum 1 -Maximum 10
    fsutil file createnew "C:\temp\robosync_test\small_files\file_$_.dat" ($size * 1KB)
}

# Benchmark against robocopy
Measure-Command { 
    .\target\release\robosync.exe C:\temp\robosync_test\small_files C:\temp\robosync_test\dest_small
}

Measure-Command {
    robocopy C:\temp\robosync_test\small_files C:\temp\robosync_test\dest_small_robocopy /E
}
```

### 3.2 Large Files Performance
```powershell
# Create large test files
fsutil file createnew C:\temp\robosync_test\large\1GB.dat 1073741824
fsutil file createnew C:\temp\robosync_test\large\500MB.dat 524288000

# Benchmark
Measure-Command {
    .\target\release\robosync.exe C:\temp\robosync_test\large C:\temp\robosync_test\dest_large
}
```

### 3.3 Run Comprehensive Benchmark
```powershell
# Use the provided benchmark script
.\benchmark_vs_robocopy.ps1
```

## Test Suite 4: ReFS-Specific Tests (if available)

### 4.1 ReFS Reflink Support
```powershell
# Verify ReFS volume
fsutil fsinfo volumeinfo R:\

# Create test file on ReFS
echo "ReFS test content" > R:\robosync_test\source.txt

# Test reflink copy
.\target\release\robosync.exe R:\robosync_test\source.txt R:\robosync_test\dest.txt

# Verify block cloning occurred (check disk usage)
```

## Test Suite 5: Error Handling and Edge Cases

### 5.1 Permission Errors
```powershell
# Create protected directory
$acl = Get-Acl C:\temp\robosync_test\protected
$acl.SetAccessRuleProtection($true, $false)
Set-Acl C:\temp\robosync_test\protected $acl

# Test error handling
.\target\release\robosync.exe C:\temp\robosync_test\source C:\temp\robosync_test\protected\
```

### 5.2 Long Path Support
```powershell
# Create deep directory structure
$longPath = "C:\temp\robosync_test\very_long_path_name_that_exceeds_normal_limits\"
1..20 | ForEach-Object { $longPath += "subdirectory_$_\" }
New-Item -ItemType Directory -Path $longPath -Force

# Test long path handling
.\target\release\robosync.exe C:\temp\robosync_test\source $longPath
```

### 5.3 Special Characters
```powershell
# Create files with special characters
echo "Test" > "C:\temp\robosync_test\source\file with spaces.txt"
echo "Test" > "C:\temp\robosync_test\source\file[brackets].txt"
echo "Test" > "C:\temp\robosync_test\source\file`$special.txt"

# Test handling
.\target\release\robosync.exe C:\temp\robosync_test\source C:\temp\robosync_test\dest_special
```

## Test Suite 6: Enterprise Features

### 6.1 Mission-Critical Mode
```powershell
# Test enterprise mode with integrity verification
.\target\release\robosync.exe C:\temp\robosync_test\source C:\temp\robosync_test\dest_enterprise --enterprise

# Test with checksum verification
.\target\release\robosync.exe C:\temp\robosync_test\source C:\temp\robosync_test\dest_checksum -c
```

## Expected Results

1. **Basic Operations**: All file and directory copies should complete successfully with correct content and metadata
2. **Windows Features**: NTFS attributes, ADS, and permissions should be preserved when using appropriate flags
3. **Performance**: RoboSync should be competitive with or faster than robocopy, especially for small files
4. **ReFS**: Should utilize reflink/block cloning when available
5. **Error Handling**: Should gracefully handle errors and provide clear error messages

## Reporting

Please report:
1. Any test failures with exact error messages
2. Performance comparison results (RoboSync vs robocopy)
3. Any Windows-specific issues or incompatibilities
4. ReFS reflink functionality status
5. Overall assessment and recommendations

## Cleanup
```powershell
# Remove test directories
Remove-Item -Recurse -Force C:\temp\robosync_test
if (Test-Path R:\robosync_test) { Remove-Item -Recurse -Force R:\robosync_test }
```