#!/usr/bin/env bash
# Build an installable tree and tarball: binary, bundled engine, desktop entry, icon, licences.
#
#   scripts/package.sh [PI_CHECKOUT]       -> dist/pipkin-<version>-x86_64.tar.zst and dist/stage/
#
# The tree uses the /usr layout, so it can be unpacked at / (what the PKGBUILD does) or into any
# prefix: the binary finds the engine at ../lib/pipkin/engine relative to itself.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here"
version=$(cargo pkgid -p pipkin-app | sed 's/.*[#@]//')
arch=$(uname -m)
stage=$here/dist/stage
rm -rf "$stage"
cargo build -p pipkin-app --release --locked
"$here/scripts/build-engine.sh" "${1:-$here/../pi-fork/pi}" "$stage/usr/lib/pipkin/engine"
install -Dm755 target/release/pipkin "$stage/usr/bin/pipkin"
install -Dm644 packaging/pipkin.desktop "$stage/usr/share/applications/pipkin.desktop"
for size in 128 256 512; do
  install -Dm644 "packaging/pipkin-$size.png" "$stage/usr/share/icons/hicolor/${size}x${size}/apps/pipkin.png"
done
install -Dm644 LICENSE "$stage/usr/share/licenses/pipkin/LICENSE"
install -Dm644 assets/icons/LICENSE "$stage/usr/share/licenses/pipkin/icons-LICENSE"
for f in Poppins-OFL.txt Lilex-OFL.txt IBMPlexSans-LICENSE.txt; do
  install -Dm644 "assets/fonts/$f" "$stage/usr/share/licenses/pipkin/$f"
done
install -Dm644 assets/PROVENANCE.md "$stage/usr/share/doc/pipkin/PROVENANCE.md"
install -Dm644 README.md "$stage/usr/share/doc/pipkin/README.md"
install -Dm644 docs/native-gate-testing.md "$stage/usr/share/doc/pipkin/native-gate-testing.md"
# The tarball also carries the installer at its root; the Arch package (which uses the stage tree
# directly) does not, since a package must not put files in /.
tar --zstd -cf "dist/pipkin-$version-$arch.tar.zst" -C "$stage" . -C "$here/packaging" install.sh
echo "built dist/pipkin-$version-$arch.tar.zst"
