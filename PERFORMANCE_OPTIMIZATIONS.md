# RoboSync Performance Optimizations for Windows

## Issues Identified

1. **Initial Pause at Startup (1/xxx files)**
   - Caused by synchronous file enumeration using WalkDir
   - Particularly slow on network drives
   - Blocks UI while scanning directories

2. **Network Throughput Limited to 100MB/s on 10GbE**
   - Small buffer sizes (256KB) not optimal for high-speed networks
   - Using std::fs::copy which doesn't optimize for network transfers
   - No Windows-specific optimizations for SMB/CIFS

## Optimizations Applied

### 1. Increased Buffer Sizes
- **Streaming copy buffer**: Increased from 256KB to 4MB for better network performance
- **File checksum buffer**: Increased from 64KB to 1MB for faster hashing
- **Network-aware copying**: Added detection for network paths with optimized streaming

### 2. Network Path Detection
- Added `is_network_path()` function to detect:
  - Windows UNC paths (\\server\share)
  - Unix network mount points (/mnt/, /media/, /net/, /smb/)
- Uses optimized streaming copy for network transfers

### 3. Optimized File Copy Implementation
- Added `streaming_copy_optimized()` with 4MB buffers
- Pre-allocated buffers to avoid repeated allocations
- Separate buffered readers/writers for source and destination

## Expected Performance Improvements

1. **Startup Time**: File enumeration still synchronous but with larger buffers should be slightly faster
2. **Network Throughput**: Should increase from 100MB/s to 400-800MB/s on 10GbE networks
3. **Memory Usage**: Slightly higher due to larger buffers (4MB per file transfer)

## Future Optimizations to Consider

1. **Asynchronous File Enumeration**
   - Use tokio for async directory scanning
   - Start transfers while still enumerating files
   - Show progress immediately instead of waiting

2. **Windows-Specific Optimizations**
   - Use Windows CopyFileEx API for local copies
   - Enable TCP_NODELAY for network sockets
   - Adjust TCP window sizes for high-latency networks

3. **Parallel Pipeline**
   - Separate threads for reading, processing, and writing
   - Use channels for inter-thread communication
   - Overlap I/O with compression/checksumming

4. **SMB/CIFS Optimizations**
   - Increase SMB2/3 credit window
   - Enable SMB multichannel if available
   - Use larger SMB read/write sizes

## Testing Recommendations

1. Test with various file sizes (small, medium, large)
2. Test on different network types (local, LAN, WAN)
3. Monitor CPU and memory usage during transfers
4. Compare performance with native Windows tools (robocopy, xcopy)
5. Test with antivirus enabled/disabled (can significantly impact performance)