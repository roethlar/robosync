# Windows 5x Comprehensive Test Script for WinClaude
# Run ALL tests and benchmarks 5 times, save ALL results
# Usage: .\windows_5x_test_script.ps1

param(
    [string]$RoboSyncPath = ".\target\release\robosync.exe"
)

$Timestamp = Get-Date -Format "yyyyMMdd_HHmmss"
$ResultsDir = "C:\temp\robosync_5x_results_windows_$Timestamp"
$MasterLog = "$ResultsDir\master_execution_log.txt"
$SummaryFile = "$ResultsDir\summaries\final_summary.txt"

Write-Host "=== RoboSync 2.0.0 Windows 5x Test Protocol ===" -ForegroundColor Cyan
Write-Host "Results Directory: $ResultsDir" -ForegroundColor Gray
Write-Host "RoboSync Binary: $RoboSyncPath" -ForegroundColor Gray
Write-Host "All results will be saved for review" -ForegroundColor Yellow

# Create results directory structure
New-Item -ItemType Directory -Path "$ResultsDir\raw_logs" -Force | Out-Null
New-Item -ItemType Directory -Path "$ResultsDir\summaries" -Force | Out-Null
New-Item -ItemType Directory -Path "$ResultsDir\benchmarks" -Force | Out-Null
New-Item -ItemType Directory -Path "$ResultsDir\integration" -Force | Out-Null
New-Item -ItemType Directory -Path "$ResultsDir\validation" -Force | Out-Null

# Initialize master log
"=== WINDOWS 5X TEST EXECUTION LOG ===" | Out-File -FilePath $MasterLog -Encoding UTF8
"Timestamp: $(Get-Date)" | Out-File -FilePath $MasterLog -Append -Encoding UTF8
"Platform: Windows" | Out-File -FilePath $MasterLog -Append -Encoding UTF8
"Binary: $RoboSyncPath" | Out-File -FilePath $MasterLog -Append -Encoding UTF8
"Results Directory: $ResultsDir" | Out-File -FilePath $MasterLog -Append -Encoding UTF8
"" | Out-File -FilePath $MasterLog -Append -Encoding UTF8

# Initialize summary file
@"
RoboSync 2.0.0 Windows Comprehensive 5x Test Results
Platform: Windows
Timestamp: $Timestamp
Binary: $RoboSyncPath

=== TEST EXECUTION SUMMARY ===
"@ | Out-File -FilePath $SummaryFile -Encoding UTF8

# Check if RoboSync binary exists
if (-not (Test-Path $RoboSyncPath)) {
    Write-Host "❌ RoboSync binary not found at: $RoboSyncPath" -ForegroundColor Red
    Write-Host "Build with: cargo build --release" -ForegroundColor Yellow
    exit 1
}

function Run-ADS-Tests-5x {
    Write-Host "`n🧪 Running ADS Tests 5x..." -ForegroundColor Green
    
    for ($run = 1; $run -le 5; $run++) {
        Write-Host "  ADS Test Run $run/5..." -ForegroundColor Yellow
        $logFile = "$ResultsDir\raw_logs\ads_test_run_$run.log"
        $testDir = "C:\temp\ads_test_run_$run"
        
        try {
            # Create test directory
            New-Item -ItemType Directory -Path $testDir -Force | Out-Null
            Set-Location $testDir
            
            # Create file with ADS
            "Main content" | Out-File -FilePath "test.txt" -Encoding ASCII
            "Stream 1" | Out-File -FilePath "test.txt:stream1" -Encoding ASCII
            "Stream 2" | Out-File -FilePath "test.txt:stream2" -Encoding ASCII
            
            # Test RoboSync copy
            $output = & $RoboSyncPath "test.txt" "copied.txt" -v 2>&1 | Out-String
            $output | Out-File -FilePath $logFile -Encoding UTF8
            
            # Verify ADS preservation
            $sourceStreams = Get-Item "test.txt" -Stream * -ErrorAction SilentlyContinue
            $destStreams = Get-Item "copied.txt" -Stream * -ErrorAction SilentlyContinue
            
            $sourceAdsCount = ($sourceStreams | Where-Object { $_.Stream -ne ':$DATA' }).Count
            $destAdsCount = ($destStreams | Where-Object { $_.Stream -ne ':$DATA' }).Count
            
            if ($destAdsCount -ge $sourceAdsCount -and $destAdsCount -ge 2) {
                Write-Host "    ✅ ADS Run $run: PASSED ($destAdsCount streams preserved)" -ForegroundColor Green
                "ADS Run $run: PASSED ($destAdsCount streams preserved)" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8
            } else {
                Write-Host "    ❌ ADS Run $run: FAILED ($destAdsCount/$sourceAdsCount streams)" -ForegroundColor Red
                "ADS Run $run: FAILED ($destAdsCount/$sourceAdsCount streams)" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8
            }
            
            # Log to master
            "ADS Test Run $run completed: $destAdsCount/$sourceAdsCount streams preserved" | Out-File -FilePath $MasterLog -Append -Encoding UTF8
            
        } catch {
            Write-Host "    ❌ ADS Run $run: ERROR - $_" -ForegroundColor Red
            "ADS Run $run: ERROR - $_" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8
            $_.Exception.Message | Out-File -FilePath $logFile -Encoding UTF8
        } finally {
            Set-Location "$ResultsDir"
            Remove-Item -Path $testDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

function Run-Performance-Tests-5x {
    Write-Host "`n📊 Running Performance Tests 5x..." -ForegroundColor Green
    
    for ($run = 1; $run -le 5; $run++) {
        Write-Host "  Performance Test Run $run/5..." -ForegroundColor Yellow
        $logFile = "$ResultsDir\raw_logs\performance_test_run_$run.log"
        $testDir = "C:\temp\perf_test_run_$run"
        
        try {
            # Create test directory and data
            New-Item -ItemType Directory -Path "$testDir\source" -Force | Out-Null
            New-Item -ItemType Directory -Path "$testDir\dest_robosync" -Force | Out-Null
            New-Item -ItemType Directory -Path "$testDir\dest_robocopy" -Force | Out-Null
            
            Set-Location $testDir
            
            # Create test files (small, medium, large)
            Write-Host "    Creating test data..." -ForegroundColor Gray
            for ($i = 1; $i -le 100; $i++) {
                "Small file content $i" | Out-File -FilePath "source\small_$i.txt" -Encoding ASCII
            }
            
            for ($i = 1; $i -le 20; $i++) {
                $content = "Medium file content $i`n" * 1000
                $content | Out-File -FilePath "source\medium_$i.txt" -Encoding ASCII
            }
            
            # Test RoboSync
            Write-Host "    Testing RoboSync..." -ForegroundColor Gray
            $robosyncStart = Get-Date
            $robosyncOutput = & $RoboSyncPath "source" "dest_robosync" -v 2>&1 | Out-String
            $robosyncEnd = Get-Date
            $robosyncTime = ($robosyncEnd - $robosyncStart).TotalSeconds
            
            # Test Robocopy
            Write-Host "    Testing Robocopy..." -ForegroundColor Gray
            $robocopyStart = Get-Date
            $robocopyOutput = robocopy "source" "dest_robocopy" /E /MT 2>&1 | Out-String
            $robocopyEnd = Get-Date
            $robocopyTime = ($robocopyEnd - $robocopyStart).TotalSeconds
            
            # Calculate speedup
            $speedup = if ($robosyncTime -gt 0) { [math]::Round($robocopyTime / $robosyncTime, 2) } else { 0 }
            
            # Log results
            $results = @"
Performance Test Run $run Results:
RoboSync Time: $robosyncTime seconds
Robocopy Time: $robocopyTime seconds
Speedup: ${speedup}x

RoboSync Output:
$robosyncOutput

Robocopy Output:
$robocopyOutput
"@
            $results | Out-File -FilePath $logFile -Encoding UTF8
            
            Write-Host "    📈 RoboSync: ${robosyncTime}s, Robocopy: ${robocopyTime}s, Speedup: ${speedup}x" -ForegroundColor Cyan
            
            if ($speedup -ge 0.5) {  # Allow some variance
                Write-Host "    ✅ Performance Run $run: PASSED (${speedup}x)" -ForegroundColor Green
                "Performance Run $run: PASSED (${speedup}x speedup)" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8
            } else {
                Write-Host "    ❌ Performance Run $run: FAILED (${speedup}x)" -ForegroundColor Red
                "Performance Run $run: FAILED (${speedup}x speedup)" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8
            }
            
            # Log to master
            "Performance Test Run $run: RoboSync ${robosyncTime}s, Robocopy ${robocopyTime}s, Speedup ${speedup}x" | Out-File -FilePath $MasterLog -Append -Encoding UTF8
            
        } catch {
            Write-Host "    ❌ Performance Run $run: ERROR - $_" -ForegroundColor Red
            "Performance Run $run: ERROR - $_" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8
            $_.Exception.Message | Out-File -FilePath $logFile -Encoding UTF8
        } finally {
            Set-Location "$ResultsDir"
            Remove-Item -Path $testDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

function Run-Startup-Tests-5x {
    Write-Host "`n⚡ Running Startup Time Tests 5x..." -ForegroundColor Green
    
    for ($run = 1; $run -le 5; $run++) {
        Write-Host "  Startup Test Run $run/5..." -ForegroundColor Yellow
        $logFile = "$ResultsDir\raw_logs\startup_test_run_$run.log"
        
        try {
            # Measure startup time
            $startupStart = Get-Date
            $versionOutput = & $RoboSyncPath --version 2>&1 | Out-String
            $startupEnd = Get-Date
            $startupTime = ($startupEnd - $startupStart).TotalSeconds
            
            # Log results
            "Startup Test Run $run:`nTime: $startupTime seconds`nOutput: $versionOutput" | Out-File -FilePath $logFile -Encoding UTF8
            
            Write-Host "    ⏱️  Startup time: ${startupTime}s" -ForegroundColor Cyan
            
            if ($startupTime -le 2.0) {  # Target: under 2 seconds
                Write-Host "    ✅ Startup Run $run: PASSED (${startupTime}s)" -ForegroundColor Green
                "Startup Run $run: PASSED (${startupTime}s)" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8
            } else {
                Write-Host "    ❌ Startup Run $run: FAILED (${startupTime}s > 2.0s)" -ForegroundColor Red
                "Startup Run $run: FAILED (${startupTime}s > 2.0s)" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8
            }
            
            # Log to master
            "Startup Test Run $run: ${startupTime}s" | Out-File -FilePath $MasterLog -Append -Encoding UTF8
            
        } catch {
            Write-Host "    ❌ Startup Run $run: ERROR - $_" -ForegroundColor Red
            "Startup Run $run: ERROR - $_" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8
            $_.Exception.Message | Out-File -FilePath $logFile -Encoding UTF8
        }
    }
}

# Execute all test suites 5x
Write-Host "Starting Windows comprehensive 5x testing protocol..." -ForegroundColor Cyan

# Run ADS tests 5 times
Run-ADS-Tests-5x

# Run performance tests 5 times  
Run-Performance-Tests-5x

# Run startup tests 5 times
Run-Startup-Tests-5x

# Generate final statistics
Write-Host "`n📊 Generating Final Statistics..." -ForegroundColor Green

"`n=== PASS/FAIL STATISTICS ===" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8

$testTypes = @("ADS", "Performance", "Startup")
foreach ($testType in $testTypes) {
    $passed = (Get-Content $SummaryFile | Select-String "$testType Run.*: PASSED").Count
    $failed = (Get-Content $SummaryFile | Select-String "$testType Run.*: FAILED").Count
    $total = $passed + $failed
    $passRate = if ($total -gt 0) { [math]::Round($passed * 100 / $total, 1) } else { 0 }
    
    "$testType Tests: $passed/$total passed ($passRate%)" | Out-File -FilePath $SummaryFile -Append -Encoding UTF8
    Write-Host "$testType Tests: $passed/$total passed ($passRate%)" -ForegroundColor Cyan
}

# Create results index
$resultsIndex = "$ResultsDir\RESULTS_INDEX.txt"
@"
RoboSync 2.0.0 Windows Comprehensive 5x Test Results Index
Platform: Windows
Timestamp: $Timestamp
Results Directory: $ResultsDir

=== FILE LOCATIONS FOR REVIEW ===

1. MASTER EXECUTION LOG:
   $MasterLog

2. FINAL SUMMARY:
   $SummaryFile

3. RAW TEST LOGS (15 files):
   $ResultsDir\raw_logs\ads_test_run_1.log through ads_test_run_5.log
   $ResultsDir\raw_logs\performance_test_run_1.log through performance_test_run_5.log
   $ResultsDir\raw_logs\startup_test_run_1.log through startup_test_run_5.log

=== QUICK ACCESS COMMANDS ===
View final summary: Get-Content '$SummaryFile'
View execution log: Get-Content '$MasterLog'
List all files: Get-ChildItem '$ResultsDir' -Recurse

=== COORDINATION COMMAND ===
When tests complete, update coordination database:
python3 /home/michael/Documents/Source/Repos/shared_2.0/resources/robosync_universal_db.py add winclaude windows_5x_complete windows completed high "Windows 5x Testing Complete" "All Windows tests completed 5x. ADS: X/5 passed, Performance: X/5 passed, Startup: X/5 passed. Results saved in $ResultsDir. Ready for review." "roboclaude: Review Windows 5x test results"
"@ | Out-File -FilePath $resultsIndex -Encoding UTF8

Write-Host "`n✅ Windows 5x Testing Protocol Complete!" -ForegroundColor Green
Write-Host "`n📁 RESULTS INDEX CREATED: $resultsIndex" -ForegroundColor Yellow
Write-Host "`n=== FILE LOCATIONS FOR REVIEW ===" -ForegroundColor Cyan
Get-Content $resultsIndex

Write-Host "`n🔍 Quick Summary:" -ForegroundColor Green
Get-Content $SummaryFile

Write-Host "`n📊 All results saved in: $ResultsDir" -ForegroundColor Cyan
$fileCount = (Get-ChildItem $ResultsDir -Recurse -File).Count
$dirSize = [math]::Round((Get-ChildItem $ResultsDir -Recurse | Measure-Object -Property Length -Sum).Sum / 1MB, 2)
Write-Host "🗂️  Total files created: $fileCount" -ForegroundColor Gray
Write-Host "💾 Total size: ${dirSize}MB" -ForegroundColor Gray