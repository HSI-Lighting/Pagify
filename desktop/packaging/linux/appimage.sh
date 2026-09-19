#!/usr/bin/env bash
# Build Pagify as an AppImage.
#
# The same nesting problem as macOS in a different costume: PDFium is dlopen'd,
# so it has to travel with the binary and be found at runtime. There is no
# signing to get wrong here, but there *is* a search path — the library sits
# next to the executable inside the AppDir, which is the first place
# pagify_shell::pdfium looks.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APPDIR="$ROOT/target/Pagify.AppDir"
DYLIB="$ROOT/third_party/pdfium/pdfium-linux-x64/lib/libpdfium.so"

[ -f "$DYLIB" ] || { echo "no Linux PDFium — run tools/fetch_pdfium.sh" >&2; exit 1; }

# What goes into the AppImage is what the checksums say it is, the
# dependencies are clear of known advisories, and nothing in the binary opens
# a socket. The macOS bundle has always run these (see
# packaging/macos/bundle.sh); Linux releases did not, and the audit found it
# (M-6). Each script fails closed, so a non-zero exit stops the build here.
echo "==> verify third_party"
"$ROOT/tools/verify_third_party.sh"

echo "==> audit dependencies"
"$ROOT/tools/audit.sh"

echo "==> no sockets"
"$ROOT/tools/no_sockets.sh"

cargo build --release --locked -p pagify_app

rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
cp "$ROOT/target/release/pagify_app" "$APPDIR/usr/bin/pagify"
cp "$DYLIB" "$APPDIR/usr/bin/libpdfium.so"
cp "$ROOT/assets/pagify-logo.png" "$APPDIR/pagify.png"

cat > "$APPDIR/pagify.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Pagify
Exec=pagify %f
Icon=pagify
Categories=Office;Viewer;Graphics;
MimeType=application/pdf;
DESKTOP

cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/pagify" "$@"
RUN
chmod +x "$APPDIR/AppRun"

if command -v appimagetool >/dev/null 2>&1; then
  appimagetool "$APPDIR" "$ROOT/target/Pagify-x86_64.AppImage"
  echo "==> $ROOT/target/Pagify-x86_64.AppImage"
else
  echo "==> AppDir built at $APPDIR"
  echo "    appimagetool is not installed, so no .AppImage was produced."
fi
