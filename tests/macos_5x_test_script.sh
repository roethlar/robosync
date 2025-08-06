#!/bin/bash
# macOS 5x Comprehensive Test Script for mac_claude
# Run ALL tests and benchmarks 5 times, save ALL results
# Usage: ./macos_5x_test_script.sh

set -e

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULTS_DIR="/tmp/robosync_5x_results_macos_${TIMESTAMP}"
ROBOSYNC_BIN="${ROBOSYNC_BIN:-./target/release/robosync}"
MASTER_LOG="$RESULTS_DIR/master_execution_log.txt"
SUMMARY_FILE="$RESULTS_DIR/summaries/final_summary.txt"

echo "=== RoboSync 2.0.0 macOS 5x Test Protocol ==="
echo "Results Directory: $RESULTS_DIR"
echo "RoboSync Binary: $ROBOSYNC_BIN"
echo "All results will be saved for review"
echo

# Create results directory structure
mkdir -p "$RESULTS_DIR"/{raw_logs,summaries,benchmarks,integration,validation}

# Initialize master log
{
    echo "=== MACOS 5X TEST EXECUTION LOG ==="
    echo "Timestamp: $(date)"
    echo "Platform: macOS"
    echo "Binary: $ROBOSYNC_BIN"
    echo "Results Directory: $RESULTS_DIR"
    echo
} > "$MASTER_LOG"

# Initialize summary file
cat > "$SUMMARY_FILE" << EOF
RoboSync 2.0.0 macOS Comprehensive 5x Test Results
Platform: macOS
Timestamp: $TIMESTAMP
Binary: $ROBOSYNC_BIN

=== TEST EXECUTION SUMMARY ===
EOF

# Check if RoboSync binary exists
if [[ ! -f "$ROBOSYNC_BIN" ]]; then
    echo "❌ RoboSync binary not found at: $ROBOSYNC_BIN"
    echo "Build with: cargo build --release"
    exit 1
fi

run_apfs_reflink_tests_5x() {
    echo
    echo "🍎 Running APFS Reflink Tests 5x..."
    
    for run in {1..5}; do
        echo "  APFS Reflink Test Run $run/5..."
        local log_file="$RESULTS_DIR/raw_logs/apfs_reflink_run_${run}.log"
        local test_dir="/tmp/apfs_reflink_test_run_${run}"
        
        {
            echo "=== APFS Reflink Test Run $run ==="
            echo "Test Directory: $test_dir"
            echo "Timestamp: $(date)"
            echo
        } > "$log_file"
        
        mkdir -p "$test_dir"
        cd "$test_dir"
        
        # Create test file (10MB)
        echo "Creating 10MB test file..."
        dd if=/dev/zero of=source_file.bin bs=1M count=10 2>/dev/null
        
        # Test RoboSync with reflink capability
        echo "Testing RoboSync reflink..."
        local robosync_start=$(date +%s.%N)
        if "$ROBOSYNC_BIN" source_file.bin dest_file.bin -v >> "$log_file" 2>&1; then
            local robosync_end=$(date +%s.%N)
            local robosync_time=$(echo "$robosync_end - $robosync_start" | bc -l 2>/dev/null || echo "0")
            echo "RoboSync time: ${robosync_time}s" >> "$log_file"
            
            # Check if reflink was used (very fast copy indicates reflink)
            if (( $(echo "$robosync_time < 0.1" | bc -l 2>/dev/null || echo "0") )); then
                echo "    ✅ APFS Reflink Run $run: PASSED (${robosync_time}s - reflink detected)"
                echo "APFS Reflink Run $run: PASSED (${robosync_time}s - reflink detected)" >> "$SUMMARY_FILE"
            else
                echo "    ⚠️  APFS Reflink Run $run: SLOW (${robosync_time}s - may not be using reflink)"
                echo "APFS Reflink Run $run: SLOW (${robosync_time}s - may not be using reflink)" >> "$SUMMARY_FILE"
            fi
            
            # Check stats reporting
            if grep -q "Reflinks succeeded.*[1-9]" "$log_file"; then
                echo "    ✅ Reflink stats reporting: CORRECT"
                echo "    Reflink stats Run $run: CORRECT" >> "$SUMMARY_FILE"
            else
                echo "    ❌ Reflink stats reporting: INCORRECT"
                echo "    Reflink stats Run $run: INCORRECT" >> "$SUMMARY_FILE"
            fi
        else
            echo "    ❌ APFS Reflink Run $run: FAILED"
            echo "APFS Reflink Run $run: FAILED" >> "$SUMMARY_FILE"
        fi
        
        # Log to master
        echo "APFS Reflink Test Run $run completed: ${robosync_time}s" >> "$MASTER_LOG"
        
        cd "$RESULTS_DIR"
        rm -rf "$test_dir"
    done
}

run_medium_file_regression_tests_5x() {
    echo
    echo "🎯 Running Medium Files Regression Tests 5x (CRITICAL)..."
    
    for run in {1..5}; do
        echo "  Medium Files Regression Test Run $run/5..."
        local log_file="$RESULTS_DIR/raw_logs/medium_files_regression_run_${run}.log"
        local test_dir="/tmp/medium_files_regression_test_run_${run}"
        
        {
            echo "=== Medium Files Regression Test Run $run ==="
            echo "Test Directory: $test_dir"
            echo "Timestamp: $(date)"
            echo "CRITICAL: Testing 1-16MB files that were 6x slower than rsync"
            echo
        } > "$log_file"
        
        mkdir -p "$test_dir"/{source,dest_robosync,dest_rsync}
        cd "$test_dir"
        
        # Create medium files (1-16MB range that was problematic)
        echo "Creating medium files (1-16MB range)..."
        for i in {1..20}; do
            size_mb=$((1 + (i % 15)))  # 1-16MB range
            dd if=/dev/zero of="source/medium_${i}_${size_mb}MB.bin" bs=1M count=$size_mb 2>/dev/null
        done
        
        # Test RoboSync
        echo "Testing RoboSync on medium files..."
        local robosync_start=$(date +%s.%N)
        if "$ROBOSYNC_BIN" source dest_robosync -v >> "$log_file" 2>&1; then
            local robosync_end=$(date +%s.%N)
            local robosync_time=$(echo "$robosync_end - $robosync_start" | bc -l 2>/dev/null || echo "0")
            echo "RoboSync time: ${robosync_time}s" >> "$log_file"
        else
            echo "RoboSync failed" >> "$log_file"
            robosync_time="999"
        fi
        
        # Test rsync for comparison
        echo "Testing rsync on medium files..."
        local rsync_start=$(date +%s.%N)
        if rsync -av source/ dest_rsync/ >> "$log_file" 2>&1; then
            local rsync_end=$(date +%s.%N)
            local rsync_time=$(echo "$rsync_end - $rsync_start" | bc -l 2>/dev/null || echo "0")
            echo "rsync time: ${rsync_time}s" >> "$log_file"
        else
            echo "rsync failed" >> "$log_file"
            rsync_time="999"
        fi
        
        # Calculate speedup
        local speedup="0"
        if [[ "$robosync_time" != "0" && "$rsync_time" != "0" ]]; then
            speedup=$(echo "scale=2; $rsync_time / $robosync_time" | bc -l 2>/dev/null || echo "0")
        fi
        
        echo "Speedup: ${speedup}x" >> "$log_file"
        echo "    📊 RoboSync: ${robosync_time}s, rsync: ${rsync_time}s, Speedup: ${speedup}x"
        
        # CRITICAL: Must not be slower than rsync (speedup >= 1.0)
        if (( $(echo "$speedup >= 1.0" | bc -l 2>/dev/null || echo "0") )); then
            echo "    ✅ Medium Files Run $run: PASSED (${speedup}x speedup)"
            echo "Medium Files Regression Run $run: PASSED (${speedup}x speedup)" >> "$SUMMARY_FILE"
        else
            echo "    ❌ Medium Files Run $run: FAILED (${speedup}x speedup - REGRESSION)"
            echo "Medium Files Regression Run $run: FAILED (${speedup}x speedup - REGRESSION)" >> "$SUMMARY_FILE"
        fi
        
        # Log to master
        echo "Medium Files Regression Test Run $run: RoboSync ${robosync_time}s, rsync ${rsync_time}s, Speedup ${speedup}x" >> "$MASTER_LOG"
        
        cd "$RESULTS_DIR"
        rm -rf "$test_dir"
    done
}

run_mixed_workload_regression_tests_5x() {
    echo
    echo "🎯 Running Mixed Workload Regression Tests 5x (CRITICAL)..."
    
    for run in {1..5}; do
        echo "  Mixed Workload Regression Test Run $run/5..."
        local log_file="$RESULTS_DIR/raw_logs/mixed_workload_regression_run_${run}.log"
        local test_dir="/tmp/mixed_workload_regression_test_run_${run}"
        
        {
            echo "=== Mixed Workload Regression Test Run $run ==="
            echo "Test Directory: $test_dir"
            echo "Timestamp: $(date)"
            echo "CRITICAL: Testing mixed workload that was 7.5x slower than rsync"
            echo
        } > "$log_file"
        
        mkdir -p "$test_dir"/{source,dest_robosync,dest_rsync}/mixed_workload
        cd "$test_dir"
        
        # Create mixed workload (small + medium + large files)
        echo "Creating mixed workload..."
        
        # Small files
        for i in {1..500}; do
            echo "small file content $i" > "source/mixed_workload/small_${i}.txt"
        done
        
        # Medium files  
        for i in {1..50}; do
            dd if=/dev/zero of="source/mixed_workload/medium_${i}.bin" bs=1M count=5 2>/dev/null
        done
        
        # Large files
        for i in {1..10}; do
            dd if=/dev/zero of="source/mixed_workload/large_${i}.bin" bs=1M count=20 2>/dev/null
        done
        
        # Test RoboSync
        echo "Testing RoboSync on mixed workload..."
        local robosync_start=$(date +%s.%N)
        if "$ROBOSYNC_BIN" source dest_robosync -v >> "$log_file" 2>&1; then
            local robosync_end=$(date +%s.%N)
            local robosync_time=$(echo "$robosync_end - $robosync_start" | bc -l 2>/dev/null || echo "0")
            echo "RoboSync time: ${robosync_time}s" >> "$log_file"
        else
            echo "RoboSync failed" >> "$log_file"
            robosync_time="999"
        fi
        
        # Test rsync for comparison
        echo "Testing rsync on mixed workload..."
        local rsync_start=$(date +%s.%N)
        if rsync -av source/ dest_rsync/ >> "$log_file" 2>&1; then
            local rsync_end=$(date +%s.%N)
            local rsync_time=$(echo "$rsync_end - $rsync_start" | bc -l 2>/dev/null || echo "0")
            echo "rsync time: ${rsync_time}s" >> "$log_file"
        else
            echo "rsync failed" >> "$log_file"
            rsync_time="999"
        fi
        
        # Calculate speedup
        local speedup="0"
        if [[ "$robosync_time" != "0" && "$rsync_time" != "0" ]]; then
            speedup=$(echo "scale=2; $rsync_time / $robosync_time" | bc -l 2>/dev/null || echo "0")
        fi
        
        echo "Speedup: ${speedup}x" >> "$log_file"
        echo "    📊 RoboSync: ${robosync_time}s, rsync: ${rsync_time}s, Speedup: ${speedup}x"
        
        # CRITICAL: Must not be slower than rsync (speedup >= 1.0)
        if (( $(echo "$speedup >= 1.0" | bc -l 2>/dev/null || echo "0") )); then
            echo "    ✅ Mixed Workload Run $run: PASSED (${speedup}x speedup)"
            echo "Mixed Workload Regression Run $run: PASSED (${speedup}x speedup)" >> "$SUMMARY_FILE"
        else
            echo "    ❌ Mixed Workload Run $run: FAILED (${speedup}x speedup - REGRESSION)"
            echo "Mixed Workload Regression Run $run: FAILED (${speedup}x speedup - REGRESSION)" >> "$SUMMARY_FILE"
        fi
        
        # Log to master
        echo "Mixed Workload Regression Test Run $run: RoboSync ${robosync_time}s, rsync ${rsync_time}s, Speedup ${speedup}x" >> "$MASTER_LOG"
        
        cd "$RESULTS_DIR"
        rm -rf "$test_dir"
    done
}

run_large_file_performance_tests_5x() {
    echo
    echo "🚀 Running Large File Performance Tests 5x..."
    
    for run in {1..5}; do
        echo "  Large File Performance Test Run $run/5..."
        local log_file="$RESULTS_DIR/raw_logs/large_file_performance_run_${run}.log"
        local test_dir="/tmp/large_file_performance_test_run_${run}"
        
        {
            echo "=== Large File Performance Test Run $run ==="
            echo "Test Directory: $test_dir"
            echo "Timestamp: $(date)"
            echo "Target: Maintain 4x speedup advantage over rsync"
            echo
        } > "$log_file"
        
        mkdir -p "$test_dir"/{source,dest_robosync,dest_rsync}
        cd "$test_dir"
        
        # Create large files (50-100MB range)
        echo "Creating large files..."
        for i in {1..10}; do
            size_mb=$((50 + (i * 5)))  # 50-100MB range
            dd if=/dev/zero of="source/large_${i}_${size_mb}MB.bin" bs=1M count=$size_mb 2>/dev/null
        done
        
        # Test RoboSync
        echo "Testing RoboSync on large files..."
        local robosync_start=$(date +%s.%N)
        if "$ROBOSYNC_BIN" source dest_robosync -v >> "$log_file" 2>&1; then
            local robosync_end=$(date +%s.%N)
            local robosync_time=$(echo "$robosync_end - $robosync_start" | bc -l 2>/dev/null || echo "0")
            echo "RoboSync time: ${robosync_time}s" >> "$log_file"
        else
            echo "RoboSync failed" >> "$log_file"
            robosync_time="999"
        fi
        
        # Test rsync for comparison
        echo "Testing rsync on large files..."
        local rsync_start=$(date +%s.%N)
        if rsync -av source/ dest_rsync/ >> "$log_file" 2>&1; then
            local rsync_end=$(date +%s.%N)
            local rsync_time=$(echo "$rsync_end - $rsync_start" | bc -l 2>/dev/null || echo "0")
            echo "rsync time: ${rsync_time}s" >> "$log_file"
        else
            echo "rsync failed" >> "$log_file"
            rsync_time="999"
        fi
        
        # Calculate speedup
        local speedup="0"
        if [[ "$robosync_time" != "0" && "$rsync_time" != "0" ]]; then
            speedup=$(echo "scale=2; $rsync_time / $robosync_time" | bc -l 2>/dev/null || echo "0")
        fi
        
        echo "Speedup: ${speedup}x" >> "$log_file"
        echo "    📊 RoboSync: ${robosync_time}s, rsync: ${rsync_time}s, Speedup: ${speedup}x"
        
        # Target: Maintain 4x speedup (allow some variance, minimum 2x)
        if (( $(echo "$speedup >= 2.0" | bc -l 2>/dev/null || echo "0") )); then
            echo "    ✅ Large Files Run $run: PASSED (${speedup}x speedup)"
            echo "Large File Performance Run $run: PASSED (${speedup}x speedup)" >> "$SUMMARY_FILE"
        else
            echo "    ❌ Large Files Run $run: FAILED (${speedup}x speedup - below 2x target)"
            echo "Large File Performance Run $run: FAILED (${speedup}x speedup - below 2x target)" >> "$SUMMARY_FILE"
        fi
        
        # Log to master
        echo "Large File Performance Test Run $run: RoboSync ${robosync_time}s, rsync ${rsync_time}s, Speedup ${speedup}x" >> "$MASTER_LOG"
        
        cd "$RESULTS_DIR"
        rm -rf "$test_dir"
    done
}

# Execute all test suites 5x
echo "Starting macOS comprehensive 5x testing protocol..."

# Run APFS reflink tests 5 times
run_apfs_reflink_tests_5x

# Run medium file regression tests 5 times (CRITICAL)
run_medium_file_regression_tests_5x

# Run mixed workload regression tests 5 times (CRITICAL)
run_mixed_workload_regression_tests_5x

# Run large file performance tests 5 times
run_large_file_performance_tests_5x

# Generate final statistics
echo
echo "📊 Generating Final Statistics..."

echo "" >> "$SUMMARY_FILE"
echo "=== PASS/FAIL STATISTICS ===" >> "$SUMMARY_FILE"

test_types=("APFS Reflink" "Medium Files Regression" "Mixed Workload Regression" "Large File Performance")
for test_type in "${test_types[@]}"; do
    passed=$(grep "$test_type Run.*: PASSED" "$SUMMARY_FILE" | wc -l)
    failed=$(grep "$test_type Run.*: FAILED" "$SUMMARY_FILE" | wc -l)
    total=$((passed + failed))
    pass_rate=0
    if [[ $total -gt 0 ]]; then
        pass_rate=$(echo "scale=1; $passed * 100 / $total" | bc -l 2>/dev/null || echo "0")
    fi
    
    echo "$test_type Tests: $passed/$total passed (${pass_rate}%)" >> "$SUMMARY_FILE"
    echo "$test_type Tests: $passed/$total passed (${pass_rate}%)"
done

# Check critical regressions
echo "" >> "$SUMMARY_FILE"
echo "=== CRITICAL REGRESSION STATUS ===" >> "$SUMMARY_FILE"

medium_passes=$(grep "Medium Files Regression Run.*: PASSED" "$SUMMARY_FILE" | wc -l)
mixed_passes=$(grep "Mixed Workload Regression Run.*: PASSED" "$SUMMARY_FILE" | wc -l)

if [[ $medium_passes -eq 5 && $mixed_passes -eq 5 ]]; then
    echo "🎉 CRITICAL REGRESSIONS RESOLVED: All 5 runs passed for both medium files and mixed workloads"
    echo "CRITICAL REGRESSIONS: RESOLVED (5/5 medium files, 5/5 mixed workloads)" >> "$SUMMARY_FILE"
    critical_status="RESOLVED"
else
    echo "❌ CRITICAL REGRESSIONS PERSIST: Medium files $medium_passes/5, Mixed workloads $mixed_passes/5"
    echo "CRITICAL REGRESSIONS: PERSIST (${medium_passes}/5 medium files, ${mixed_passes}/5 mixed workloads)" >> "$SUMMARY_FILE"
    critical_status="PERSIST"
fi

# Create results index
results_index="$RESULTS_DIR/RESULTS_INDEX.txt"
cat > "$results_index" << EOF
RoboSync 2.0.0 macOS Comprehensive 5x Test Results Index
Platform: macOS
Timestamp: $TIMESTAMP
Results Directory: $RESULTS_DIR
Critical Regression Status: $critical_status

=== FILE LOCATIONS FOR REVIEW ===

1. MASTER EXECUTION LOG:
   $MASTER_LOG

2. FINAL SUMMARY:
   $SUMMARY_FILE

3. RAW TEST LOGS (20 files):
   $RESULTS_DIR/raw_logs/apfs_reflink_run_1.log through apfs_reflink_run_5.log
   $RESULTS_DIR/raw_logs/medium_files_regression_run_1.log through medium_files_regression_run_5.log
   $RESULTS_DIR/raw_logs/mixed_workload_regression_run_1.log through mixed_workload_regression_run_5.log
   $RESULTS_DIR/raw_logs/large_file_performance_run_1.log through large_file_performance_run_5.log

=== QUICK ACCESS COMMANDS ===
View final summary: cat $SUMMARY_FILE
View execution log: cat $MASTER_LOG
List all files: find $RESULTS_DIR -type f

=== COORDINATION COMMAND ===
When tests complete, update coordination database:
python3 /home/michael/Documents/Source/Repos/shared_2.0/resources/robosync_universal_db.py add mac_claude macos_5x_complete macos completed critical "macOS 5x Testing Complete" "All macOS tests completed 5x. APFS: X/5 passed, Medium Files: $medium_passes/5 passed, Mixed Workload: $mixed_passes/5 passed, Large Files: X/5 passed. Critical regression status: $critical_status. Results saved in $RESULTS_DIR. Ready for review." "roboclaude: Review macOS 5x test results"
EOF

echo
echo "✅ macOS 5x Testing Protocol Complete!"
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

# Return appropriate exit code
if [[ $critical_status == "RESOLVED" ]]; then
    exit 0
else
    exit 1
fi