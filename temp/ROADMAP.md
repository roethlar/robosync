# RoboSync Performance Optimization Roadmap

This document outlines planned performance optimizations and enhancements for RoboSync, organized by priority and implementation complexity.

## Overview

RoboSync currently provides excellent cross-platform file synchronization with delta transfer capabilities. However, significant performance improvements are possible by leveraging filesystem-specific features and optimizing for different storage technologies.

## Phase 1: High-Impact, Low-Complexity Optimizations

### 1.1 Same-Filesystem Detection
**Priority**: CRITICAL  
**Effort**: Low  
**Impact**: Enables all CoW optimizations  
**Implementation**:
- Add device ID comparison on Unix systems
- Add volume serial number comparison on Windows
- Create `are_same_filesystem()` helper function
- Integrate into strategy selection logic

### 1.2 Basic Reflink Support (Linux BTRFS/XFS)
**Priority**: HIGH  
**Effort**: Medium  
**Impact**: 10-100x speedup for same-filesystem copies  
**Implementation**:
- Add `FICLONE` ioctl support for BTRFS
- Add `copy_file_range()` with reflink flag for XFS
- Create `try_reflink_copy()` in platform_api.rs
- Graceful fallback to standard copy

### 1.3 Dynamic Buffer Sizing
**Priority**: HIGH  
**Effort**: Low  
**Impact**: 2-8x network throughput improvement  
**Implementation**:
- Detect network link speed on Windows
- Scale buffers from 1MB to 32MB based on:
  - Filesystem type (ZFS prefers 1MB, BTRFS handles 16MB)
  - Network speed (10GbE+ gets larger buffers)
  - Available memory

### 1.4 FAT32/ExFAT Reliability Mode
**Priority**: MEDIUM  
**Effort**: Low  
**Impact**: Prevent data corruption on removable media  
**Implementation**:
- Detect FAT-based filesystems
- Force sync after each write
- Enable automatic verification
- Increase retry attempts

## Phase 2: Platform-Specific Optimizations

### 2.1 Windows ReFS Block Cloning
**Priority**: HIGH  
**Effort**: Medium  
**Impact**: Instant copies on ReFS volumes  
**Implementation**:
- Use `FSCTL_DUPLICATE_EXTENTS_TO_FILE` DeviceIoControl
- Add ReFS capability detection
- Integrate with reflink infrastructure

### 2.2 macOS APFS Clonefile Support
**Priority**: MEDIUM  
**Effort**: Medium  
**Impact**: Instant copies on APFS  
**Implementation**:
- Add `clonefile()` system call support
- Detect APFS filesystem
- Handle macOS-specific error codes

### 2.3 NTFS Sparse File Optimization
**Priority**: HIGH  
**Effort**: Medium  
**Impact**: 90%+ space/time savings for sparse files  
**Implementation**:
- Use `FSCTL_QUERY_ALLOCATED_RANGES` to detect sparse regions
- Skip unallocated regions during copy
- Preserve sparse file attributes

### 2.4 NTFS Alternate Data Streams
**Priority**: MEDIUM  
**Effort**: Medium  
**Impact**: Complete NTFS metadata preservation  
**Implementation**:
- Use `FindFirstStreamW`/`FindNextStreamW` APIs
- Copy all named streams
- Add option to include/exclude ADS

## Phase 3: Advanced Filesystem Integration

### 3.1 Native Checksum Verification
**Priority**: MEDIUM  
**Effort**: High  
**Impact**: 15-25% CPU reduction  
**Filesystems**: ZFS, BTRFS, ReFS with integrity streams  
**Implementation**:
- Detect filesystems with built-in checksums
- Query filesystem checksum status
- Skip redundant BLAKE3 computation
- Provide option to force verification

### 3.2 SMB Multichannel Support
**Priority**: HIGH  
**Effort**: High  
**Impact**: 2-4x throughput with multiple NICs  
**Implementation**:
- Detect SMB 3.0+ capabilities
- Query available network paths
- Enable parallel data streams
- Load balance across channels

### 3.3 Volume Shadow Copy Integration
**Priority**: LOW  
**Effort**: High  
**Impact**: Consistent backups of active files  
**Implementation**:
- VSS snapshot creation
- Copy from shadow copies
- Automatic cleanup
- Error handling for VSS failures

### 3.4 ZFS Send/Receive Integration
**Priority**: LOW  
**Effort**: Very High  
**Impact**: Native ZFS replication  
**Implementation**:
- Detect ZFS-to-ZFS transfers
- Use zfs send/receive for incremental updates
- Handle snapshot management
- Fallback for cross-pool transfers

## Phase 4: Network and Protocol Optimizations

### 4.1 SMB Direct (RDMA) Support
**Priority**: LOW  
**Effort**: Very High  
**Impact**: 10x+ throughput on RDMA networks  
**Implementation**:
- Detect RDMA-capable NICs
- Implement SMB Direct protocol
- Zero-copy data transfer
- Fallback to standard SMB

### 4.2 Advanced SMB Credit Management
**Priority**: MEDIUM  
**Effort**: High  
**Impact**: 30-50% improvement on high-latency networks  
**Implementation**:
- Dynamic credit window sizing
- Latency-based optimization
- Protocol-specific tuning

### 4.3 Network Filesystem Detection
**Priority**: MEDIUM  
**Effort**: Low  
**Impact**: Better optimization decisions  
**Implementation**:
- Detect NFS vs SMB vs SSHFS
- Protocol-specific optimizations
- Appropriate retry strategies

## Phase 5: Memory and I/O Optimizations

### 5.1 Memory-Mapped I/O for Large Files
**Priority**: MEDIUM  
**Effort**: Medium  
**Impact**: 30-50% improvement for multi-GB files  
**Implementation**:
- Use mmap for files > 1GB
- Platform-specific implementations
- Graceful fallback for 32-bit systems

### 5.2 io_uring Integration (Linux)
**Priority**: LOW  
**Effort**: Very High  
**Impact**: 2-3x I/O throughput  
**Implementation**:
- Already partially implemented
- Extend to all I/O operations
- Batch submission/completion
- Kernel version detection

### 5.3 Extent-Based Copying
**Priority**: LOW  
**Effort**: High  
**Impact**: Optimized I/O patterns  
**Filesystems**: ext4, XFS, NTFS  
**Implementation**:
- Query file extent maps
- Optimize read/write patterns
- Reduce fragmentation

## Implementation Guidelines

### Testing Requirements
- Automated tests for each filesystem type
- Performance benchmarks before/after
- Data integrity verification
- Cross-platform compatibility tests

### Fallback Strategy
- All optimizations must gracefully degrade
- Never compromise data integrity
- Maintain current performance as baseline
- Clear error messages for failures

### Configuration
- Add filesystem detection logging
- Provide options to disable optimizations
- Performance metrics collection
- Debugging capabilities

## Success Metrics

### Performance Targets
- Same-filesystem copies: 10-100x improvement
- Network transfers: 2-8x improvement  
- CPU usage: 15-25% reduction with native checksums
- Memory usage: No significant increase

### Reliability Targets
- Zero data corruption
- Graceful handling of all error cases
- No regression in current capabilities
- Improved reliability on FAT32/ExFAT

## Timeline Estimates

**Phase 1**: 2-3 weeks (Foundation and quick wins)  
**Phase 2**: 4-6 weeks (Platform-specific features)  
**Phase 3**: 6-8 weeks (Advanced filesystem integration)  
**Phase 4**: 8-12 weeks (Network optimizations)  
**Phase 5**: 4-6 weeks (Memory and I/O)  

**Total**: 6-8 months for full implementation

## Notes

1. Start with Phase 1 for immediate impact
2. Phases can be implemented in parallel by different developers
3. Each optimization should be feature-flagged initially
4. Extensive testing required for filesystem-specific code
5. Documentation updates needed for new capabilities