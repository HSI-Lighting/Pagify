#!/bin/bash
# Build the shared engine for whichever slice Xcode is currently building.
#
# The port brief's rule stands: one crate, two platform bridges. This script is
# the whole of "how does Xcode get a Rust library" — there is no checked-in
# binary to go stale.
set -euo pipefail

# Xcode's PATH is not a login shell's, and rustup lives in the user's home.
export PATH="$HOME/.cargo/bin:$PATH"

CRATE="$SRCROOT/../rust/pdf_core"

case "${PLATFORM_NAME}" in
    iphonesimulator) TARGET="aarch64-apple-ios-sim" ;;
    iphoneos)        TARGET="aarch64-apple-ios" ;;
    *)
        echo "error: pdf_core has no Rust target for ${PLATFORM_NAME}" >&2
        exit 1
        ;;
esac

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo is not on PATH. Install rustup and the iOS targets:" >&2
    echo "  rustup target add aarch64-apple-ios aarch64-apple-ios-sim" >&2
    exit 1
fi

# `release-ios` rather than `release`: the release profile turns LTO on, which
# drops the #[no_mangle] exports when the output is a static archive. The reasons
# are in rust/pdf_core/Cargo.toml next to the profile.
cargo build \
    --manifest-path "$CRATE/Cargo.toml" \
    --profile release-ios \
    --target "$TARGET" \
    --lib

ARCHIVE="$CRATE/target/$TARGET/release-ios/libpdf_core.a"

# Checked here rather than left to the linker: "undefined symbol
# _pagify_open_document" 200 lines into a link log is a much worse way to find
# out that the profile stripped the bridge.
EXPORTS=$(nm -g --defined-only "$ARCHIVE" 2>/dev/null | grep -c "_pagify_" || true)
if [ "$EXPORTS" -eq 0 ]; then
    echo "error: $ARCHIVE exports no pagify_* symbols — check that profile.release-ios still sets lto = false" >&2
    exit 1
fi
echo "pdf_core: $TARGET, $EXPORTS exported entry points"
