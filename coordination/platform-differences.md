# Platform Differences Cheat Sheet

## File System Differences

| Feature | Linux | macOS | Windows |
|---------|-------|-------|---------|
| Path Separator | `/` | `/` | `\` |
| Path Length Limit | ~4096 | 1024 | 260 (unless LongPath enabled) |
| Case Sensitive | Yes | Optional | No |
| Symlinks | Yes | Yes | Admin/DevMode required |
| Hard Links | Yes | Yes | NTFS only |
| Extended Attrs | xattr | xattr | Alternate Data Streams |
| Permissions | rwx (octal) | rwx + ACLs | ACLs only |

## API Differences

### File Copy APIs
```rust
// Linux
copy_file_range() // Fast kernel copy
sendfile()        // Zero-copy transfer

// macOS  
copyfile()        // Preserves all metadata
clonefile()       // CoW on APFS

// Windows
CopyFileEx()      // With progress callback
```

### Symlink Creation
```rust
// Unix (Linux/macOS)
std::os::unix::fs::symlink(target, link_path)

// Windows - must distinguish file vs dir
std::os::windows::fs::symlink_file(target, link_path)
std::os::windows::fs::symlink_dir(target, link_path)
```

## Path Formats

### Linux/macOS
```
/home/user/file.txt
/mnt/network/share
../relative/path
```

### Windows
```
C:\Users\User\file.txt
\\server\share\file.txt
..\relative\path
\\?\C:\very\long\path  // Extended length
```

## Environment Differences

### Build Tools
- **Linux**: gcc/clang, make
- **macOS**: Xcode Command Line Tools
- **Windows**: MSVC/MinGW, Visual Studio

### Package Managers
- **Linux**: apt/yum/pacman, AUR, Snap, AppImage
- **macOS**: Homebrew, MacPorts
- **Windows**: Scoop, Chocolatey, winget

## Testing Considerations

### Linux
- Test on ext4, btrfs, xfs
- Test with SELinux enabled
- Check different distros

### macOS
- Test on APFS and HFS+
- Check SIP restrictions
- Test on Intel and Apple Silicon

### Windows
- Test on NTFS, FAT32, exFAT
- Check with antivirus active
- Test UNC paths and mapped drives

## Common Pitfalls

### All Platforms
- Unicode in filenames
- Very long paths
- Special characters in paths
- File locking behavior

### Platform-Specific
- **Linux**: Different `libc` versions
- **macOS**: Gatekeeper, notarization
- **Windows**: UAC, Windows Defender

## Performance Characteristics

### Optimal Thread Counts
- **Linux**: CPU count (up to 32)
- **macOS**: CPU count (max 64 due to system limits)
- **Windows**: CPU count * 2 (max 256)

### File System Performance
- **ext4**: Good all-around
- **APFS**: Fast metadata, CoW support
- **NTFS**: Good for large files, slower metadata

## Debugging Tools

### Linux
```bash
strace ./robosync ...
lsof -p $(pgrep robosync)
perf record ./robosync ...
```

### macOS
```bash
dtruss ./robosync ...
lsof -p $(pgrep robosync)
instruments -t "Time Profiler" ./robosync ...
```

### Windows
```powershell
procmon /backingfile robosync.pml
handle -p robosync
wpr -start CPU -start FileIO
```