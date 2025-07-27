#!/bin/bash
# Build release binaries for multiple platforms

set -e

echo "Building RoboSync release binaries..."

# Create release directory
mkdir -p releases

# Get version from Cargo.toml
VERSION=$(grep "^version" Cargo.toml | sed 's/version = "\(.*\)"/\1/')

# Build for current platform
echo "Building for current platform..."
cargo build --release
CURRENT_TARGET=$(rustc -vV | sed -n 's/host: //p')

# Copy current platform binary
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    cp target/release/robosync.exe "releases/robosync-v${VERSION}-${CURRENT_TARGET}.exe"
else
    cp target/release/robosync "releases/robosync-v${VERSION}-${CURRENT_TARGET}"
    chmod +x "releases/robosync-v${VERSION}-${CURRENT_TARGET}"
fi

echo "Release binaries built in ./releases/"
echo ""
echo "To build for other platforms, you can use:"
echo "  cargo build --release --target x86_64-pc-windows-gnu"
echo "  cargo build --release --target x86_64-apple-darwin"
echo "  cargo build --release --target x86_64-unknown-linux-gnu"
echo ""
echo "Note: Cross-compilation requires additional setup."