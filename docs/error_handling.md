# Error Handling in RoboSync v0.8.15

## Overview

RoboSync v0.8.15 introduces improved error handling that reduces console noise while maintaining visibility into issues that occur during synchronization.

## Key Features

### 1. Non-Fatal Metadata Warnings

When copying files with metadata preservation (permissions, ownership, timestamps), failures are now treated as warnings rather than errors. This is because:

- The file data is successfully copied even if metadata preservation fails
- Ownership changes often require root/administrator privileges
- Some filesystems don't support all metadata types

### 2. Summary Reporting

Instead of printing every warning, RoboSync now:
- Counts metadata warnings during operation
- Shows a summary at the end if there were warnings
- Suggests using `--copy DAT` if there are many metadata warnings

Example output:
```
     ⚠️  47 metadata warnings (permissions/ownership/timestamps)
        These are non-fatal - files were copied successfully.
        Consider using --copy DAT to skip metadata preservation.
```

### 3. Error Reporting API

For programmatic use, RoboSync provides an error reporting API:

```rust
use robosync::error_report::{ErrorReporter, ErrorReportHandle};

// Create error reporter
let error_reporter = ErrorReporter::new(source_path, dest_path);
let handle = error_reporter.get_handle();

// Add errors/warnings
handle.add_error(path, "Error message");
handle.add_warning(path, "Warning message");

// Write report to file
if let Ok(Some(report_path)) = error_reporter.write_report() {
    println!("Error report saved to: {}", report_path.display());
}
```

Report files are named with timestamp and source/destination info:
```
robosync_errors_20250730_165432_source__to__dest.log
```

## Command Line Usage

### Default Behavior
By default, RoboSync copies data, attributes, and timestamps (DAT flags):
```bash
robosync /source /dest
```

### Full Metadata Copy
To copy all metadata including ownership (requires privileges):
```bash
robosync /source /dest --copyall
# or
robosync /source /dest --copy DATSOU
```

### Data Only
To skip all metadata and just copy file data:
```bash
robosync /source /dest --copy D
```

### Common Copy Flag Combinations
- `D` - Data only
- `DAT` - Data, Attributes, Timestamps (default)
- `DATS` - Data, Attributes, Timestamps, Security (permissions)
- `DATSOU` - All metadata (equivalent to --copyall)

## Best Practices

1. **For most users**: Use the default `DAT` flags
2. **For system backups**: Use `--copyall` with appropriate privileges
3. **For cross-filesystem copies**: Consider using `--copy D` or `--copy DAT`
4. **When seeing many warnings**: Use `--copy DAT` to skip problematic metadata