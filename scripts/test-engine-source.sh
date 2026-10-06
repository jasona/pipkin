#!/usr/bin/env bash
# Disposable source/staging qualification; no owner checkout or dependencies are modified.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
mkdir -p "$root/app/scripts" "$root/app/packaging" "$root/pi/packages/example" "$root/pi/node_modules" "$root/pi/packages/ai/scripts" "$root/pi/packages/ai/src/providers/data"
cp "$here/scripts/"{check-engine-source,build-engine,stage-model-data}.sh "$root/app/scripts/"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
unset PIPKIN_ENGINE_DEV
pi=$root/pi
check=$root/app/scripts/check-engine-source.sh
printf 'node_modules/\ndist/\npackages/ai/src/providers/data/\n' > "$pi/.gitignore"
printf '// fixture validator\n' > "$pi/packages/ai/scripts/check-model-data.ts"
printf '{}\n' > "$pi/packages/ai/src/providers/data/.manifest.json"
printf '#!/usr/bin/env bash\n' > "$pi/pi-test.sh"
printf 'source\n' > "$pi/packages/example/source.txt"
git -C "$pi" init -q
git -C "$pi" add .
git -C "$pi" commit -qm initial
revision=$(git -C "$pi" rev-parse HEAD)
printf '%s\n' "$revision" > "$root/app/packaging/pi-engine-revision"
archive="pi-model-data-$revision.tar.zst"
tar --zstd -cf "$root/app/packaging/$archive" -C "$pi/packages/ai/src/providers/data" .
(cd "$root/app/packaging" && sha256sum "$archive" > "$archive.sha256")
[ "$("$check" "$pi")" = "$revision" ]
reject() { if "$check" "$pi" > "$root/output" 2>&1; then echo "unexpected source acceptance" >&2; exit 1; fi; }
printf 'changed\n' >> "$pi/pi-test.sh"
reject
git -C "$pi" checkout -- pi-test.sh
printf 'untracked\n' > "$pi/local.txt"
reject
rm "$pi/local.txt"
printf 'second\n' >> "$pi/pi-test.sh"
git -C "$pi" add pi-test.sh
git -C "$pi" commit -qm second
reject
PIPKIN_ENGINE_DEV=1 "$check" "$pi" > "$root/output"
git -C "$pi" checkout -q "$revision"
printf 'dependency\n' > "$pi/node_modules/input.txt"
mkdir -p "$pi/dist"
printf 'obsolete\n' > "$pi/dist/obsolete.txt"
PIPKIN_ENGINE_KEEP_DEV=1 "$root/app/scripts/build-engine.sh" "$pi" "$root/stage"
[ -f "$root/stage/node_modules/input.txt" ]
[ -f "$root/stage/packages/example/source.txt" ]
[ -f "$root/stage/packages/ai/src/providers/data/.manifest.json" ]
[ ! -e "$root/stage/dist/obsolete.txt" ]
node -e 'const fs=require("fs"); const m=JSON.parse(fs.readFileSync(process.argv[1])); if(m.version!==process.argv[2] || m.sourceRevision!==process.argv[2] || m.development!==false) process.exit(1)' "$root/stage/engine.json" "$revision"
# Clean source checkouts need no generated data: the build restores the pinned snapshot.
rm "$pi/packages/ai/src/providers/data/.manifest.json"
PIPKIN_ENGINE_KEEP_DEV=1 "$root/app/scripts/build-engine.sh" "$pi" "$root/stage"
[ -f "$root/stage/packages/ai/src/providers/data/.manifest.json" ]
# Corrupt immutable inputs are rejected as well as missing ones.
cp "$root/app/packaging/$archive" "$root/original-archive"
printf 'corrupt\n' >> "$root/app/packaging/$archive"
if "$root/app/scripts/stage-model-data.sh" --check > "$root/output" 2>&1; then
  echo 'corrupt model data archive accepted' >&2; exit 1
fi
cp "$root/original-archive" "$root/app/packaging/$archive"
# Missing immutable inputs must fail before replacing an already-staged engine.
rm "$root/app/packaging/$archive"
if PIPKIN_ENGINE_KEEP_DEV=1 "$root/app/scripts/build-engine.sh" "$pi" "$root/stage" > "$root/output" 2>&1; then
  echo 'missing generated model data was accepted' >&2; exit 1
fi
[ -f "$root/stage/engine.json" ]
printf 'invalid\n' > "$root/app/packaging/pi-engine-revision"
reject
echo 'engine source pin, dirty/untracked rejection, development override, and identified staging verified'
