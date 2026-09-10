#!/usr/bin/env bash
# Build, bundle, sign and notarize Pagify.app.
#
# The order below is the whole point of this file. **The nested dylib is signed
# before the bundle that contains it**, because signing the outer bundle first
# seals a hash of its contents — re-signing something inside afterwards
# invalidates the outer signature, and the failure does not appear until
# Gatekeeper rejects the app on a machine that is not yours.
#
# Build plan §9 names this exactly: "this is where projects discover that the
# nested dylib was never signed."
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APP="$ROOT/target/Pagify.app"
IDENTITY="${CODESIGN_IDENTITY:-}"          # "Developer ID Application: ..."
PROFILE="${NOTARY_PROFILE:-}"              # `xcrun notarytool store-credentials`

ARCH="$(uname -m)"
case "$ARCH" in
  arm64) SLICE="pdfium-mac-arm64" ;;
  x86_64) SLICE="pdfium-mac-x64" ;;
  *) echo "unsupported arch $ARCH" >&2; exit 1 ;;
esac

DYLIB="$ROOT/third_party/pdfium/$SLICE/lib/libpdfium.dylib"
if [ ! -f "$DYLIB" ]; then
  # Fall back to the Pagify repo's vendored copy, which is where development
  # builds find it.
  DYLIB="$ROOT/../workspace/Pagify/third_party/pdfium/$SLICE/lib/libpdfium.dylib"
fi
[ -f "$DYLIB" ] || { echo "no PDFium for $ARCH — run tools/fetch_pdfium.sh" >&2; exit 1; }

echo "==> build"
cargo build --release -p pagify_app

echo "==> bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/pagify_app" "$APP/Contents/MacOS/Pagify"
# Beside the executable, which is the first place pagify_shell::pdfium looks.
cp "$DYLIB" "$APP/Contents/MacOS/libpdfium.dylib"
cp "$(dirname "$0")/Info.plist" "$APP/Contents/Info.plist"
cp "$(dirname "$0")/Pagify.icns" "$APP/Contents/Resources/Pagify.icns"

# The recognition models, in the first place `pagify_shell::models` looks.
#
# Without them a bundle still runs *here*, because that search ends at a path
# compiled in from `CARGO_MANIFEST_DIR` — so the gap is invisible on the machine
# that built it and total on every other one: Extract Text would refuse a scan
# with "no usable recognition models" and nothing would say why.
MODELS="$ROOT/third_party/ocr"
if [ -d "$MODELS" ]; then
  mkdir -p "$APP/Contents/Resources/ocr"
  cp "$MODELS"/*.rten "$APP/Contents/Resources/ocr/"
else
  echo "    warning: no recognition models at $MODELS — Extract Text will refuse scans"
fi

if [ -z "$IDENTITY" ]; then
  echo "==> CODESIGN_IDENTITY not set — bundle built unsigned."
  echo "    It will run locally and be refused on any other machine."
  exit 0
fi

echo "==> sign the nested dylib FIRST"
codesign --force --timestamp --options runtime \
         --sign "$IDENTITY" "$APP/Contents/MacOS/libpdfium.dylib"

echo "==> then the bundle"
codesign --force --timestamp --options runtime \
         --entitlements "$(dirname "$0")/entitlements.plist" \
         --sign "$IDENTITY" "$APP"

echo "==> verify"
codesign --verify --deep --strict --verbose=2 "$APP"

if [ -z "$PROFILE" ]; then
  echo "==> NOTARY_PROFILE not set — signed but not notarized."
  exit 0
fi

echo "==> notarize"
ZIP="$ROOT/target/Pagify.zip"
ditto -c -k --keepParent "$APP" "$ZIP"
xcrun notarytool submit "$ZIP" --keychain-profile "$PROFILE" --wait
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
echo "==> done: $APP"
