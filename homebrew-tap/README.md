# Homebrew Tap for RoboSync

This tap allows you to install RoboSync using Homebrew.

## Installation

```bash
brew tap roethlar/robosync
brew install robosync
```

## What is RoboSync?

RoboSync is a high-performance file synchronization tool that combines the best features of RoboCopy and rsync with modern Rust performance.

- 🚀 **Lightning Fast**: Parallel processing and intelligent strategy selection
- 🔄 **Delta Transfer**: Only sync changed parts of large files (>100MB)
- 🖥️ **Cross-Platform**: Windows, macOS, Linux
- 📦 **Smart Compression**: Automatic compression for network transfers
- 🛡️ **Reliable**: Automatic retries and comprehensive error reporting

## Usage

```bash
# Basic sync
robosync /source/path /destination/path

# Mirror directories (remove extra files in destination)
robosync /source/path /destination/path --mir

# Show what would be done without actually doing it
robosync /source/path /destination/path -n

# Compress during transfer
robosync /source/path /destination/path -z
```

## Links

- [GitHub Repository](https://github.com/roethlar/robosync)
- [Crates.io Package](https://crates.io/crates/robosync)
- [Documentation](https://docs.rs/robosync)