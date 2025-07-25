# RoboSync

Fast, parallel file synchronization with delta-transfer algorithm.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=flat&logo=rust&logoColor=white)](https://www.rust-lang.org/)

RoboSync combines the best features of RoboCopy and rsync into a modern, high-performance file synchronization tool written in Rust.

## Features

- **Delta-Transfer Algorithm**: Only transfers changed portions of files, minimizing bandwidth usage
- **Parallel Processing**: Utilizes multiple CPU cores for blazing-fast synchronization
- **RoboCopy-Compatible Interface**: Familiar command-line options for Windows users
- **Cross-Platform**: Works on Windows, macOS, and Linux
- **Advanced Compression**: Supports Zstandard and LZ4 compression
- **Retry Logic**: Automatically retries failed operations with configurable parameters
- **File Filtering**: Exclude/include files and directories with pattern matching
- **Metadata Preservation**: Maintains timestamps, permissions, and ownership
- **Progress Tracking**: Real-time progress bars and ETA calculations
- **Dry Run Mode**: Preview changes before executing

## Installation

### From Source

```bash
git clone https://github.com/yourusername/robosync.git
cd robosync
cargo build --release
```

The binary will be available at `target/release/robosync`

### From Releases

Download pre-built binaries from the [releases page](https://github.com/yourusername/robosync/releases).

## Usage

### Basic Synchronization

```bash
# Copy all files from source to destination
robosync /path/to/source /path/to/destination

# Mirror directories (includes deletion of extra files)
robosync /source /dest --mir

# Dry run to preview changes
robosync /source /dest --dry-run
```

### Advanced Options

```bash
# Use 8 threads for parallel processing
robosync /source /dest --mt 8

# Enable compression for network transfers
robosync /source /dest --compress

# Retry failed operations 3 times with 5-second delays
robosync /source /dest --r 3 --w 5

# Exclude certain file patterns
robosync /source /dest --xf "*.tmp" --xf "*.log"

# Move files (delete from source after successful copy)
robosync /source /dest --mov

# Verbose output with file-by-file progress
robosync /source /dest --verbose
```

## Command-Line Options

### Core Options
- `--s` - Copy subdirectories, but not empty ones
- `--e` - Copy subdirectories, including empty ones
- `--mir` - Mirror a directory tree (equivalent to /E plus /PURGE)
- `--purge` - Delete dest files/dirs that no longer exist in source
- `--mov` - Move files (delete source after successful copy)

### File Selection
- `--xf <PATTERN>` - Exclude files matching given patterns
- `--xd <PATTERN>` - Exclude directories matching given patterns
- `--min <SIZE>` - Minimum file size to copy
- `--max <SIZE>` - Maximum file size to copy

### Copy Options
- `--copy <FLAGS>` - What to copy (D=Data, A=Attributes, T=Timestamps, S=Security, O=Owner)
- `--copyall` - Copy all file info (equivalent to /COPY:DATSOU)

### Performance
- `--mt <NUM>` - Number of threads (default: CPU cores)
- `--compress` - Enable compression
- `--block-size <SIZE>` - Block size for delta algorithm

### Retry Options
- `--r <NUM>` - Number of retries on failed copies
- `--w <SECONDS>` - Wait time between retries

### Output Options
- `--verbose` - Detailed output
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