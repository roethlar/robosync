# RoboSync 2.0.0 Comprehensive Test Plan

## Overview
This document outlines the comprehensive testing required to validate RoboSync 2.0.0 across all supported filesystems and platforms. All major features must be tested with real performance data before release.

## Test Status Summary

### ✅ Completed Tests
- [x] Network filesystem detection (NFS4)
- [x] BTRFS reflink functionality 
- [x] Buffer sizing optimizations
- [x] ZFS filesystem detection
- [x] ZFS reflink error handling

### ❌ Pending Tests
- [ ] XFS reflink functionality
- [ ] ext4 extent-based copying
- [ ] NTFS on Linux (FUSE)
- [ ] APFS on macOS
- [ ] Cross-filesystem scenarios
- [ ] io_uring async I/O
- [ ] Windows ReFS
- [ ] FreeBSD ZFS native support

## Detailed Test Requirements

### 1. Linux Filesystem Tests

#### 1.1 BTRFS (✅ Tested)
- **Location**: `/home` on development machine
- **Results**: 
  - Reflink working: Initial copy 0.107s, second copy 0.006s
  - Detection working: Shows "BTRFS (/home)"
  - Performance: ~1.5 GB/s throughput

#### 1.2 ZFS (✅ Tested)
- **Location**: TrueNAS server (AMD EPYC 7313)
- **Results**:
  - Detection fixed: Shows "ZFS (/mnt/specific-pool/home)"
  - Reflink correctly disabled on Linux (no FICLONE support)
  - Performance: 1.15-1.63 GiB/s throughput
  - Buffer optimization: 1MB buffers recommended

#### 1.3 XFS (❌ Not Tested)
- **Test Requirements**:
  ```bash
  # Create XFS test volume
  dd if=/dev/zero of=/tmp/xfs.img bs=1M count=1024
  mkfs.xfs /tmp/xfs.img
  mkdir -p /mnt/xfs_test
  mount -o loop /tmp/xfs.img /mnt/xfs_test
  ```
- **Expected**: Reflink support via FICLONE, extent-based copying

#### 1.4 ext4 (❌ Not Tested)
- **Test Requirements**:
  ```bash
  # Create ext4 test volume
  dd if=/dev/zero of=/tmp/ext4.img bs=1M count=1024
  mkfs.ext4 /tmp/ext4.img
  mkdir -p /mnt/ext4_test
  mount -o loop /tmp/ext4.img /mnt/ext4_test
  ```
- **Expected**: Extent-based copying via FIEMAP, no reflink

#### 1.5 NTFS (FUSE) (❌ Not Tested)
- **Test Requirements**: Mount NTFS volume or use ntfs-3g
- **Expected**: Basic copy functionality, Windows attribute preservation

### 2. Network Filesystem Tests

#### 2.1 NFS4 (✅ Tested)
- **Location**: `/mnt/home` on development machine
- **Results**:
  - Detection working after autofs fix
  - Performance: 187.40 MiB/s
  - Buffer optimization: 1MB buffers

#### 2.2 SMB/CIFS (❌ Not Tested)
- **Test Requirements**: Mount SMB share
- **Expected**: Detection, 512KB buffer optimization

#### 2.3 SSHFS (❌ Not Tested)
- **Test Requirements**: Mount SSHFS filesystem
- **Expected**: Detection, 64KB buffer limit

### 3. macOS Tests (❌ Not Tested)

#### 3.1 APFS
- **Test Requirements**: macOS system with APFS volume
- **Expected**: 
  - Reflink via clonefile() system call
  - Extended attribute preservation
  - 256KB buffer optimization

#### 3.2 HFS+
- **Test Requirements**: macOS system with HFS+ volume
- **Expected**: No reflink support, standard copy

### 4. Windows Tests (❌ Not Tested)

#### 4.1 NTFS (Native)
- **Test Requirements**: Windows system with NTFS
- **Expected**: Windows attribute preservation

#### 4.2 ReFS
- **Test Requirements**: Windows Server with ReFS
- **Expected**: Potential reflink support (future)

### 5. Cross-Filesystem Tests (❌ Not Tested)

Test matrix for copying between different filesystems:

| Source | Destination | Expected Behavior |
|--------|-------------|-------------------|
| BTRFS | ext4 | Fallback to regular copy |
| XFS | BTRFS | Fallback to regular copy |
| ZFS | NFS | Network optimization |
| ext4 | SMB | Network optimization |

### 6. Performance Benchmarks

Each filesystem should be tested with:

1. **Small files** (< 1MB): 1000 files
2. **Medium files** (10-100MB): 100 files  
3. **Large files** (1GB+): 10 files
4. **Mixed workload**: Combination of all sizes

Metrics to capture:
- Throughput (MB/s)
- Files per second
- CPU usage
- Memory usage
- I/O wait time

### 7. Feature-Specific Tests

#### 7.1 Reflink Modes
Test all three modes on each filesystem:
- `--reflink never`: Always regular copy
- `--reflink auto`: Attempt reflink, fallback if unsupported
- `--reflink always`: Fail if reflink not possible

#### 7.2 Buffer Sizing
- Verify adaptive buffer sizing based on:
  - File size
  - Available memory
  - Filesystem type

#### 7.3 Extent-Based Copying
- Test FIEMAP ioctl on ext4/XFS
- Verify sparse file handling
- Check hole detection

#### 7.4 Error Handling
- Permission errors
- Disk full scenarios
- Network interruptions
- Invalid paths

### 8. Regression Tests

Ensure no regressions from 1.0.x:
- [ ] Basic file copy
- [ ] Directory recursion
- [ ] Symlink handling
- [ ] Metadata preservation
- [ ] Delete functionality

## Test Execution Instructions

### Quick Test Script
```bash
#!/bin/bash
# Save as test_all_filesystems.sh

echo "=== RoboSync 2.0.0 Comprehensive Filesystem Test ==="

# Test each mounted filesystem
for mount in $(mount | grep -E 'ext4|xfs|btrfs|zfs|nfs|cifs' | awk '{print $3}'); do
    echo "Testing filesystem at: $mount"
    ./robosync --version
    
    # Create test data
    mkdir -p "$mount/robosync_test/source"
    dd if=/dev/zero of="$mount/robosync_test/source/test_100mb.bin" bs=1M count=100 2>/dev/null
    
    # Test with verbose output
    ./robosync "$mount/robosync_test/source/" "$mount/robosync_test/dest/" -e -v --reflink auto
    
    # Cleanup
    rm -rf "$mount/robosync_test"
    echo "---"
done
```

### Performance Test Script
```bash
#!/bin/bash
# Save as benchmark_filesystems.sh

for fs in btrfs xfs ext4 zfs; do
    echo "=== Benchmarking $fs ==="
    # Mount filesystem if needed
    # Run comprehensive performance tests
    # Capture metrics
done
```

## Success Criteria

RoboSync 2.0.0 can be released when:

1. **All filesystem types are correctly detected**
   - Shows proper filesystem name in debug output
   - Applies correct optimizations

2. **Reflink functionality works correctly**
   - BTRFS: ✅ Working with FICLONE
   - XFS: ❌ Needs testing with FICLONE
   - ZFS Linux: ✅ Correctly disabled (no FICLONE support)
   - APFS: ❌ Needs testing with clonefile()

3. **Performance meets expectations**
   - Local SSD: > 1 GB/s
   - Local HDD: > 100 MB/s  
   - Gigabit network: > 100 MB/s
   - No performance regressions from 1.0.x

4. **Error handling is robust**
   - No data corruption on failures
   - Clear error messages
   - Graceful fallbacks

5. **Cross-platform compatibility**
   - Linux (glibc): ✅ Tested
   - Linux (musl): ✅ Tested
   - macOS: ❌ Needs testing
   - Windows: ❌ Needs testing
   - FreeBSD: ❌ Needs testing

## Test Results Archive

### NFS4 Performance (2025-08-05)
- Detection: Working after autofs fix
- Throughput: 187.40 MiB/s
- Configuration: /mnt/home NFS4 mount

### BTRFS Reflink (2025-08-05)
- First copy: 26.77 MiB in 0.107s = 250.2 MiB/s
- Second copy: 26.77 MiB in 0.006s = 4461.7 MiB/s (reflink working)

### ZFS TrueNAS (2025-08-05)
- Hardware: AMD EPYC 7313, 157GB RAM
- Throughput: 1.15-1.63 GiB/s
- ZFS 2.3.0 with LZ4 compression
- Reflink correctly disabled on Linux

## Known Issues

1. **ZFS on Linux**: No FICLONE support (by design, not a bug)
2. **io_uring**: Not yet implemented (skeleton only)
3. **Windows ReFS**: Reflink support not implemented

## Release Checklist

- [ ] All critical filesystems tested
- [ ] Performance benchmarks documented
- [ ] No data corruption issues
- [ ] Error handling verified
- [ ] Cross-platform binaries built
- [ ] CHANGELOG.md updated with actual test results
- [ ] README.md updated with supported filesystems
- [ ] Version bumped to 2.0.0
- [ ] Git tag created
- [ ] GitHub release created

---

**Last Updated**: 2025-08-05  
**Test Plan Version**: 1.0  
**Author**: Development Team