# Changelog

All notable changes to RoboSync will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.29] - 2025-07-31

### Added
- Comprehensive error logging system with different verbosity levels
- `--no-report-errors` flag to disable automatic error report generation
- Error logger module for centralized error handling
- Visual spinners for all sync stages
- Delta transfer support for very large files (>100MB) with 64KB block size
- Mixed mode as default strategy with concurrent processing

### Changed
- Updated README.md to reflect actual functionality (removed non-existent AI features)
- Fixed format string warnings using clippy --fix
- Improved compression with dynamic buffer sizing
- Better error handling throughout the codebase

### Fixed
- All 27 dead code annotations removed
- All 53 unwrap() calls replaced with proper error handling
- Windows compilation errors
- Duplicate progress bars in mixed mode
- Missing scanning spinner feedback
- --confirm flag now works properly in mixed mode
- Progress bar speed display
- Spinner alignment and checkmark display

### Removed
- Dead code files: multi_progress.rs, scrolling_progress.rs
- Unused AvailableTools struct and related functionality
- ~150 lines of dead code total

### Code Quality
- Reduced panic points from 53 to 0
- Improved error handling throughout
- Build now succeeds with minimal warnings

## [0.6.0] - 2025-07-25

### Added
- Multi-level verbosity system:
  - `-v` shows operation preview before starting, then displays progress bar
  - `-vv` shows detailed file-by-file operations without progress bar
- `--confirm` flag for operation confirmation:
  - Shows pending operation summary (counts only)
  - Prompts user with "Continue? Y/n" before proceeding
  - Works with verbosity levels appropriately
- Live scanning progress during file analysis phase
- Cargo-style sticky progress bars that stay at bottom while output scrolls above
- Dynamic OS-based thread limits with `get_max_thread_count()`:
  - Linux: Based on ulimit file descriptor limits
  - macOS/BSD: 64 threads max (system limitations)
  - Windows: 256 threads default
- Archive mode (`-a`) now properly sets all metadata preservation flags (DATSOU)
- Safety warning for dangerous flag combinations (e.g., `--mov` with `--mir`)

### Changed
- Upgraded indicatif from 0.17 to 0.18.0 for improved MultiProgress support
- Progress bars now work correctly with verbose mode
- Confirmation mode now shows summary counts instead of full file list for better UX with large file counts
- Archive mode now correctly sets copy flags to DATSOU for full metadata preservation
- Improved help text to warn about data loss risks with `--mov` and `--mir` combination

### Fixed
- Progress bars not showing during file analysis with verbose flag
- O(n²) performance bottleneck in confirmation counting causing multi-minute delays with 100k+ files
- O(n*m) bottleneck in byte calculation for large file sets
- Duplicate delete operations causing metadata errors in mirror/purge mode
- Progress bar clearing issues with MultiProgress and verbose output
- Empty `SyncStats::add_bytes_transferred()` method now properly tracks transferred bytes
- Archive mode not setting proper copy flags
- 48 compiler warnings reduced to 21 (remaining are dead code warnings)

### Performance
- Optimized file lookup from O(n) linear search to O(1) HashMap lookup
- Significantly improved performance with large file counts (100k+ files)
- Better memory efficiency during file analysis phase
- Tested performance with network shares: maintains efficiency despite network latency

## [0.5.0] - 2025-01-24

### Added
- Symlink support with proper metadata preservation
- Cross-platform symlink handling for Windows/Unix
- Compress option `-Z` as alias for `-z/--compress`

### Changed
- Major version bump to reflect stability and feature completeness

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

[0.6.0]: https://github.com/yourusername/robosync/releases/tag/v0.6.0
[0.5.0]: https://github.com/yourusername/robosync/releases/tag/v0.5.0
[0.1.1]: https://github.com/yourusername/robosync/releases/tag/v0.1.1
[0.1.0]: https://github.com/yourusername/robosync/releases/tag/v0.1.0