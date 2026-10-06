#!/usr/bin/env bash
# Stage a self-contained Pi engine for Pipkin's installation layout.
#
#   scripts/build-engine.sh [PI_CHECKOUT [OUT_DIR]]     (default ../pi-fork/pi, ./dist/engine)
#
# The result is a directory holding the engine sources, its installed dependencies and an
# engine.json manifest. Pipkin finds it at <prefix>/lib/pipkin/engine, so the installed app needs
# no Pi checkout. It runs on the system Node (>= 22.19), which a package declares as a dependency.
# Requires the clean revision in packaging/pi-engine-revision. PIPKIN_ENGINE_DEV=1 is an
# explicit non-release override. Needs installed dependencies; pruning needs no network.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
pi=$(cd "${1:-$here/../pi-fork/pi}" && pwd)
out=${2:-$here/dist/engine}
protocol=8   # keep in step with PROTOCOL in crates/pipkin-app/src/install.rs
commit=$("$here/scripts/check-engine-source.sh" "$pi")

[ -d "$pi/node_modules" ] || { echo "no node_modules in $pi (run npm ci there)" >&2; exit 1; }
[ -f "$pi/pi-test.sh" ] || { echo "$pi is not a Pi checkout" >&2; exit 1; }
# Provider JSON is generated/ignored, but required by the tracked source entry points.
# Validate it against the pinned contracts before touching a previous staged build.
[ -f "$pi/packages/ai/src/providers/data/.manifest.json" ] || {
  echo "missing generated model data; run npm run hydrate:model-data in the pinned Pi checkout" >&2; exit 1;
}
node "$pi/packages/ai/scripts/check-model-data.ts"
rm -rf "$out"
mkdir -p "$out"
# Stage only identified source files, not stale ignored build output or checkout-local extras.
if [ "${PIPKIN_ENGINE_DEV:-0}" = 1 ]; then
  git -C "$pi" ls-files --cached --others --exclude-standard -z |
    tar -C "$pi" --null --ignore-failed-read -T - -cf - | tar -C "$out" -xf -
else
  git -C "$pi" archive "$commit" | tar -C "$out" -xf -
fi
mkdir -p "$out/packages/ai/src/providers"
cp -a "$pi/packages/ai/src/providers/data" "$out/packages/ai/src/providers/data"
model_data_hash=$(sha256sum "$out/packages/ai/src/providers/data/.manifest.json" | cut -d' ' -f1)
# Installed dependencies are build inputs; preserve workspace links within the staged tree.
cp -a "$pi/node_modules" "$out/node_modules"
while IFS= read -r -d '' dependencies; do
  relative=${dependencies#"$pi/"}
  mkdir -p "$out/$(dirname "$relative")"
  cp -a "$dependencies" "$out/$relative"
done < <(find "$pi/packages" -mindepth 2 -maxdepth 2 -type d -name node_modules -print0)
if [ "${PIPKIN_ENGINE_KEEP_DEV:-0}" != 1 ]; then
  (cd "$out" && npm prune --omit=dev --offline --no-audit --no-fund >/dev/null 2>&1) \
    || echo "warning: could not prune dev dependencies; the engine is larger than needed" >&2
fi
cat > "$out/engine.json" <<JSON
{"name":"pi","version":"$commit","sourceRevision":"$commit","modelDataManifestSha256":"$model_data_hash","development":$([ "${PIPKIN_ENGINE_DEV:-0}" = 1 ] && echo true || echo false),"protocol":$protocol,"minClient":"0.0.1","builtAt":"$(date -u +%Y-%m-%dT%H:%M:%SZ)"}
JSON
echo "staged engine $commit at $out ($(du -sh "$out" | cut -f1))"
