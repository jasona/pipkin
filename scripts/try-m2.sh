#!/usr/bin/env bash
# Try Pipkin's real-engine workflow offline: a managed Pi engine, a scripted provider, a scratch
# git project. Usage: scripts/try-m2.sh [path-to-pi-checkout]   (default ../pi-fork/pi)
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
pi=$(cd "${1:-$here/../pi-fork/pi}" && pwd)
root=${PIPKIN_TRY_ROOT:-/tmp/pipkin-try}   # keep short: Unix socket paths are limited to ~108 bytes
port=${PIPKIN_TRY_PORT:-18765}
mkdir -p "$root/agent" "$root/server" "$root/data" "$root/project"
chmod 700 "$root/server"
cat > "$root/agent/models.json" <<JSON
{"providers":{"stub":{"baseUrl":"http://127.0.0.1:$port/v1","api":"openai-completions","apiKey":"stub",
 "models":[{"id":"scripted","name":"Scripted stub"}]}}}
JSON
if [ ! -d "$root/project/.git" ]; then
  git -C "$root/project" init -q
  echo "original" > "$root/project/notes.txt"
  git -C "$root/project" add . && git -C "$root/project" -c user.name=t -c user.email=t@t commit -qm init
fi
node "$here/scripts/stub-provider.mjs" "$port" & stub=$!
trap 'kill $stub 2>/dev/null' EXIT
export PI_OFFLINE=1
cd "$here"
cargo run -p pipkin-app --release -- --pi-repo "$pi" --pi-dir "$root/server" \
  --pi-agent-dir "$root/agent" --project "$root/project" --data-dir "$root/data"
