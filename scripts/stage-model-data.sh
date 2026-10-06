#!/usr/bin/env bash
# Restore the immutable generated provider data paired with the engine source pin.
# --check verifies the input only; ENGINE_ROOT restores it into a staged/clean engine.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
pin=$(<"$here/packaging/pi-engine-revision")
[[ "$pin" =~ ^[0-9a-f]{40}$ ]] || { echo 'invalid engine pin' >&2; exit 1; }
archive="pi-model-data-$pin.tar.zst"
(cd "$here/packaging" && sha256sum --check --status "$archive.sha256") || {
  echo 'pinned generated model data is missing or has a checksum mismatch' >&2; exit 1;
}
if [ "${1:-}" = --check ]; then exit 0; fi
engine=${1:?usage: stage-model-data.sh ENGINE_ROOT or --check}
# The checked-in archive contains JSON only. Reject unexpected paths even with a matching hash.
while IFS= read -r path; do
  [[ "$path" = ./ || "$path" = ./.manifest.json || "$path" =~ ^\./[a-z0-9-]+\.json$ ]] || {
    echo "unexpected model data archive entry: $path" >&2; exit 1;
  }
done < <(tar --zstd -tf "$here/packaging/$archive")
mkdir -p "$engine/packages/ai/src/providers/data"
tar --zstd --no-same-owner --no-same-permissions -xf "$here/packaging/$archive" -C "$engine/packages/ai/src/providers/data"
node "$engine/packages/ai/scripts/check-model-data.ts"
