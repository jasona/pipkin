#!/usr/bin/env bash
# Verify the default-release subagent overlay against the immutable Pi pin, without touching its checkout.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
pi=${1:-"$here/../pi-fork/pi"}
revision=$(<"$here/packaging/pi-engine-revision")
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
# Do not use the source working tree (which may contain unrelated owner edits).
git -C "$pi" archive "$revision" | tar -C "$stage" -xf -
git -C "$stage" init -q
(
  cd "$stage"
  git apply --check "$here/packaging/pi-onboarding.patch"
  git apply "$here/packaging/pi-onboarding.patch"
  git apply --check "$here/packaging/pi-subagents.patch"
  git apply "$here/packaging/pi-subagents.patch"
)
for file in subagents.ts subagents-provider.ts; do
  install -m 0644 "$here/packaging/pi-subagents/$file" "$stage/packages/coding-agent/src/experimental/services/$file"
done
grep -Fq 'registry.install(Subagent)' "$stage/packages/coding-agent/src/experimental/session-worker.ts"
grep -Fq 'createSubagentsServiceFacet' "$stage/packages/coding-agent/src/experimental/services/worker.ts"
grep -Fq 'defineService<Subagents>("pi.subagents")' "$stage/packages/coding-agent/src/experimental/services/subagents.ts"
grep -Fq 'git apply --check "$here/packaging/pi-subagents.patch"' "$here/scripts/build-engine.sh"
grep -Fq 'pipkinSubagentsSha256' "$here/scripts/build-engine.sh"
if grep -Fq 'PIPKIN_SUBAGENTS_DRAFT' "$here/scripts/build-engine.sh"; then
  echo 'release staging still gates native subagents behind a draft flag' >&2; exit 1
fi
echo "default-release subagent overlay applies to $revision"
