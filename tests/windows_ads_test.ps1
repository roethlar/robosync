# Windows Alternate Data Streams (ADS) Test for RoboSync
# Run this in PowerShell as Administrator

param(
    [string]$RoboSyncPath = ".\target\release\robosync.exe"
)

Write-Host "=== Windows ADS Test for RoboSync 2.0.0 ===" -ForegroundColor Cyan
Write-Host "RoboSync Binary: $RoboSyncPath" -ForegroundColor Gray

# Check if RoboSync exists
if (-not (Test-Path $RoboSyncPath)) {
    Write-Host "❌ RoboSync binary not found at: $RoboSyncPath" -ForegroundColor Red
    Write-Host "Build with: cargo build --release" -ForegroundColor Yellow
    exit 1
}

# Create test directory
$TestDir = "C:\temp\robosync_ads_test_$(Get-Date -Format 'yyyyMMdd_HHmmss')"
New-Item -ItemType Directory -Path $TestDir -Force | Out-Null
Set-Location $TestDir

Write-Host "📁 Test Directory: $TestDir" -ForegroundColor Gray

# Test 1: Create file with ADS
Write-Host "`n🧪 Test 1: Creating file with Alternate Data Streams..." -ForegroundColor Green

New-Item -ItemType Directory -Path "source" -Force | Out-Null
$TestFile = "source\ads_test.txt"

# Main data stream
"Main file content" | Out-File -FilePath $TestFile -Encoding ASCII

# Create alternate data streams
"Stream 1 content" | Out-File -FilePath "${TestFile}:stream1" -Encoding ASCII
"Stream 2 content" | Out-File -FilePath "${TestFile}:stream2" -Encoding ASCII
"Hidden data" | Out-File -FilePath "${TestFile}:hidden" -Encoding ASCII

Write-Host "✅ Created test file with 3 alternate data streams" -ForegroundColor Green

# Verify ADS creation
Write-Host "📋 Verifying ADS on source file:" -ForegroundColor Yellow
Get-Item $TestFile -Stream * | ForEach-Object {
    $streamSize = $_.Length
    Write-Host "  - $($_.Stream): $streamSize bytes" -ForegroundColor Gray
}

# Test 2: Copy with RoboSync
Write-Host "`n🧪 Test 2: Copying with RoboSync..." -ForegroundColor Green

$StartTime = Get-Date
& $RoboSyncPath "source" "dest_robosync" -v
$EndTime = Get-Date
$Duration = ($EndTime - $StartTime).TotalSeconds

Write-Host "✅ RoboSync copy completed in $Duration seconds" -ForegroundColor Green

# Test 3: Verify ADS preservation
Write-Host "`n🧪 Test 3: Verifying ADS preservation..." -ForegroundColor Green

$DestFile = "dest_robosync\ads_test.txt"
if (Test-Path $DestFile) {
    Write-Host "✅ Destination file exists" -ForegroundColor Green
    
    # Check for alternate data streams
    $DestStreams = Get-Item $DestFile -Stream * -ErrorAction SilentlyContinue
    
    if ($DestStreams) {
        Write-Host "📋 ADS found on destination file:" -ForegroundColor Yellow
        $DestStreams | ForEach-Object {
            $streamSize = $_.Length
            Write-Host "  - $($_.Stream): $streamSize bytes" -ForegroundColor Gray
        }
        
        # Count non-default streams (exclude :$DATA)
        $AdsCount = ($DestStreams | Where-Object { $_.Stream -ne ':$DATA' }).Count
        
        if ($AdsCount -ge 3) {
            Write-Host "✅ ADS preservation: SUCCESS ($AdsCount streams found)" -ForegroundColor Green
            $AdsResult = "PASS"
        } else {
            Write-Host "❌ ADS preservation: FAILED (Expected 3+ streams, found $AdsCount)" -ForegroundColor Red
            $AdsResult = "FAIL"
        }
    } else {
        Write-Host "❌ ADS preservation: FAILED (No streams found)" -ForegroundColor Red
        $AdsResult = "FAIL"
    }
} else {
    Write-Host "❌ Destination file not found!" -ForegroundColor Red
    $AdsResult = "FAIL"
}

# Test 4: Content verification
Write-Host "`n🧪 Test 4: Verifying stream content..." -ForegroundColor Green

if ($AdsResult -eq "PASS") {
    $ContentMatch = $true
    
    # Check main content
    $SourceMain = Get-Content $TestFile -Raw
    $DestMain = Get-Content $DestFile -Raw
    if ($SourceMain -eq $DestMain) {
        Write-Host "✅ Main stream content matches" -ForegroundColor Green
    } else {
        Write-Host "❌ Main stream content mismatch" -ForegroundColor Red
        $ContentMatch = $false
    }
    
    # Check alternate streams
    @("stream1", "stream2", "hidden") | ForEach-Object {
        $StreamName = $_
        try {
            $SourceContent = Get-Content "${TestFile}:$StreamName" -Raw -ErrorAction Stop
            $DestContent = Get-Content "${DestFile}:$StreamName" -Raw -ErrorAction Stop
            
            if ($SourceContent -eq $DestContent) {
                Write-Host "✅ Stream '$StreamName' content matches" -ForegroundColor Green
            } else {
                Write-Host "❌ Stream '$StreamName' content mismatch" -ForegroundColor Red
                $ContentMatch = $false
            }
        } catch {
            Write-Host "❌ Stream '$StreamName' not accessible: $($_.Exception.Message)" -ForegroundColor Red
            $ContentMatch = $false
        }
    }
    
    if ($ContentMatch) {
        Write-Host "✅ Content verification: SUCCESS" -ForegroundColor Green
    } else {
        Write-Host "❌ Content verification: FAILED" -ForegroundColor Red
        $AdsResult = "FAIL"
    }
}

# Test 5: Compare with native copy
Write-Host "`n🧪 Test 5: Comparing with native Windows copy..." -ForegroundColor Green

Copy-Item "source\ads_test.txt" "dest_native\" -Force -ErrorAction SilentlyContinue
$NativeFile = "dest_native\ads_test.txt"

if (Test-Path $NativeFile) {
    $NativeStreams = Get-Item $NativeFile -Stream * -ErrorAction SilentlyContinue
    $NativeAdsCount = ($NativeStreams | Where-Object { $_.Stream -ne ':$DATA' }).Count
    
    Write-Host "📋 Native copy ADS count: $NativeAdsCount" -ForegroundColor Gray
    Write-Host "📋 RoboSync ADS count: $(($DestStreams | Where-Object { $_.Stream -ne ':$DATA' }).Count)" -ForegroundColor Gray
    
    if ($NativeAdsCount -eq 0) {
        Write-Host "ℹ️  Note: Native copy doesn't preserve ADS (expected behavior)" -ForegroundColor Blue
    }
}

# Test Results Summary
Write-Host "`n📊 TEST RESULTS SUMMARY" -ForegroundColor Cyan
Write-Host "======================" -ForegroundColor Cyan
Write-Host "ADS Creation: PASS" -ForegroundColor Green
Write-Host "RoboSync Copy: PASS" -ForegroundColor Green
Write-Host "ADS Preservation: $AdsResult" -ForegroundColor $(if ($AdsResult -eq "PASS") { "Green" } else { "Red" })

if ($AdsResult -eq "PASS") {
    Write-Host "`n🎉 OVERALL: ADS SUPPORT WORKING!" -ForegroundColor Green
    $ExitCode = 0
} else {
    Write-Host "`n❌ OVERALL: ADS SUPPORT BROKEN!" -ForegroundColor Red
    Write-Host "`n🔧 DEBUGGING INFORMATION:" -ForegroundColor Yellow
    Write-Host "1. Check if RoboSync is calling copy_ntfs_streams()" -ForegroundColor Gray
    Write-Host "2. Verify FindFirstStreamW/FindNextStreamW implementation" -ForegroundColor Gray
    Write-Host "3. Check stream path format in copy operations" -ForegroundColor Gray
    Write-Host "4. Ensure Windows APIs are used instead of std::fs::copy" -ForegroundColor Gray
    $ExitCode = 1
}

Write-Host "`n📁 Test files preserved at: $TestDir" -ForegroundColor Gray
Write-Host "🧹 Clean up with: Remove-Item -Recurse -Force '$TestDir'" -ForegroundColor Gray

exit $ExitCode