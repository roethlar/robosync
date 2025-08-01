# URGENT: Winget Submission Required

**To: winroboclaude**
**From: robosync (Linux)**
**Priority: IMMEDIATE**

## Task: Submit RoboSync to Windows Package Manager (winget)

## Steps:

1. Fork winget-pkgs:
```powershell
gh repo fork microsoft/winget-pkgs --clone
cd winget-pkgs
```

2. Create manifest directory:
```powershell
mkdir manifests\r\roethlar\robosync\1.0.3
```

3. Create manifest files:

**roethlar.robosync.yaml**:
```yaml
PackageIdentifier: roethlar.robosync
PackageVersion: 1.0.3
PackageLocale: en-US
Publisher: roethlar
PackageName: RoboSync
License: MIT
ShortDescription: High-performance file synchronization with intelligent concurrent processing
Installers:
- Architecture: x64
  InstallerType: portable
  InstallerUrl: https://github.com/roethlar/robosync/releases/download/v1.0.3/robosync-x86_64-pc-windows-msvc.zip
  InstallerSha256: [GET SHA256 OF ZIP FILE]
ManifestType: singleton
ManifestVersion: 1.0.0
```

4. Validate manifest:
```powershell
winget validate manifests\r\roethlar\robosync\1.0.3
```

5. Commit and push:
```powershell
git add .
git commit -m "New package: roethlar.robosync version 1.0.3"
git push origin roethlar-robosync-1.0.3
```

6. Create PR:
```powershell
gh pr create --title "New package: roethlar.robosync version 1.0.3" `
  --body "High-performance file synchronization tool"
```

## Required:
- Get SHA256 of robosync-x86_64-pc-windows-msvc.zip from release
- Ensure Windows binary exists at release URL

**COMPLETE THIS IMMEDIATELY**