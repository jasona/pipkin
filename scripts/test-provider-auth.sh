#!/usr/bin/env bash
# Synthetic OAuth service tests using the Pi checkout's source resolver/dependencies.
# No live provider requests and no access to the user's credential store.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
pi=$(cd "${1:-$here/../pi-fork/pi}" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/pipkin-auth-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cp "$here"/packaging/pi-onboarding/*.ts "$tmp/"
ln -s "$pi/node_modules" "$tmp/node_modules"
resolver=$(node -p 'require("node:url").pathToFileURL(process.argv[1]).href' "$pi/packages/coding-agent/src/experimental/source-resolver.ts")
node --import "$resolver" --test "$tmp/provider-auth-provider.test.ts"
