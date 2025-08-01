# RoboSync Cross-Platform Coordination Center

## Overview
This folder facilitates communication between Claude instances running on different platforms to debug and fix platform-specific issues without breaking working code.

## Active Instances
- **Linux** (Primary/Original) - Working ✅
- **macOS** - Has issues, being debugged
- **Windows** - Needs testing and binary building

## Communication Protocol

### 1. Status Files
Each platform maintains status files:
- `linux-to-{platform}-status.md` - Outgoing updates from Linux
- `{platform}-to-linux-status.md` - Incoming updates to Linux
- Similar patterns for Mac↔Windows communication

### 2. File Naming Convention
```
{source}-to-{destination}-{type}.md
```
Types: status, bugfix, test-results, code-changes

### 3. Update Process
1. Read the latest status files from other platforms
2. Make changes and test on your platform
3. Update your outgoing status file
4. Include:
   - What you changed
   - Test results
   - Any new issues found
   - Code snippets if needed

## Quick Start for New Instance

### macOS Claude:
```bash
# Read these first
cat coordination/context-archive.md
cat coordination/linux-to-mac-status.md
cat coordination/mac-troubleshooting-guide.md

# After testing, create:
coordination/mac-to-linux-status.md
coordination/mac-to-windows-status.md
```

### Windows Claude:
```powershell
# Read these first
type coordination\context-archive.md
type coordination\linux-to-windows-status.md
type coordination\windows-troubleshooting-guide.md

# After testing, create:
coordination\windows-to-linux-status.md
coordination\windows-to-mac-status.md
```

## Platform Status Summary

| Platform | Build | Tests | Binary | Issues |
|----------|-------|-------|--------|---------|
| Linux    | ✅    | ✅    | ✅     | None    |
| macOS    | ?     | ?     | ❌     | Reported issues (TBD) |
| Windows  | ?     | ?     | ❌     | Not tested yet |

## Critical: Do Not Modify
These files are working correctly on Linux:
- `src/progress.rs` - Progress system (carefully debugged)
- `src/formatted_display.rs` - Display formatting
- `src/mixed_strategy.rs` - Core strategy implementation
- Package files (PKGBUILD, homebrew formulas, etc.)

## Code Changes Protocol
If you need to modify shared code:
1. Create a patch file: `coordination/mac-changes.patch`
2. Explain why the change is needed
3. Show test results on your platform
4. Wait for other platforms to test

## Current Priorities
1. **Mac**: Identify and fix the reported issues
2. **Windows**: Build and test, create binary for release
3. **All**: Fix GitHub Actions CI for automated builds

## Binary Upload Process
Once you have a working binary:
```bash
# Create archive
tar czf robosync-{target}.tar.gz robosync  # Unix
# or
Compress-Archive robosync.exe robosync-{target}.zip  # Windows

# Calculate SHA256
sha256sum robosync-{target}.tar.gz  # Unix
# or
(Get-FileHash robosync-{target}.zip).Hash  # Windows

# Upload to release
gh release upload v1.0.0 robosync-{target}.{ext}

# Update package manager configs with SHA256
```

## Success Criteria
- [ ] All platforms build without errors
- [ ] All tests pass on all platforms
- [ ] Binaries uploaded for all platforms
- [ ] GitHub Actions CI working
- [ ] Package managers have correct SHA256 hashes

---
Remember: We're all working on the same codebase. Communicate clearly and test thoroughly! 🚀