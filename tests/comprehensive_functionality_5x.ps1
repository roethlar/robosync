# Comprehensive Functionality Testing 5x - Windows PowerShell Version
# Tests EVERY RoboSync command line option, feature, and edge case 5 times for statistical validation

param(
    [string]$Platform = "windows",
    [string]$RoboSyncBin = ".\target\release\robosync.exe"
)

$ErrorActionPreference = "Stop"
$Timestamp = Get-Date -Format "yyyyMMdd_HHmmss"
$TestRoot = "C:\temp\robosync_comprehensive_functionality_windows_$Timestamp"
$ResultsDir = "$TestRoot\results"
$LogDir = "$TestRoot\logs"

# Create directories
New-Item -ItemType Directory -Path $ResultsDir -Force | Out-Null
New-Item -ItemType Directory -Path $LogDir -Force | Out-Null

Write-Host "🧪 RoboSync 2.0.0 Comprehensive Functionality Testing (5x)" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "Platform: $Platform"
Write-Host "Binary: $RoboSyncBin"
Write-Host "Test Root: $TestRoot"
Write-Host "Results: $ResultsDir"
Write-Host ""

# Verify binary exists
if (-not (Test-Path $RoboSyncBin)) {
    Write-Host "❌ RoboSync binary not found: $RoboSyncBin" -ForegroundColor Red
    exit 1
}

# Test result tracking
$script:TestResults = @{}
$script:TotalTests = 0
$script:PassedTests = 0
$script:FailedTests = 0

function Log-TestResult {
    param(
        [string]$TestName,
        [int]$Cycle,
        [string]$Result,
        [string]$Details
    )
    
    "$TestName,$Cycle,$Result,$Details" | Add-Content -Path "$ResultsDir\functionality_results.csv"
    $script:TestResults["${TestName}_${Cycle}"] = $Result
    $script:TotalTests++
    
    if ($Result -eq "PASS") {
        $script:PassedTests++
        Write-Host "  ✅ Cycle $Cycle`: PASS - $Details" -ForegroundColor Green
    } else {
        $script:FailedTests++
        Write-Host "  ❌ Cycle $Cycle`: FAIL - $Details" -ForegroundColor Red
    }
}

function Create-TestData {
    param([string]$BaseDir)
    
    $SourceDir = "$BaseDir\source"
    $DestDir = "$BaseDir\dest"
    $TempDir = "$BaseDir\temp"
    
    New-Item -ItemType Directory -Path $SourceDir -Force | Out-Null
    New-Item -ItemType Directory -Path $DestDir -Force | Out-Null
    New-Item -ItemType Directory -Path $TempDir -Force | Out-Null
    
    # Basic files
    "content1" | Out-File -FilePath "$SourceDir\file1.txt" -Encoding ASCII
    "content2" | Out-File -FilePath "$SourceDir\file2.txt" -Encoding ASCII
    
    # Different sizes for threshold testing
    fsutil file createnew "$SourceDir\small.dat" 1024 | Out-Null        # 1KB - small
    fsutil file createnew "$SourceDir\medium.dat" 524288 | Out-Null     # 512KB - medium
    fsutil file createnew "$SourceDir\large.dat" 52428800 | Out-Null    # 50MB - large
    
    # Directory structure
    New-Item -ItemType Directory -Path "$SourceDir\subdir1\subdir2" -Force | Out-Null
    New-Item -ItemType Directory -Path "$SourceDir\empty_dir" -Force | Out-Null
    "nested" | Out-File -FilePath "$SourceDir\subdir1\nested.txt" -Encoding ASCII
    "deep" | Out-File -FilePath "$SourceDir\subdir1\subdir2\deep.txt" -Encoding ASCII
    
    # Windows-specific files
    "windows specific" | Out-File -FilePath "$SourceDir\windows_file.txt" -Encoding ASCII
    
    # Set some attributes for testing
    try {
        Set-ItemProperty -Path "$SourceDir\file1.txt" -Name Attributes -Value ([System.IO.FileAttributes]::ReadOnly)
    } catch {
        # Ignore attribute errors
    }
}

function Run-FunctionalityTest {
    param(
        [string]$TestName,
        [int]$Cycle,
        [string]$RoboSyncArgs,
        [string]$ExpectedBehavior
    )
    
    $TestDir = "$TestRoot\test_${TestName}_${Cycle}"
    Create-TestData -BaseDir $TestDir
    
    $LogFile = "$LogDir\${TestName}_cycle_${Cycle}.log"
    $Source = "$TestDir\source"
    $Dest = "$TestDir\dest"
    
    Write-Host "    Running: $RoboSyncBin $RoboSyncArgs $Source $Dest"
    
    # Run the test
    $StartTime = Get-Date
    $ExitCode = 0
    
    try {
        $Process = Start-Process -FilePath $RoboSyncBin -ArgumentList "$RoboSyncArgs `"$Source`" `"$Dest`"" -Wait -PassThru -RedirectStandardOutput $LogFile -RedirectStandardError "$LogFile.err" -NoNewWindow
        $ExitCode = $Process.ExitCode
    } catch {
        $ExitCode = 1
        $_.Exception.Message | Out-File -FilePath $LogFile -Append
    }
    
    $EndTime = Get-Date
    $Duration = ($EndTime - $StartTime).TotalSeconds
    
    # Validate results based on expected behavior
    $Result = "FAIL"
    $Details = "exit_code=$ExitCode, duration=${Duration}s"
    
    switch ($ExpectedBehavior) {
        "copy_files" {
            if ($ExitCode -eq 0 -and (Test-Path "$Dest\file1.txt") -and (Test-Path "$Dest\file2.txt")) {
                $Result = "PASS"
                $Details = "Files copied successfully, $Details"
            } else {
                $Details = "Copy failed or files missing, $Details"
            }
        }
        "list_only" {
            if ($ExitCode -eq 0 -and -not (Test-Path "$Dest\file1.txt")) {
                $Result = "PASS"
                $Details = "List mode worked (no files copied), $Details"
            } else {
                $Details = "List mode failed (files were copied), $Details"
            }
        }
        "exclude_files" {
            if ($ExitCode -eq 0 -and -not (Test-Path "$Dest\file1.txt") -and (Test-Path "$Dest\file2.txt")) {
                $Result = "PASS"
                $Details = "Exclusion worked correctly, $Details"
            } else {
                $Details = "Exclusion failed, $Details"
            }
        }
        "mirror_mode" {
            # Create extra file in dest that should be deleted
            New-Item -ItemType Directory -Path $Dest -Force | Out-Null
            "extra" | Out-File -FilePath "$Dest\extra.txt" -Encoding ASCII
            
            try {
                $Process = Start-Process -FilePath $RoboSyncBin -ArgumentList "$RoboSyncArgs `"$Source`" `"$Dest`"" -Wait -PassThru -RedirectStandardOutput "$LogFile.mirror" -RedirectStandardError "$LogFile.mirror.err" -NoNewWindow
                $ExitCode = $Process.ExitCode
            } catch {
                $ExitCode = 1
            }
            
            if ($ExitCode -eq 0 -and (Test-Path "$Dest\file1.txt") -and -not (Test-Path "$Dest\extra.txt")) {
                $Result = "PASS"
                $Details = "Mirror mode deleted extra files, $Details"
            } else {
                $Details = "Mirror mode failed, $Details"
            }
        }
        "should_fail" {
            if ($ExitCode -ne 0) {
                $Result = "PASS"
                $Details = "Correctly failed as expected, $Details"
            } else {
                $Details = "Should have failed but succeeded, $Details"
            }
        }
        default {
            if ($ExitCode -eq 0) {
                $Result = "PASS"
                $Details = "Command succeeded, $Details"
            } else {
                $Details = "Command failed, $Details"
            }
        }
    }
    
    Log-TestResult -TestName $TestName -Cycle $Cycle -Result $Result -Details $Details
    
    # Cleanup
    try {
        Remove-Item -Path $TestDir -Recurse -Force -ErrorAction SilentlyContinue
    } catch {
        # Ignore cleanup errors
    }
}

# Initialize results CSV
"test_name,cycle,result,details" | Out-File -FilePath "$ResultsDir\functionality_results.csv" -Encoding ASCII

Write-Host "🔍 Testing ALL RoboSync functionality across 5 cycles..." -ForegroundColor Yellow
Write-Host ""

# Define all functionality tests (Windows-adapted)
$FunctionalityTests = @{
    # Basic operations
    "basic_copy" = @("-e", "copy_files")
    "subdirs_nonempty" = @("-s", "copy_files")
    "list_only" = @("-l", "list_only")
    "dry_run" = @("-n", "copy_files")
    
    # Mirror and purge operations
    "mirror_mode" = @("--mir", "mirror_mode")
    "purge_only" = @("--purge", "copy_files")
    
    # Exclusion patterns
    "exclude_files" = @("--xf file1.txt", "exclude_files")
    "exclude_dirs" = @("--xd subdir1", "copy_files")
    "min_size" = @("--min 1000", "copy_files")
    "max_size" = @("--max 1000000", "copy_files")
    
    # Copy options
    "copy_data" = @("--copy D", "copy_files")
    "copy_attrs" = @("--copy A", "copy_files")
    "copy_times" = @("--copy T", "copy_files")
    "copy_all" = @("--copy DATSOU", "copy_files")
    "copyall_flag" = @("--copyall", "copy_files")
    
    # Verbository and progress
    "verbose" = @("--verbose", "copy_files")
    "very_verbose" = @("-v -v", "copy_files")
    "progress" = @("--progress", "copy_files")
    "eta" = @("--eta", "copy_files")
    "debug" = @("--debug", "copy_files")
    
    # Retry and reliability
    "retry" = @("--retry 2", "copy_files")
    "wait_retry" = @("--retry 1 --wait 1", "copy_files")
    "multithreaded" = @("--mt 2", "copy_files")
    
    # Block and threshold sizes
    "block_size" = @("--block-size 2048", "copy_files")
    "small_threshold" = @("--small-threshold 131072", "copy_files")
    "medium_threshold" = @("--medium-threshold 8388608", "copy_files")
    "large_threshold" = @("--large-threshold 52428800", "copy_files")
    
    # Advanced options
    "archive_mode" = @("--archive", "copy_files")
    "compression" = @("--compress", "copy_files")
    "checksums" = @("--checksum", "copy_files")
    
    # Link handling
    "preserve_links" = @("--links", "copy_files")
    "deref_links" = @("--deref", "copy_files")
    "no_links" = @("--no-links", "copy_files")
    
    # Reflink options
    "reflink_auto" = @("--reflink auto", "copy_files")
    "reflink_always" = @("--reflink always", "copy_files")
    "reflink_never" = @("--reflink never", "copy_files")
}

# Run all tests 5 times
foreach ($TestName in $FunctionalityTests.Keys) {
    Write-Host "🧪 Testing: $TestName" -ForegroundColor Cyan
    
    $TestConfig = $FunctionalityTests[$TestName]
    $Args = $TestConfig[0]
    $Expected = $TestConfig[1]
    
    for ($Cycle = 1; $Cycle -le 5; $Cycle++) {
        Run-FunctionalityTest -TestName $TestName -Cycle $Cycle -RoboSyncArgs $Args -ExpectedBehavior $Expected
    }
    
    Write-Host ""
}

# Generate summary
Write-Host "📊 FUNCTIONALITY TEST SUMMARY" -ForegroundColor Green
Write-Host "=============================" -ForegroundColor Green
Write-Host "Total tests: $script:TotalTests"
Write-Host "Passed: $script:PassedTests"
Write-Host "Failed: $script:FailedTests"
if ($script:TotalTests -gt 0) {
    $SuccessRate = [math]::Round(($script:PassedTests / $script:TotalTests) * 100, 1)
    Write-Host "Success rate: $SuccessRate%"
} else {
    Write-Host "Success rate: N/A (no tests run)"
}
Write-Host ""

# Generate detailed report
$SummaryFile = "$ResultsDir\functionality_summary.txt"
@"
RoboSync 2.0.0 Comprehensive Functionality Test Results
Platform: $Platform
Timestamp: $Timestamp
Binary: $RoboSyncBin

=== SUMMARY STATISTICS ===
Total Tests: $script:TotalTests
Passed: $script:PassedTests
Failed: $script:FailedTests
Success Rate: $(if ($script:TotalTests -gt 0) { [math]::Round(($script:PassedTests / $script:TotalTests) * 100, 1) } else { "N/A" })%

=== TEST RESULTS BY FEATURE ===
"@ | Out-File -FilePath $SummaryFile -Encoding ASCII

# Add detailed results for each feature
foreach ($TestName in $FunctionalityTests.Keys) {
    "" | Add-Content -Path $SummaryFile
    "$TestName`:" | Add-Content -Path $SummaryFile
    
    $Passes = 0
    for ($Cycle = 1; $Cycle -le 5; $Cycle++) {
        $Result = $script:TestResults["${TestName}_${Cycle}"]
        "  Cycle $Cycle`: $Result" | Add-Content -Path $SummaryFile
        if ($Result -eq "PASS") { $Passes++ }
    }
    
    $PassRate = ($Passes * 20)
    "  Pass Rate: $Passes/5 ($PassRate%)" | Add-Content -Path $SummaryFile
}

Write-Host "✅ Comprehensive functionality testing complete!" -ForegroundColor Green
Write-Host "📁 Results saved to: $ResultsDir"
Write-Host "📄 Summary: $SummaryFile" 
Write-Host "📊 Raw data: $ResultsDir\functionality_results.csv"
Write-Host "🗂️  Logs: $LogDir"

# Cleanup test root (optional)
# Remove-Item -Path $TestRoot -Recurse -Force -ErrorAction SilentlyContinue