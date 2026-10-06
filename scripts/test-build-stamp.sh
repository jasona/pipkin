#!/usr/bin/env bash
# Exercise source identity without rebuilding or modifying the owner's repository.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
rustc --edition=2024 "$here/crates/pipkin-app/build.rs" -o "$root/stamp"
repo=$root/repo
mkdir -p "$repo/crates/pipkin-app/src"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
printf 'source\n' > "$repo/crates/pipkin-app/src/main.rs"
git -C "$repo" init -q
git -C "$repo" add .
git -C "$repo" commit -qm initial
CARGO_MANIFEST_DIR="$repo/crates/pipkin-app" "$root/stamp" > "$root/output"
grep -q "PIPKIN_APP_REVISION=$(git -C "$repo" rev-parse HEAD)" "$root/output"
grep -q 'PIPKIN_APP_DIRTY=false' "$root/output"
printf 'changed\n' >> "$repo/crates/pipkin-app/src/main.rs"
CARGO_MANIFEST_DIR="$repo/crates/pipkin-app" "$root/stamp" > "$root/output"
grep -q 'PIPKIN_APP_DIRTY=true' "$root/output"
git -C "$repo" add .
git -C "$repo" commit -qm changed
git -C "$repo" worktree add -q --detach "$root/worktree" HEAD
CARGO_MANIFEST_DIR="$root/worktree/crates/pipkin-app" "$root/stamp" > "$root/output"
grep -q "PIPKIN_APP_REVISION=$(git -C "$repo" rev-parse HEAD)" "$root/output"
grep -q 'PIPKIN_APP_DIRTY=false' "$root/output"
grep -q '/worktrees/worktree/HEAD' "$root/output"
mkdir -p "$root/unidentified/crates/pipkin-app"
CARGO_MANIFEST_DIR="$root/unidentified/crates/pipkin-app" "$root/stamp" > "$root/output"
grep -q 'PIPKIN_APP_REVISION=unknown' "$root/output"
grep -q 'PIPKIN_APP_DIRTY=unknown' "$root/output"
echo 'clean, dirty, new revision, worktree and unidentified source stamps verified'
