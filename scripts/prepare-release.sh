#!/bin/bash
# Script to prepare a new release of RoboSync

set -e

VERSION=$1
if [ -z "$VERSION" ]; then
    echo "Usage: $0 <version>"
    echo "Example: $0 1.0.1"
    exit 1
fi

echo "Preparing release v$VERSION..."

# Update version in Cargo.toml
echo "Updating Cargo.toml..."
sed -i "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml

# Update lock file
echo "Updating Cargo.lock..."
cargo update

# Run tests
echo "Running tests..."
cargo test

# Build release
echo "Building release..."
cargo build --release

# Update package files
echo "Updating package files..."

# Update PKGBUILD
sed -i "s/^pkgver=.*/pkgver=$VERSION/" PKGBUILD

# Update Homebrew formula
sed -i "s/version \".*\"/version \"$VERSION\"/" homebrew-formula-source.rb

# Update Scoop manifest
sed -i "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" robosync.json

# Create git tag
echo "Creating git tag..."
git add -A
git commit -m "Release v$VERSION"
git tag -a "v$VERSION" -m "Release v$VERSION"

echo "Release prepared! Next steps:"
echo "1. Push to GitHub: git push && git push --tags"
echo "2. Wait for CI to build binaries"
echo "3. Create GitHub release"
echo "4. Publish to crates.io: cargo publish"
echo "5. Update package manager submissions"