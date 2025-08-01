# Windows PowerShell test data creation script
# Creates identical test data for performance comparison

Write-Host "Creating standardized test data..." -ForegroundColor Green

# Clean up any existing test data
Remove-Item -Path perf_test, test_src, test_dst*, perf_dst*, robocopy_dst, robosync_dst, cp_dst -Recurse -Force -ErrorAction SilentlyContinue

# Create directory structure
New-Item -ItemType Directory -Path perf_test\small, perf_test\medium, perf_test\large -Force | Out-Null
New-Item -ItemType Directory -Path test_src\small, test_src\medium, test_src\large -Force | Out-Null

# Function to create small files
function Create-SmallFiles {
    Write-Host "Creating 10,000 small files (1KB each)..." -ForegroundColor Yellow
    $content = "This is test file number {0} with some padding content to reach approximately 1KB in size. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum. Adding more text to ensure we reach close to 1KB. The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs. How vexingly quick daft zebras jump! Bright vixens jump; dozy fowl quack."
    
    for ($i = 1; $i -le 10000; $i++) {
        $content -f $i | Out-File -FilePath "perf_test\small\file_$i.txt" -Encoding UTF8
        
        # Progress indicator
        if ($i % 1000 -eq 0) {
            Write-Host "  Created $i/10000 small files..."
        }
    }
}

# Function to create medium files
function Create-MediumFiles {
    Write-Host "Creating 100 medium files (10MB each)..." -ForegroundColor Yellow
    $buffer = New-Object byte[] (10MB)
    
    for ($i = 1; $i -le 100; $i++) {
        $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
        $rng.GetBytes($buffer)
        [System.IO.File]::WriteAllBytes("perf_test\medium\file_$i.bin", $buffer)
        Write-Host "  Created medium file $i/100"
    }
}

# Function to create large files
function Create-LargeFiles {
    Write-Host "Creating 5 large files (200MB each)..." -ForegroundColor Yellow
    $buffer = New-Object byte[] (10MB)
    
    for ($i = 1; $i -le 5; $i++) {
        $fs = [System.IO.File]::Create("perf_test\large\file_$i.bin")
        $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
        
        for ($j = 1; $j -le 20; $j++) {
            $rng.GetBytes($buffer)
            $fs.Write($buffer, 0, $buffer.Length)
        }
        $fs.Close()
        Write-Host "  Created large file $i/5"
    }
}

# Create test files for basic operations
Write-Host "Creating basic test structure..." -ForegroundColor Yellow
"test content" | Out-File -FilePath "test_src\small\1kb.txt" -Encoding UTF8

# 5MB file
$buffer = New-Object byte[] (5MB)
[System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($buffer)
[System.IO.File]::WriteAllBytes("test_src\medium\5mb.bin", $buffer)

# 150MB file
$fs = [System.IO.File]::Create("test_src\large\150mb.bin")
$buffer = New-Object byte[] (10MB)
$rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
for ($i = 1; $i -le 15; $i++) {
    $rng.GetBytes($buffer)
    $fs.Write($buffer, 0, $buffer.Length)
}
$fs.Close()

# Create performance test data
Create-SmallFiles
Create-MediumFiles
Create-LargeFiles

# Calculate total size
Write-Host ""
Write-Host "Test data creation complete!" -ForegroundColor Green

$basicSize = (Get-ChildItem test_src -Recurse | Measure-Object -Property Length -Sum).Sum / 1MB
$perfSize = (Get-ChildItem perf_test -Recurse | Measure-Object -Property Length -Sum).Sum / 1MB

Write-Host "Basic test data size: $([math]::Round($basicSize, 2)) MB"
Write-Host "Performance test data size: $([math]::Round($perfSize, 2)) MB"
Write-Host ""
Write-Host "File counts:"
Write-Host "  Small files: $((Get-ChildItem perf_test\small -File).Count)"
Write-Host "  Medium files: $((Get-ChildItem perf_test\medium -File).Count)"
Write-Host "  Large files: $((Get-ChildItem perf_test\large -File).Count)"