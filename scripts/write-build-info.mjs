#!/usr/bin/env node
// Record the binary's identity and public build inputs, never arbitrary environment values.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(fileURLToPath(new URL('..', import.meta.url)));
const stage = resolve(process.argv[2]);
const source = resolve(process.argv[3]);
const run = (program, args) => execFileSync(program, args, { cwd: root, encoding: 'utf8' }).trim();
const hash = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const binary = join(stage, 'usr/bin/pipkin');
const identity = run(binary, ['--version']).match(/^pipkin (\S+) \(source ([^,]+), dirty: (true|false|unknown), protocol (\d+), schema (\d+)\)$/);
if (!identity) throw new Error('binary has no identifiable build stamp');
const engine = join(stage, 'usr/lib/pipkin/engine');
const manifest = JSON.parse(readFileSync(join(engine, 'engine.json'), 'utf8'));
const info = {
  formatVersion: 1,
  app: { version: identity[1], revision: identity[2], dirty: identity[3], binarySha256: hash(binary) },
  compatibility: { protocol: Number(identity[4]), schema: Number(identity[5]), nodeMinimum: '22.19' },
  engine: { revision: manifest.sourceRevision, development: manifest.development,
    productionPruneSucceeded: manifest.productionPruneSucceeded,
    modelDataManifestSha256: manifest.modelDataManifestSha256,
    stagedLockSha256: hash(join(engine, 'package-lock.json')) },
  inputs: { cargoLockSha256: hash(join(root, 'Cargo.lock')),
    engineSourceLockSha256: hash(join(source, 'package-lock.json')),
    modelDataArchiveSha256: manifest.development ? null : hash(join(root, 'packaging', `pi-model-data-${manifest.sourceRevision}.tar.zst`)),
    rustToolchainSha256: hash(join(root, 'rust-toolchain.toml')),
    rustc: run('rustc', ['-vV']), cargo: run('cargo', ['--version']), node: process.version,
    npm: run('npm', ['--version']), python: run('python3', ['--version']),
    platform: process.platform, architecture: process.arch,
    customRustFlags: Boolean(process.env.RUSTFLAGS || process.env.CARGO_ENCODED_RUSTFLAGS) },
};
writeFileSync(join(stage, 'usr/share/doc/pipkin/build-info.json'), `${JSON.stringify(info, null, 2)}\n`);
