# AUR Package Preparation - RoboClaude

**From**: RoboClaude
**Internal**: Linux package manager preparation
**Time**: 2025-07-31 20:12 UTC

## AUR Submission Plan

### 1. Update PKGBUILD for v1.0.1

```bash
# Maintainer: RoboSync Team <robosync@example.com>
pkgname=robosync
pkgver=1.0.1
pkgrel=1
pkgdesc="High-performance file synchronization with intelligent concurrent processing"
arch=('x86_64' 'aarch64')
url="https://github.com/roethlar/robosync"
license=('MIT')
depends=()
makedepends=('rust' 'cargo')
source=("$pkgname-$pkgver.tar.gz::https://github.com/roethlar/robosync/archive/v$pkgver.tar.gz")
sha256sums=('NEED_NEW_SOURCE_SHA256')

build() {
    cd "$srcdir/$pkgname-$pkgver"
    cargo build --release --locked
}

package() {
    cd "$srcdir/$pkgname-$pkgver"
    install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
    install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}
```

### 2. Get Source Tarball SHA256

```bash
curl -L -o robosync-1.0.1-source.tar.gz https://github.com/roethlar/robosync/archive/v1.0.1.tar.gz
sha256sum robosync-1.0.1-source.tar.gz
```

### 3. AUR Submission Process

```bash
# 1. Clone AUR repo (need AUR account first)
git clone ssh://aur@aur.archlinux.org/robosync.git
cd robosync

# 2. Copy updated PKGBUILD
cp /path/to/updated/PKGBUILD .

# 3. Generate .SRCINFO
makepkg --printsrcinfo > .SRCINFO

# 4. Test build locally
makepkg -si

# 5. Push to AUR
git add PKGBUILD .SRCINFO
git commit -m "Update to version 1.0.1"
git push
```

## Snap Package Preparation

### snapcraft.yaml
```yaml
name: robosync
version: '1.0.1'
summary: High-performance file synchronization
description: |
  RoboSync is a fast, cross-platform file synchronization tool that combines
  the reliability of RoboCopy and rsync with modern performance optimizations.

grade: stable
confinement: classic

base: core22

parts:
  robosync:
    plugin: rust
    source: https://github.com/roethlar/robosync.git
    source-tag: v1.0.1
    build-packages:
      - build-essential
      - pkg-config

apps:
  robosync:
    command: bin/robosync
```

### Build and Publish
```bash
snapcraft
snapcraft upload robosync_1.0.1_amd64.snap
snapcraft release robosync 1.0.1 stable
```

## AppImage Preparation

```bash
# Use existing binary
mkdir -p AppDir/usr/bin
cp target/release/robosync AppDir/usr/bin/

# Create desktop file
cat > AppDir/robosync.desktop << EOF
[Desktop Entry]
Type=Application
Name=RoboSync
Exec=robosync
Icon=robosync
Categories=Utility;
EOF

# Create AppImage
wget https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x appimagetool-x86_64.AppImage
./appimagetool-x86_64.AppImage AppDir robosync-1.0.1-x86_64.AppImage
```

## Timeline
- **Today**: Update PKGBUILD, get source SHA256
- **Tomorrow**: Submit to AUR
- **This week**: Snap and AppImage

All ready for Linux distribution! 🐧

---
**RoboClaude** 📦