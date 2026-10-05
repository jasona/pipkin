#!/usr/bin/env bash
# Stage a self-contained Pi engine for Pipkin's installation layout.
#
#   scripts/build-engine.sh [PI_CHECKOUT [OUT_DIR]]     (default ../pi-fork/pi, ./dist/engine)
#
# The result is a directory holding the engine sources, its installed dependencies and an
# engine.json manifest. Pipkin finds it at <prefix>/lib/pipkin/engine, so the installed app needs
# no Pi checkout. It runs on the system Node (>= 22.19), which a package declares as a dependency.
# Needs the checkout's dependencies installed (npm ci there); pruning dev dependencies needs no network.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
pi=$(cd "${1:-$here/../pi-fork/pi}" && pwd)
out=${2:-$here/dist/engine}
protocol=8   # keep in step with PROTOCOL in crates/pipkin-app/src/install.rs

[ -d "$pi/node_modules" ] || { echo "no node_modules in $pi (run npm ci there)" >&2; exit 1; }
[ -f "$pi/pi-test.sh" ] || { echo "$pi is not a Pi checkout" >&2; exit 1; }
rm -rf "$out"
mkdir -p "$out"
# Copy the tree without version control, the evaluation suite and editor/nix extras.
tar -C "$pi" --exclude=.git --exclude=packages/evals --exclude=nix --exclude=.cache -cf - . | tar -C "$out" -xf -
if [ "${PIPKIN_ENGINE_KEEP_DEV:-0}" != 1 ]; then
  (cd "$out" && npm prune --omit=dev --offline --no-audit --no-fund >/dev/null 2>&1) \
    || echo "warning: could not prune dev dependencies; the engine is larger than needed" >&2
fi
commit=$(git -C "$pi" rev-parse --short HEAD 2>/dev/null || echo unknown)
cat > "$out/engine.json" <<JSON
{"name":"pi","version":"$commit","protocol":$protocol,"minClient":"0.0.1","builtAt":"$(date -u +%Y-%m-%dT%H:%M:%SZ)"}
JSON
echo "staged engine $commit at $out ($(du -sh "$out" | cut -f1))"
