#!/bin/bash
# Compile the app's own editing sources for the host and run the wire probe.
#
# The `ffi` module is built on macOS as well as iOS precisely so this is
# possible: a bridge only compiled on the device is a bridge whose ownership
# contracts are only tested on the device.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

cargo build --manifest-path rust/pdf_core/Cargo.toml --profile release-ios --lib

OUT="$(mktemp -d)/wire-probe"
xcrun swiftc -O \
    -import-objc-header ios/Support/Pagify-Bridging-Header.h \
    -I ios/CPagifyCore/include \
    ios/Sources/PagifyEngine.swift \
    ios/Sources/PagifyDocument.swift \
    ios/Sources/PagifyEdit.swift \
    ios/Sources/SessionRecorder.swift \
    ios/Sources/TextSelection.swift \
    ios/Sources/HitTesting.swift \
    ios/Sources/LineStyle.swift \
    ios/Sources/AnnotationTool.swift \
    ios/Sources/AnnotationColors.swift \
    ios/Sources/ShapeStrokes.swift \
    ios/Sources/PagifyFont.swift \
    ios/Sources/TextLayout.swift \
    ios/Scripts/wire-probe/main.swift \
    rust/pdf_core/target/release-ios/libpdf_core.a \
    -o "$OUT"

"$OUT" "$ROOT"
