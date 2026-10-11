#!/usr/bin/env bash
# FORK(no-cloud): guard AI Workstation (AIW)'s "no network of its own" guarantee (NO_CLOUD.md).
#
# Fails if
#   1. the root package's normal (non-dev, non-build) dependency tree links an
#      HTTP/WebSocket/TLS client or server library, or tokio with its `net`
#      feature; or
#   2. src/ uses std::net sockets or shells out to curl/wget.
#
# Usage: scripts/check-no-network.sh   (from anywhere inside the repo)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

toolchain="${ACC_TOOLCHAIN:-+1.99.0}"
banned_crates=(ureq reqwest hyper axum tungstenite h2 rustls native-tls openssl curl isahc surf attohttpc)
banned_source='std::net([^A-Za-z0-9_]|$)|TcpStream|TcpListener|UdpSocket|Command::new\("(curl|wget)"\)'

failures=0
fail() {
    printf 'FAIL: %s\n' "$1" >&2
    failures=$((failures + 1))
}

echo "== Dependency tree (cargo ${toolchain} tree -e normal -p ai-workstation --locked)"
if ! tree="$(cargo "$toolchain" tree -e normal -p ai-workstation --locked \
    --prefix none --format '{p} [{f}]' 2>&1)"; then
    printf '%s\n' "$tree" >&2
    fail "cargo tree failed (is Cargo.lock up to date?)"
    tree=""
fi
# One "name vX.Y.Z [features]" line per package; dedupe repeated subtrees.
packages="$(printf '%s\n' "$tree" | sed -e 's/ (\*)$//' | sort -u)"
echo "$(printf '%s\n' "$packages" | grep -c . || true) distinct packages linked into the root package"

for crate in "${banned_crates[@]}"; do
    # The crate itself or a family member (hyper-util, rustls-webpki,
    # openssl-sys, curl-sys, tokio-tungstenite, async-tungstenite, ...).
    hits="$(printf '%s\n' "$packages" | awk -v c="$crate" '{
        n = $1
        if (n == c || index(n, c "-") == 1 || n ~ ("-" c "$")) print
    }')"
    if [[ -n "$hits" ]]; then
        fail "network crate '${crate}' is linked:"
        printf '%s\n' "$hits" | sed 's/^/      /' >&2
    fi
done

tokio_lines="$(printf '%s\n' "$packages" | awk '$1 == "tokio"')"
if [[ -n "$tokio_lines" ]] && printf '%s\n' "$tokio_lines" | grep -Eq '[[,]net[],]'; then
    fail "tokio is linked with its 'net' feature: ${tokio_lines}"
fi

echo "== Sources (src/)"
if src_hits="$(grep -rnE "$banned_source" src)"; then
    fail "network APIs used in src/:"
    printf '%s\n' "$src_hits" | sed 's/^/      /' >&2
fi

if ((failures > 0)); then
    printf '\ncheck-no-network: %d problem(s). AIW must not open network connections itself; see NO_CLOUD.md.\n' "$failures" >&2
    exit 1
fi

cat <<'EOF'
OK: no HTTP/WebSocket/TLS client or server crate, no tokio `net`, and no
std::net sockets or curl/wget in src/.

Allowed (by design, see NO_CLOUD.md):
  - `gh` and `git` (fetch/push/clone) run only when the user starts a GitHub or
    Git action: posting an issue comment, the explicit PR badge refresh key
    (R / leader r), the issue fixer, and the syntax-parser download.
  - Agents (Claude, Codex, Gemini, Cursor, Pi, ...) reach their own providers; that
    is the agents' business, not AIW's.
  - Unix-domain sockets (tmux, AIW's own IPC in src/ipc.rs) and pipes to local
    child processes (e.g. the Codex settings proxy) are local, not network.
EOF
