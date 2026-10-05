#!/usr/bin/env bash
# Verify a downloaded release directory: every file matches SHA256SUMS, and SHA256SUMS carries a valid
# signature when SHA256SUMS.asc is present (the signer's public key must already be in your gpg keyring).
#
#   scripts/verify-release.sh DIR [--require-signature]
set -euo pipefail
dir=${1:?usage: verify-release.sh DIR [--require-signature]}
require=0; [ "${2:-}" = "--require-signature" ] && require=1
cd "$dir"
[ -f SHA256SUMS ] || { echo "no SHA256SUMS in $dir" >&2; exit 1; }
sha256sum --check --strict SHA256SUMS
if [ -f SHA256SUMS.asc ]; then
  gpg --verify SHA256SUMS.asc SHA256SUMS
  echo "signature ok"
elif [ $require = 1 ]; then
  echo "no signature, and one is required" >&2; exit 1
else
  echo "no signature present (checksums only)"
fi
