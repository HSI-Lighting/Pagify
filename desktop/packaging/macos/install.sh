#!/usr/bin/env bash
# Build Pagify and install it to ~/Applications, leaving exactly one copy.
#
# The staging bundle lives in `target/`, and macOS indexes that directory like
# any other — so a plain build-then-copy leaves *two* registered apps, and
# Launchpad shows both. Rebuilding repeatedly leaves a trail of them, some
# pointing at bundles that have since been replaced, which is what the
# no-entry-badge icons are.
#
# So this unregisters the staging copy after installing, and unregisters the
# previous install before replacing it. One app, one entry, every time.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="${PAGIFY_INSTALL_DIR:-$HOME/Applications}"
APP="$DEST/Pagify.app"
LSREG=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister

"$(dirname "$0")/bundle.sh"

echo "==> installing to $DEST"
mkdir -p "$DEST"

# Quit a running copy first: replacing a bundle under a live process leaves it
# running from a bundle that no longer exists.
if pgrep -f "Pagify.app/Contents/MacOS/Pagify" >/dev/null 2>&1; then
    echo "    quitting the running copy"
    pkill -f "Pagify.app/Contents/MacOS/Pagify" || true
    sleep 1
fi

[ -d "$APP" ] && "$LSREG" -u "$APP" 2>/dev/null || true
rm -rf "$APP"
cp -R "$ROOT/target/Pagify.app" "$APP"

# Unregister *and* delete the staging copy. macOS re-scans and re-registers
# any bundle it can still see, so an unregister alone lasts until the next
# scan and the duplicate quietly comes back; and a bundle deleted while still
# registered leaves a ghost entry behind until the database is next rebuilt —
# seen after a bundle-only build that was installed later.
"$LSREG" -u "$ROOT/target/Pagify.app" 2>/dev/null || true
rm -rf "$ROOT/target/Pagify.app"
"$LSREG" -f "$APP" 2>/dev/null || true

echo
echo "==> $APP"
"$LSREG" -dump 2>/dev/null | grep -oE "/[^ ]*Pagify\.app" | sort -u | while read -r p; do
    [ "$p" = "$APP" ] && continue
    case "$p" in
        *CoreSimulator*|*DerivedData*) echo "    note: an iOS build is also registered: $p" ;;
        *) echo "    warning: another desktop Pagify is registered: $p" ;;
    esac
done

echo "    open it from ~/Applications, or:  open -a \"$APP\" file.pdf"
