# Changelog

All notable changes to RoboSync will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2025-01-24

### Fixed
- Fixed file permission error when copying Git object files and other read-only files on Windows
- Temporarily removes read-only attribute during timestamp setting, then restores it

### Changed
- Rationalized command-line parameters to follow standard Unix/Linux conventions
- Single-letter options now use single dash: `-s`, `-e`, `-r`, `-w`, `-z`, `-n`, `-v`, `-l`, `-a`, `-b`
- Multi-letter options use double dash: `--mir`, `--purge`, `--xf`, `--xd`, etc.
- Added long-form alternatives for clarity: `--retry` for `-r`, `--wait` for `-w`
- Removed excessive case-insensitive aliases for cleaner interface
- Updated help text to remove Windows-style notation (removed `/S`, `/E`, `/V` references)
- Resolved parameter conflicts (e.g., `-r` for retry instead of recursive)

### Removed
- Removed duplicate `parallel_sync 2.rs` file
- Cleaned up excessive command-line aliases

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

[0.1.1]: https://github.com/yourusername/robosync/releases/tag/v0.1.1
[0.1.0]: https://github.com/yourusername/robosync/releases/tag/v0.1.0