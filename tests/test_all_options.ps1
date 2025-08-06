# Test ALL RoboSync Options - Simple Direct Execution
param(
    [string]$RoboSyncBin = ".\target\release\robosync.exe"
)

$Timestamp = Get-Date -Format "yyyyMMdd_HHmmss"
$TestRoot = "C:\temp\robosync_all_options_$Timestamp"
$Source = "$TestRoot\source"
$Dest = "$TestRoot\dest"

Write-Host "Testing ALL RoboSync Options" -ForegroundColor Cyan
Write-Host "=============================" -ForegroundColor Cyan

# Create test directories
New-Item -ItemType Directory -Path $Source -Force | Out-Null
New-Item -ItemType Directory -Path $Dest -Force | Out-Null

# Create test files
"content1" | Out-File "$Source\file1.txt"
"content2" | Out-File "$Source\file2.txt"
fsutil file createnew "$Source\small.dat" 1024 | Out-Null
fsutil file createnew "$Source\large.dat" 10485760 | Out-Null
New-Item -ItemType Directory -Path "$Source\subdir" -Force | Out-Null
"nested" | Out-File "$Source\subdir\nested.txt"
New-Item -ItemType Directory -Path "$Source\empty" -Force | Out-Null

$TotalTests = 0
$Passed = 0
$Failed = 0

function Test-Option {
    param($Name, $Args)
    
    Write-Host "`nTesting: $Name"
    Write-Host "  Command: $RoboSyncBin $Args $Source $Dest"
    
    $script:TotalTests++
    
    # Clear destination
    Remove-Item "$Dest\*" -Recurse -Force -ErrorAction SilentlyContinue
    
    try {
        $output = & cmd /c "$RoboSyncBin $Args `"$Source`" `"$Dest`" 2>&1"
        $exitCode = $LASTEXITCODE
        
        if ($exitCode -eq 0) {
            Write-Host "  ✅ PASS (exit code: $exitCode)" -ForegroundColor Green
            $script:Passed++
        } else {
            Write-Host "  ❌ FAIL (exit code: $exitCode)" -ForegroundColor Red
            $script:Failed++
        }
    } catch {
        Write-Host "  ❌ EXCEPTION: $_" -ForegroundColor Red
        $script:Failed++
    }
}

# Test ALL options from --help
Write-Host "`n=== BASIC OPERATIONS ===" -ForegroundColor Yellow
Test-Option "Subdirs non-empty (-s)" "-s"
Test-Option "Subdirs with empty (-e)" "-e"
Test-Option "Mirror (--mir)" "--mir"
Test-Option "Purge (--purge)" "--purge"
Test-Option "List only (-l)" "-l"
Test-Option "Move (--mov)" "--mov"

Write-Host "`n=== EXCLUSIONS ===" -ForegroundColor Yellow
Test-Option "Exclude files (--xf)" "--xf file1.txt"
Test-Option "Exclude dirs (--xd)" "--xd subdir"
Test-Option "Min size (--min)" "--min 500"
Test-Option "Max size (--max)" "--max 5000000"

Write-Host "`n=== COPY FLAGS ===" -ForegroundColor Yellow
Test-Option "Copy data (--copy D)" "--copy D"
Test-Option "Copy attributes (--copy A)" "--copy A"
Test-Option "Copy timestamps (--copy T)" "--copy T"
Test-Option "Copy security (--copy S)" "--copy S"
Test-Option "Copy owner (--copy O)" "--copy O"
Test-Option "Copy auditing (--copy U)" "--copy U"
Test-Option "Copy all (--copy DATSOU)" "--copy DATSOU"
Test-Option "Copyall (--copyall)" "--copyall"

Write-Host "`n=== VERBOSITY/PROGRESS ===" -ForegroundColor Yellow
Test-Option "Verbose (-v)" "-v"
Test-Option "Very verbose (-vv)" "-v -v"
Test-Option "Progress (-p)" "-p"
Test-Option "Progress long (--progress)" "--progress"
Test-Option "ETA (--eta)" "--eta"
Test-Option "Debug (--debug)" "--debug"

Write-Host "`n=== ERROR HANDLING ===" -ForegroundColor Yellow
Test-Option "No report errors (--no-report-errors)" "--no-report-errors"
# Skip --confirm as it requires user input
# Skip --log as it needs special handling

Write-Host "`n=== RETRY OPTIONS ===" -ForegroundColor Yellow
Test-Option "Retry (-r)" "-r 1"
Test-Option "Retry long (--retry)" "--retry 1"
Test-Option "Wait (-w)" "-w 1"
Test-Option "Wait long (--wait)" "--wait 1"
Test-Option "Retry with wait" "--retry 1 --wait 1"

Write-Host "`n=== PERFORMANCE ===" -ForegroundColor Yellow
Test-Option "Multi-threaded (--mt)" "--mt 2"
Test-Option "Block size (-b)" "-b 2048"
Test-Option "Block size long (--block-size)" "--block-size 2048"
Test-Option "Small threshold" "--small-threshold 131072"
Test-Option "Medium threshold" "--medium-threshold 5242880"
Test-Option "Large threshold" "--large-threshold 52428800"

Write-Host "`n=== ARCHIVE/RECURSIVE ===" -ForegroundColor Yellow
Test-Option "Archive (-a)" "-a"
Test-Option "Archive long (--archive)" "--archive"
Test-Option "Recursive (--recursive)" "--recursive"

Write-Host "`n=== COMPRESSION/CHECKSUM ===" -ForegroundColor Yellow
Test-Option "Compress (-z)" "-z"
Test-Option "Compress long (--compress)" "--compress"
Test-Option "Checksum (-c)" "-c"
Test-Option "Checksum long (--checksum)" "--checksum"

Write-Host "`n=== LINK HANDLING ===" -ForegroundColor Yellow
Test-Option "Links (--links)" "--links"
Test-Option "Deref (--deref)" "--deref"
Test-Option "No links (--no-links)" "--no-links"

Write-Host "`n=== DRY RUN ===" -ForegroundColor Yellow
Test-Option "Dry run (-n)" "-n"
Test-Option "Dry run long (--dry-run)" "--dry-run"

Write-Host "`n=== REFLINK ===" -ForegroundColor Yellow
Test-Option "Reflink auto" "--reflink auto"
Test-Option "Reflink always" "--reflink always"
Test-Option "Reflink never" "--reflink never"

Write-Host "`n=== VERSION/HELP ===" -ForegroundColor Yellow
Write-Host "`nTesting: Version (-V)"
& $RoboSyncBin -V
if ($LASTEXITCODE -eq 0) {
    Write-Host "  ✅ PASS" -ForegroundColor Green
    $Passed++
} else {
    Write-Host "  ❌ FAIL" -ForegroundColor Red
    $Failed++
}
$TotalTests++

Write-Host "`nTesting: Version (--version)"
& $RoboSyncBin --version
if ($LASTEXITCODE -eq 0) {
    Write-Host "  ✅ PASS" -ForegroundColor Green
    $Passed++
} else {
    Write-Host "  ❌ FAIL" -ForegroundColor Red
    $Failed++
}
$TotalTests++

Write-Host "`nTesting: Help (-h)"
& $RoboSyncBin -h | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Host "  ✅ PASS" -ForegroundColor Green
    $Passed++
} else {
    Write-Host "  ❌ FAIL" -ForegroundColor Red
    $Failed++
}
$TotalTests++

Write-Host "`nTesting: Help (--help)"
& $RoboSyncBin --help | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Host "  ✅ PASS" -ForegroundColor Green
    $Passed++
} else {
    Write-Host "  ❌ FAIL" -ForegroundColor Red
    $Failed++
}
$TotalTests++

# Test --log option separately
Write-Host "`n=== LOG OUTPUT ===" -ForegroundColor Yellow
Write-Host "`nTesting: Log output (--log)"
$LogFile = "$TestRoot\output.log"
& $RoboSyncBin --log "$LogFile" "$Source" "$Dest"
if ($LASTEXITCODE -eq 0 -and (Test-Path $LogFile)) {
    Write-Host "  ✅ PASS (log file created)" -ForegroundColor Green
    $Passed++
} else {
    Write-Host "  ❌ FAIL" -ForegroundColor Red
    $Failed++
}
$TotalTests++

# Summary
Write-Host "`n========================================" -ForegroundColor Cyan
Write-Host "COMPLETE TEST SUMMARY" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "Total tests: $TotalTests"
Write-Host "Passed: $Passed" -ForegroundColor Green
Write-Host "Failed: $Failed" -ForegroundColor Red
$SuccessRate = [math]::Round(($Passed / $TotalTests) * 100, 1)
Write-Host "Success rate: $SuccessRate%"

# Cleanup
Remove-Item -Path $TestRoot -Recurse -Force -ErrorAction SilentlyContinue

# Return results
@{
    TotalTests = $TotalTests
    Passed = $Passed
    Failed = $Failed
    SuccessRate = $SuccessRate
}