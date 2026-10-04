#!/usr/bin/env bash
# Start a throwaway Pi experimental server for Pipkin's conformance tests.
#
# Everything lives under one scratch root, never your real ~/.pi: the socket directory,
# the logical server id, and the agent directory that holds session storage. PI_OFFLINE=1
# keeps the server from opening any remote relay. No model provider is configured, so no
# credentials are used and nothing is sent anywhere.
#
# Requires the Pi checkout's dependencies to be installed (npm ci there) and Node >= 22.19.
#
#   scripts/pi-test-server.sh [ROOT]            # runs in the foreground; Ctrl-C stops it
#
# It prints the variables the opt-in conformance tests read:
#   eval "$(scripts/pi-test-server.sh ROOT --print-env)"
#   cargo test -p pipkin-app --release real_pi -- --ignored --nocapture
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PI_REPO="${PI_REPO:-$(cd "$HERE/../../pi" && pwd)}"
ROOT="${1:-$(mktemp -d "${TMPDIR:-/tmp}/pipkin-pi-XXXXXX")}"
# A fixed canonical UUIDv4, so tests know which logical server to expect.
SERVER_ID="${PI_SERVER_ID:-5f0c7b1e-2d4a-4f6b-9a3e-1c8d7e6f5a40}"

mkdir -p "$ROOT/server" "$ROOT/agent"
chmod 700 "$ROOT" "$ROOT/server"

if [[ "${2:-}" == "--print-env" ]]; then
  printf 'export PIPKIN_REAL_PI_DIR=%q\nexport PIPKIN_REAL_PI_SERVER_ID=%q\n' "$ROOT/server" "$SERVER_ID"
  exit 0
fi

if [[ ! -d "$PI_REPO/node_modules" ]]; then
  echo "Pi dependencies are not installed in $PI_REPO (run: npm ci)" >&2
  exit 1
fi

echo "pi repo:    $PI_REPO" >&2
echo "server dir: $ROOT/server" >&2
echo "server id:  $SERVER_ID" >&2
exec env \
  PI_EXPERIMENTAL=1 \
  PI_OFFLINE=1 \
  PI_SERVER_DIR="$ROOT/server" \
  PI_SERVER_ID="$SERVER_ID" \
  PI_CODING_AGENT_DIR="$ROOT/agent" \
  "$PI_REPO/pi-test.sh" server
