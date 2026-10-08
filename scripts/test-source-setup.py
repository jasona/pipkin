#!/usr/bin/env python3
"""Source setup contracts with disposable files and mocked subprocesses; never desktop/owner installs."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import sys
import tarfile
import tempfile
from unittest.mock import patch

sys.dont_write_bytecode = True
scripts = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('setup', scripts / 'setup.py')
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)
assert setup.target_for('Darwin', 'arm64') == 'aarch64-apple-darwin'
assert setup.target_for('Darwin', 'x86_64') == 'x86_64-apple-darwin'
assert setup.target_for('Linux', 'x86_64') == 'x86_64-unknown-linux-gnu'
try:
    setup.target_for('Linux', 'aarch64')
    raise AssertionError('unqualified target accepted')
except RuntimeError:
    pass

with tempfile.TemporaryDirectory() as scratch:
    root = Path(scratch)
    (root / 'scripts').mkdir(); (root / 'packaging').mkdir()
    shutil.copy2(scripts / 'stage-node-runtime.py', root / 'scripts/stage-node-runtime.py')
    (root / 'packaging/pi-engine-revision').write_text('a' * 40)
    base = root / 'dist/source-setup'
    with patch.object(setup, 'ROOT', root):
        setup.owned_directory(base)
        unidentified = root / 'unidentified'; unidentified.mkdir()
        try:
            setup.owned_directory(unidentified)
            raise AssertionError('unidentified output accepted')
        except RuntimeError:
            pass
        prefix = 'node-v22.23.3-linux-x64'
        archive = base / (prefix + '.tar.gz')
        with tarfile.open(archive, 'w:gz') as output:
            for relative, data in [('bin/node', b'fixture node'), ('LICENSE', b'full license'), ('lib/node_modules/npm/bin/npm-cli.js', b'fixture npm')]:
                member = tarfile.TarInfo(prefix + '/' + relative); member.size = len(data)
                output.addfile(member, io.BytesIO(data))
        (root / 'packaging/node-runtime.json').write_text(json.dumps({'version': '22.23.3', 'targets': {'x86_64-unknown-linux-gnu': {'archive': archive.name, 'sha256': hashlib.sha256(archive.read_bytes()).hexdigest()}}}))
        calls = []
        fail_probe = False
        missing_subagents = False

        def command(args, **kwargs):
            global fail_probe, missing_subagents
            args = list(map(str, args)); calls.append(args)
            if args[:2] == ['git', 'init']:
                Path(args[2]).mkdir(parents=True)
            if args[0].endswith('build-engine.sh'):
                engine = Path(args[2]); engine.mkdir(parents=True)
                manifest = {'development': False}
                if not missing_subagents:
                    manifest['pipkinSubagentsSha256'] = 'b' * 64
                (engine / 'engine.json').write_text(json.dumps(manifest))
            if args[:2] == ['cargo', 'build']:
                binary = root / 'target/release/pipkin'; binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_text('fixture binary')
            if '--probe' in args:
                assert set(kwargs['env']) == {'HOME', 'TMPDIR', 'PATH', 'PI_OFFLINE'}
                assert kwargs['env']['PATH'] == '/usr/bin:/bin:/usr/sbin:/sbin'
                assert kwargs['env']['HOME'].startswith('/tmp/')
                if fail_probe:
                    raise RuntimeError('injected failed probe')

        with patch.object(setup, 'run', side_effect=command), patch.object(setup, 'preflight'), patch.object(setup.platform, 'system', return_value='Linux'), patch.object(setup.platform, 'machine', return_value='x86_64'), patch.object(sys, 'argv', ['setup.py']):
            setup.main()
            first = (base / 'current').resolve()
            assert json.loads((first / 'lib/pipkin/engine/engine.json').read_text())['requiresBundledNode']
            assert (base / 'build-tools/bin/npm').stat().st_mode & 0o111
            assert (base / 'build-tools/lib/node_modules/npm/bin/npm-cli.js').exists()
            assert not (first / 'lib/pipkin/runtime/lib').exists()  # Installed runtime remains minimal.
            setup.main()
            second = (base / 'current').resolve()
            assert second != first and (first / 'bin/pipkin').exists()  # Running build not overwritten.
            fail_probe = True
            try:
                setup.main()
                raise AssertionError('failed probe accepted')
            except RuntimeError:
                assert (base / 'current').resolve() == second
            fail_probe = False; missing_subagents = True
            try:
                setup.main()
                raise AssertionError('missing subagents accepted')
            except RuntimeError:
                assert (base / 'current').resolve() == second
        assert not any('--install' in call or 'sudo' in call for call in calls)
print('source setup: native targets, private npm/minimal runtime, default subagent gate, immutable rebuilds and failed-build recovery verified')
