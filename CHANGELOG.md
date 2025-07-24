# Changelog

All notable changes to RoboSync will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2024-01-24

### Added
- Initial release of RoboSync
- Delta-transfer algorithm with rolling checksums
- Parallel file synchronization with configurable thread pools
- RoboCopy-compatible command-line interface
- Cross-platform support (Windows, macOS, Linux)
- Compression support (Zstandard and LZ4)
- Retry logic with configurable parameters
- File filtering with include/exclude patterns
- Metadata preservation (timestamps, permissions, ownership)
- Progress tracking with ETA calculations
- Dry run mode for previewing changes
- Move functionality (delete source after successful copy)
- Comprehensive logging with file output option
- Mirror mode for exact directory synchronization

### Features
- Multi-threaded processing for high performance
- Memory-mapped file handling for large files
- BLAKE3 cryptographic checksums
- Smart compression (only compresses when beneficial)
- Case-insensitive command-line parameters
- Real-time progress bars
- Verbose output mode

[0.1.0]: https://github.com/yourusername/robosync/releases/tag/v0.1.0