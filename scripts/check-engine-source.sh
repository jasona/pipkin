#!/usr/bin/env bash
# Print the full source revision only after validating the release engine pin.
# PIPKIN_ENGINE_DEV=1 explicitly allows dirty/unpinned development sources.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
pi=${1:?usage: check-engine-source.sh PI_CHECKOUT}
pin=$(<"$here/packaging/pi-engine-revision")
[[ "$pin" =~ ^[0-9a-f]{40}$ ]] || { echo "invalid Pi engine revision pin" >&2; exit 1; }
revision=$(git -C "$pi" rev-parse --verify HEAD 2>/dev/null) || {
  echo "engine sources must be an identifiable Git checkout" >&2; exit 1;
}
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || { echo "invalid engine source revision" >&2; exit 1; }
if [ "${PIPKIN_ENGINE_DEV:-0}" != 1 ]; then
  [ "$revision" = "$pin" ] || {
    echo "engine revision $revision does not match release pin $pin; checkout the pin or explicitly use PIPKIN_ENGINE_DEV=1" >&2; exit 1;
  }
  [ -z "$(git -C "$pi" status --porcelain --untracked-files=normal)" ] || {
    echo "engine source checkout is dirty; use clean pinned sources or explicitly use PIPKIN_ENGINE_DEV=1" >&2; exit 1;
  }
else
  echo "warning: staging a development engine; this is not release qualification" >&2
fi
printf '%s\n' "$revision"
