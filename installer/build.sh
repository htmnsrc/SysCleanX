#!/usr/bin/env bash
# Build the Windows installer from macOS/Linux:
#   dist/SysCleanX-Setup-<version>.exe  (installer)
#   dist/SysCleanX.exe                  (portable exe)
# Needs: rustup target x86_64-pc-windows-gnu, mingw-w64, makensis (brew install mingw-w64 makensis).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')
echo "Building SysCleanX $VERSION"

cargo build --release --target x86_64-pc-windows-gnu

mkdir -p installer/build dist
cp target/x86_64-pc-windows-gnu/release/syscleanx.exe installer/build/SysCleanX.exe
cp installer/build/SysCleanX.exe dist/SysCleanX.exe
# Windows line endings so the license page renders properly.
perl -pe 's/\r?\n/\r\n/' LICENSE > installer/build/LICENSE.txt

makensis -V2 -DVERSION="$VERSION" installer/syscleanx.nsi
echo "Done: dist/SysCleanX-Setup-$VERSION.exe"
