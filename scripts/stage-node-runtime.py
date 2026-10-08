#!/usr/bin/env python3
"""Stage pinned minimal Node runtime; optional --build-tools adds checkout-private npm, never global tools."""
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def stage(target, out, archive_path, build_tools=False):
    catalog = json.loads((ROOT / 'packaging/node-runtime.json').read_text())
    pin = catalog['targets'][target]
    if hashlib.sha256(archive_path.read_bytes()).hexdigest() != pin['sha256']:
        raise RuntimeError('Node archive checksum mismatch')
    if out.exists() and not (out / 'runtime.json').is_file():
        raise RuntimeError('refusing to replace unidentified runtime directory')
    # Never extract an archive tree: select exact regular members, avoiding symlink/traversal entries.
    prefix = pin['archive'].removesuffix('.tar.gz')
    payloads = {}
    with tarfile.open(archive_path, 'r:gz') as archive:
        for member_name, relative in [(f'{prefix}/bin/node', 'bin/node'), (f'{prefix}/LICENSE', 'LICENSE')]:
            matches = [m for m in archive.getmembers() if m.name == member_name]
            if len(matches) != 1 or not matches[0].isfile():
                raise RuntimeError('missing, duplicate or non-regular Node payload')
            payloads[relative] = archive.extractfile(matches[0]).read()
        if build_tools:
            # npm is a checkout-local build tool, never added to the shipped runtime.
            for member in archive.getmembers():
                relative = member.name.removeprefix(prefix + '/')
                if not member.name.startswith(prefix + '/lib/node_modules/npm/'):
                    continue
                if not member.isfile():
                    continue
                if '..' in Path(relative).parts or str(Path(relative)) != relative or relative in payloads:
                    raise RuntimeError('unsafe or duplicate npm payload')
                payloads[relative] = archive.extractfile(member).read()
            if 'lib/node_modules/npm/bin/npm-cli.js' not in payloads:
                raise RuntimeError('missing pinned npm build tool')
    if out.exists():
        shutil.rmtree(out)
    (out / 'bin').mkdir(parents=True)
    for relative, data in payloads.items():
        (out / relative).parent.mkdir(parents=True, exist_ok=True)
        (out / relative).write_bytes(data)
    (out / 'bin/node').chmod(0o755)
    if build_tools:
        npm = out / 'bin/npm'
        npm.write_text('#!/bin/sh\nbase=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexec "$base/node" "$base/../lib/node_modules/npm/bin/npm-cli.js" "$@"\n')
        npm.chmod(0o755)
    report = {'formatVersion': 1, 'version': catalog['version'], 'target': target, 'buildTools': build_tools,
              'archive': pin['archive'], 'archiveSha256': pin['sha256'],
              'source': f'https://nodejs.org/dist/v{catalog["version"]}/{pin["archive"]}',
              'binarySha256': hashlib.sha256(payloads['bin/node']).hexdigest(),
              'licenseSha256': hashlib.sha256(payloads['LICENSE']).hexdigest(),
              'limitations': 'Pinned HTTPS/checksum inputs, not independent signature authentication or security audit. Full Node/dependency LICENSE retained.'}
    (out / 'runtime.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    target, destination = sys.argv[1:3]
    catalog = json.loads((ROOT / 'packaging/node-runtime.json').read_text())
    pin = catalog['targets'][target]
    url = f'https://nodejs.org/dist/v{catalog["version"]}/{pin["archive"]}'
    with tempfile.TemporaryDirectory(prefix='pipkin-node-input-') as scratch:
        archive = Path(scratch) / pin['archive']
        with urllib.request.urlopen(url, timeout=90) as response, archive.open('wb') as stream:
            shutil.copyfileobj(response, stream)
        report = stage(target, Path(destination), archive, build_tools='--build-tools' in sys.argv[3:])
    print(f'Node {report["version"]} staged for {target}; archive {report["archiveSha256"]}')


if __name__ == '__main__':
    main()
