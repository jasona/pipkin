#!/usr/bin/env python3
"""Finder metadata only; no Finder launch, AppleScript, pip, or owner preferences.

Tool APIs/licenses inspected: ds_store 1.3.3 DSStore/PlistCodec/ILocCodec;
mac_alias 2.2.3 Alias.for_file/from_bytes (both MIT). Libraries stay in a
private disposable directory and run under Python -I -S, never global imports.
"""
from contextlib import contextmanager
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import stat
import subprocess
import sys
import tempfile
import urllib.request
import zipfile


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def extract_wheel(data, destination, expected):
    if hashlib.sha256(data).hexdigest() != expected:
        raise RuntimeError('DMG tools wheel checksum mismatch')
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        names = set()
        for entry in archive.infolist():
            path = PurePosixPath(entry.filename)
            mode = entry.external_attr >> 16
            if (path.is_absolute() or '..' in path.parts or '\\' in entry.filename
                    or entry.filename in names or stat.S_ISLNK(mode)
                    or (stat.S_IFMT(mode) not in (0, stat.S_IFREG, stat.S_IFDIR))
                    or entry.file_size > 8 * 1024 * 1024):
                raise RuntimeError('unsafe DMG tools wheel member')
            names.add(entry.filename)
        if sum(e.file_size for e in archive.infolist()) > 16 * 1024 * 1024:
            raise RuntimeError('oversized DMG tools wheel')
        archive.extractall(destination)


@contextmanager
def private_tools(manifest):
    config = json.loads(Path(manifest).read_text())
    if config['formatVersion'] != 1 or len(config['wheels']) != 2:
        raise RuntimeError('unsupported DMG tools manifest')
    with tempfile.TemporaryDirectory(prefix='pipkin-dmg-tools-') as temporary:
        tools = Path(temporary)
        for wheel in config['wheels']:
            if (not wheel['url'].startswith('https://files.pythonhosted.org/packages/')
                    or not wheel['url'].endswith('-py3-none-any.whl')):
                raise RuntimeError('DMG tools require pure Python wheels from PyPI')
            with urllib.request.urlopen(wheel['url'], timeout=60) as response:
                data = response.read(8 * 1024 * 1024 + 1)
            if len(data) > 8 * 1024 * 1024:
                raise RuntimeError('oversized DMG tools download')
            extract_wheel(data, tools, wheel['sha256'])
        yield tools


def run_layout(tools, action, mount, layout, background):
    subprocess.run([sys.executable, '-I', '-S', '-B', str(Path(__file__).resolve()),
                    str(tools), action, str(mount), str(layout), str(background)], check=True)


def read_layout(path):
    layout = json.loads(Path(path).read_text())
    # Deliberately constrain the contract: artwork is supplied separately.
    if (layout['canvas'] != [800, 520] or layout['windowOrigin'] != [100, 100]
            or layout['iconSize'] != 128 or layout['textSize'] != 14
            or layout['icons'] != {'Pipkin.app': [260, 322], 'Applications': [580, 322]}
            or layout['background'] != '.background/install.tiff'):
        raise RuntimeError('unsupported DMG layout contract')
    return layout


def settings(layout, alias):
    x, y = layout['windowOrigin']
    width, height = layout['canvas']
    bwsp = {'WindowBounds': '{{%d, %d}, {%d, %d}}' % (x, y, width, height),
            'ShowToolbar': False, 'ShowSidebar': False, 'ShowStatusBar': False,
            'ShowPathbar': False, 'ShowTabView': False, 'ContainerShowSidebar': False}
    icvp = {'viewOptionsVersion': 1, 'backgroundType': 2, 'backgroundImageAlias': alias,
            'iconSize': float(layout['iconSize']), 'textSize': float(layout['textSize']),
            'arrangeBy': 'none', 'gridOffsetX': 0.0, 'gridOffsetY': 0.0,
            'gridSpacing': 100.0, 'labelOnBottom': True, 'showItemInfo': False,
            'showIconPreview': False, 'scrollPositionX': 0.0, 'scrollPositionY': 0.0}
    return bwsp, icvp


def portable_alias(path):
    from mac_alias import Alias
    alias = Alias.for_file(str(path))
    # for_file records the mounted HFS+ volume name/birth time and target CNIDs.
    # hdiutil convert preserves these identities. Drop the *build* mountpoint and
    # any backing image alias: target.posix_path is volume-relative, not absolute.
    alias.volume.posix_path = None
    alias.volume.disk_image_alias = None
    return alias.to_bytes()


def write_layout(mount, layout_path, source):
    from ds_store import DSStore
    mount = Path(mount)
    layout = read_layout(layout_path)
    target = mount / layout['background']
    target.parent.mkdir(exist_ok=True)
    target.write_bytes(Path(source).read_bytes())
    bwsp, icvp = settings(layout, portable_alias(target))
    with DSStore.open(str(mount / '.DS_Store'), 'w+') as store:
        store['.']['bwsp'] = bwsp
        store['.']['icvp'] = icvp
        store['.']['vSrn'] = ('long', 1)
        store['.']['icvl'] = ('type', 'icnv')
        for name, position in sorted(layout['icons'].items()):
            store[name]['Iloc'] = tuple(position)


def verify_layout(mount, layout_path, source):
    from ds_store import DSStore
    from mac_alias import Alias
    mount = Path(mount)
    layout = read_layout(layout_path)
    target = mount / layout['background']
    if target.is_symlink() or not target.is_file() or digest(target) != digest(source):
        raise RuntimeError('mounted DMG background checksum mismatch or missing background')
    try:
        with DSStore.open(str(mount / '.DS_Store'), 'r') as store:
            icvp = store['.']['icvp']
            alias_data = icvp['backgroundImageAlias']
            bwsp, expected = settings(layout, alias_data)
            if (store['.']['bwsp'] != bwsp or icvp != expected
                    or store['.']['icvl'] != (b'type', b'icnv')
                    or store['.']['vSrn'] != (b'long', 1)
                    or any(store[name]['Iloc'] != tuple(point)
                           for name, point in layout['icons'].items())):
                raise ValueError('Finder settings/icons mismatch')
        alias = Alias.from_bytes(alias_data)
        current_data = portable_alias(target)
        current = Alias.from_bytes(current_data)
        if (alias_data != current_data or alias.volume.posix_path is not None or alias.volume.disk_image_alias is not None
                or alias.target.posix_path != '/' + layout['background']
                or alias.target.filename != target.name
                or alias.target.cnid != current.target.cnid
                or alias.target.folder_cnid != current.target.folder_cnid
                or alias.target.creation_date != current.target.creation_date
                or alias.volume.name != current.volume.name
                or alias.volume.creation_date != current.volume.creation_date
                or alias.target.carbon_path != current.target.carbon_path
                or alias.target.cnid_path != current.target.cnid_path):
            raise ValueError('background alias target/volume identity mismatch')
    except Exception as error:
        raise RuntimeError('mounted DMG Finder metadata missing or invalid: ' + str(error)) from error


def enable_auto_open(mount):
    # Apple's current bless.8 omits --openfolder and describes privileged boot
    # policy operations, especially on Apple Silicon. Never elevate or setBoot.
    # Older bless supports the HFS+ Finder open-folder field; gate on local help.
    # Source: https://github.com/apple-oss-distributions/bless/blob/main/bless.8
    try:
        help_text = subprocess.check_output(['/usr/sbin/bless', '--help'],
                                            stderr=subprocess.STDOUT, text=True)
    except subprocess.CalledProcessError as error:
        help_text = error.output or ''
    except FileNotFoundError:
        help_text = ''
    if '--openfolder' not in help_text:
        print('DMG auto-open unverified: local bless does not advertise --openfolder', file=sys.stderr)
        return False
    subprocess.run(['/usr/sbin/bless', '--folder', str(mount), '--openfolder', str(mount)],
                   check=True, stdin=subprocess.DEVNULL)
    return True


if __name__ == '__main__':
    tools, action, mount, layout, source = sys.argv[1:]
    sys.path.insert(0, str(Path(tools).resolve()))
    {'write': write_layout, 'verify': verify_layout}[action](mount, layout, source)
