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
# Generated provider data comes from the immutable input paired with the source pin,
# not the owner's checkout or mutable public model catalogs.
if [ "${PIPKIN_ENGINE_DEV:-0}" = 1 ]; then
  node "$pi/packages/ai/scripts/check-model-data.ts"
else
  "$here/scripts/stage-model-data.sh" --check
fi
rm -rf "$out"
mkdir -p "$out"
# Stage only identified source files, not stale ignored build output or checkout-local extras.
if [ "${PIPKIN_ENGINE_DEV:-0}" = 1 ]; then
  git -C "$pi" ls-files --cached --others --exclude-standard -z |
    tar -C "$pi" --null --ignore-failed-read -T - -cf - | tar -C "$out" -xf -
else
  git -C "$pi" archive "$commit" | tar -C "$out" -xf -
fi
# The pinned upstream engine predates Pipkin's tested OAuth service. Apply the reviewed,
# first-party bridge to the staged copy only: never dirty the owner's Pi checkout or claim
# this is the unmodified pinned revision. Exact-context git apply fails on source drift.
bridge="$here/packaging/pi-onboarding.patch"
bridge_src="$here/packaging/pi-onboarding"
bridge_dest="$out/packages/coding-agent/src/experimental/services"
if [ "${PIPKIN_ENGINE_DEV:-0}" = 1 ]; then
  for file in provider-auth.ts provider-auth-provider.ts; do
    cmp "$bridge_src/$file" "$bridge_dest/$file" >/dev/null || {
      echo "development Pi checkout's $file differs from the reviewed packaging bridge" >&2; exit 1;
    }
  done
else
  # git apply searches for a parent Git worktree. A macOS staging path under dist/
  # would otherwise silently patch the Pipkin checkout instead of the engine copy.
  # Give the staged copy its own temporary Git boundary, then remove it before packaging.
  git -C "$out" init -q
  (cd "$out" && git apply --check "$bridge" && git apply "$bridge")
  rm -rf "$out/.git"
  install -m 0644 "$bridge_src/provider-auth.ts" "$bridge_dest/provider-auth.ts"
  install -m 0644 "$bridge_src/provider-auth-provider.ts" "$bridge_dest/provider-auth-provider.ts"
fi
# Ensure the patch really affected the staged worktree, not a parent checkout.
grep -Fq 'providerAuth: {' "$out/packages/coding-agent/src/experimental/server.ts" &&
  grep -Fq 'service: ProviderAuth,' "$bridge_dest/server.ts" || {
    echo "OAuth bridge was not applied to the staged engine" >&2; exit 1;
  }
bridge_hash=$(cat "$bridge" "$bridge_src/provider-auth.ts" "$bridge_src/provider-auth-provider.ts" | sha256sum | cut -d' ' -f1)
# Installed dependencies are build inputs; preserve workspace links within the staged tree.
cp -a "$pi/node_modules" "$out/node_modules"
while IFS= read -r -d '' dependencies; do
  relative=${dependencies#"$pi/"}
  mkdir -p "$out/$(dirname "$relative")"
  cp -a "$dependencies" "$out/$relative"
done < <(find "$pi/packages" -mindepth 2 -maxdepth 2 -type d -name node_modules -print0)
if [ "${PIPKIN_ENGINE_DEV:-0}" = 1 ]; then
  mkdir -p "$out/packages/ai/src/providers"
  cp -a "$pi/packages/ai/src/providers/data" "$out/packages/ai/src/providers/data"
  node "$out/packages/ai/scripts/check-model-data.ts"
else
  "$here/scripts/stage-model-data.sh" "$out"
fi
model_data_hash=$(sha256sum "$out/packages/ai/src/providers/data/.manifest.json" | cut -d' ' -f1)
pruned=false
if [ "${PIPKIN_ENGINE_KEEP_DEV:-0}" != 1 ]; then
  if (cd "$out" && npm prune --omit=dev --offline --no-audit --no-fund >/dev/null 2>&1); then
    pruned=true
  else
    echo "warning: could not prune dev dependencies; the engine is larger than needed" >&2
  fi
fi
cat > "$out/engine.json" <<JSON
{"name":"pi","version":"$commit","sourceRevision":"$commit","pipkinAuthBridgeSha256":"$bridge_hash","modelDataManifestSha256":"$model_data_hash","productionPruneSucceeded":$pruned,"development":$([ "${PIPKIN_ENGINE_DEV:-0}" = 1 ] && echo true || echo false),"protocol":$protocol,"minClient":"0.0.1","builtAt":"$(date -u +%Y-%m-%dT%H:%M:%SZ)"}
JSON
echo "staged engine $commit at $out ($(du -sh "$out" | cut -f1))"
