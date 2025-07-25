# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

RoboSync is a high-performance file synchronization tool written in Rust that combines features from RoboCopy and rsync. It provides delta-transfer capabilities, parallel processing, and cross-platform support.

## Development Commands

### Building
```bash
# Development build
cargo build

# Release build (optimized)
cargo build --release

# Build for all platforms
./build-releases.sh
```

### Testing
```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test module
cargo test <module_name>
```

### Code Quality
```bash
# Format code
cargo fmt

# Run linter
cargo clippy -- -D warnings

# Check for compile errors without building
cargo check
```

## Architecture

The codebase is organized into modular components in the `src/` directory:

- **main.rs**: Entry point and CLI argument parsing using clap
- **lib.rs**: Core library exports and public API
- **algorithm.rs**: Delta-transfer algorithm implementation
- **checksum.rs**: File hashing (BLAKE3, xxHash, MD5)
- **compression.rs**: Zstandard and LZ4 compression support
- **file_list.rs**: File enumeration and filtering logic
- **logging.rs**: Logging infrastructure
- **metadata.rs**: File metadata handling (timestamps, permissions)
- **options.rs**: Command-line option structures
- **parallel_sync.rs**: Multi-threaded synchronization engine
- **progress.rs**: Progress tracking and reporting (indicatif)
- **retry.rs**: Retry logic for failed operations
- **sync.rs**: Core synchronization logic

## Key Dependencies

- **tokio**: Async runtime for I/O operations
- **rayon**: Data parallelism for CPU-bound tasks
- **blake3**: Fast cryptographic hashing
- **zstd/lz4_flex**: Compression algorithms
- **indicatif**: Progress bars and reporting
- **walkdir**: Recursive directory traversal
- **clap**: Command-line argument parsing

## Platform-Specific Code

- Unix-specific code uses `libc` crate
- Windows-specific code uses `winapi` crate
- Platform differences are handled with conditional compilation (`cfg` attributes)

## Testing Strategy

The project includes test directories:
- `test-sync/`: Basic synchronization tests
- `test-copy-flags/`: File copy flag tests
- `test-copy-flags-dir/`: Directory copy tests

Integration tests should cover:
- Basic file synchronization
- Delta transfer algorithm
- Compression/decompression
- Retry logic
- Cross-platform compatibility

## Command-Line Parameters

The CLI follows standard conventions:
- Single-letter flags use single dash: `-s`, `-e`, `-r`, `-w`, `-z`, `-n`, `-v`
- Multi-letter options use double dash: `--mir`, `--purge`, `--xf`, `--xd`
- RoboCopy compatibility maintained where possible
- Case-insensitive aliases removed for clarity
- Conflicts resolved (e.g., `-r` for retry instead of recursive)