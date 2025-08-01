#!/bin/bash
# Quick test for demonstration

echo "Running quick Linux platform tests..."

# Create small test data
mkdir -p quick_test/small quick_test/large
for i in {1..100}; do echo "test $i" > quick_test/small/file_$i.txt; done
dd if=/dev/urandom of=quick_test/large/100mb.bin bs=1M count=100 2>/dev/null

# Test basic operations
echo -e "\n=== Basic Copy Test ==="
time ./target/release/robosync quick_test test_out -e -v

echo -e "\n=== No-Progress Test ==="
output=$(./target/release/robosync quick_test test_out2 -e --np 2>&1)
if [ -z "$output" ]; then
    echo "✅ No-progress test PASSED (no output)"
else
    echo "❌ No-progress test FAILED (produced output)"
fi

echo -e "\n=== Compression Test ==="
time ./target/release/robosync quick_test test_out3 -e -z

# Quick performance test
echo -e "\n=== Performance Summary ==="
size_mb=$(du -sm quick_test | cut -f1)
echo "Test data size: ${size_mb}MB"

# Compare with native tools
if command -v rsync &> /dev/null; then
    echo -e "\nrsync comparison:"
    time rsync -a quick_test/ rsync_out/
fi

# Cleanup
rm -rf quick_test test_out* rsync_out