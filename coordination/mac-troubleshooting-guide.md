# Mac Troubleshooting Guide

## Quick Diagnostics

### 1. Build Issues
```bash
# Clean build
cargo clean
cargo build --release

# Verbose build to see errors
cargo build --release -vv

# Check for missing dependencies
cargo tree
```

### 2. Common Mac-Specific Issues

#### File System Permissions
```bash
# Test with sudo if permission errors
sudo cargo run -- /source /dest -e -v

# Check file system type
diskutil info / | grep "File System"
```

#### Extended Attributes
```bash
# List extended attributes
xattr -l testfile

# Test preserving attributes
cargo run -- testfile testfile_copy -copyflags DATS
xattr -l testfile_copy
```

#### Symlinks
```bash
# Create test symlink
ln -s /tmp/target /tmp/link

# Test symlink handling
cargo run -- /tmp/link /tmp/link_copy -l
```

### 3. Debug Output
```bash
# Run with maximum verbosity
RUST_LOG=debug cargo run -- /source /dest -vv

# Run specific tests
cargo test --test symlink_tests -- --nocapture
cargo test --test metadata_tests -- --nocapture
```

### 4. Platform Detection
Add this debug code temporarily to main.rs:
```rust
#[cfg(target_os = "macos")]
println!("Running on macOS");
#[cfg(target_arch = "aarch64")]
println!("Running on ARM64");
#[cfg(target_arch = "x86_64")]
println!("Running on x86_64");
```

### 5. Common Fixes

#### For "Operation not permitted"
- System Integrity Protection (SIP) might be blocking
- Grant Full Disk Access in System Preferences
- Use a test directory in /tmp or ~/Desktop

#### For Extended Attributes
- APFS supports xattrs differently than HFS+
- Some attributes are protected (com.apple.*)
- May need to filter certain attributes

#### For Performance Issues
- Disable Spotlight indexing on test directories
- Check if antivirus is scanning
- Use caffeinate to prevent sleep: `caffeinate -i cargo run ...`

## Please Report Back

In `coordination/mac-to-linux-status.md`, please include:
1. Exact error messages
2. macOS version (`sw_vers`)
3. File system type (`diskutil info /`)
4. Architecture (`uname -m`)
5. Any code changes that fix issues

Good luck! 🚀