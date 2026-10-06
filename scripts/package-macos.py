#!/usr/bin/env python3
"""Package the current native macOS build; no signing, engine bundling or release publication."""
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
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
    notices = ROOT / 'dist/macos-notices/usr/share/doc/pipkin/third-party'
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
                    'binarySha256': sha256(executable), 'target': target},
            'engine': {'bundled': False, 'requiredRevision': (ROOT / 'packaging/pi-engine-revision').read_text().strip()},
            'signing': 'No Developer ID signing or notarization; linker ad-hoc signatures are not release authentication.',
            'directMachODependencies': [line.strip() for line in dependencies],
            'limitations': 'Build artifact, not native acceptance. Dependency notices are evidence, not legal clearance.'}
    (out / 'build-info.json').write_text(json.dumps(info, indent=2) + '\n')
    shutil.copy2(out / 'build-info.json', resources / 'build-info.json')
    archive = out / f'pipkin-{version}-{target}-experimental.zip'
    subprocess.run(['ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', str(out / 'Pipkin.app'), str(archive)], check=True)
    # Avoid uploading the app twice; the zip retains executable permissions and bundle layout.
    shutil.rmtree(out / 'Pipkin.app')
    (out / 'SHA256SUMS').write_text(f'{sha256(archive)}  {archive.name}\n')
    print(f'experimental macOS package: {archive.name}; {sha256(archive)}')


if __name__ == '__main__':
    main()
