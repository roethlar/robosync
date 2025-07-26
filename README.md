# RoboSync

Fast, parallel file synchronization with delta-transfer algorithm.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=flat&logo=rust&logoColor=white)](https://www.rust-lang.org/)

RoboSync combines the best features of RoboCopy and rsync into a modern, high-performance file synchronization tool written in Rust.

## Features

- **Delta-Transfer Algorithm**: Only transfers changed portions of files, minimizing bandwidth usage
- **Parallel Processing**: Utilizes multiple CPU cores for blazing-fast synchronization (8.76 GB/s on local SSD)
- **RoboCopy-Compatible Interface**: Familiar command-line options for Windows users
- **Cross-Platform**: Works on Windows, macOS, and Linux
- **Advanced Compression**: Supports Zstandard and LZ4 compression
- **Retry Logic**: Automatically retries failed operations with configurable parameters
- **File Filtering**: Exclude/include files and directories with pattern matching
- **Metadata Preservation**: Maintains timestamps, permissions, and ownership
- **Progress Tracking**: Real-time progress bars and ETA calculations
- **Dry Run Mode**: Preview changes before executing
- **Archive Mode**: Full metadata preservation with `-a` flag (equivalent to --copy:DATSOU)
- **Symlink Support**: Preserves symbolic links across platforms
- **Interactive Confirmation**: `--confirm` flag for reviewing operations before execution
- **Multi-level Verbosity**: `-v` for summary, `-vv` for detailed file-by-file output
- **Checksum Verification**: `-c` flag for content-based comparison using BLAKE3

## Installation

### From Source

```bash
git clone https://github.com/roethlar/robosync.git
cd robosync
cargo build --release
```

The binary will be available at `target/release/robosync`

### From Releases

Download pre-built binaries from the [releases page](https://github.com/roethlar/robosync/releases).

## Usage

### Basic Synchronization

```bash
# Copy all files from source to destination
robosync /path/to/source /path/to/destination

# Mirror directories (includes deletion of extra files)
robosync /source /dest --mir

# Dry run to preview changes
robosync /source /dest --dry-run  # or -n
```

### Advanced Options

```bash
# Use 8 threads for parallel processing
robosync /source /dest --mt 8

# Enable compression for network transfers
robosync /source /dest --compress  # or -z

# Retry failed operations 3 times with 5-second delays
robosync /source /dest -r 3 -w 5

# Exclude certain file patterns
robosync /source /dest --xf "*.tmp" --xf "*.log"

# Move files (delete from source after successful copy)
robosync /source /dest --mov

# Verbose output with operation summary
robosync /source /dest -v

# Very verbose output with file-by-file details
robosync /source /dest -vv

# Archive mode (preserves all metadata)
robosync /source /dest -a

# Checksum-based comparison
robosync /source /dest -c

# Interactive confirmation before sync
robosync /source /dest --confirm
```

## Command-Line Options

### Core Options
- `-s` - Copy subdirectories, but not empty ones
- `-e` - Copy subdirectories, including empty ones
- `-a` - Archive mode (equivalent to -e plus --copy:DATSOU)
- `--mir` - Mirror a directory tree (equivalent to -e plus --purge)
- `--purge` - Delete dest files/dirs that no longer exist in source
- `--mov` - Move files (delete source after successful copy) ⚠️ Use with caution
- `-c` - Use checksums for file comparison instead of timestamps
- `--confirm` - Ask for confirmation before starting sync

### File Selection
- `--xf <PATTERN>` - Exclude files matching given patterns
- `--xd <PATTERN>` - Exclude directories matching given patterns
- `--min <SIZE>` - Minimum file size to copy
- `--max <SIZE>` - Maximum file size to copy

### Copy Options
- `--copy <FLAGS>` - What to copy (D=Data, A=Attributes, T=Timestamps, S=Security, O=Owner)
- `--copyall` - Copy all file info (equivalent to /COPY:DATSOU)

### Performance
- `--mt <NUM>` - Number of threads (default: CPU cores, max varies by OS)
- `-z` or `--compress` - Enable compression
- `--block-size <SIZE>` - Block size for delta algorithm

### Retry Options
- `-r <NUM>` or `--retry <NUM>` - Number of retries on failed copies
- `-w <SECONDS>` or `--wait <SECONDS>` - Wait time between retries

### Output Options
- `-v` - Verbose output (show operation summary)
- `-vv` - Very verbose output (show file-by-file details)
- `--np` - No progress bar
- `--log <FILE>` - Log output to file
- `--eta` - Show estimated time of arrival

## Performance

RoboSync achieves high performance through:

- **Parallel I/O**: Multiple threads handle file operations simultaneously
- **Efficient Delta Algorithm**: Only changed blocks are transferred
- **BLAKE3 Hashing**: Fast cryptographic checksums
- **Memory-Mapped Files**: Efficient handling of large files
- **Smart Compression**: Automatic compression of beneficial data

## Building from Source

### Prerequisites

- Rust 1.70 or higher
- Cargo

### Build Commands

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Run benchmarks
cargo bench
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/AmazingFeature`)
3. Commit your changes (`git commit -m 'Add some AmazingFeature'`)
4. Push to the branch (`git push origin feature/AmazingFeature`)
5. Open a Pull Request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Inspired by [RoboCopy](https://docs.microsoft.com/en-us/windows-server/administration/windows-commands/robocopy) and [rsync](https://rsync.samba.org/)
- Built with [Rust](https://www.rust-lang.org/) and amazing crates from the community

## Comparison with Similar Tools

| Feature | RoboSync | rsync | RoboCopy |
|---------|----------|-------|----------|
| Delta transfer | ✅ | ✅ | ❌ |
| Parallel processing | ✅ | ❌ | ✅ |
| Cross-platform | ✅ | ✅ | ❌ |
| Windows-style options | ✅ | ❌ | ✅ |
| Compression | ✅ | ✅ | ❌ |
| Retry logic | ✅ | ❌ | ✅ |
| Modern codebase | ✅ | ❌ | ❌ |