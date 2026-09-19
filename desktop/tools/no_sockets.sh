#!/usr/bin/env bash
# Check that nothing in Pagify opens a network connection.
#
# "Pagify makes no network connections" is a sentence the security posture
# relies on: the one socket the program had — the timestamp verb's HTTP client
# over TcpStream — went with the SM signatures plan (§8a), and a claim about
# the whole program deserves a check on the whole program rather than a
# comment in one module. Two checks, both must pass:
#
#   1. No source file in the engine or the desktop crates names a socket type
#      or a network module — std's, tokio's, async-std's, libc's, mio's.
#   2. No crate in the dependency graph of the shipped binaries is a network
#      client, server, TLS stack or async runtime.
#
# Release builds run it (see packaging/macos/bundle.sh); run it by hand after
# adding a dependency or a module.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENGINE="$ROOT/../rust/pdf_core"
failed=0

# -- 1. the source ------------------------------------------------------------
#
# Word-bounded, so `hyperlink` is not `hyper` and `unsafe_socket_free` is
# not a socket. `grep -w` treats `::` as a boundary, which is what is wanted:
# `std::net::TcpStream` matches on `TcpStream` and on `std::net`.
PATTERN='TcpStream|TcpListener|UdpSocket|UnixStream|UnixListener|UnixDatagram|SocketAddr|std::net|std::os::unix::net|tokio::net|async_std::net|mio::net|socket2|libc::socket|libc::connect|libc::bind|libc::listen|libc::accept'
echo "==> no socket in the source"
hits="$(grep -rEnw --include='*.rs' "$PATTERN" \
  "$ENGINE/src" "$ENGINE/examples" "$ENGINE/tests" "$ROOT/crates" 2>/dev/null || true)"
if [ -n "$hits" ]; then
  echo "$hits" >&2
  echo "a socket, or a network module, is named in the source above" >&2
  failed=1
fi

# -- 2. the dependency graph --------------------------------------------------
#
# Normal dependencies only — what is linked into the shipped binaries — for
# the desktop workspace (which brings the engine with it) and for the engine
# on its own, since it is also built alone.
DENY='^(reqwest|ureq|hyper|hyper-util|hyper-rustls|hyper-tls|h2|h3|curl|curl-sys|isahc|attohttpc|minreq|surf|ehttp|ewebsock|tungstenite|tokio-tungstenite|websocket|quinn|tokio|async-std|smol|mio|socket2|native-tls|rustls|rustls-native-certs|openssl|openssl-sys|webpki|webpki-roots|http|httparse|http-body|trust-dns-resolver|hickory-resolver)$'
echo "==> no network crate in the graph"
for tree in "$ROOT" "$ENGINE"; do
  # cargo tree's exit status is checked explicitly. Found by audit: when it
  # failed (bad lockfile, missing manifest, network-less index) the pipeline
  # saw empty output, found nothing, and printed success — a gate that fails
  # open is worse than no gate.
  err="$(mktemp)"
  if ! tree_out="$( (cd "$tree" && cargo tree -e normal --prefix none) 2>"$err" )"; then
    echo "in $tree:" >&2
    echo "cargo tree failed - the dependency graph could not be read, so nothing here" >&2
    echo "was checked and this gate cannot pass:" >&2
    sed 's/^/  /' "$err" >&2
    rm -f "$err"
    failed=1
    continue
  fi
  rm -f "$err"
  found="$(printf '%s\n' "$tree_out" \
    | awk '{print $1}' | sort -u | grep -E "$DENY" || true)"
  if [ -n "$found" ]; then
    echo "in $tree:" >&2
    echo "$found" | sed 's/^/  /' >&2
    echo "a network crate is in the dependency graph of the shipped binaries" >&2
    failed=1
  fi
done

if [ "$failed" -ne 0 ]; then
  exit 1
fi
echo "no sockets: the source names none, the graph carries none"
