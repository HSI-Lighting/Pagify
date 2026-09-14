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
#
# **What is downloaded is checked before it is kept.** The library in each
# slice must hash to the line for it in third_party/CHECKSUMS.sha256; one that
# does not is deleted and the script fails. Found by audit: this used to be
# `curl | tar` with nothing between a release page and a native library that
# runs in-process with every document. Moving the pin means re-deriving the
# checksums from the new release's own assets and changing both on purpose.
set -euo pipefail

PINNED="chromium/7881"
TAG="${PDFIUM_TAG:-$PINNED}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${1:-$ROOT/third_party/pdfium}"
BASE="https://github.com/bblanchon/pdfium-binaries/releases/download"
CHECKSUMS="$ROOT/third_party/CHECKSUMS.sha256"

if [ "$TAG" != "$PINNED" ]; then
  echo "PDFIUM_TAG=$TAG is not the pinned $PINNED: the checksums in $CHECKSUMS are for the" >&2
  echo "pinned release, so anything fetched will be refused. Change both together." >&2
  exit 1
fi

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# The library file inside a slice, as CHECKSUMS names it (relative to third_party).
library_in() {
  case "$1" in
    pdfium-win-x64) echo "pdfium/$1/bin/pdfium.dll" ;;
    pdfium-linux-x64) echo "pdfium/$1/lib/libpdfium.so" ;;
    *) echo "pdfium/$1/lib/libpdfium.dylib" ;;
  esac
}

# Check a slice's library against its pinned checksum; delete the slice on a
# mismatch so nothing unverified is left where the loader looks.
verify_slice() {
  local slice="$1" rel expected actual
  rel="$(library_in "$slice")"
  expected="$(grep -E "  ${rel}\$" "$CHECKSUMS" | cut -d' ' -f1 || true)"
  if [ -z "$expected" ]; then
    echo "        REFUSED — no checksum for $rel in $CHECKSUMS" >&2
    rm -rf "$DEST/$slice"
    return 1
  fi
  if [ ! -f "$ROOT/third_party/$rel" ]; then
    echo "        REFUSED — $rel is not in the archive" >&2
    rm -rf "$DEST/$slice"
    return 1
  fi
  actual="$(sha256_of "$ROOT/third_party/$rel")"
  if [ "$actual" != "$expected" ]; then
    echo "        REFUSED — $rel hashes to $actual, expected $expected" >&2
    rm -rf "$DEST/$slice"
    return 1
  fi
  echo "        verified $rel"
}

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
  verify_slice "$slice"
done

echo
echo "Slices present:"
for slice in "$DEST"/*/; do
  name="$(basename "$slice")"
  lib="$(find "$slice" -maxdepth 2 -name 'libpdfium.*' -o -maxdepth 2 -name 'pdfium.dll' 2>/dev/null | head -1)"
  printf '  %-28s %s\n' "$name" "${lib:-MISSING}"
done
