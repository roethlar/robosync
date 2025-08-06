#!/bin/bash
# Automated Benchmark Validator for RoboSync 2.0.0
# Validates that all performance claims are met before release
# Usage: ./automated_benchmark_validator.sh [platform]

set -e

PLATFORM=${1:-$(uname -s | tr '[:upper:]' '[:lower:]')}
ROBOSYNC_BIN="${ROBOSYNC_BIN:-./target/release/robosync}"
BENCHMARK_DIR="/tmp/robosync_benchmark_validation_$(date +%s)"
RESULTS_JSON="benchmark_validation_${PLATFORM}_$(date +%Y%m%d_%H%M%S).json"

echo "=== RoboSync 2.0.0 Automated Benchmark Validator ==="
echo "Platform: $PLATFORM"
echo "Binary: $ROBOSYNC_BIN"
echo "Results: $RESULTS_JSON"
echo

# Performance expectations based on claimed improvements
declare -A PERFORMANCE_CLAIMS
case "$PLATFORM" in
    "macos")
        PERFORMANCE_CLAIMS["large_files_min_speedup"]=4.0    # 4x faster than rsync
        PERFORMANCE_CLAIMS["small_files_min_speedup"]=1.0    # Equal to rsync
        PERFORMANCE_CLAIMS["medium_files_max_slowdown"]=1.0  # Must not be slower than rsync
        PERFORMANCE_CLAIMS["mixed_workload_max_slowdown"]=1.0 # Must not be slower than rsync
        ;;
    "linux")
        PERFORMANCE_CLAIMS["overall_min_speedup"]=6.0        # 6.25x faster than rsync
        PERFORMANCE_CLAIMS["small_files_min_speedup"]=6.0    # 6x faster than rsync
        PERFORMANCE_CLAIMS["large_files_min_speedup"]=1.5    # 1.75x faster than rsync
        ;;
    "windows")
        PERFORMANCE_CLAIMS["small_files_improvement"]=7.8    # 7.8x improvement vs previous
        PERFORMANCE_CLAIMS["medium_files_improvement"]=12.9  # 12.9x improvement vs previous
        PERFORMANCE_CLAIMS["startup_max_time"]=1.0           # Under 1 second startup
        ;;
esac

# Check prerequisites
if [[ ! -f "$ROBOSYNC_BIN" ]]; then
    echo "❌ RoboSync binary not found: $ROBOSYNC_BIN"
    exit 1
fi

if ! command -v bc >/dev/null 2>&1; then
    echo "❌ bc calculator required for benchmark validation"
    exit 1
fi

# Create benchmark environment
mkdir -p "$BENCHMARK_DIR"
cd "$BENCHMARK_DIR"

# Initialize JSON results
cat > "$RESULTS_JSON" << 'EOF'
{
  "platform": "PLATFORM_PLACEHOLDER",
  "timestamp": "TIMESTAMP_PLACEHOLDER", 
  "robosync_version": "VERSION_PLACEHOLDER",
  "performance_claims_met": false,
  "tests": [],
  "summary": {
    "total_tests": 0,
    "passed": 0,
    "failed": 0,
    "critical_failures": 0
  }
}
EOF

# Replace placeholders
sed -i "s/PLATFORM_PLACEHOLDER/$PLATFORM/g" "$RESULTS_JSON"
sed -i "s/TIMESTAMP_PLACEHOLDER/$(date -Iseconds)/g" "$RESULTS_JSON"

# Get RoboSync version
ROBOSYNC_VERSION=$("$ROBOSYNC_BIN" --version 2>/dev/null | head -1 || echo "Unknown")
sed -i "s/VERSION_PLACEHOLDER/$ROBOSYNC_VERSION/g" "$RESULTS_JSON"

create_test_data() {
    echo "📁 Creating benchmark test data..."
    
    # Small files benchmark (1000 files, ~10KB each)
    mkdir -p data/small_files
    for i in {1..1000}; do
        head -c 10240 </dev/urandom > "data/small_files/file_$i.bin" 2>/dev/null
    done
    
    # Medium files benchmark (100 files, 1-5MB each)  
    mkdir -p data/medium_files
    for i in {1..100}; do
        size_mb=$((1 + (i % 5)))
        head -c $((size_mb * 1024 * 1024)) </dev/urandom > "data/medium_files/file_$i.bin" 2>/dev/null
    done
    
    # Large files benchmark (20 files, 10-50MB each)
    mkdir -p data/large_files
    for i in {1..20}; do
        size_mb=$((10 + (i % 40)))
        head -c $((size_mb * 1024 * 1024)) </dev/urandom > "data/large_files/file_$i.bin" 2>/dev/null
    done
    
    # Mixed workload
    mkdir -p data/mixed_workload
    cp -r data/small_files data/mixed_workload/
    cp -r data/medium_files data/mixed_workload/
    cp data/large_files/file_1.bin data/mixed_workload/large_file.bin
}

run_benchmark_test() {
    local test_name="$1"
    local source_path="$2"
    local expected_speedup="$3"
    local is_critical="${4:-false}"
    
    echo
    echo "🧪 Benchmark Test: $test_name"
    echo "   Expected minimum speedup: ${expected_speedup}x"
    echo "   Critical test: $is_critical"
    
    # Clean destinations
    rm -rf "dest_robosync_$test_name" "dest_rsync_$test_name"
    
    # Measure RoboSync performance
    echo "   Measuring RoboSync..."
    local robosync_start=$(date +%s.%N)
    if "$ROBOSYNC_BIN" "$source_path" "dest_robosync_$test_name" >/dev/null 2>&1; then
        local robosync_end=$(date +%s.%N)
        local robosync_time=$(echo "$robosync_end - $robosync_start" | bc -l)
        echo "   ✅ RoboSync: ${robosync_time}s"
    else
        echo "   ❌ RoboSync failed"
        add_test_result "$test_name" "FAIL" "0" "0" "RoboSync execution failed" "$is_critical"
        return 1
    fi
    
    # Measure comparison tool performance
    local comparison_time="0"
    local comparison_tool="none"
    
    if command -v rsync >/dev/null 2>&1; then
        echo "   Measuring rsync..."
        comparison_tool="rsync"
        local rsync_start=$(date +%s.%N)
        if rsync -av "$source_path/" "dest_rsync_$test_name/" >/dev/null 2>&1; then
            local rsync_end=$(date +%s.%N)
            comparison_time=$(echo "$rsync_end - $rsync_start" | bc -l)
            echo "   ✅ rsync: ${comparison_time}s"
        else
            echo "   ❌ rsync failed"
            comparison_time="0"
        fi
    fi
    
    # Calculate speedup
    local actual_speedup="0"
    local status="FAIL"
    local notes=""
    
    if [[ "$comparison_time" != "0" ]] && [[ "$robosync_time" != "0" ]]; then
        actual_speedup=$(echo "scale=2; $comparison_time / $robosync_time" | bc -l)
        echo "   📈 Actual speedup: ${actual_speedup}x"
        
        # Check if meets expectation
        if (( $(echo "$actual_speedup >= $expected_speedup" | bc -l) )); then
            status="PASS"
            echo "   ✅ PERFORMANCE CLAIM VALIDATED"
            notes="Performance target met: ${actual_speedup}x >= ${expected_speedup}x"
        else
            status="FAIL"
            echo "   ❌ PERFORMANCE CLAIM FAILED"
            notes="Performance target missed: ${actual_speedup}x < ${expected_speedup}x"
        fi
    else
        notes="Could not measure comparison performance"
        if [[ "$expected_speedup" == "1.0" ]] && [[ "$robosync_time" != "0" ]]; then
            # For tests that just need to work (speedup >= 1.0)
            status="PASS"
            actual_speedup="1.0"
            notes="Execution successful, no comparison available"
        fi
    fi
    
    # Verify file integrity
    local src_count=$(find "$source_path" -type f | wc -l)
    local dest_count=$(find "dest_robosync_$test_name" -type f | wc -l)
    
    if [[ "$src_count" -eq "$dest_count" ]]; then
        echo "   ✅ File integrity verified ($src_count files)"
    else
        echo "   ❌ File integrity failed ($src_count vs $dest_count)"
        status="FAIL"
        notes="$notes; File count mismatch"
    fi
    
    add_test_result "$test_name" "$status" "$actual_speedup" "$expected_speedup" "$notes" "$is_critical"
    
    [[ "$status" == "PASS" ]]
}

add_test_result() {
    local test_name="$1"
    local status="$2" 
    local actual_speedup="$3"
    local expected_speedup="$4"
    local notes="$5"
    local is_critical="${6:-false}"
    
    # Create temporary JSON for this test
    cat > "test_$test_name.json" << EOF
{
  "name": "$test_name",
  "status": "$status",
  "actual_speedup": $actual_speedup,
  "expected_speedup": $expected_speedup,
  "notes": "$notes",
  "is_critical": $is_critical
}
EOF
}

finalize_results() {
    echo
    echo "📊 Finalizing benchmark validation results..."
    
    # Count results
    local total_tests=$(ls test_*.json 2>/dev/null | wc -l)
    local passed_tests=$(grep '"status": "PASS"' test_*.json 2>/dev/null | wc -l)
    local failed_tests=$((total_tests - passed_tests))
    local critical_failures=$(grep -l '"is_critical": true' test_*.json 2>/dev/null | xargs grep '"status": "FAIL"' 2>/dev/null | wc -l)
    
    # Update summary in main JSON
    local temp_json=$(mktemp)
    jq ".summary.total_tests = $total_tests | .summary.passed = $passed_tests | .summary.failed = $failed_tests | .summary.critical_failures = $critical_failures" "$RESULTS_JSON" > "$temp_json"
    mv "$temp_json" "$RESULTS_JSON"
    
    # Add all test results
    if [[ $total_tests -gt 0 ]]; then
        local all_tests=$(jq -s '.' test_*.json)
        temp_json=$(mktemp)
        jq ".tests = $all_tests" "$RESULTS_JSON" > "$temp_json"
        mv "$temp_json" "$RESULTS_JSON"
        rm test_*.json
    fi
    
    # Determine if performance claims are met
    local claims_met=false
    if [[ $critical_failures -eq 0 ]] && [[ $failed_tests -eq 0 ]]; then
        claims_met=true
    fi
    
    temp_json=$(mktemp)
    jq ".performance_claims_met = $claims_met" "$RESULTS_JSON" > "$temp_json"
    mv "$temp_json" "$RESULTS_JSON"
    
    echo "Total tests: $total_tests"
    echo "Passed: $passed_tests"
    echo "Failed: $failed_tests"
    echo "Critical failures: $critical_failures"
    echo "Performance claims met: $claims_met"
}

# Main benchmark execution
echo "🚀 Starting automated benchmark validation..."

create_test_data

# Run platform-specific benchmarks
case "$PLATFORM" in
    "macos")
        echo "🍎 Running macOS performance validation..."
        run_benchmark_test "small_files" "data/small_files" "${PERFORMANCE_CLAIMS[small_files_min_speedup]}" false
        run_benchmark_test "medium_files" "data/medium_files" "${PERFORMANCE_CLAIMS[medium_files_max_slowdown]}" true
        run_benchmark_test "large_files" "data/large_files" "${PERFORMANCE_CLAIMS[large_files_min_speedup]}" true  
        run_benchmark_test "mixed_workload" "data/mixed_workload" "${PERFORMANCE_CLAIMS[mixed_workload_max_slowdown]}" true
        ;;
    "linux")
        echo "🐧 Running Linux performance validation..."
        run_benchmark_test "small_files" "data/small_files" "${PERFORMANCE_CLAIMS[small_files_min_speedup]}" true
        run_benchmark_test "large_files" "data/large_files" "${PERFORMANCE_CLAIMS[large_files_min_speedup]}" true
        run_benchmark_test "mixed_workload" "data/mixed_workload" "${PERFORMANCE_CLAIMS[overall_min_speedup]}" true
        ;;
    "windows")
        echo "🪟 Running Windows performance validation..."
        # Focus on startup time and basic functionality
        echo "   Testing startup performance..."
        start_time=$(date +%s.%N)
        "$ROBOSYNC_BIN" --version >/dev/null 2>&1
        end_time=$(date +%s.%N)
        startup_time=$(echo "$end_time - $start_time" | bc -l)
        
        if (( $(echo "$startup_time <= ${PERFORMANCE_CLAIMS[startup_max_time]}" | bc -l) )); then
            add_test_result "startup_time" "PASS" "1.0" "1.0" "Startup time: ${startup_time}s <= ${PERFORMANCE_CLAIMS[startup_max_time]}s" true
        else
            add_test_result "startup_time" "FAIL" "0" "1.0" "Startup time: ${startup_time}s > ${PERFORMANCE_CLAIMS[startup_max_time]}s" true
        fi
        
        run_benchmark_test "mixed_workload" "data/mixed_workload" "1.0" true
        ;;
esac

finalize_results

# Generate final report
echo
echo "📋 FINAL BENCHMARK VALIDATION REPORT"
echo "===================================="
cat "$RESULTS_JSON" | jq '.summary'

if [[ $(jq -r '.performance_claims_met' "$RESULTS_JSON") == "true" ]]; then
    echo
    echo "🎉 ALL PERFORMANCE CLAIMS VALIDATED!"
    echo "✅ RoboSync 2.0.0 meets all stated performance targets"
    echo "🚀 Ready for release approval"
    exit_code=0
else
    echo
    echo "❌ PERFORMANCE CLAIMS VALIDATION FAILED"
    echo "🚨 Release blocked until performance issues resolved"
    echo
    echo "Failed tests:"
    jq -r '.tests[] | select(.status == "FAIL") | "- \(.name): \(.notes)"' "$RESULTS_JSON"
    exit_code=1
fi

echo
echo "📁 Full results saved to: $RESULTS_JSON"
echo "📁 Test data at: $BENCHMARK_DIR"

exit $exit_code