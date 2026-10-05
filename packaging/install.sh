#!/usr/bin/env bash
# Install Pipkin from the release tarball it ships in, on any Linux with Wayland and Node >= 22.19.
#
#   ./install.sh                  install into ~/.local (add --prefix DIR for elsewhere)
#   ./install.sh --list           show installed versions and which is current
#   ./install.sh --rollback       make the previous version current again
#   ./install.sh --uninstall      remove the links and every installed version (data is kept)
#   --keep N                      versions to keep after an install (default 2)
#
# Each install goes to <prefix>/lib/pipkin/versions/<version>/ and `current` points at it, so an
# upgrade never overwrites a working version and a rollback is one link switch. The binary finds its
# engine relative to itself, so every version carries its own. Your drafts and history live in
# ~/.local/share/pipkin and are not touched; a newer database is refused by an older Pipkin, with
# the backup made at the upgrade to restore (see docs/packaging.md).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
prefix=${PREFIX:-$HOME/.local}
keep=2
action=install
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) prefix=$2; shift 2 ;;
    --keep) keep=$2; shift 2 ;;
    --list) action=list; shift ;;
    --rollback) action=rollback; shift ;;
    --uninstall) action=uninstall; shift ;;
    -h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

base=$prefix/lib/pipkin
versions=$base/versions
current=$base/current

version_of() { # version directory name from a tree
  local m="$1/usr/lib/pipkin/engine/engine.json" v
  v=$(sed -n 's/.*"version":"\([^"]*\)".*/\1/p' "$m" 2>/dev/null | head -1)
  echo "${v:-unknown}"
}
app_version() { "$1/usr/bin/pipkin" --version 2>/dev/null | awk '{print $2}'; }
installed() { ls -1t "$versions" 2>/dev/null || true; }
current_name() { [ -L "$current" ] && basename "$(readlink "$current")" || true; }

link_all() { # point the user-visible files at <versions>/<name>
  local name=$1 root=$versions/$1
  ln -sfn "$root" "$current"
  mkdir -p "$prefix/bin" "$prefix/share/applications"
  ln -sfn "$current/usr/bin/pipkin" "$prefix/bin/pipkin"
  # The desktop entry names the binary by absolute path, so a launcher works without PATH.
  sed "s|^Exec=pipkin|Exec=$prefix/bin/pipkin|" "$root/usr/share/applications/pipkin.desktop" \
    > "$prefix/share/applications/pipkin.desktop"
  for size in 128 256 512; do
    local icon="$root/usr/share/icons/hicolor/${size}x${size}/apps/pipkin.png"
    [ -f "$icon" ] && install -Dm644 "$icon" "$prefix/share/icons/hicolor/${size}x${size}/apps/pipkin.png"
  done
  command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$prefix/share/icons/hicolor" 2>/dev/null || true
}

case "$action" in
  install)
    [ -d "$here/usr/lib/pipkin/engine" ] || { echo "run this from the unpacked release (no usr/ here)" >&2; exit 1; }
    command -v node >/dev/null || { echo "Node.js >= 22.19 is required (node not found)" >&2; exit 1; }
    "$here/usr/bin/pipkin" --version >/dev/null || { echo "this build does not run here" >&2; exit 1; }
    name="$(app_version "$here")-$(version_of "$here")"
    mkdir -p "$versions"
    if [ -d "$versions/$name" ]; then
      echo "$name is already installed; making it current"
    else
      stage=$(mktemp -d "$versions/.incoming-XXXXXX")
      cp -a "$here/usr" "$stage/usr"
      mv "$stage" "$versions/$name"
    fi
    link_all "$name"
    # Keep the newest N (by install time) and always the current one.
    count=0
    for v in $(installed); do
      count=$((count + 1))
      if [ $count -gt "$keep" ] && [ "$v" != "$(current_name)" ]; then rm -rf "$versions/$v"; fi
    done
    echo "installed $name; run $prefix/bin/pipkin ($prefix/bin must be on PATH for the plain command)"
    ;;
  list)
    cur=$(current_name)
    for v in $(installed); do
      if [ "$v" = "$cur" ]; then echo "* $v (current)"; else echo "  $v"; fi
    done
    ;;
  rollback)
    cur=$(current_name)
    prev=""
    for v in $(installed); do [ "$v" != "$cur" ] && { prev=$v; break; }; done
    [ -n "$prev" ] || { echo "no previous version to roll back to" >&2; exit 1; }
    link_all "$prev"
    # The version just left stays installed (and is now the 'previous' one) until the next install.
    touch "$versions/$prev"
    echo "rolled back to $prev (was $cur)"
    ;;
  uninstall)
    rm -f "$prefix/bin/pipkin" "$prefix/share/applications/pipkin.desktop"
    for size in 128 256 512; do rm -f "$prefix/share/icons/hicolor/${size}x${size}/apps/pipkin.png"; done
    rm -rf "$base"
    echo "removed Pipkin from $prefix (your data in ~/.local/share/pipkin was kept)"
    ;;
esac
