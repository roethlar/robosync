# Windows Troubleshooting Guide

## Prerequisites

### Required Tools
```powershell
# Check if Rust is installed
rustc --version

# If not, install via winget
winget install Rustlang.Rust

# Or via Chocolatey
choco install rust

# Ensure MSVC toolchain
rustup default stable-msvc
```

### Visual Studio Build Tools
If build fails with "link.exe not found":
1. Download Visual Studio Installer
2. Install "Desktop development with C++"
3. Or just "MSVC build tools"

## Common Windows Issues

### 1. Build Errors

#### Link.exe Not Found
```powershell
# Install Visual Studio Build Tools
# Or set environment manually
$env:PATH += ";C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Tools\MSVC\14.29.30133\bin\Hostx64\x64"
```

#### Permission Errors During Build
```powershell
# Run as Administrator
# Or build in user directory
cd $HOME\Documents
git clone https://github.com/roethlar/robosync
cd robosync
cargo build --release
```

### 2. Runtime Issues

#### Symlink Creation Fails
```powershell
# Option 1: Run as Administrator
Start-Process powershell -Verb runAs

# Option 2: Enable Developer Mode
# Settings > Update & Security > For Developers > Developer Mode

# Test symlinks
New-Item -ItemType SymbolicLink -Path "link" -Target "target"
```

#### Long Path Issues
```powershell
# Enable long path support (requires admin)
New-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" `
  -Name "LongPathsEnabled" -Value 1 -PropertyType DWORD -Force

# Or use \\?\ prefix for paths
cargo run -- "\\?\C:\very\long\path" "\\?\D:\destination"
```

#### Access Denied on System Files
```powershell
# Skip system directories
cargo run -- C:\source C:\dest --xd "System Volume Information" --xd "$RECYCLE.BIN"
```

### 3. Performance Testing

#### Compare with Robocopy
```powershell
# Measure Robocopy
Measure-Command { robocopy C:\source C:\dest /E /MT:16 }

# Measure RoboSync
Measure-Command { .\target\release\robosync.exe C:\source C:\dest -e --mt 16 }
```

#### Test Different File Systems
```powershell
# NTFS
cargo run -- C:\test D:\test -e -v

# FAT32 (USB drive)
cargo run -- C:\test E:\test -e -v

# Network share
cargo run -- C:\test \\server\share\test -e -v -z
```

### 4. Debug Commands

#### Verbose Output
```powershell
# Maximum verbosity
$env:RUST_LOG="debug"
cargo run -- source dest -vv

# Windows-specific debug
cargo run -- source dest /V /L  # List only, verbose
```

#### Test Specific Features
```powershell
# Test compression
cargo run -- source dest -z

# Test retry logic (copy to a file in use)
# Open a file in notepad, then:
cargo run -- file.txt dest\ -r 3 -w 5

# Test mirror mode
cargo run -- source dest --mir --confirm
```

### 5. Windows-Specific Checks

#### File Attributes
```powershell
# Check if attributes are preserved
Get-ItemProperty -Path "source\file.txt" | Select-Object Attributes
cargo run -- source\file.txt dest\
Get-ItemProperty -Path "dest\file.txt" | Select-Object Attributes
```

#### Security Descriptors
```powershell
# Check ACLs
Get-Acl "source\file.txt" | Format-List
cargo run -- source\file.txt dest\ -copyflags DATSOU
Get-Acl "dest\file.txt" | Format-List
```

## Creating Windows Release

```powershell
# Build release
cargo build --release

# Create release archive
$version = "1.0.0"
Compress-Archive -Path target\release\robosync.exe -DestinationPath "robosync-x86_64-pc-windows-msvc.zip"

# Calculate SHA256
(Get-FileHash "robosync-x86_64-pc-windows-msvc.zip" -Algorithm SHA256).Hash

# Update Scoop manifest with hash
notepad robosync.json
```

## Platform-Specific Code Locations

- Windows path handling: Search for `#[cfg(windows)]`
- Symlink logic: `src/sync.rs:409-440`
- Copy flags mapping: `src/metadata.rs`
- Platform detection: `src/strategy.rs`

## Please Report

In `coordination/windows-to-linux-status.md`:
1. Windows version: `winver`
2. Build errors/warnings
3. Test results for each feature
4. Performance vs Robocopy
5. Any code changes needed

Good luck! 🚀