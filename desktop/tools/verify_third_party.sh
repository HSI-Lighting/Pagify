#!/usr/bin/env bash
# Check every binary under third_party/ against third_party/CHECKSUMS.sha256.
#
# Fails on the first mismatch or missing file. Run it before a release build,
# and from CI; `tools/fetch_pdfium.sh` runs the PDFium part of it itself after
# every download, so a substituted release never gets as far as being loaded.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/third_party"

# The comment lines come off first: `--strict` rightly refuses to guess at a
# line it cannot read, and a comment is one.
if command -v sha256sum >/dev/null 2>&1; then
  grep -v '^#' CHECKSUMS.sha256 | sha256sum --check --strict -
elif command -v shasum >/dev/null 2>&1; then
  grep -v '^#' CHECKSUMS.sha256 | shasum -a 256 --check --strict -
else
  echo "neither sha256sum nor shasum is available" >&2
  exit 2
fi
