# Package Manager Submission Guide

## Current Status

✅ **Published:**
- crates.io: `cargo install robosync`

📦 **Ready for submission (source-based):**
- Homebrew Formula (homebrew-formula-source.rb)
- AUR PKGBUILD

⏳ **Waiting for binaries:**
- Scoop (needs Windows binary)
- Snap (needs snapcraft build)
- Nix (can work from source)

## Submission Instructions

### 1. Homebrew
See `homebrew-submission-guide.md` for detailed instructions.

Quick option - Create your own tap:
```bash
# Create repo: github.com/roethlar/homebrew-robosync
# Then users can:
brew tap roethlar/robosync
brew install robosync
```

### 2. AUR (Arch User Repository)
```bash
# 1. Create AUR account at https://aur.archlinux.org
# 2. Clone the AUR repository
git clone ssh://aur@aur.archlinux.org/robosync.git
cd robosync

# 3. Copy PKGBUILD
cp /path/to/robosync/PKGBUILD .

# 4. Generate .SRCINFO
makepkg --printsrcinfo > .SRCINFO

# 5. Test locally
makepkg -si

# 6. Push to AUR
git add PKGBUILD .SRCINFO
git commit -m "Initial commit: robosync 1.0.0"
git push
```

### 3. Scoop (Windows)
Once Windows binaries are available:
```bash
# Fork https://github.com/ScoopInstaller/Extras
# Add robosync.json to bucket/
# Update SHA256 in the manifest
# Submit PR
```

### 4. Snap
```bash
# Build snap package
snapcraft

# Upload to Snapcraft store
snapcraft upload robosync_1.0.0_amd64.snap
snapcraft release robosync 1.0.0 stable
```

### 5. Nix
```bash
# Submit to nixpkgs
# Fork https://github.com/NixOS/nixpkgs
# Add robosync.nix to pkgs/tools/filesystems/
# Submit PR
```

## Binary Distribution Strategy

For binary-based package managers, we need:
1. CI/CD to build binaries for all platforms
2. Signed releases for security
3. Automated SHA256 calculation

Current options:
1. Fix GitHub Actions CI to build releases
2. Use cargo-dist for automated distribution
3. Build binaries locally and upload manually

## Community Packages

Once established, the community often maintains packages for:
- Debian/Ubuntu (PPA)
- Fedora (COPR)
- openSUSE (OBS)
- Void Linux
- Alpine Linux