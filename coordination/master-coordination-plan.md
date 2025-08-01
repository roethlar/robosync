# RoboSync Cross-Platform Master Coordination Plan

## Phase 1: Platform Verification (Immediate)

### All Platforms - Basic Build Test
Please run these commands and report results in your status files:

```bash
# Clean build
cargo clean
cargo build --release
cargo test

# Version check
./target/release/robosync --version  # Unix
# or
.\target\release\robosync.exe --version  # Windows
```

**Report Format**:
```markdown
## Build Results
- Build Success: Yes/No
- Warnings: [count and types]
- Test Results: X/Y passed
- Binary Size: XX MB
- Build Time: XX seconds
```

## Phase 2: Feature Testing Matrix

Each platform should test these features and report pass/fail:

### 2.1 Basic Operations
```bash
# Create test structure
mkdir -p test_src/{small,medium,large}
echo "test" > test_src/small/1kb.txt
dd if=/dev/urandom of=test_src/medium/5mb.bin bs=1M count=5
dd if=/dev/urandom of=test_src/large/150mb.bin bs=1M count=150

# Test 1: Basic copy
robosync test_src test_dst -e -v
# Verify: All files copied correctly

# Test 2: Mirror mode
echo "new file" > test_src/newfile.txt
robosync test_src test_dst --mir -v
# Verify: newfile.txt added, nothing deleted

# Test 3: Compression
robosync test_src test_dst2 -e -z -v
# Verify: Transfer completes, files match

# Test 4: No-progress flag
robosync test_src test_dst3 -e --np
# Verify: NO progress output shown
```

### 2.2 Platform-Specific Tests

**Linux/Mac - Symlinks**:
```bash
ln -s /tmp/target /tmp/test_src/symlink
robosync test_src test_dst -e -l
ls -la test_dst/symlink  # Should be symlink
```

**Windows - Admin Features**:
```powershell
# Run as admin
New-Item -ItemType SymbolicLink -Path "test_src\symlink" -Target "C:\Windows\Temp"
.\robosync.exe test_src test_dst /E /L
```

**All - Network Paths** (if available):
```bash
# Mount a network share, then:
robosync test_src /mnt/network/test -e -z -v
```

## Phase 3: Performance Benchmarking

### Standardized Performance Test
Create this exact test data on all platforms:

```bash
#!/bin/bash
# create-test-data.sh
mkdir -p perf_test/{small,medium,large}

# 10,000 small files (1KB each)
for i in {1..10000}; do
    echo "File $i content" > perf_test/small/file_$i.txt
done

# 100 medium files (10MB each) 
for i in {1..100}; do
    dd if=/dev/urandom of=perf_test/medium/file_$i.bin bs=1M count=10 2>/dev/null
done

# 5 large files (200MB each)
for i in {1..5}; do
    dd if=/dev/urandom of=perf_test/large/file_$i.bin bs=1M count=200 2>/dev/null
done

echo "Total size: $(du -sh perf_test)"
```

### Benchmark Commands
Run each 3 times and report average:

```bash
# Test 1: Small files performance
time robosync perf_test/small perf_dst1 -e

# Test 2: Large files performance  
time robosync perf_test/large perf_dst2 -e

# Test 3: Mixed workload
time robosync perf_test perf_dst3 -e

# Test 4: Compression overhead
time robosync perf_test perf_dst4 -e -z

# Test 5: Different thread counts
for threads in 1 4 8 16 32; do
    echo "Testing with $threads threads"
    time robosync perf_test perf_dst_t$threads -e --mt $threads
done
```

**Report Format**:
```markdown
## Performance Results
Platform: [Linux/macOS/Windows]
CPU: [model and cores]
Storage: [SSD/HDD type]
RAM: [amount]

| Test | Files | Size | Time | Throughput |
|------|-------|------|------|------------|
| Small files | 10000 | 10MB | Xs | XX MB/s |
| Large files | 5 | 1GB | Xs | XX MB/s |
| Mixed | 10105 | 1.01GB | Xs | XX MB/s |
| Compressed | 10105 | 1.01GB | Xs | XX MB/s |

Thread Scaling:
- 1 thread: XX MB/s
- 4 threads: XX MB/s
- 8 threads: XX MB/s
- 16 threads: XX MB/s
- 32 threads: XX MB/s
```

## Phase 4: Platform Comparison Test

### Native Tool Comparison
Each platform should compare with native tools:

**Linux**:
```bash
time rsync -av perf_test/ rsync_dst/
time robosync perf_test robosync_dst -e -v
```

**Windows**:
```powershell
Measure-Command { robocopy perf_test robocopy_dst /E /MT:16 }
Measure-Command { .\robosync.exe perf_test robosync_dst /E }
```

**macOS**:
```bash
time cp -R perf_test/ cp_dst/
time robosync perf_test robosync_dst -e
```

## Phase 5: Binary Preparation

### Build Release Binaries
Each platform:

```bash
# Build optimized binary
cargo build --release --locked

# Strip debug symbols (Unix)
strip target/release/robosync

# Create archive with version and target
# Linux example:
tar czf robosync-1.0.0-x86_64-unknown-linux-gnu.tar.gz -C target/release robosync
sha256sum robosync-1.0.0-*.tar.gz > checksums.txt
```

### Upload to GitHub Release
```bash
gh release upload v1.0.0 robosync-1.0.0-[target].tar.gz
```

## Phase 6: Package Manager Preparation

### Update Package Files
Once all binaries are uploaded, update SHA256 hashes:

1. **Homebrew** (`homebrew-formula.rb`):
   - Update URLs for macOS binaries
   - Add SHA256 for each architecture

2. **Scoop** (`robosync.json`):
   - Update Windows binary URL
   - Add SHA256 hash

3. **AUR** - Already uses source build ✓

4. **Snap/Nix** - Can proceed with source builds

## Communication Protocol

### Status Updates
Create/update your status files every 30 minutes:
- `[platform]-to-linux-status.md`
- `[platform]-to-[other]-status.md`

### Issue Reporting
If you find a bug:
1. Create `[platform]-bug-[issue].md`
2. Include:
   - Exact error message
   - Steps to reproduce
   - Suggested fix (if any)

### Success Criteria
- [ ] All platforms build successfully
- [ ] All basic tests pass
- [ ] Performance is comparable across platforms
- [ ] No platform-specific bugs remain
- [ ] All binaries uploaded to GitHub
- [ ] Package managers have correct hashes

## Timeline
- Phase 1-2: Next 1 hour
- Phase 3-4: Following 2 hours  
- Phase 5-6: Once all tests pass

Let's begin! Please start with Phase 1 and report your build results.

---
**Linux Lead**: I'll compile results and create a summary report once all platforms report in.