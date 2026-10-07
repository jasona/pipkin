#!/usr/bin/env python3
"""Check the committed desktop icons without requiring ImageMagick or macOS."""
from pathlib import Path
import struct

root = Path(__file__).resolve().parent.parent
source = root / 'assets/brand/app-icon.png'
assert source.read_bytes().startswith(b'\x89PNG\r\n\x1a\n')
assert struct.unpack('>II', source.read_bytes()[16:24]) == (1254, 1254)
icon = (root / 'packaging/pipkin.icns').read_bytes()
assert icon[:4] == b'icns' and struct.unpack('>I', icon[4:8])[0] == len(icon)
sizes = {b'icp4': 16, b'icp5': 32, b'icp6': 64, b'ic07': 128,
         b'ic08': 256, b'ic09': 512, b'ic10': 1024}
seen = set()
offset = 8
while offset < len(icon):
    kind = icon[offset:offset + 4]
    length = struct.unpack('>I', icon[offset + 4:offset + 8])[0]
    assert kind in sizes and kind not in seen and length > 8
    image = icon[offset + 8:offset + length]
    assert image[:8] == b'\x89PNG\r\n\x1a\n'
    assert struct.unpack('>II', image[16:24]) == (sizes[kind],) * 2
    if sizes[kind] in (128, 256, 512):
        assert image == (root / f'packaging/pipkin-{sizes[kind]}.png').read_bytes()
    seen.add(kind)
    offset += length
assert offset == len(icon) and seen == set(sizes)
print('Linux icons and macOS ICNS verified against the same supplied image set')
