# Simple PowerShell script to test raw copy performance
$source = "C:\Program Files (x86)\Steam\steamapps\common\Counter-Strike Global Offensive"
$dest = "H:\stuff\backup\steam\steamapps\test"

Write-Host "Testing simple copy performance..."
Write-Host "Source: $source"
Write-Host "Destination: $dest"

# Create destination
New-Item -ItemType Directory -Force -Path $dest | Out-Null

# Get files
$files = Get-ChildItem -Path $source -Recurse -File

Write-Host "Found $($files.Count) files"

# Measure copy time
$start = Get-Date

$jobs = foreach ($file in $files) {
    Start-Job -ScriptBlock {
        param($src, $dst, $root)
        $relative = $src.Substring($root.Length + 1)
        $destPath = Join-Path $dst $relative
        $destDir = Split-Path $destPath -Parent
        if (!(Test-Path $destDir)) {
            New-Item -ItemType Directory -Force -Path $destDir | Out-Null
        }
        Copy-Item -Path $src -Destination $destPath -Force
    } -ArgumentList $file.FullName, $dest, $source
}

# Wait for all jobs
$jobs | Wait-Job | Out-Null
$jobs | Remove-Job

$elapsed = (Get-Date) - $start
Write-Host "Completed in $($elapsed.TotalSeconds) seconds"