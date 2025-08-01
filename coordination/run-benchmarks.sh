#!/bin/bash
# Automated benchmark script for Unix platforms (Linux/macOS)

ROBOSYNC="./target/release/robosync"
RESULTS_FILE="coordination/$(uname -s)-benchmark-results.md"

# Check if robosync binary exists
if [ ! -f "$ROBOSYNC" ]; then
    echo "Error: RoboSync binary not found at $ROBOSYNC"
    echo "Please run: cargo build --release"
    exit 1
fi

# Check if test data exists
if [ ! -d "perf_test" ]; then
    echo "Test data not found. Creating..."
    bash coordination/create-test-data.sh
fi

echo "Starting RoboSync benchmarks on $(uname -s)..."
echo "Results will be saved to: $RESULTS_FILE"

# Start results file
cat > "$RESULTS_FILE" << EOF
# Benchmark Results - $(uname -s)

**Date**: $(date)
**Platform**: $(uname -s) $(uname -m)
**CPU**: $(case "$(uname -s)" in
    Linux) lscpu | grep "Model name" | cut -d: -f2 | xargs ;;
    Darwin) sysctl -n machdep.cpu.brand_string ;;
esac)
**RAM**: $(case "$(uname -s)" in
    Linux) free -h | grep Mem | awk '{print $2}' ;;
    Darwin) echo $(($(sysctl -n hw.memsize) / 1024 / 1024 / 1024))GB ;;
esac)
**Storage**: $(df -h . | tail -1 | awk '{print $1}')

## Feature Tests

EOF

# Function to run timed test
run_test() {
    local test_name="$1"
    local cmd="$2"
    echo "Running: $test_name"
    
    # Clean destination
    rm -rf perf_dst* robosync_dst rsync_dst cp_dst test_dst*
    
    # Run 3 times and calculate average
    local total_time=0
    local times=()
    
    for i in 1 2 3; do
        local start=$(date +%s.%N)
        eval "$cmd" > /dev/null 2>&1
        local end=$(date +%s.%N)
        local duration=$(echo "$end - $start" | bc)
        times+=($duration)
        total_time=$(echo "$total_time + $duration" | bc)
    done
    
    local avg_time=$(echo "scale=2; $total_time / 3" | bc)
    echo "Average time: ${avg_time}s"
    
    # Calculate size and throughput
    local size_mb=$(du -sm $3 2>/dev/null | cut -f1)
    local throughput=$(echo "scale=2; $size_mb / $avg_time" | bc)
    
    echo "| $test_name | ${size_mb}MB | ${avg_time}s | ${throughput} MB/s |" >> "$RESULTS_FILE"
}

# Basic feature tests
echo "### Basic Operations" >> "$RESULTS_FILE"
echo "| Test | Result |" >> "$RESULTS_FILE"
echo "|------|--------|" >> "$RESULTS_FILE"

# Test basic copy
$ROBOSYNC test_src test_dst -e -v > /dev/null 2>&1
if [ -f "test_dst/large/150mb.bin" ]; then
    echo "| Basic copy | ✅ Pass |" >> "$RESULTS_FILE"
else
    echo "| Basic copy | ❌ Fail |" >> "$RESULTS_FILE"
fi

# Test no-progress
output=$($ROBOSYNC test_src test_dst_np -e --np 2>&1)
if [ -z "$output" ]; then
    echo "| No-progress flag | ✅ Pass |" >> "$RESULTS_FILE"
else
    echo "| No-progress flag | ❌ Fail |" >> "$RESULTS_FILE"
fi

# Test compression
$ROBOSYNC test_src test_dst_z -e -z > /dev/null 2>&1
if [ -f "test_dst_z/large/150mb.bin" ]; then
    echo "| Compression | ✅ Pass |" >> "$RESULTS_FILE"
else
    echo "| Compression | ❌ Fail |" >> "$RESULTS_FILE"
fi

# Performance benchmarks
echo -e "\n## Performance Benchmarks\n" >> "$RESULTS_FILE"
echo "| Test | Size | Time | Throughput |" >> "$RESULTS_FILE"
echo "|------|------|------|------------|" >> "$RESULTS_FILE"

# Run performance tests
run_test "Small files (10k)" "$ROBOSYNC perf_test/small perf_dst1 -e" "perf_test/small"
run_test "Large files (5)" "$ROBOSYNC perf_test/large perf_dst2 -e" "perf_test/large"
run_test "Mixed workload" "$ROBOSYNC perf_test perf_dst3 -e" "perf_test"
run_test "With compression" "$ROBOSYNC perf_test perf_dst4 -e -z" "perf_test"

# Thread scaling tests
echo -e "\n## Thread Scaling\n" >> "$RESULTS_FILE"
echo "| Threads | Time | Throughput |" >> "$RESULTS_FILE"
echo "|---------|------|------------|" >> "$RESULTS_FILE"

for threads in 1 4 8 16 32; do
    run_test "$threads threads" "$ROBOSYNC perf_test perf_dst_t$threads -e --mt $threads" "perf_test"
done

# Native tool comparison
echo -e "\n## Native Tool Comparison\n" >> "$RESULTS_FILE"
echo "| Tool | Time | Throughput |" >> "$RESULTS_FILE"
echo "|------|------|------------|" >> "$RESULTS_FILE"

# Compare with native tools
if command -v rsync &> /dev/null; then
    run_test "rsync" "rsync -a perf_test/ rsync_dst/" "perf_test"
fi

if [[ "$(uname -s)" == "Darwin" ]]; then
    run_test "cp -R" "cp -R perf_test cp_dst" "perf_test"
fi

run_test "robosync" "$ROBOSYNC perf_test robosync_dst -e" "perf_test"

echo -e "\nBenchmarks complete! Results saved to: $RESULTS_FILE"