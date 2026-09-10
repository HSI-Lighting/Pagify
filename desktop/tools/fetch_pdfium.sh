#!/usr/bin/env bash
# Fetch the PDFium slices Pagify Desktop needs.
#
# The Pagify repo ships `tools/fetch_pdfium.ps1`, which is PowerShell and
# fetches the Apple slices only — so on a Mac, building for Windows or Linux
# has never been possible. Build plan phase 0 says to fix that in phase 0
# rather than phase 12, because "a missing Linux slice discovered in month
# seven is a bad month".
#
# PDFium is pinned to chromium/7881 to match `pdfium-render`'s `pdfium_latest`
# feature. Changing one without the other is an ABI mismatch that presents as a
# crash inside PDFium with no Rust frame to look at.
set -euo pipefail

TAG="${PDFIUM_TAG:-chromium/7881}"
DEST="${1:-$(cd "$(dirname "$0")/.." && pwd)/third_party/pdfium}"
BASE="https://github.com/bblanchon/pdfium-binaries/releases/download"

# slice directory : release asset
SLICES=(
  "pdfium-mac-arm64:pdfium-mac-arm64.tgz"
  "pdfium-mac-x64:pdfium-mac-x64.tgz"
  "pdfium-win-x64:pdfium-win-x64.tgz"
  "pdfium-linux-x64:pdfium-linux-x64.tgz"
)

mkdir -p "$DEST"
for entry in "${SLICES[@]}"; do
  slice="${entry%%:*}"
  asset="${entry##*:}"

  if [ -d "$DEST/$slice/lib" ] || [ -d "$DEST/$slice/bin" ]; then
    echo "have    $slice"
    continue
  fi

  echo "fetch   $slice ($TAG)"
  tmp="$(mktemp -d)"
  if ! curl -fsSL "$BASE/${TAG//\//%2F}/$asset" -o "$tmp/$asset"; then
    echo "        FAILED — $asset is not published for $TAG." >&2
    rm -rf "$tmp"
    continue
  fi
  mkdir -p "$DEST/$slice"
  tar -xzf "$tmp/$asset" -C "$DEST/$slice"
  rm -rf "$tmp"
done

echo
echo "Slices present:"
for slice in "$DEST"/*/; do
  name="$(basename "$slice")"
  lib="$(find "$slice" -maxdepth 2 -name 'libpdfium.*' -o -maxdepth 2 -name 'pdfium.dll' 2>/dev/null | head -1)"
  printf '  %-28s %s\n' "$name" "${lib:-MISSING}"
done
