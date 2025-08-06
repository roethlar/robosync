# Complete Functionality Testing - Tests EVERY RoboSync option
# Based on actual --help output, not assumptions

param(
    [string]$Platform = "windows",
    [string]$RoboSyncBin = ".\target\release\robosync.exe"
)

$ErrorActionPreference = "Stop"
$Timestamp = Get-Date -Format "yyyyMMdd_HHmmss"
$TestRoot = "C:\temp\robosync_complete_test_$Timestamp"
$ResultsDir = "$TestRoot\results"
$LogDir = "$TestRoot\logs"

# Create directories
New-Item -ItemType Directory -Path $ResultsDir -Force | Out-Null
New-Item -ItemType Directory -Path $LogDir -Force | Out-Null

Write-Host "🧪 RoboSync COMPLETE Functionality Testing" -ForegroundColor Cyan
Write-Host "==========================================" -ForegroundColor Cyan
Write-Host "Platform: $Platform"
Write-Host "Binary: $RoboSyncBin"
Write-Host "Test Root: $TestRoot"
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
    
    "$TestName,$Cycle,$Result,$Details" | Add-Content -Path "$ResultsDir\complete_results.csv"
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
    "content3" | Out-File -FilePath "$SourceDir\file3.log" -Encoding ASCII
    
    # Different sizes for threshold testing
    fsutil file createnew "$SourceDir\tiny.dat" 512 | Out-Null          # 512B - tiny
    fsutil file createnew "$SourceDir\small.dat" 1024 | Out-Null       # 1KB - small
    fsutil file createnew "$SourceDir\medium.dat" 524288 | Out-Null    # 512KB - medium
    fsutil file createnew "$SourceDir\large.dat" 52428800 | Out-Null   # 50MB - large
    
    # Directory structure
    New-Item -ItemType Directory -Path "$SourceDir\subdir1\subdir2" -Force | Out-Null
    New-Item -ItemType Directory -Path "$SourceDir\empty_dir" -Force | Out-Null
    New-Item -ItemType Directory -Path "$SourceDir\exclude_me" -Force | Out-Null
    "nested" | Out-File -FilePath "$SourceDir\subdir1\nested.txt" -Encoding ASCII
    "deep" | Out-File -FilePath "$SourceDir\subdir1\subdir2\deep.txt" -Encoding ASCII
    "exclude" | Out-File -FilePath "$SourceDir\exclude_me\secret.txt" -Encoding ASCII
    
    # Windows-specific files with attributes
    "windows specific" | Out-File -FilePath "$SourceDir\windows_file.txt" -Encoding ASCII
    "hidden content" | Out-File -FilePath "$SourceDir\hidden.txt" -Encoding ASCII
    
    # Set attributes for testing
    try {
        Set-ItemProperty -Path "$SourceDir\file1.txt" -Name Attributes -Value ([System.IO.FileAttributes]::ReadOnly)
        Set-ItemProperty -Path "$SourceDir\hidden.txt" -Name Attributes -Value ([System.IO.FileAttributes]::Hidden)
    } catch {
        # Ignore attribute errors
    }
    
    # Create symlink if possible (requires admin on Windows)
    try {
        New-Item -ItemType SymbolicLink -Path "$SourceDir\link_to_file.txt" -Target "$SourceDir\file1.txt" -ErrorAction SilentlyContinue
        New-Item -ItemType SymbolicLink -Path "$SourceDir\link_to_dir" -Target "$SourceDir\subdir1" -ErrorAction SilentlyContinue
    } catch {
        # Symlinks require admin privileges
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
        # Special handling for different test types
        if ($TestName -eq "version_check") {
            $Process = Start-Process -FilePath $RoboSyncBin -ArgumentList "--version" -Wait -PassThru -RedirectStandardOutput $LogFile -RedirectStandardError "$LogFile.err" -NoNewWindow
            $ExitCode = $Process.ExitCode
        } elseif ($TestName -eq "help_check") {
            $Process = Start-Process -FilePath $RoboSyncBin -ArgumentList "--help" -Wait -PassThru -RedirectStandardOutput $LogFile -RedirectStandardError "$LogFile.err" -NoNewWindow
            $ExitCode = $Process.ExitCode
        } elseif ($TestName -eq "log_output") {
            $LogOutput = "$TestDir\output.log"
            $Process = Start-Process -FilePath $RoboSyncBin -ArgumentList "--log `"$LogOutput`" `"$Source`" `"$Dest`"" -Wait -PassThru -RedirectStandardOutput $LogFile -RedirectStandardError "$LogFile.err" -NoNewWindow
            $ExitCode = $Process.ExitCode
        } else {
            $Process = Start-Process -FilePath $RoboSyncBin -ArgumentList "$RoboSyncArgs `"$Source`" `"$Dest`"" -Wait -PassThru -RedirectStandardOutput $LogFile -RedirectStandardError "$LogFile.err" -NoNewWindow
            $ExitCode = $Process.ExitCode
        }
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
        "copy_subdirs" {
            if ($ExitCode -eq 0 -and (Test-Path "$Dest\subdir1\nested.txt")) {
                $Result = "PASS"
                $Details = "Subdirectories copied, $Details"
            } else {
                $Details = "Subdirectory copy failed, $Details"
            }
        }
        "copy_empty_dirs" {
            if ($ExitCode -eq 0 -and (Test-Path "$Dest\empty_dir")) {
                $Result = "PASS"
                $Details = "Empty directories copied, $Details"
            } else {
                $Details = "Empty directory copy failed, $Details"
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
        "dry_run" {
            if ($ExitCode -eq 0 -and -not (Test-Path "$Dest\file1.txt")) {
                $Result = "PASS"
                $Details = "Dry run worked (no files copied), $Details"
            } else {
                $Details = "Dry run failed (files were copied), $Details"
            }
        }
        "move_files" {
            # For move operation, source files should be deleted
            if ($ExitCode -eq 0 -and (Test-Path "$Dest\file1.txt") -and -not (Test-Path "$Source\file1.txt")) {
                $Result = "PASS"
                $Details = "Move operation succeeded (source deleted), $Details"
            } else {
                $Details = "Move operation failed, $Details"
            }
        }
        "exclude_files" {
            if ($ExitCode -eq 0 -and -not (Test-Path "$Dest\file1.txt") -and (Test-Path "$Dest\file2.txt")) {
                $Result = "PASS"
                $Details = "File exclusion worked correctly, $Details"
            } else {
                $Details = "File exclusion failed, $Details"
            }
        }
        "exclude_dirs" {
            if ($ExitCode -eq 0 -and -not (Test-Path "$Dest\exclude_me")) {
                $Result = "PASS"
                $Details = "Directory exclusion worked correctly, $Details"
            } else {
                $Details = "Directory exclusion failed, $Details"
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
        "version_check" {
            if ($ExitCode -eq 0) {
                $Result = "PASS"
                $Details = "Version displayed successfully, $Details"
            } else {
                $Details = "Version check failed, $Details"
            }
        }
        "help_check" {
            if ($ExitCode -eq 0) {
                $Result = "PASS"
                $Details = "Help displayed successfully, $Details"
            } else {
                $Details = "Help check failed, $Details"
            }
        }
        "log_output" {
            $LogOutput = "$TestDir\output.log"
            if ($ExitCode -eq 0 -and (Test-Path $LogOutput)) {
                $Result = "PASS"
                $Details = "Log file created successfully, $Details"
            } else {
                $Details = "Log file creation failed, $Details"
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
"test_name,cycle,result,details" | Out-File -FilePath "$ResultsDir\complete_results.csv" -Encoding ASCII

Write-Host "🔍 Testing EVERY RoboSync option..." -ForegroundColor Yellow
Write-Host ""

# Define ALL functionality tests based on actual --help output
$FunctionalityTests = @{
    # Basic operations
    "subdirs_nonempty" = @("-s", "copy_subdirs")
    "subdirs_empty" = @("-e", "copy_empty_dirs")
    "mirror_mode" = @("--mir", "mirror_mode")
    "purge_only" = @("--purge", "copy_files")
    "list_only" = @("-l", "list_only")
    "move_operation" = @("--mov", "move_files")
    
    # Exclusion patterns
    "exclude_files" = @("--xf file1.txt", "exclude_files")
    "exclude_dirs" = @("--xd exclude_me", "exclude_dirs")
    "min_size" = @("--min 1000", "copy_files")
    "max_size" = @("--max 1000000", "copy_files")
    
    # Copy options
    "copy_data" = @("--copy D", "copy_files")
    "copy_attrs" = @("--copy A", "copy_files")
    "copy_times" = @("--copy T", "copy_files")
    "copy_security" = @("--copy S", "copy_files")
    "copy_owner" = @("--copy O", "copy_files")
    "copy_auditing" = @("--copy U", "copy_files")
    "copy_all" = @("--copy DATSOU", "copy_files")
    "copyall_flag" = @("--copyall", "copy_files")
    
    # Verbosity and progress
    "verbose" = @("-v", "copy_files")
    "very_verbose" = @("-vv", "copy_files")
    "progress" = @("-p", "copy_files")
    "progress_long" = @("--progress", "copy_files")
    "eta" = @("--eta", "copy_files")
    "debug" = @("--debug", "copy_files")
    
    # Confirmation and error handling
    "confirm" = @("--confirm", "copy_files")
    "no_report_errors" = @("--no-report-errors", "copy_files")
    "log_output" = @("", "log_output")  # Special handling
    
    # Retry and reliability
    "retry" = @("-r 2", "copy_files")
    "retry_long" = @("--retry 2", "copy_files")
    "wait" = @("-w 1", "copy_files")
    "wait_long" = @("--wait 1", "copy_files")
    "retry_with_wait" = @("--retry 2 --wait 1", "copy_files")
    "multithreaded" = @("--mt 2", "copy_files")
    "multithreaded_4" = @("--mt 4", "copy_files")
    
    # Block and threshold sizes
    "block_size" = @("-b 2048", "copy_files")
    "block_size_long" = @("--block-size 2048", "copy_files")
    "small_threshold" = @("--small-threshold 131072", "copy_files")
    "medium_threshold" = @("--medium-threshold 8388608", "copy_files")
    "large_threshold" = @("--large-threshold 52428800", "copy_files")
    
    # Archive and recursive
    "archive" = @("-a", "copy_files")
    "archive_long" = @("--archive", "copy_files")
    "recursive" = @("--recursive", "copy_files")
    
    # Compression and checksums
    "compress" = @("-z", "copy_files")
    "compress_long" = @("--compress", "copy_files")
    "checksum" = @("-c", "copy_files")
    "checksum_long" = @("--checksum", "copy_files")
    
    # Link handling
    "links" = @("--links", "copy_files")
    "deref" = @("--deref", "copy_files")
    "no_links" = @("--no-links", "copy_files")
    
    # Dry run
    "dry_run" = @("-n", "dry_run")
    "dry_run_long" = @("--dry-run", "dry_run")
    
    # Reflink options
    "reflink_auto" = @("--reflink auto", "copy_files")
    "reflink_always" = @("--reflink always", "copy_files")
    "reflink_never" = @("--reflink never", "copy_files")
    
    # Version and help
    "version_short" = @("-V", "version_check")
    "version_long" = @("--version", "version_check")
    "help_short" = @("-h", "help_check")
    "help_long" = @("--help", "help_check")
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
Write-Host "📊 COMPLETE FUNCTIONALITY TEST SUMMARY" -ForegroundColor Green
Write-Host "======================================" -ForegroundColor Green
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
$SummaryFile = "$ResultsDir\complete_summary.txt"
@"
RoboSync COMPLETE Functionality Test Results
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

Write-Host "✅ COMPLETE functionality testing finished!" -ForegroundColor Green
Write-Host "📁 Results saved to: $ResultsDir"
Write-Host "📄 Summary: $SummaryFile" 
Write-Host "📊 Raw data: $ResultsDir\complete_results.csv"
Write-Host "🗂️  Logs: $LogDir"

# Return summary for reporting
return @{
    TotalTests = $script:TotalTests
    Passed = $script:PassedTests
    Failed = $script:FailedTests
    SuccessRate = if ($script:TotalTests -gt 0) { [math]::Round(($script:PassedTests / $script:TotalTests) * 100, 1) } else { 0 }
    ResultsDir = $ResultsDir
}