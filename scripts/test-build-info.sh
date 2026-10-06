#!/usr/bin/env bash
# Public-input manifest and privacy checks, without reading a real profile or credentials.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
stage=$root/stage
mkdir -p "$stage/usr/bin" "$stage/usr/share/doc/pipkin" "$stage/usr/lib/pipkin/engine" "$root/source"
pin=$(<"$here/packaging/pi-engine-revision")
cat > "$stage/usr/bin/pipkin" <<SH
#!/usr/bin/env bash
echo 'pipkin 0.0.1 (source $pin, dirty: true, protocol 8, schema 8)'
SH
chmod +x "$stage/usr/bin/pipkin"
printf '{"sourceRevision":"%s","development":false,"modelDataManifestSha256":"fixture","productionPruneSucceeded":true}\n' "$pin" > "$stage/usr/lib/pipkin/engine/engine.json"
printf '{}\n' > "$stage/usr/lib/pipkin/engine/package-lock.json"
printf '{"source":true}\n' > "$root/source/package-lock.json"
RUSTFLAGS='private-secret-not-for-reports' node "$here/scripts/write-build-info.mjs" "$stage" "$root/source"
node --input-type=module - "$stage/usr/share/doc/pipkin/build-info.json" <<'JS'
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const text = readFileSync(process.argv[2], 'utf8');
const info = JSON.parse(text);
assert.equal(info.app.dirty, 'true');
assert.equal(info.compatibility.protocol, 8);
assert.equal(info.compatibility.schema, 8);
assert.equal(info.engine.productionPruneSucceeded, true);
assert.notEqual(info.engine.stagedLockSha256, info.inputs.engineSourceLockSha256);
assert.equal(info.inputs.customRustFlags, true);
assert.match(info.app.binarySha256, /^[a-f0-9]{64}$/);
assert.match(info.inputs.modelDataArchiveSha256, /^[a-f0-9]{64}$/);
assert(!text.includes('private-secret-not-for-reports'));
JS
echo 'identified package inputs and flag redaction verified'
