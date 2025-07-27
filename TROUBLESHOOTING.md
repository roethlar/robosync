# RoboSync Troubleshooting Guide

## Slow Startup on Network Shares

### Problem
RoboSync shows "1/xxx files" and appears stuck when syncing to network shares (SMB/CIFS).

### Causes
1. **Initial SMB Connection**: Windows takes 10-30 seconds to establish the first SMB connection
2. **Authentication**: Network authentication and negotiation can add delays
3. **Antivirus Scanning**: Some antivirus software scans all network operations
4. **SMB Protocol Version**: Older SMB versions (1.0/2.0) are slower than SMB 3.0+

### Solutions

1. **Pre-establish Connection**:
   ```cmd
   # Run this before RoboSync to warm up the connection
   dir "\\server\share" > nul
   ```

2. **Use Mapped Drive Instead of UNC**:
   ```cmd
   # Map network drive first
   net use Z: \\server\share
   robosync source Z:\destination
   ```

3. **Disable SMB 1.0** (if not needed):
   ```powershell
   # Check SMB versions
   Get-SmbServerConfiguration | Select EnableSMB1Protocol, EnableSMB2Protocol
   
   # Disable SMB 1.0
   Set-SmbServerConfiguration -EnableSMB1Protocol $false
   ```

4. **Increase SMB Client Cache**:
   ```powershell
   # Increase directory cache timeout
   Set-SmbClientConfiguration -DirectoryCacheLifetime 120
   Set-SmbClientConfiguration -FileInfoCacheLifetime 120
   ```

5. **Check Antivirus Exclusions**:
   - Add RoboSync executable to antivirus exclusions
   - Consider excluding the destination path temporarily during large syncs

6. **Use Verbose Mode** to see what's happening:
   ```cmd
   robosync source dest -v
   ```

## Performance Optimization Tips

1. **For Large Files over Network**:
   - Use fewer threads: `--mt 8` (reduces connection overhead)
   - Increase block size: `-b 4096` or higher

2. **For Many Small Files**:
   - Use more threads: `--mt 64` (better parallelization)
   - Keep default block size

3. **Monitor Network Usage**:
   - Open Task Manager > Performance > Ethernet
   - Check if you're hitting network limits
   - 100MB/s = ~800Mbps (reasonable for gigabit)
   - 1000MB/s = ~8Gbps (requires 10GbE)

4. **Windows Specific Optimizations**:
   ```powershell
   # Increase TCP window size for high-latency networks
   netsh int tcp set global autotuninglevel=normal
   
   # Enable TCP timestamps
   netsh int tcp set global timestamps=enabled
   ```