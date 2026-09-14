#!/usr/bin/env bash
# Check the dependency lockfile against the RustSec advisory database.
#
# Fails on any vulnerability not deliberately accepted in .cargo/audit.toml;
# unmaintained-crate warnings are printed and do not fail. Release builds run
# it (see packaging/macos/bundle.sh); run it by hand after touching
# Cargo.lock. Security audit L6/L7: there was no gate, and the advisory that
# is accepted was accepted by nobody.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! cargo audit --version >/dev/null 2>&1; then
  echo "cargo-audit is not installed:  cargo install cargo-audit --locked" >&2
  exit 2
fi
cargo audit
