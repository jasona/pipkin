#!/usr/bin/env python3
"""Runtime archive fixtures; no network, executable launch or owner runtime changes."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('stage_node', Path(__file__).with_name('stage-node-runtime.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
with tempfile.TemporaryDirectory() as scratch:
    root = Path(scratch); (root / 'packaging').mkdir()
    archive = root / 'input.tar.gz'; prefix = 'node-v22.23.3-darwin-arm64'
    with tarfile.open(archive, 'w:gz') as output:
        for name, data in [(prefix + '/bin/node', b'fixture node'), (prefix + '/LICENSE', b'full fixture notices'), (prefix + '/lib/node_modules/npm/bin/npm-cli.js', b'fixture npm'), ('../../escape', b'not extracted')]:
            member = tarfile.TarInfo(name); member.size = len(data)
            output.addfile(member, io.BytesIO(data))
    catalog = {'version': '22.23.3', 'targets': {'aarch64-apple-darwin': {'archive': prefix + '.tar.gz', 'sha256': hashlib.sha256(archive.read_bytes()).hexdigest()}}}
    (root / 'packaging/node-runtime.json').write_text(json.dumps(catalog))
    with patch.object(module, 'ROOT', root):
        destination = root / 'runtime'
        report = module.stage('aarch64-apple-darwin', destination, archive)
        assert report['binarySha256'] == hashlib.sha256(b'fixture node').hexdigest()
        assert (destination / 'bin/node').stat().st_mode & 0o111
        assert (destination / 'LICENSE').read_bytes() == b'full fixture notices'
        assert sorted(str(p.relative_to(destination)) for p in destination.rglob('*') if p.is_file()) == ['LICENSE', 'bin/node', 'runtime.json']
        module.stage('aarch64-apple-darwin', destination, archive)
        tools = root / 'private-tools'
        module.stage('aarch64-apple-darwin', tools, archive, build_tools=True)
        assert (tools / 'bin/npm').stat().st_mode & 0o111
        assert (tools / 'lib/node_modules/npm/bin/npm-cli.js').read_bytes() == b'fixture npm'
        assert '"$@"' in (tools / 'bin/npm').read_text()
        assert not (destination / 'bin/npm').exists()
        archive.write_bytes(b'tampered archive')
        try:
            module.stage('aarch64-apple-darwin', destination, archive)
            raise AssertionError('checksum mismatch accepted')
        except RuntimeError as error:
            assert 'checksum' in str(error)
        assert (destination / 'bin/node').read_bytes() == b'fixture node'
print('pinned runtime hash, minimal payload, safe extraction, notices and replacement policy verified')
