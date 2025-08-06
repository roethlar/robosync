# Comprehensive Benchmarks 5x - Windows PowerShell Version
# Tests ALL performance scenarios 5 times for statistical validation against Robocopy

param(
    [string]$Platform = "windows",
    [string]$RoboSyncBin = ".\target\release\robosync.exe"
)

$ErrorActionPreference = "Stop"
$Timestamp = Get-Date -Format "yyyyMMdd_HHmmss"
$TestRoot = "C:\temp\robosync_comprehensive_benchmarks_windows_$Timestamp"
$ResultsDir = "$TestRoot\results"
$DataDir = "$TestRoot\benchmark_data"

# Create directories
New-Item -ItemType Directory -Path $ResultsDir -Force | Out-Null
New-Item -ItemType Directory -Path $DataDir -Force | Out-Null

Write-Host "📊 RoboSync 2.0.0 Comprehensive Benchmarks (5x Statistical Validation)" -ForegroundColor Cyan
Write-Host "======================================================================" -ForegroundColor Cyan
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

# Verify robocopy exists
$CompareTool = "robocopy"
if (-not (Get-Command robocopy -ErrorAction SilentlyContinue)) {
    Write-Host "❌ robocopy not found - required for benchmarking on Windows" -ForegroundColor Red
    exit 1
}

Write-Host "🎯 Comparison tool: $CompareTool"
Write-Host ""

# Initialize results CSV
$ResultsCSV = "$ResultsDir\benchmark_results.csv"
"benchmark_name,cycle,tool,files,total_size_mb,duration_seconds,throughput_mb_per_s,notes" | Out-File -FilePath $ResultsCSV -Encoding ASCII

# Benchmark scenarios
$Benchmarks = @{
    "small_files_1k" = "1000 files, 1KB each (~1MB)"
    "small_files_10k" = "500 files, 10KB each (~5MB)"
    "medium_files_100k" = "100 files, 100KB each (~10MB)"
    "medium_files_1mb" = "50 files, 1MB each (~50MB)"
    "large_files_10mb" = "10 files, 10MB each (~100MB)"
    "large_files_100mb" = "5 files, 100MB each (~500MB)"
    "mixed_workload" = "Mixed file sizes (~100MB total)"
    "deep_hierarchy" = "100 files in 10 nested dirs"
    "wide_hierarchy" = "100 files in 10 parallel dirs"
    "sparse_files" = "5 sparse files, 10MB each"
}

function Create-BenchmarkData {
    param([string]$BenchmarkName)
    
    $DataDirPath = "$DataDir\${BenchmarkName}_source"
    
    if (Test-Path $DataDirPath) {
        Remove-Item -Path $DataDirPath -Recurse -Force
    }
    New-Item -ItemType Directory -Path $DataDirPath -Force | Out-Null
    
    switch ($BenchmarkName) {
        "small_files_1k" {
            for ($i = 1; $i -le 1000; $i++) {
                fsutil file createnew "$DataDirPath\small_$i.dat" 1024 | Out-Null
            }
        }
        "small_files_10k" {
            for ($i = 1; $i -le 500; $i++) {
                fsutil file createnew "$DataDirPath\small_$i.dat" 10240 | Out-Null
            }
        }
        "medium_files_100k" {
            for ($i = 1; $i -le 100; $i++) {
                fsutil file createnew "$DataDirPath\medium_$i.dat" 102400 | Out-Null
            }
        }
        "medium_files_1mb" {
            for ($i = 1; $i -le 50; $i++) {
                fsutil file createnew "$DataDirPath\medium_$i.dat" 1048576 | Out-Null
            }
        }
        "large_files_10mb" {
            for ($i = 1; $i -le 10; $i++) {
                fsutil file createnew "$DataDirPath\large_$i.dat" 10485760 | Out-Null
            }
        }
        "large_files_100mb" {
            for ($i = 1; $i -le 5; $i++) {
                fsutil file createnew "$DataDirPath\large_$i.dat" 104857600 | Out-Null
            }
        }
        "mixed_workload" {
            # Mix of different file sizes
            for ($i = 1; $i -le 50; $i++) {
                fsutil file createnew "$DataDirPath\small_$i.dat" 1024 | Out-Null
            }
            for ($i = 1; $i -le 20; $i++) {
                fsutil file createnew "$DataDirPath\medium_$i.dat" 512000 | Out-Null
            }
            for ($i = 1; $i -le 5; $i++) {
                fsutil file createnew "$DataDirPath\large_$i.dat" 20971520 | Out-Null
            }
        }
        "deep_hierarchy" {
            $CurrentPath = $DataDirPath
            for ($depth = 1; $depth -le 10; $depth++) {
                $CurrentPath = "$CurrentPath\level$depth"
                New-Item -ItemType Directory -Path $CurrentPath -Force | Out-Null
                for ($i = 1; $i -le 10; $i++) {
                    fsutil file createnew "$CurrentPath\file_$i.dat" 10240 | Out-Null
                }
            }
        }
        "wide_hierarchy" {
            for ($dir = 1; $dir -le 10; $dir++) {
                $DirPath = "$DataDirPath\dir$dir"
                New-Item -ItemType Directory -Path $DirPath -Force | Out-Null
                for ($i = 1; $i -le 10; $i++) {
                    fsutil file createnew "$DirPath\file_$i.dat" 10240 | Out-Null
                }
            }
        }
        "sparse_files" {
            for ($i = 1; $i -le 5; $i++) {
                fsutil file createnew "$DataDirPath\sparse_$i.dat" 10485760 | Out-Null
                fsutil sparse setflag "$DataDirPath\sparse_$i.dat" | Out-Null
            }
        }
    }
    
    return $DataDirPath
}

function Get-DirectorySize {
    param([string]$Path)
    
    $Size = (Get-ChildItem -Path $Path -Recurse -File | Measure-Object -Property Length -Sum).Sum
    return [math]::Round($Size / 1MB, 2)
}

function Get-FileCount {
    param([string]$Path)
    
    return (Get-ChildItem -Path $Path -Recurse -File).Count
}

function Run-Benchmark {
    param(
        [string]$BenchmarkName,
        [int]$Cycle,
        [string]$Tool,
        [string]$SourcePath,
        [string]$DestPath
    )
    
    # Get source stats
    $FileCount = Get-FileCount -Path $SourcePath
    $TotalSizeMB = Get-DirectorySize -Path $SourcePath
    
    Write-Host "    [$Tool] Files: $FileCount, Size: ${TotalSizeMB}MB"
    
    $StartTime = Get-Date
    $ExitCode = 0
    $Notes = ""
    
    try {
        switch ($Tool) {
            "RoboSync" {
                $Process = Start-Process -FilePath $RoboSyncBin -ArgumentList "-s --mt 8 `"$SourcePath`" `"$DestPath`"" -Wait -PassThru -WindowStyle Hidden
                $ExitCode = $Process.ExitCode
            }
            "Robocopy" {
                $Process = Start-Process -FilePath "robocopy" -ArgumentList "`"$SourcePath`" `"$DestPath`" /E /MT:8 /NFL /NDL /NJH /NJS" -Wait -PassThru -WindowStyle Hidden
                # Robocopy exit codes 0-3 are success
                if ($Process.ExitCode -le 3) { $ExitCode = 0 } else { $ExitCode = $Process.ExitCode }
            }
        }
    } catch {
        $ExitCode = 1
        $Notes = "Exception: $($_.Exception.Message)"
    }
    
    $EndTime = Get-Date
    $Duration = ($EndTime - $StartTime).TotalSeconds
    
    if ($ExitCode -ne 0) {
        $Notes += " Failed (exit_code=$ExitCode)"
        $Throughput = 0
    } else {
        if ($Duration -gt 0) {
            $Throughput = [math]::Round($TotalSizeMB / $Duration, 2)
        } else {
            $Throughput = $TotalSizeMB  # Instant copy
        }
    }
    
    # Log result
    "$BenchmarkName,$Cycle,$Tool,$FileCount,$TotalSizeMB,$Duration,$Throughput,$Notes" | Add-Content -Path $ResultsCSV
    
    Write-Host "      Duration: ${Duration}s, Throughput: ${Throughput}MB/s"
    
    return @{
        Duration = $Duration
        Throughput = $Throughput
        ExitCode = $ExitCode
    }
}

# Run all benchmarks 5 times
Write-Host "🚀 Starting comprehensive benchmarks..." -ForegroundColor Yellow
Write-Host ""

foreach ($BenchmarkName in $Benchmarks.Keys) {
    Write-Host "📊 Benchmark: $BenchmarkName" -ForegroundColor Cyan
    Write-Host "    Description: $($Benchmarks[$BenchmarkName])"
    
    # Create benchmark data once
    Write-Host "    Creating test data..."
    $SourcePath = Create-BenchmarkData -BenchmarkName $BenchmarkName
    
    for ($Cycle = 1; $Cycle -le 5; $Cycle++) {
        Write-Host "  🔄 Cycle $Cycle/5:"
        
        # Test RoboSync
        $DestPathRS = "$DataDir\${BenchmarkName}_dest_rs_$Cycle"
        if (Test-Path $DestPathRS) { Remove-Item -Path $DestPathRS -Recurse -Force }
        $RSResult = Run-Benchmark -BenchmarkName $BenchmarkName -Cycle $Cycle -Tool "RoboSync" -SourcePath $SourcePath -DestPath $DestPathRS
        
        # Test Robocopy
        $DestPathRC = "$DataDir\${BenchmarkName}_dest_rc_$Cycle"
        if (Test-Path $DestPathRC) { Remove-Item -Path $DestPathRC -Recurse -Force }
        $RCResult = Run-Benchmark -BenchmarkName $BenchmarkName -Cycle $Cycle -Tool "Robocopy" -SourcePath $SourcePath -DestPath $DestPathRC
        
        # Compare
        if ($RSResult.ExitCode -eq 0 -and $RCResult.ExitCode -eq 0) {
            $SpeedupRatio = if ($RCResult.Throughput -gt 0) { [math]::Round($RSResult.Throughput / $RCResult.Throughput, 2) } else { "N/A" }
            if ($SpeedupRatio -is [double]) {
                if ($SpeedupRatio -gt 1) {
                    Write-Host "      💚 RoboSync ${SpeedupRatio}x FASTER" -ForegroundColor Green
                } elseif ($SpeedupRatio -lt 1) {
                    $SlowerRatio = [math]::Round(1 / $SpeedupRatio, 2)
                    Write-Host "      🟡 RoboSync ${SlowerRatio}x slower" -ForegroundColor Yellow
                } else {
                    Write-Host "      🟦 Equal performance" -ForegroundColor Blue
                }
            }
        }
        
        # Cleanup destinations
        Remove-Item -Path $DestPathRS -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -Path $DestPathRC -Recurse -Force -ErrorAction SilentlyContinue
    }
    
    Write-Host ""
    
    # Cleanup source data
    Remove-Item -Path $SourcePath -Recurse -Force -ErrorAction SilentlyContinue
}

# Generate summary statistics
Write-Host "📈 Generating summary statistics..." -ForegroundColor Yellow

$Results = Import-Csv -Path $ResultsCSV

$SummaryFile = "$ResultsDir\benchmark_summary.txt"
$Summary = @"
RoboSync 2.0.0 Comprehensive Benchmark Results (5x Statistical Validation)
Platform: $Platform  
Timestamp: $Timestamp
Binary: $RoboSyncBin
Comparison: vs $CompareTool

=== PERFORMANCE SUMMARY ===
"@

$Summary | Out-File -FilePath $SummaryFile -Encoding ASCII

foreach ($BenchmarkName in $Benchmarks.Keys) {
    "" | Add-Content -Path $SummaryFile
    "$BenchmarkName - $($Benchmarks[$BenchmarkName])" | Add-Content -Path $SummaryFile
    "=" * 50 | Add-Content -Path $SummaryFile
    
    $RSResults = $Results | Where-Object { $_.benchmark_name -eq $BenchmarkName -and $_.tool -eq "RoboSync" -and $_.notes -notlike "*Failed*" }
    $RCResults = $Results | Where-Object { $_.benchmark_name -eq $BenchmarkName -and $_.tool -eq "Robocopy" -and $_.notes -notlike "*Failed*" }
    
    if ($RSResults -and $RCResults) {
        $RSAvgThroughput = ($RSResults | Measure-Object -Property throughput_mb_per_s -Average).Average
        $RCAvgThroughput = ($RCResults | Measure-Object -Property throughput_mb_per_s -Average).Average
        
        $RSAvgDuration = ($RSResults | Measure-Object -Property duration_seconds -Average).Average
        $RCAvgDuration = ($RCResults | Measure-Object -Property duration_seconds -Average).Average
        
        "RoboSync Average: $([math]::Round($RSAvgThroughput, 2)) MB/s ($([math]::Round($RSAvgDuration, 2))s)" | Add-Content -Path $SummaryFile
        "Robocopy Average: $([math]::Round($RCAvgThroughput, 2)) MB/s ($([math]::Round($RCAvgDuration, 2))s)" | Add-Content -Path $SummaryFile
        
        if ($RCAvgThroughput -gt 0) {
            $SpeedupRatio = [math]::Round($RSAvgThroughput / $RCAvgThroughput, 2)
            if ($SpeedupRatio -gt 1) {
                "Result: RoboSync ${SpeedupRatio}x FASTER ✅" | Add-Content -Path $SummaryFile
            } elseif ($SpeedupRatio -lt 1) {
                $SlowerRatio = [math]::Round(1 / $SpeedupRatio, 2)
                "Result: RoboSync ${SlowerRatio}x slower ⚠️" | Add-Content -Path $SummaryFile
            } else {
                "Result: Equal performance ✅" | Add-Content -Path $SummaryFile
            }
        }
    } else {
        "Insufficient data for comparison" | Add-Content -Path $SummaryFile
    }
}

Write-Host "✅ Comprehensive benchmarks complete!" -ForegroundColor Green
Write-Host "📁 Results saved to: $ResultsDir"
Write-Host "📄 Summary: $SummaryFile"
Write-Host "📊 Raw data: $ResultsCSV"

# Optional: Cleanup test root
# Remove-Item -Path $TestRoot -Recurse -Force -ErrorAction SilentlyContinue