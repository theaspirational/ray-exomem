#!/usr/bin/env bash
# Local rebuild + redeploy for ray-exomem.
# Mirrors the live-test loop documented in CLAUDE.md.

set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> cargo build --release --bin ray-exomem"
cargo build --release --bin ray-exomem

echo "==> stopping running daemon (if any)"
pgrep -lf "ray-exomem serve" | awk '{print $1}' | xargs -r kill || true
sleep 1

echo "==> hardlinking new binary into ~/.local/bin"
ln -f target/release/ray-exomem "$HOME/.local/bin/ray-exomem"

echo "==> launching daemon"
set -a
# shellcheck disable=SC1091
source .env
set +a
nohup "$HOME/.local/bin/ray-exomem" serve --bind 127.0.0.1:9780 \
  --auth-provider google --google-client-id "$GOOGLE_CLIENT_ID" \
  --allowed-domains "$ALLOWED_DOMAINS" --database-url "$DATABASE_URL" \
  > /tmp/ray-exomem.log 2>&1 &
disown

sleep 2

echo "==> verifying liveness"
if curl -fsS http://127.0.0.1:9780/auth/info; then
  echo
  echo "==> ok — daemon up; logs at /tmp/ray-exomem.log"
else
  echo
  echo "!!  /auth/info did not respond — check /tmp/ray-exomem.log" >&2
  exit 1
fi
