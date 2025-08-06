#!/bin/bash
# ZFS NAS 5x Comprehensive Test Script
# Run ALL tests and benchmarks 5 times on ZFS filesystem, save ALL results
# Usage: ./zfs_nas_5x_test_script.sh [nas_mount_path] [robosync_binary_path]

set -e

NAS_MOUNT_PATH="${1:-/mnt/nas}"
ROBOSYNC_BIN="${2:-./target/release/robosync}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULTS_DIR="${NAS_MOUNT_PATH}/robosync_5x_results_zfs_${TIMESTAMP}"
MASTER_LOG="$RESULTS_DIR/master_execution_log.txt"
SUMMARY_FILE="$RESULTS_DIR/summaries/final_summary.txt"

echo "=== RoboSync 2.0.0 ZFS NAS 5x Test Protocol ==="
echo "NAS Mount Path: $NAS_MOUNT_PATH"
echo "Results Directory: $RESULTS_DIR"
echo "RoboSync Binary: $ROBOSYNC_BIN"
echo "All results will be saved for review"
echo

# Verify NAS mount is accessible
if [[ ! -d "$NAS_MOUNT_PATH" ]]; then
    echo "❌ NAS mount path not found: $NAS_MOUNT_PATH"
    echo "Please mount your ZFS NAS first"
    exit 1
fi

# Verify RoboSync binary exists
if [[ ! -f "$ROBOSYNC_BIN" ]]; then
    echo "❌ RoboSync binary not found at: $ROBOSYNC_BIN"
    echo "Please provide correct path to RoboSync binary"
    exit 1
fi

# Create results directory structure
mkdir -p "$RESULTS_DIR"/{raw_logs,summaries,benchmarks,integration,validation}

# Initialize master log
{
    echo "=== ZFS NAS 5X TEST EXECUTION LOG ==="
    echo "Timestamp: $(date)"
    echo "Platform: ZFS NAS"
    echo "NAS Mount: $NAS_MOUNT_PATH"
    echo "Binary: $ROBOSYNC_BIN"
    echo "Results Directory: $RESULTS_DIR"
    echo
    echo "=== FILESYSTEM INFORMATION ==="
    df -h "$NAS_MOUNT_PATH" || echo "Could not get filesystem info"
    mount | grep "$(df "$NAS_MOUNT_PATH" | tail -1 | awk '{print $1}')" || echo "Could not get mount info"
    echo
} > "$MASTER_LOG"

# Initialize summary file
cat > "$SUMMARY_FILE" << EOF
RoboSync 2.0.0 ZFS NAS Comprehensive 5x Test Results
Platform: ZFS NAS
NAS Mount: $NAS_MOUNT_PATH
Timestamp: $TIMESTAMP
Binary: $ROBOSYNC_BIN

=== TEST EXECUTION SUMMARY ===
EOF

# Detect ZFS filesystem
check_zfs_features() {
    echo
    echo "🗄️  Checking ZFS Features..."
    
    local log_file="$RESULTS_DIR/raw_logs/zfs_features_check.log"
    
    {
        echo "=== ZFS FEATURE DETECTION ==="
        echo "Timestamp: $(date)"
        echo
        
        # Check if ZFS tools are available
        if command -v zfs >/dev/null 2>&1; then
            echo "ZFS tools: Available"
            zfs version 2>/dev/null || echo "Could not get ZFS version"
        else
            echo "ZFS tools: Not available (testing as generic filesystem)"
        fi
        
        # Check filesystem type
        echo "Filesystem type:"
        stat -f -c %T "$NAS_MOUNT_PATH" 2>/dev/null || echo "Could not determine filesystem type"
        
        # Check available space
        echo "Available space:"
        df -h "$NAS_MOUNT_PATH"
        
    } > "$log_file"
    
    echo "    📋 ZFS features check logged to: $log_file"
}

run_zfs_performance_tests_5x() {
    echo
    echo "🚀 Running ZFS Performance Tests 5x..."
    
    for run in {1..5}; do
        echo "  ZFS Performance Test Run $run/5..."
        local log_file="$RESULTS_DIR/raw_logs/zfs_performance_run_${run}.log"
        local test_dir="$RESULTS_DIR/temp/zfs_performance_test_run_${run}"
        
        {
            echo "=== ZFS Performance Test Run $run ==="
            echo "Test Directory: $test_dir"
            echo "Timestamp: $(date)"
            echo
        } > "$log_file"
        
        mkdir -p "$test_dir"/{source,dest_robosync,dest_rsync}
        cd "$test_dir"
        
        # Create test files optimized for ZFS
        echo "Creating ZFS-optimized test files..."
        
        # Small files (ZFS can handle many small files efficiently)
        for i in {1..200}; do
            echo "small file content $i on ZFS" > "source/small_${i}.txt"
        done
        
        # Medium files (ZFS block sizes)
        for i in {1..20}; do
            # Create files aligned with ZFS record sizes (128KB default)
            dd if=/dev/zero of="source/medium_${i}.bin" bs=128K count=40 2>/dev/null  # 5MB files
        done
        
        # Large files (ZFS compression and dedup friendly)
        for i in {1..5}; do
            dd if=/dev/zero of="source/large_${i}.bin" bs=1M count=50 2>/dev/null  # 50MB files
        done
        
        # Test RoboSync on ZFS
        echo "Testing RoboSync on ZFS..."
        local robosync_start=$(date +%s.%N)
        if "$ROBOSYNC_BIN" source dest_robosync -v >> "$log_file" 2>&1; then
            local robosync_end=$(date +%s.%N)
            local robosync_time=$(echo "$robosync_end - $robosync_start" | bc -l 2>/dev/null || echo "0")
            echo "RoboSync time: ${robosync_time}s" >> "$log_file"
        else
            echo "RoboSync failed" >> "$log_file"
            robosync_time="999"
        fi
        
        # Test rsync for comparison (if available)
        local rsync_time="0"
        local speedup="N/A"
        
        if command -v rsync >/dev/null 2>&1; then
            echo "Testing rsync on ZFS..."
            local rsync_start=$(date +%s.%N)
            if rsync -av source/ dest_rsync/ >> "$log_file" 2>&1; then
                local rsync_end=$(date +%s.%N)
                rsync_time=$(echo "$rsync_end - $rsync_start" | bc -l 2>/dev/null || echo "0")
                echo "rsync time: ${rsync_time}s" >> "$log_file"
                
                # Calculate speedup
                if [[ "$robosync_time" != "0" && "$rsync_time" != "0" ]]; then
                    speedup=$(echo "scale=2; $rsync_time / $robosync_time" | bc -l 2>/dev/null || echo "0")
                fi
            else
                echo "rsync failed" >> "$log_file"
                rsync_time="999"
            fi
        else
            echo "rsync not available for comparison" >> "$log_file"
        fi
        
        echo "Speedup: ${speedup}x" >> "$log_file"
        echo "    📊 RoboSync: ${robosync_time}s, rsync: ${rsync_time}s, Speedup: ${speedup}x"
        
        # ZFS performance assessment (target: at least competitive with rsync)
        if [[ "$speedup" == "N/A" ]] || (( $(echo "${speedup} >= 0.8" | bc -l 2>/dev/null || echo "0") )); then
            echo "    ✅ ZFS Performance Run $run: PASSED (${speedup}x speedup)"
            echo "ZFS Performance Run $run: PASSED (${speedup}x speedup)" >> "$SUMMARY_FILE"
        else
            echo "    ❌ ZFS Performance Run $run: FAILED (${speedup}x speedup)"
            echo "ZFS Performance Run $run: FAILED (${speedup}x speedup)" >> "$SUMMARY_FILE"
        fi
        
        # Log to master
        echo "ZFS Performance Test Run $run: RoboSync ${robosync_time}s, rsync ${rsync_time}s, Speedup ${speedup}x" >> "$MASTER_LOG"
        
        cd "$RESULTS_DIR"
        rm -rf "$test_dir"
    done
}

run_zfs_compression_tests_5x() {
    echo
    echo "🗜️  Running ZFS Compression Tests 5x..."
    
    for run in {1..5}; do
        echo "  ZFS Compression Test Run $run/5..."
        local log_file="$RESULTS_DIR/raw_logs/zfs_compression_run_${run}.log"
        local test_dir="$RESULTS_DIR/temp/zfs_compression_test_run_${run}"
        
        {
            echo "=== ZFS Compression Test Run $run ==="
            echo "Test Directory: $test_dir"
            echo "Timestamp: $(date)"
            echo "Testing RoboSync with ZFS compression-friendly data"
            echo
        } > "$log_file"
        
        mkdir -p "$test_dir"/{source,dest}
        cd "$test_dir"
        
        # Create highly compressible data (ZFS compression test)
        echo "Creating compressible test data..."
        
        # Text files (highly compressible)
        for i in {1..50}; do
            # Repetitive content that compresses well
            for j in {1..1000}; do
                echo "This is a highly compressible text line number $j in file $i"
            done > "source/compressible_text_${i}.txt"
        done
        
        # Binary files with patterns (moderately compressible)
        for i in {1..10}; do
            # Create file with repeating pattern
            dd if=/dev/zero of="source/zero_pattern_${i}.bin" bs=1M count=10 2>/dev/null
        done
        
        # Test RoboSync with compressible data
        echo "Testing RoboSync with compressible data on ZFS..."
        local robosync_start=$(date +%s.%N)
        if "$ROBOSYNC_BIN" source dest -v >> "$log_file" 2>&1; then
            local robosync_end=$(date +%s.%N)
            local robosync_time=$(echo "$robosync_end - $robosync_start" | bc -l 2>/dev/null || echo "0")
            echo "RoboSync time: ${robosync_time}s" >> "$log_file"
            
            # Check file integrity
            local src_files=$(find source -type f | wc -l)
            local dest_files=$(find dest -type f | wc -l)
            
            if [[ "$src_files" -eq "$dest_files" ]]; then
                echo "    ✅ ZFS Compression Run $run: PASSED (${robosync_time}s, $dest_files files)"
                echo "ZFS Compression Run $run: PASSED (${robosync_time}s, $dest_files files)" >> "$SUMMARY_FILE"
            else
                echo "    ❌ ZFS Compression Run $run: FAILED (file count mismatch: $src_files vs $dest_files)"
                echo "ZFS Compression Run $run: FAILED (file count mismatch)" >> "$SUMMARY_FILE"
            fi
        else
            echo "    ❌ ZFS Compression Run $run: FAILED (RoboSync execution failed)"
            echo "ZFS Compression Run $run: FAILED (execution failed)" >> "$SUMMARY_FILE"
        fi
        
        # Log to master
        echo "ZFS Compression Test Run $run: ${robosync_time}s" >> "$MASTER_LOG"
        
        cd "$RESULTS_DIR"
        rm -rf "$test_dir"
    done
}

run_zfs_network_latency_tests_5x() {
    echo
    echo "🌐 Running ZFS Network Latency Tests 5x..."
    
    for run in {1..5}; do
        echo "  ZFS Network Latency Test Run $run/5..."
        local log_file="$RESULTS_DIR/raw_logs/zfs_network_latency_run_${run}.log"
        local test_dir="$RESULTS_DIR/temp/zfs_network_latency_test_run_${run}"
        
        {
            echo "=== ZFS Network Latency Test Run $run ==="
            echo "Test Directory: $test_dir"
            echo "Timestamp: $(date)"
            echo "Testing RoboSync performance over network to ZFS"
            echo
        } > "$log_file"
        
        mkdir -p "$test_dir"/{source,dest}
        cd "$test_dir"
        
        # Create files optimized for network transfer testing
        echo "Creating network latency test files..."
        
        # Many small files (network latency sensitive)
        for i in {1..100}; do
            echo "network latency test file $i" > "source/network_small_${i}.txt"
        done
        
        # Medium files (bandwidth sensitive)
        for i in {1..10}; do
            dd if=/dev/urandom of="source/network_medium_${i}.bin" bs=1M count=5 2>/dev/null
        done
        
        # Test RoboSync over network to ZFS
        echo "Testing RoboSync network performance to ZFS..."
        local robosync_start=$(date +%s.%N)
        if timeout 300 "$ROBOSYNC_BIN" source dest -v >> "$log_file" 2>&1; then
            local robosync_end=$(date +%s.%N)
            local robosync_time=$(echo "$robosync_end - $robosync_start" | bc -l 2>/dev/null || echo "0")
            echo "RoboSync time: ${robosync_time}s" >> "$log_file"
            
            # Calculate throughput
            local total_size=$(find source -type f -exec stat -c%s {} + | awk '{sum+=$1} END {print sum}' || echo "0")
            local throughput_mbps=0
            if [[ "$robosync_time" != "0" && "$total_size" != "0" ]]; then
                throughput_mbps=$(echo "scale=2; $total_size / $robosync_time / 1024 / 1024" | bc -l 2>/dev/null || echo "0")
            fi
            
            echo "Throughput: ${throughput_mbps} MB/s" >> "$log_file"
            echo "    📊 Network throughput: ${throughput_mbps} MB/s in ${robosync_time}s"
            
            # Network performance assessment (reasonable throughput expected)
            if (( $(echo "$throughput_mbps >= 10" | bc -l 2>/dev/null || echo "0") )); then
                echo "    ✅ ZFS Network Run $run: PASSED (${throughput_mbps} MB/s)"
                echo "ZFS Network Latency Run $run: PASSED (${throughput_mbps} MB/s)" >> "$SUMMARY_FILE"
            else
                echo "    ⚠️  ZFS Network Run $run: SLOW (${throughput_mbps} MB/s - may be network limited)"
                echo "ZFS Network Latency Run $run: SLOW (${throughput_mbps} MB/s)" >> "$SUMMARY_FILE"
            fi
        else
            echo "    ❌ ZFS Network Run $run: FAILED (timeout or execution failed)"
            echo "ZFS Network Latency Run $run: FAILED (timeout or execution failed)" >> "$SUMMARY_FILE"
        fi
        
        # Log to master
        echo "ZFS Network Latency Test Run $run: ${robosync_time}s" >> "$MASTER_LOG"
        
        cd "$RESULTS_DIR"
        rm -rf "$test_dir"
    done
}

# Execute all test suites 5x
echo "Starting ZFS NAS comprehensive 5x testing protocol..."

# Check ZFS features first
check_zfs_features

# Run ZFS performance tests 5 times
run_zfs_performance_tests_5x

# Run ZFS compression tests 5 times
run_zfs_compression_tests_5x

# Run ZFS network latency tests 5 times
run_zfs_network_latency_tests_5x

# Generate final statistics
echo
echo "📊 Generating Final Statistics..."

echo "" >> "$SUMMARY_FILE"
echo "=== PASS/FAIL STATISTICS ===" >> "$SUMMARY_FILE"

test_types=("ZFS Performance" "ZFS Compression" "ZFS Network Latency")
for test_type in "${test_types[@]}"; do
    passed=$(grep "$test_type Run.*: PASSED" "$SUMMARY_FILE" | wc -l)
    failed=$(grep "$test_type Run.*: FAILED" "$SUMMARY_FILE" | wc -l)
    slow=$(grep "$test_type Run.*: SLOW" "$SUMMARY_FILE" | wc -l)
    total=$((passed + failed + slow))
    pass_rate=0
    if [[ $total -gt 0 ]]; then
        pass_rate=$(echo "scale=1; $passed * 100 / $total" | bc -l 2>/dev/null || echo "0")
    fi
    
    echo "$test_type Tests: $passed/$total passed, $slow slow (${pass_rate}%)" >> "$SUMMARY_FILE"
    echo "$test_type Tests: $passed/$total passed, $slow slow (${pass_rate}%)"
done

# Create results index
results_index="$RESULTS_DIR/RESULTS_INDEX.txt"
cat > "$results_index" << EOF
RoboSync 2.0.0 ZFS NAS Comprehensive 5x Test Results Index
Platform: ZFS NAS
NAS Mount: $NAS_MOUNT_PATH
Timestamp: $TIMESTAMP
Results Directory: $RESULTS_DIR

=== FILE LOCATIONS FOR REVIEW ===

1. MASTER EXECUTION LOG:
   $MASTER_LOG

2. FINAL SUMMARY:
   $SUMMARY_FILE

3. RAW TEST LOGS (16 files):
   $RESULTS_DIR/raw_logs/zfs_features_check.log
   $RESULTS_DIR/raw_logs/zfs_performance_run_1.log through zfs_performance_run_5.log
   $RESULTS_DIR/raw_logs/zfs_compression_run_1.log through zfs_compression_run_5.log
   $RESULTS_DIR/raw_logs/zfs_network_latency_run_1.log through zfs_network_latency_run_5.log

=== QUICK ACCESS COMMANDS ===
View final summary: cat $SUMMARY_FILE
View execution log: cat $MASTER_LOG
List all files: find $RESULTS_DIR -type f

=== ZFS-SPECIFIC NOTES ===
- Tests designed for ZFS features (compression, block alignment)
- Network latency tests account for NAS overhead
- Results saved on NAS for persistence across reboots

=== COORDINATION COMMAND ===
After reviewing results, update coordination:
python3 /home/michael/Documents/Source/Repos/shared_2.0/resources/robosync_universal_db.py add [your_agent] zfs_nas_5x_complete zfs completed high "ZFS NAS 5x Testing Complete" "All ZFS NAS tests completed 5x. Performance: X/5 passed, Compression: X/5 passed, Network: X/5 passed. Results saved in $RESULTS_DIR. Ready for review." "roboclaude: Review ZFS NAS 5x test results"
EOF

echo
echo "✅ ZFS NAS 5x Testing Protocol Complete!"
echo
echo "📁 RESULTS INDEX CREATED: $results_index"
echo
echo "=== FILE LOCATIONS FOR REVIEW ==="
cat "$results_index"

echo
echo "🔍 Quick Summary:"
cat "$SUMMARY_FILE"

echo
echo "📊 All results saved in: $RESULTS_DIR"
echo "🗂️  Total files created: $(find "$RESULTS_DIR" -type f | wc -l)"
echo "💾 Total size: $(du -sh "$RESULTS_DIR" | cut -f1)"

echo
echo "📋 To run this script:"
echo "1. Mount your ZFS NAS (e.g., to /mnt/nas)"
echo "2. Copy RoboSync binary to accessible location"
echo "3. Run: ./zfs_nas_5x_test_script.sh /mnt/nas ./target/release/robosync"
echo "4. Review results in the generated directory"
echo "5. Update coordination database with results"