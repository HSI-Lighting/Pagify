#!/bin/bash
# Fetch the pinned PDFium for macOS (host tests) and both iOS slices.
#
# The tag is pinned, and the pin is not arbitrary: pdfium-render 0.9.3's
# `pdfium_latest` feature binds the chromium/7881 C API. Pair those bindings with
# a different PDFium and the first render fails to resolve a symbol — a crash far
# from the cause. The Android counterpart is tools/fetch_pdfium.ps1; keep the two
# on the same tag.
set -euo pipefail

TAG="chromium/7881"
ENCODED="chromium%2F7881"
DEST="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/third_party/pdfium"
BASE="https://github.com/bblanchon/pdfium-binaries/releases/download/$ENCODED"

mkdir -p "$DEST"
for build in pdfium-mac-arm64 pdfium-ios-simulator-arm64 pdfium-ios-device-arm64; do
    if [ -f "$DEST/$build/lib/libpdfium.dylib" ]; then
        echo "$build: already present"
        continue
    fi
    echo "fetching $build ($TAG)"
    curl -sSL --fail -o "$DEST/$build.tgz" "$BASE/$build.tgz"
    mkdir -p "$DEST/$build"
    tar xzf "$DEST/$build.tgz" -C "$DEST/$build"
    rm "$DEST/$build.tgz"
    echo "  -> $(tr '\n' ' ' < "$DEST/$build/VERSION")"
done
