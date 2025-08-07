# RoboSync 🚀

**High-performance file synchronization with intelligent strategy selection**

[![Version](https://img.shields.io/badge/version-2.0.0-green.svg)](https://github.com/yourusername/robosync/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg)](#platform-support)

RoboSync is a cross-platform file synchronization tool built in Rust, designed to automatically select the optimal copying strategy based on your files. It combines parallel processing, delta transfers, and platform-specific optimizations to achieve maximum performance.

## 🎯 Key Features

- **🏎️ Adaptive Performance**: Automatically selects optimal strategy based on file sizes and counts
- **🧠 Smart Categorization**: Different strategies for small, medium, and large files running concurrently
- **🔒 Enterprise Ready**: Comprehensive error handling, retry logic, and detailed logging
- **🌍 Cross-Platform**: Native support for Linux, macOS, and Windows with platform-specific optimizations
- **📊 Real-time Feedback**: Progress bars, transfer speeds, ETAs, and detailed operation logs

## ✨ Features

### ⚡ Performance Optimizations
- **Mixed Strategy Mode** - Concurrent execution of different strategies:
  - Small files (< 256KB): Parallel batch processing for maximum throughput
  - Medium files (256KB - 16MB): Optimized platform-specific APIs
  - Large files (16MB - 100MB): Standard optimized copying
  - Huge files (> 100MB): Delta transfer algorithm with configurable block size
- **Delta Transfer** - Only transfer changed blocks in large files
- **Compression Support** - Automatic algorithm selection (Zstandard/LZ4)
- **Platform-Specific Features**:
  - Linux: io_uring support, splice system calls, FIEMAP for extent-based copying
  - macOS: Optimized for APFS with copy-on-write support
  - Windows: Native Win32 APIs, NTFS alternate data streams support

### 🛡️ Reliability Features
- **Error Management** - Automatic error report generation with detailed failure information
- **Retry Mechanism** - Configurable retry attempts with customizable wait periods
- **Metadata Preservation** - Full support for timestamps, permissions, ownership, and attributes
- **Symlink Handling** - Three modes: preserve, follow, or skip
- **Network Filesystem Support** - Optimized for SMB/CIFS, NFS, SSHFS, and WebDAV
- **Enterprise Mode** - Optional integrity verification and atomic operations

### 🎮 User Experience
- **Familiar Syntax** - RoboCopy-style commands with Unix conventions
- **Flexible Verbosity** - Multiple levels from silent to debug output
- **Safety Features** - Dry run mode, confirmation prompts, list-only mode
- **Progress Tracking** - Real-time progress bars with speed and ETA

## 🚀 Quick Start

### Prerequisites

- **Rust 1.70+** (for building from source or cargo install)
- **Zstandard library** (libzstd) - for compression support

### Installation

#### From Package Managers

```bash
# Arch Linux (AUR) - using yay
yay -S robosync

# Arch Linux (AUR) - using paru
paru -S robosync

# Using Rust's Cargo (requires Rust 1.70+)
cargo install robosync
```

#### Build from Source

```bash
# Clone the repository
git clone https://github.com/yourusername/robosync.git
cd robosync

# Build optimized binary
cargo build --release

# Install to system (Unix/Linux/macOS)
sudo cp target/release/robosync /usr/local/bin/

# Or add to PATH (Windows)
# Add target\release to your PATH environment variable
```

### Your First Sync

```bash
# Simple copy
robosync /source /destination -e

# Mirror with confirmation
robosync /source /dest --mir --confirm

# Sync to network mount with compression
robosync /local/path /mnt/network/path -e -z

# See what would happen
robosync /source /dest -e -n
```

## 📚 Common Use Cases

### 🗄️ Backup Your Home Directory
```bash
robosync ~/ /backup/home/ -e \
  --xd ".cache" --xd ".local/share/Trash" \
  --xf "*.tmp" --xf ".DS_Store" \
  -v --log backup.log
```

### 🔄 Keep Servers in Sync
```bash
robosync /var/www/ /mnt/server/var/www/ --mir \
  --compress \
  --retry 3 --wait 5 \
  --mt 16
```

### 📸 Backup Large Media Files
```bash
robosync /camera/DCIM/ /backup/photos/ -e \
  --min 1048576 \  # Skip files < 1MB
  --copyall \      # Preserve all metadata
  --progress        # Show transfer progress
```

### 🎮 Sync Game Saves
```bash
robosync "C:\Users\Gamer\SavedGames" "D:\Backup\Saves" \
  --mir \
  --xf "*.log" \
  --confirm
```

## 🎛️ Command Reference

### Essential Options

| Option | Description |
|--------|-------------|
| `-e` | Copy all subdirectories (even empty ones) |
| `-n` | Dry run - preview without changes |
| `-v` | Verbose - show what's happening |
| `-z` | Compress during transfer |
| `--mir` | Mirror mode - make dest exactly like source |

### File Selection

| Option | Description | Example |
|--------|-------------|---------|
| `--xf` | Exclude files | `--xf "*.tmp" --xf "*.cache"` |
| `--xd` | Exclude directories | `--xd "node_modules" --xd ".git"` |
| `--min` | Minimum file size | `--min 1024` (skip < 1KB) |
| `--max` | Maximum file size | `--max 1073741824` (skip > 1GB) |

### Performance Tuning

| Option | Description | Default |
|--------|-------------|---------|
| `--mt` | Thread count | CPU cores |
| `-b` | Delta algorithm block size | 1024 bytes |
| `--strategy` | Force specific strategy | auto-select |

### File Size Categories

| Category | Default Range | Strategy | Configurable |
|----------|--------------|----------|-------------|
| Small | < 256KB | Parallel batch processing | `--small-file-threshold` |
| Medium | 256KB - 16MB | Optimized transfer | `--medium-file-threshold` |
| Large | 16MB - 100MB | Standard copy | `--large-file-threshold` |
| Huge | > 100MB | Delta transfer algorithm | `--large-file-threshold` |

### Safety & Control

| Option | Description |
|--------|-------------|
| `--confirm` | Ask before starting |
| `--no-report-errors` | Don't create error report |
| `-r` | Retry count on failures |
| `-w` | Wait seconds between retries |

## 📊 Performance Benchmarks

Performance comparison vs native tools (rsync/robocopy):

| Platform | Scenario | RoboSync | Native Tool | Improvement |
|----------|----------|----------|-------------|-------------|
| Linux | Small files (5K × 1KB) | 0.008s | 0.051s (rsync) | 6.4× faster |
| Linux | Medium files (100 × 512KB) | 0.013s | 0.064s (rsync) | 4.9× faster |
| macOS | Large files (50 × 30MB) | 1s | 4s (rsync) | 4× faster |
| Windows | Sparse files (100MB) | 3146 MB/s | 3105 MB/s (robocopy) | 1.01× faster |

*Results vary based on hardware, filesystem, and network conditions*

## 🔧 Advanced Usage

### Force Specific Strategies

```bash
# Force parallel processing for many small files
robosync /source /dest --strategy parallel

# Force delta transfer for large files with changes
robosync /source /dest --strategy delta

# Use automatic mixed strategy (default)
robosync /source /dest --strategy mixed
```

### Platform-Specific Optimizations

```bash
# Linux: Enable all platform optimizations
robosync /source /dest --linux-optimized

# Control reflink/COW behavior
robosync /source /dest --reflink always  # Force copy-on-write
robosync /source /dest --reflink never   # Disable COW

# Enterprise mode with integrity checks
robosync /source /dest --enterprise --verify
```

### Error Handling

RoboSync's error handling adapts to your verbosity level:

```bash
# Default: Errors only in report file
robosync /source /dest -e

# -v: Errors on console + report file
robosync /source /dest -e -v

# -vv: Everything on console + report
robosync /source /dest -e -vv

# Disable error reports
robosync /source /dest -e --no-report-errors
```

## 📝 Important Notes

### Network Filesystem Support
RoboSync automatically detects and optimizes for network filesystems:
- **NFS**: Large buffers (1MB) for maximum throughput
- **SMB/CIFS**: Moderate buffers (512KB) to respect protocol limits
- **SSHFS**: Small buffers (64KB) due to SSH encryption overhead
- **WebDAV**: Minimal buffers (32KB) for HTTP efficiency

Mount your network drives first, then use RoboSync for optimized transfers.

## 🏗️ Architecture

RoboSync uses a sophisticated strategy selection system:

```
┌─────────────────┐
│  File Analysis  │ ← Scan source and destination
└────────┬────────┘
         │
┌────────▼────────┐
│ Strategy Select │ ← Choose based on file sizes
└────────┬────────┘
         │
┌────────▼────────┐
│  Mixed Executor │ ← Run multiple strategies concurrently
└────────┬────────┘
         │
    ┌────┴────┬─────────┬──────────┐
    ▼         ▼         ▼          ▼
[Parallel] [Platform] [Delta]  [Native]
```

## 🌍 Platform Support

### Linux 🐧
- **io_uring** support for asynchronous I/O (when available)
- **FIEMAP** for extent-based copying on ext4/XFS
- **splice** system calls for zero-copy transfers
- Adaptive thread limits based on file descriptor limits

### macOS 🍎
- **APFS** optimizations with copy-on-write support
- **clonefile** support for instant copies when possible
- Extended attribute preservation
- Conservative threading (64 threads max) for stability

### Windows 🪟
- **Win32 API** integration for optimal performance
- **NTFS** alternate data streams support
- **ReFS** reflink support for instant copies
- Higher thread limits (256) for parallel operations

## 🤝 Contributing

We love contributions! Whether it's:
- 🐛 Bug reports
- 💡 Feature requests
- 📖 Documentation improvements
- 🚀 Performance optimizations
- 🧪 Test coverage

Check out our [Contributing Guide](CONTRIBUTING.md) to get started.

## 📜 License

MIT License - see [LICENSE](LICENSE) for details.

## 🙏 Acknowledgments

Standing on the shoulders of giants:
- **rsync** - The grandfather of smart syncing
- **RoboCopy** - Windows' robust file copier
- **Rust Community** - For amazing crates like tokio, rayon, and blake3
- **You** - For choosing RoboSync!

---

**Ready to sync at the speed of light?** 🚀

See the [Installation](#installation) section above to get started!