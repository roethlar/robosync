#!/bin/bash
set -e

# Create release directory
mkdir -p release

echo "Building RoboSync for all platforms..."

# Linux x64 (static musl)
echo "Building Linux x64..."
cargo zigbuild --release --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/robosync release/robosync-linux-x64

# Linux ARM64 (static musl)
echo "Building Linux ARM64..."
cargo zigbuild --release --target aarch64-unknown-linux-musl
cp target/aarch64-unknown-linux-musl/release/robosync release/robosync-linux-arm64

# Windows x64
echo "Building Windows x64..."
if cargo zigbuild --release --target x86_64-pc-windows-gnu 2>/dev/null; then
    cp target/x86_64-pc-windows-gnu/release/robosync.exe release/robosync-windows-x64.exe
else
    echo "Windows build failed - this is normal on Linux. Use GitHub Actions for Windows builds."
fi

# macOS x64
echo "Building macOS x64..."
if cargo zigbuild --release --target x86_64-apple-darwin 2>/dev/null; then
    cp target/x86_64-apple-darwin/release/robosync release/robosync-macos-x64
else
    echo "macOS x64 build failed - this is normal on Linux."
fi

# macOS ARM64 (Apple Silicon)
echo "Building macOS ARM64..."
if cargo zigbuild --release --target aarch64-apple-darwin 2>/dev/null; then
    cp target/aarch64-apple-darwin/release/robosync release/robosync-macos-arm64
else
    echo "macOS ARM64 build failed - this is normal on Linux."
fi

# Create universal macOS binary
if [[ -f release/robosync-macos-x64 && -f release/robosync-macos-arm64 ]]; then
    echo "Creating universal macOS binary..."
    lipo -create release/robosync-macos-x64 release/robosync-macos-arm64 -output release/robosync-macos-universal || echo "lipo not available, skipping universal binary"
fi

# Create archives
echo "Creating archives..."
cd release

# Create tar.gz for each platform
for file in robosync-*; do
    if [[ -f "$file" ]]; then
        tar -czf "${file}.tar.gz" "$file"
        echo "Created ${file}.tar.gz"
    fi
done

# Create a zip for Windows (if it exists)
if [[ -f "robosync-windows-x64.exe" ]]; then
    zip "robosync-windows-x64.zip" "robosync-windows-x64.exe"
    echo "Created robosync-windows-x64.zip"
fi

cd ..

echo "All builds complete! Check the 'release' directory."
ls -lh release/