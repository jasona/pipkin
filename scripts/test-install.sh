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
python3 - "$root/v2/usr/lib/pipkin/engine/engine.json" <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1]); data = json.loads(path.read_text())
data['version'] = 'upgrade2'; path.write_text(json.dumps(data, indent=2) + '\n')
PY
prefix="$root/prefix with spaces"
mkdir -p "$root/shims"
printf '#!/bin/sh\nexit 93\n' > "$root/shims/node"
chmod +x "$root/shims/node"
# Deliberately break system node; the packaged app and probe must use their private runtime.
run() { env -i HOME="$root/home" PATH="$root/shims:/usr/bin:/bin" "$@"; }
mkdir -p "$root/home/.local/share/pipkin"
echo keep > "$root/home/.local/share/pipkin/draft-marker"
mkdir -p "$prefix/bin"
printf 'unrelated program\n' > "$prefix/bin/pipkin"
if run "$root/v1/install.sh" --prefix "$prefix" >"$root/conflict.log" 2>&1; then
  echo 'FAIL unrelated program was overwritten'; fail=1
else
  check "another app at the install path is preserved" "grep -q 'unrelated program' '$prefix/bin/pipkin'"
fi
if run "$root/v1/install.sh" --prefix "$prefix" --uninstall >"$root/no-uninstall.log" 2>&1; then
  echo 'FAIL uninstall removed an unrelated program'; fail=1
else
  check "uninstall without an owned version changes nothing" "grep -q 'unrelated program' '$prefix/bin/pipkin'"
fi
rm "$prefix/bin/pipkin"

first_install=$(run "$root/v1/install.sh" --prefix "$prefix")
check "plain output reports completed steps" "echo \"\$first_install\" | grep -q '\[OK\].*100%.*Desktop shortcut ready'"
if [[ "$first_install" == *$'\033'* ]]; then echo 'FAIL non-TTY output contains color codes'; fail=1; fi
check "first install links the binary" "[ -x '$prefix/bin/pipkin' ]"
check "the installed binary runs" "run '$prefix/bin/pipkin' --version | grep -q '^pipkin '"
check "the desktop entry quotes the versioned binary path" "grep -Fq 'Exec=\"$prefix/bin/pipkin\"' '$prefix/share/applications/pipkin.desktop'"
check "an icon is installed" "[ -f '$prefix/share/icons/hicolor/256x256/apps/pipkin.png' ]"
check "installed runtime is present" "[ -x '$prefix/lib/pipkin/current/usr/lib/pipkin/runtime/bin/node' ]"
if command -v script >/dev/null; then
  script -q -e -c "env -i HOME='$root/home' PATH='$root/shims:/usr/bin:/bin' TERM=xterm '$root/v1/install.sh' --prefix '$prefix'" "$root/tty.log" >/dev/null
  if grep -Fq $'\033[32m' "$root/tty.log"; then
    echo 'ok   interactive terminal prints colored ASCII progress'
  else
    echo 'FAIL interactive progress lacked color'; fail=1
  fi
  script -q -e -c "env -i HOME='$root/home' PATH='$root/shims:/usr/bin:/bin' TERM=xterm NO_COLOR=1 '$root/v1/install.sh' --prefix '$prefix'" "$root/no-color.log" >/dev/null
  if grep -Fq $'\033[' "$root/no-color.log"; then
    echo 'FAIL NO_COLOR still emitted escape codes'; fail=1
  else
    echo 'ok   NO_COLOR keeps interactive progress plain'
  fi
fi
out=$(run "$prefix/bin/pipkin" --diagnose --probe 2>&1) || true
check "the engine is found beside the installed binary and answers" "echo \"\$out\" | grep -q 'engine probe: OK'"
v1=$(run "$root/v1/install.sh" --prefix "$prefix" --list | awk '/current/ {print $2}')

# A bad app payload fails before it can replace a working install.
v2_binary="$root/v2/usr/bin/pipkin"
mv "$v2_binary" "$root/valid-binary"
printf 'tampered' > "$v2_binary"
chmod +x "$v2_binary"
if run "$root/v2/install.sh" --prefix "$prefix" >"$root/bad-app.log" 2>&1; then
  echo 'FAIL altered app was installed'; fail=1
else
  check "tampered app fails closed" "grep -q 'executable checksum mismatch' '$root/bad-app.log'"
fi
mv "$root/valid-binary" "$v2_binary"

# A bad Node payload fails before it can be run or replace a working install.
v2_node="$root/v2/usr/lib/pipkin/runtime/bin/node"
mv "$v2_node" "$root/valid-node"
printf 'tampered' > "$v2_node"
chmod +x "$v2_node"
if run "$root/v2/install.sh" --prefix "$prefix" >"$root/bad-runtime.log" 2>&1; then
  echo 'FAIL altered Node was installed'; fail=1
else
  check "tampered Node fails closed" "grep -q 'Node checksum mismatch' '$root/bad-runtime.log'"
fi
mv "$root/valid-node" "$v2_node"

# A broken engine must not activate over the working version.
v2_launcher="$root/v2/usr/lib/pipkin/engine/pi-test.sh"
mv "$v2_launcher" "$root/original-launcher"
printf '#!/bin/sh\nexit 91\n' > "$v2_launcher"
chmod +x "$v2_launcher"
if run "$root/v2/install.sh" --prefix "$prefix" >"$root/broken-install.log" 2>&1; then
  echo 'FAIL broken engine was installed'; fail=1
else
  after_failure=$(run "$root/v1/install.sh" --prefix "$prefix" --list | awk '/current/ {print $2}')
  check "failed probe leaves the old version current" "[ '$after_failure' = '$v1' ]"
fi
mv "$root/original-launcher" "$v2_launcher"
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
