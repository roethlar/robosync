# Linux to Windows Coordination - Status Update

## Current State (as of 2025-07-31)

### Project Status
- **Version**: 1.0.0 published to crates.io ✅
- **GitHub Release**: Created at https://github.com/roethlar/robosync/releases/tag/v1.0.0
- **Linux Build**: Working perfectly, binary uploaded to release
- **Windows Build**: Needs testing and binary creation

### Known Windows-Specific Considerations

#### 1. Path Handling
- Windows uses backslashes `\` vs Unix forward slashes `/`
- UNC paths like `\\server\share` need special handling
- Drive letters (C:, D:) are Windows-specific

#### 2. Symlinks on Windows
- Requires admin privileges or Developer Mode
- Different APIs for file vs directory symlinks
- Code already has Windows-specific handling in `src/sync.rs:409-440`

#### 3. File Permissions
- Windows ACLs are different from Unix permissions
- Copy flags might need adjustment (see `src/metadata.rs`)
- ROBOCOPY compatibility is important

#### 4. Build Requirements
- Visual Studio Build Tools or full Visual Studio
- Windows SDK
- Rust with MSVC toolchain

### Key Files for Windows Testing

1. **Platform-specific code**:
   - `src/metadata.rs` - Windows file attributes
   - `src/platform_impl/` - Platform implementations
   - `src/sync.rs:409-440` - Windows symlink handling
   - `src/strategy.rs:330-333` - UNC path detection

2. **Test Commands**:
   ```cmd
   cargo test
   cargo build --release
   cargo run -- C:\test\source C:\test\dest /E /V
   
   REM Test with ROBOCOPY-style flags
   cargo run -- source dest /MIR /Z /V
   ```

### Potential Windows Issues

1. **Long Path Support** - Windows has 260 char path limit by default
2. **File Locking** - Windows locks open files more aggressively
3. **Case Sensitivity** - Windows is case-insensitive by default
4. **Hidden/System Files** - Special attributes need handling
5. **Performance** - Different optimal thread counts for NTFS

### Windows-Specific Features Already Implemented
- ROBOCOPY-compatible command line parsing
- Windows path handling
- Retry logic for locked files
- Windows-specific symlink creation

### Package Manager: Scoop
The Scoop manifest (`robosync.json`) is ready but needs:
1. Windows binary built and uploaded to release
2. SHA256 hash calculated and added to manifest

### Build Instructions for Windows
```powershell
# Install Rust if needed
winget install Rustlang.Rust

# Build
cargo build --release

# Binary will be at: target\release\robosync.exe

# Create release archive
Compress-Archive -Path target\release\robosync.exe -DestinationPath robosync-x86_64-pc-windows-msvc.zip
```

### Coordination Protocol
Please update findings in `coordination/windows-to-linux-status.md` including:
- Build success/failures
- Test results on NTFS, FAT32, network drives
- Performance comparisons with ROBOCOPY
- Any Windows-specific bugs found

### Testing Checklist
- [ ] Build completes without errors
- [ ] Basic file copy works
- [ ] Mirror mode (--mir) works
- [ ] Symlinks work (with admin rights)
- [ ] Network paths work (\\server\share)
- [ ] Long paths work
- [ ] Retry logic works for locked files
- [ ] Progress bars display correctly
- [ ] --no-progress flag works

Good luck with Windows testing! 🪟