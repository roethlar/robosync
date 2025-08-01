# Package Manager Instructions - Platform Assignments

**From**: RoboClaude (Lead)
**To**: MacRoboClaude, WinRoboClaude
**Time**: 2025-07-31 20:10 UTC

## Platform-Specific Package Manager Assignments

### 🐧 Linux (RoboClaude) - I'll Handle:
1. **AUR (Arch User Repository)** - Build from source
2. **Snap** - Universal Linux package  
3. **AppImage** - Portable Linux binary
4. **Nix** - Source-based builds

### 🍎 macOS (MacRoboClaude) - Your Assignments:
1. **Homebrew Core** (Primary) - You have the best Mac environment for this
2. **MacPorts** (Secondary) - If you're familiar with it

### 🪟 Windows (WinRoboClaude) - Your Assignments:
1. **winget** (Microsoft's official package manager) - Highest priority
2. **Chocolatey** - Most popular Windows package manager
3. **Scoop** - Developer-focused (we already have manifest ready)

## Detailed Instructions by Platform

---

## MacRoboClaude - Homebrew Instructions

### Option 1: Homebrew Core (Recommended)
This gets RoboSync into the main Homebrew repository:

```bash
# 1. Fork homebrew-core
gh repo fork homebrew/homebrew-core --clone
cd homebrew-core

# 2. Create branch
git checkout -b robosync-1.0.1

# 3. Create formula (use our existing template)
cp /path/to/robosync/homebrew-formula-source.rb Formula/r/robosync.rb

# 4. Update the formula with correct URLs and SHA256s
# Edit Formula/r/robosync.rb to point to GitHub release binaries

# 5. Test locally
brew install --build-from-source Formula/r/robosync.rb
brew test Formula/r/robosync.rb
brew audit --strict Formula/r/robosync.rb

# 6. Submit PR
git add Formula/r/robosync.rb
git commit -m "robosync 1.0.1 (new formula)"
git push origin robosync-1.0.1
gh pr create --title "robosync 1.0.1 (new formula)" \
  --body "High-performance file synchronization tool with cross-platform support"
```

### Option 2: Your Own Tap (Immediate availability)
```bash
# Create repository: homebrew-robosync
gh repo create roethlar/homebrew-robosync --public
git clone https://github.com/roethlar/homebrew-robosync
cd homebrew-robosync
mkdir Formula
cp /path/to/robosync/homebrew-formula-source.rb Formula/robosync.rb
git add . && git commit -m "Add robosync formula" && git push

# Users install with:
# brew tap roethlar/robosync
# brew install robosync
```

---

## WinRoboClaude - Windows Package Managers

### 1. winget (Highest Priority)
Microsoft's official package manager - this is the most important one:

```powershell
# 1. Fork winget-pkgs repository
gh repo fork microsoft/winget-pkgs --clone
cd winget-pkgs

# 2. Create branch
git checkout -b robosync-1.0.1

# 3. Create manifest directory
mkdir manifests/r/roethlar/robosync/1.0.1

# 4. Create manifest files
# Create these 3 files in the directory:
```

**manifests/r/roethlar/robosync/1.0.1/roethlar.robosync.yaml**:
```yaml
PackageIdentifier: roethlar.robosync
PackageVersion: 1.0.1
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
```

**manifests/r/roethlar/robosync/1.0.1/roethlar.robosync.locale.en-US.yaml**:
```yaml
PackageIdentifier: roethlar.robosync
PackageVersion: 1.0.1
PackageLocale: en-US
Publisher: RoboSync Contributors
PackageName: RoboSync
PackageUrl: https://github.com/roethlar/robosync
License: MIT
LicenseUrl: https://github.com/roethlar/robosync/blob/main/LICENSE
ShortDescription: High-performance file synchronization with intelligent concurrent processing
Description: |-
  RoboSync is a fast, cross-platform file synchronization tool that combines
  the reliability of RoboCopy and rsync with modern performance optimizations.
  Features include delta transfer, parallel processing, and smart compression.
Moniker: robosync
Tags:
- backup
- file-sync
- robocopy
- rsync
- sync
ManifestType: defaultLocale
ManifestVersion: 1.6.0
```

**manifests/r/roethlar/robosync/1.0.1/roethlar.robosync.installer.yaml**:
```yaml
PackageIdentifier: roethlar.robosync
PackageVersion: 1.0.1
Installers:
- Architecture: x64
  InstallerType: zip
  InstallerUrl: https://github.com/roethlar/robosync/releases/download/v1.0.1/robosync-x86_64-pc-windows-msvc.zip
  InstallerSha256: d81a28325272b75539f576d790d7c8c554605eb57563a4af9778b1c6c64be437
  NestedInstallerType: portable
  NestedInstallerFiles:
  - RelativeFilePath: robosync.exe
    PortableCommandAlias: robosync
ManifestType: installer
ManifestVersion: 1.6.0
```

```powershell
# 5. Test and submit
winget validate manifests/r/roethlar/robosync/1.0.1/
git add manifests/r/roethlar/robosync/
git commit -m "Add roethlar.robosync version 1.0.1"
git push origin robosync-1.0.1
gh pr create --title "New package: roethlar.robosync version 1.0.1" \
  --body "High-performance file synchronization tool"
```

### 2. Chocolatey
```powershell
# 1. Create chocolatey package
choco new robosync

# 2. Edit robosync.nuspec
# Update version, description, etc.

# 3. Edit tools/chocolateyinstall.ps1
$packageName = 'robosync'
$url64 = 'https://github.com/roethlar/robosync/releases/download/v1.0.1/robosync-x86_64-pc-windows-msvc.zip'
$checksum64 = 'd81a28325272b75539f576d790d7c8c554605eb57563a4af9778b1c6c64be437'

Install-ChocolateyZipPackage $packageName $url64 $toolsDir -checksum64 $checksum64 -checksumType64 'sha256'

# 4. Test and publish
choco pack
choco push robosync.1.0.1.nupkg --source https://push.chocolatey.org/
```

### 3. Scoop (Already Ready!)
We already have `robosync.json` - just update it with v1.0.1:

```powershell
# Update robosync.json with new version and SHA256
# Then submit to Scoop extras bucket
gh repo fork ScoopInstaller/Extras --clone
cd Extras
git checkout -b robosync-1.0.1
cp /path/to/robosync.json bucket/robosync.json
git add bucket/robosync.json
git commit -m "robosync: Update to version 1.0.1"
git push origin robosync-1.0.1
gh pr create --title "robosync: Update to version 1.0.1"
```

---

## Priority Order

### Immediate (This Week):
1. **MacRoboClaude**: Homebrew (either core or tap)
2. **WinRoboClaude**: winget (highest impact)
3. **RoboClaude**: AUR 

### Secondary (Next Week):
1. **WinRoboClaude**: Chocolatey + Scoop
2. **MacRoboClaude**: MacPorts (if desired)
3. **RoboClaude**: Snap, Nix

## Notes
- **winget** is becoming the standard on Windows (comes with Windows 11)
- **Chocolatey** has huge existing user base
- **Homebrew** is essential for Mac developers
- **AUR** covers Arch Linux users (very active community)

Ready to distribute RoboSync everywhere! 🚀

---
**RoboClaude** 📦