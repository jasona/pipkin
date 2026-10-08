#!/usr/bin/env python3
"""Real DS_Store/alias binary fixtures; not native Finder/auto-open acceptance.

By default fetch the two hash-pinned wheels into a private temporary directory.
For offline runs pass --wheel-dir DIR containing the original wheel filenames.
No pip installation, global module path changes, or desktop automation.
"""
import argparse
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import stat
import sys
import tempfile
from unittest.mock import patch
import zipfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('layout', ROOT / 'scripts/macos-dmg-layout.py')
layout = importlib.util.module_from_spec(spec)
spec.loader.exec_module(layout)


def fails(callback, text):
    try:
        callback()
    except RuntimeError as error:
        assert text in str(error), str(error)
    else:
        raise AssertionError('invalid fixture accepted: ' + text)


def wheel_bytes(name, mode=stat.S_IFREG | 0o644):
    output = io.BytesIO()
    with zipfile.ZipFile(output, 'w') as archive:
        entry = zipfile.ZipInfo(name)
        entry.external_attr = mode << 16
        archive.writestr(entry, b'fixture')
    return output.getvalue()


with tempfile.TemporaryDirectory() as scratch:
    scratch = Path(scratch)
    for name, mode in [('../escape', stat.S_IFREG), ('/absolute', stat.S_IFREG),
                       ('foo\\bar', stat.S_IFREG), ('link', stat.S_IFLNK)]:
        data = wheel_bytes(name, mode)
        fails(lambda: layout.extract_wheel(data, scratch, hashlib.sha256(data).hexdigest()), 'unsafe')
    fails(lambda: layout.extract_wheel(b'tampered', scratch, '0' * 64), 'checksum')
    data = wheel_bytes('module.py')
    layout.extract_wheel(data, scratch, hashlib.sha256(data).hexdigest())
    assert (scratch / 'module.py').read_bytes() == b'fixture'
    with patch.object(layout.subprocess, 'run') as run:
        layout.run_layout(scratch, 'verify', scratch, scratch / 'layout.json', scratch / 'bg.tiff')
        args = run.call_args.args[0]
        assert args[1:4] == ['-I', '-S', '-B'] and args[5] == str(scratch)
        assert run.call_args.kwargs == {'check': True}
    with patch.object(layout.subprocess, 'check_output', return_value='no openfolder'), \
            patch.object(layout.subprocess, 'run') as run:
        assert layout.enable_auto_open(scratch) is False
        run.assert_not_called()
    with patch.object(layout.subprocess, 'check_output', return_value='--openfolder directory'), \
            patch.object(layout.subprocess, 'run') as run:
        assert layout.enable_auto_open(scratch) is True
        assert run.call_args.args[0] == ['/usr/sbin/bless', '--folder', str(scratch), '--openfolder', str(scratch)]
        assert run.call_args.kwargs['stdin'] == layout.subprocess.DEVNULL

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--wheel-dir', type=Path)
options = parser.parse_args()
real_urlopen = layout.urllib.request.urlopen


def fetch(url, timeout):
    if options.wheel_dir:
        return io.BytesIO((options.wheel_dir / url.rsplit('/', 1)[1]).read_bytes())
    return real_urlopen(url, timeout=timeout)


with patch.object(layout.urllib.request, 'urlopen', side_effect=fetch):
    with layout.private_tools(ROOT / 'packaging/dmg-layout-tools.json') as tools:
        assert tools.stat().st_mode & 0o077 == 0
        assert list(tools.glob('*.dist-info/licenses/LICENSE'))
        # Test-only import; production uses an isolated subprocess instead.
        sys.path.insert(0, str(tools))
        from ds_store import DSStore
        from mac_alias import Alias, VolumeInfo, TargetInfo
        from mac_alias.utils import mac_epoch

        def for_file(path):
            # Model stable HFS+ birth dates/catalog IDs across converted images
            # mounted at different locations; no fabricated native acceptance.
            return Alias(volume=VolumeInfo('Pipkin', mac_epoch, b'H+', 0, 0, b'\0\0',
                                          posix_path=str(Path(path).parent.parent)),
                         target=TargetInfo(0, 'install.tiff', 42, 43, mac_epoch, b'\0' * 4, b'\0' * 4,
                                           folder_name='.background', cnid_path=[42],
                                           carbon_path=b'Pipkin:.background:\0install.tiff',
                                           posix_path='/.background/install.tiff'))

        with tempfile.TemporaryDirectory() as scratch, patch.object(Alias, 'for_file', side_effect=for_file):
            scratch = Path(scratch)
            contract = {'canvas': [800, 520], 'windowOrigin': [100, 100], 'iconSize': 128,
                        'textSize': 14, 'icons': {'Pipkin.app': [260, 322], 'Applications': [580, 322]},
                        'background': '.background/install.tiff'}
            config = scratch / 'layout.json'
            config.write_text(json.dumps(contract))
            source = scratch / 'background.tiff'
            source.write_bytes(b'fixture background; artwork tested separately')
            writable = scratch / 'writable'
            writable.mkdir()
            layout.write_layout(writable, config, source)
            original = (writable / '.DS_Store').read_bytes()
            layout.write_layout(writable, config, source)
            assert (writable / '.DS_Store').read_bytes() == original  # Deterministic bytes.
            layout.verify_layout(writable, config, source)
            mounted = scratch / 'different-mount-location'
            shutil.copytree(writable, mounted)
            layout.verify_layout(mounted, config, source)
            with DSStore.open(str(mounted / '.DS_Store'), 'r') as store:
                assert store['Pipkin.app']['Iloc'] == (260, 322)
                assert store['Applications']['Iloc'] == (580, 322)
                assert store['.']['icvp']['arrangeBy'] == 'none'
                alias = Alias.from_bytes(store['.']['icvp']['backgroundImageAlias'])
                assert alias.volume.posix_path is None
                assert alias.target.posix_path == '/.background/install.tiff'
            for code, value in [('bwsp', {'ShowToolbar': True}), ('icvp', {'arrangeBy': 'name'}),
                                ('icvl', ('type', 'Nlsv')), ('vSrn', ('long', 9))]:
                with DSStore.open(str(mounted / '.DS_Store'), 'r+') as store:
                    store['.'][code] = value
                fails(lambda: layout.verify_layout(mounted, config, source), 'Finder metadata')
                (mounted / '.DS_Store').write_bytes(original)
            with DSStore.open(str(mounted / '.DS_Store'), 'r+') as store:
                store['Pipkin.app']['Iloc'] = (1, 1)
            fails(lambda: layout.verify_layout(mounted, config, source), 'Finder metadata')
            (mounted / '.DS_Store').write_bytes(original)
            for attribute, value in [('cnid', 99), ('posix_path', '/outside.tiff')]:
                with DSStore.open(str(mounted / '.DS_Store'), 'r+') as store:
                    icvp = store['.']['icvp']
                    alias = Alias.from_bytes(icvp['backgroundImageAlias'])
                    setattr(alias.target, attribute, value)
                    icvp['backgroundImageAlias'] = alias.to_bytes()
                    store['.']['icvp'] = icvp
                fails(lambda: layout.verify_layout(mounted, config, source), 'alias target')
                (mounted / '.DS_Store').write_bytes(original)
            with DSStore.open(str(mounted / '.DS_Store'), 'r+') as store:
                icvp = store['.']['icvp']
                alias = Alias.from_bytes(icvp['backgroundImageAlias'])
                alias.volume.posix_path = '/tmp/obsolete-build-mount'
                icvp['backgroundImageAlias'] = alias.to_bytes()
                store['.']['icvp'] = icvp
            fails(lambda: layout.verify_layout(mounted, config, source), 'alias target')
            (mounted / '.DS_Store').unlink()
            fails(lambda: layout.verify_layout(mounted, config, source), 'Finder metadata')
            (mounted / '.DS_Store').write_bytes(original)
            background = mounted / '.background/install.tiff'
            background.write_bytes(b'tampered')
            fails(lambda: layout.verify_layout(mounted, config, source), 'background checksum')
            background.unlink()
            fails(lambda: layout.verify_layout(mounted, config, source), 'missing background')
        sys.path.remove(str(tools))
    assert not tools.exists()  # Private tools and licenses removed even after use.

# Hash failure must also clean up the private directory before yielding it.
created = []
real_temporary = tempfile.TemporaryDirectory


def track_temporary(**kwargs):
    context = real_temporary(**kwargs)
    created.append(Path(context.name))
    return context


with patch.object(layout.tempfile, 'TemporaryDirectory', side_effect=track_temporary), \
        patch.object(layout.urllib.request, 'urlopen', return_value=io.BytesIO(b'wrong wheel')):
    def reject_tools():
        with layout.private_tools(ROOT / 'packaging/dmg-layout-tools.json'):
            raise AssertionError('tampered tools yielded')
    fails(reject_tools, 'checksum')
assert all(not path.exists() for path in created)
print('DMG binary Finder metadata/alias relocation/private hash-pinned tools fixtures passed; native presentation and auto-open unverified')
