#!/usr/bin/env bash
# Exercise the generic installer in a scratch prefix: install, upgrade, list, roll back, uninstall, and
# check each step runs. Builds a second, different "version" by repacking the first tarball with a new
# engine version, so no second build is needed.
#
#   scripts/test-install.sh [TARBALL]         (default: the newest dist/pipkin-*.tar.zst)
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
tarball=${1:-$(ls -t "$here"/dist/pipkin-*.tar.zst | head -1)}
root=$(mktemp -d /tmp/pk-inst-XXXXXX)
trap 'rm -rf "$root"' EXIT
fail=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
unpack() { mkdir -p "$1" && tar --zstd -C "$1" -xf "$tarball"; }

unpack "$root/v1"; unpack "$root/v2"
sed -i 's/"version":"[^"]*"/"version":"upgrade2"/' "$root/v2/usr/lib/pipkin/engine/engine.json"
prefix=$root/prefix
run() { env -i HOME="$root/home" PATH="/usr/bin:/bin" "$@"; }
mkdir -p "$root/home/.local/share/pipkin"
echo keep > "$root/home/.local/share/pipkin/draft-marker"

run "$root/v1/install.sh" --prefix "$prefix" >/dev/null
check "first install links the binary" "[ -x '$prefix/bin/pipkin' ]"
check "the installed binary runs" "run '$prefix/bin/pipkin' --version | grep -q '^pipkin '"
check "the desktop entry names the binary by path" "grep -q '^Exec=$prefix/bin/pipkin' '$prefix/share/applications/pipkin.desktop'"
check "an icon is installed" "[ -f '$prefix/share/icons/hicolor/256x256/apps/pipkin.png' ]"
out=$(run "$prefix/bin/pipkin" --diagnose --probe 2>&1) || true
check "the engine is found beside the installed binary and answers" "echo \"\$out\" | grep -q 'engine probe: OK'"
v1=$(run "$root/v1/install.sh" --prefix "$prefix" --list | awk '/current/ {print $2}')

sleep 1
run "$root/v2/install.sh" --prefix "$prefix" >/dev/null
check "an upgrade makes the new version current" "run '$root/v2/install.sh' --prefix '$prefix' --list | grep -q 'upgrade2.*(current)'"
check "the old version is still installed" "run '$root/v2/install.sh' --prefix '$prefix' --list | grep -q '$v1'"
out=$(run "$prefix/bin/pipkin" --diagnose 2>&1) || true
check "the upgraded engine is the one in use" "echo \"\$out\" | grep -q 'upgrade2'"

run "$root/v2/install.sh" --prefix "$prefix" --rollback >/dev/null
check "a rollback makes the previous version current" "run '$root/v2/install.sh' --prefix '$prefix' --list | grep -q '$v1 (current)'"
out=$(run "$prefix/bin/pipkin" --diagnose 2>&1) || true
check "the rolled-back engine is the old one" "! echo \"\$out\" | grep -q 'upgrade2'"

run "$root/v2/install.sh" --prefix "$prefix" --uninstall >/dev/null
check "uninstall removes the links and versions" "[ ! -e '$prefix/bin/pipkin' ] && [ ! -d '$prefix/lib/pipkin' ]"
check "your data survives install, upgrade, rollback and uninstall" "[ -f '$root/home/.local/share/pipkin/draft-marker' ]"
[ $fail = 0 ] && echo "installer verified" || { echo "installer NOT verified"; exit 1; }
