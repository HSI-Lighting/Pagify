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
# Set by install.sh, and by nothing else: this bundle is for the machine that
# builds it. **A bundle that ships is signed with a Developer ID, or it is not
# built.** The root certificate the signature check pins is compiled into the
# executable, and a pin inside an unsigned executable is a file anybody can
# patch; only a Developer ID signature turns a patched binary into one macOS
# refuses to run. So an unsigned bundle is refused here unless the caller
# says, in so many words, that it will not leave this machine.
LOCAL_ONLY="${PAGIFY_LOCAL_UNSIGNED_BUILD:-}"

ARCH="$(uname -m)"
case "$ARCH" in
  arm64) SLICE="pdfium-mac-arm64" ;;
  x86_64) SLICE="pdfium-mac-x64" ;;
  *) echo "unsupported arch $ARCH" >&2; exit 1 ;;
esac

# Only this workspace's own, checked tree — the fallback to the phone builds'
# copy in the parent repository went with the audit: nothing checked it.
DYLIB="$ROOT/third_party/pdfium/$SLICE/lib/libpdfium.dylib"
[ -f "$DYLIB" ] || { echo "no PDFium for $ARCH — run tools/fetch_pdfium.sh" >&2; exit 1; }

# What goes into the bundle is what the checksums say it is. A native library
# that runs in-process with every document is not worth shipping on trust.
echo "==> verify third_party"
"$ROOT/tools/verify_third_party.sh"

# And the dependencies against the advisory database: a release built on a
# known vulnerability nobody accepted is not a release.
echo "==> audit dependencies"
"$ROOT/tools/audit.sh"

# And that nothing in what is about to ship opens a socket. "Pagify makes no
# network connections" is a sentence the security posture rests on, and a
# sentence about the whole program is checked on the whole program.
echo "==> no sockets"
"$ROOT/tools/no_sockets.sh"

# Refused before the build, not after it: the answer does not depend on the
# binary, and a release build is minutes nobody should wait to be told.
if [ -z "$IDENTITY" ] && [ "$LOCAL_ONLY" != "1" ]; then
  echo "==> CODESIGN_IDENTITY is not set, and this is not a local install." >&2
  echo "    A Pagify bundle that ships is signed with a Developer ID; the certificate" >&2
  echo "    pinned inside the executable is only as safe as the signature that seals it." >&2
  echo "    Set CODESIGN_IDENTITY=\"Developer ID Application: ...\" to build one that" >&2
  echo "    ships, or use packaging/macos/install.sh for a copy that stays on this machine." >&2
  exit 1
fi

echo "==> build"
# `--locked`: the audit just above ran against `Cargo.lock` as it stands, so
# the build has to use exactly that lock, not update it and build something
# the gate never saw. Found by audit.
cargo build --release -p pagify_app --locked

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
  # Allowed through above only because PAGIFY_LOCAL_UNSIGNED_BUILD said so.
  echo "==> CODESIGN_IDENTITY not set — bundle built unsigned, for this machine only."
  echo "    It will run here and be refused on any other machine. Do not ship it."
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
# Signed, and signed with the right kind of certificate: an ad-hoc signature
# or a self-made one passes --verify and still lets a patched binary run
# wherever Gatekeeper is not looking. The chain must go up to Apple through a
# Developer ID Application certificate.
if ! codesign -dvv "$APP" 2>&1 | grep -q "^Authority=Developer ID Application:"; then
  echo "the bundle is signed, but not with a Developer ID Application certificate:" >&2
  codesign -dvv "$APP" 2>&1 | grep "^Authority=" >&2 || true
  rm -rf "$APP"
  exit 1
fi

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
