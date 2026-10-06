#!/usr/bin/env python3
"""Package/verify a self-contained ad-hoc signed macOS bundle; no Developer ID/notarization."""
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    if sys.platform != 'darwin':
        raise SystemExit('macOS packaging must inspect a native build on macOS')
    target = sys.argv[1]
    if target not in ('aarch64-apple-darwin', 'x86_64-apple-darwin'):
        raise SystemExit('expected a supported native macOS Rust target')
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT))
    binary = ROOT / 'target/release/pipkin'
    stage = ROOT / 'dist/macos-notices'
    notices = stage / 'usr/share/doc/pipkin/third-party'
    engine = stage / 'usr/lib/pipkin/engine'
    runtime = stage / 'usr/lib/pipkin/runtime'
    engine_info = json.loads((engine / 'engine.json').read_text())
    runtime_info = json.loads((runtime / 'runtime.json').read_text())
    required_revision = (ROOT / 'packaging/pi-engine-revision').read_text().strip()
    if engine_info.get('sourceRevision') != required_revision or engine_info.get('development') is not False:
        raise SystemExit('Mac package requires the clean pinned production engine')
    if runtime_info['target'] != target or sha256(runtime / 'bin/node') != runtime_info['binarySha256']:
        raise SystemExit('bundled runtime identity/target mismatch')
    if not (notices / 'inventory.json').is_file():
        raise SystemExit('target-specific dependency notices must be generated first')
    out = ROOT / 'dist/macos'
    if out.exists():
        if not (out / 'build-info.json').is_file():
            raise SystemExit('refusing to replace an unidentified output directory')
        shutil.rmtree(out)
    app = out / 'Pipkin.app/Contents'
    executable = app / 'MacOS/pipkin'
    executable.parent.mkdir(parents=True)
    shutil.copy2(binary, executable)
    resources = app / 'Resources'
    resources.mkdir()
    components = app / 'lib/pipkin'
    components.mkdir(parents=True)
    shutil.copytree(engine, components / 'engine', symlinks=True)
    shutil.copytree(runtime, components / 'runtime')
    # Existing install discovery resolves Contents/lib/pipkin/engine relative to MacOS/pipkin.
    bundled_manifest = components / 'engine/engine.json'
    engine_info['requiresBundledNode'] = True
    bundled_manifest.write_text(json.dumps(engine_info, indent=2) + '\n')
    shutil.copytree(notices, resources / 'third-party')
    shutil.copytree(ROOT / 'assets/fonts', resources / 'font-notices', ignore=shutil.ignore_patterns('*.ttf', '*.otf'))
    for source in ['LICENSE', 'SECURITY.md', 'assets/PROVENANCE.md']:
        shutil.copy2(ROOT / source, resources / Path(source).name)
    shutil.copy2(ROOT / 'docs/macos-first-pass.md', out / 'README.md')
    shutil.copy2(ROOT / 'docs/macos-first-pass.md', resources / 'README.md')
    with (app / 'Info.plist').open('wb') as stream:
        plistlib.dump({'CFBundleExecutable': 'pipkin', 'CFBundleIdentifier': 'org.last-refuge.pipkin',
                       'CFBundleName': 'Pipkin', 'CFBundlePackageType': 'APPL',
                       'CFBundleShortVersionString': version, 'CFBundleVersion': version,
                       'NSHighResolutionCapable': True}, stream)
    dependencies = subprocess.check_output(['otool', '-L', str(executable)], text=True).splitlines()[1:]
    info = {'formatVersion': 1, 'experimental': True,
            'app': {'version': version, 'revision': revision, 'dirty': dirty,
                    'preBundleSigningBinarySha256': sha256(executable), 'target': target},
            'engine': {'bundled': True, 'requiredRevision': required_revision,
                       'productionPruneSucceeded': engine_info.get('productionPruneSucceeded')},
            'nodeRuntime': runtime_info,
            'signing': 'Ad-hoc signed completed app bundle; no Developer ID or notarization. Not publisher authentication or Gatekeeper acceptance.',
            'directMachODependencies': [line.strip() for line in dependencies],
            'limitations': 'Build artifact, not native acceptance. Dependency notices are evidence, not legal clearance.'}
    # The embedded report cannot contain the final signed-binary hash: it is itself sealed
    # into that signature. Embed pre-sign identity, then retain the final hash outside the app.
    (resources / 'build-info.json').write_text(json.dumps(info, indent=2) + '\n')
    bundle = out / 'Pipkin.app'
    subprocess.run(['codesign', '--force', '--sign', '-', '--identifier', 'org.last-refuge.pipkin', str(bundle)], check=True)
    verify = ['codesign', '--verify', '--deep', '--strict', '--verbose=4']
    subprocess.run(verify + [str(bundle)], check=True)
    info['app']['binarySha256'] = sha256(executable)
    (out / 'build-info.json').write_text(json.dumps(info, indent=2) + '\n')
    archive = out / f'pipkin-{version}-{target}-experimental.zip'
    subprocess.run(['ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', str(bundle), str(archive)], check=True)
    # Validate what a user extracts, not just the pre-archive directory. Never modify the app
    # after signing, and reject any extraction that changes its executable or resource seal.
    with tempfile.TemporaryDirectory(prefix='verify-macos-', dir=out) as scratch:
        subprocess.run(['ditto', '-x', '-k', str(archive), scratch], check=True)
        restored = Path(scratch) / 'Pipkin.app'
        subprocess.run(verify + [str(restored)], check=True)
        if sha256(restored / 'Contents/MacOS/pipkin') != info['app']['binarySha256']:
            raise RuntimeError('extracted signed executable checksum mismatch')
        if sha256(restored / 'Contents/lib/pipkin/runtime/bin/node') != runtime_info['binarySha256']:
            raise RuntimeError('extracted Node checksum mismatch')
        # A real packaged CLI, not source-only tests: offline private profile, no provider calls.
        # Finder-like bare PATH excludes setup-node/Homebrew and the source checkout.
        with tempfile.TemporaryDirectory(prefix='pk-', dir='/tmp') as profile:
            environment = {'PATH': '/usr/bin:/bin:/usr/sbin:/sbin', 'HOME': profile,
                           'TMPDIR': profile, 'PI_OFFLINE': '1'}
            subprocess.run([str(restored / 'Contents/MacOS/pipkin'), '--diagnose', '--probe',
                            '--data-dir', str(Path(profile) / 'app-data')], env=environment, check=True)
        subprocess.run(verify + [str(restored)], check=True)  # Engine startup must not mutate sealed resources.
    # Avoid uploading the app twice; the zip retains executable permissions and bundle layout.
    shutil.rmtree(out / 'Pipkin.app')
    (out / 'SHA256SUMS').write_text(f'{sha256(archive)}  {archive.name}\n')
    print(f'experimental macOS package: {archive.name}; {sha256(archive)}')


if __name__ == '__main__':
    main()
