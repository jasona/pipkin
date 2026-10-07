#!/usr/bin/env python3
"""Regenerate committed Linux and macOS app icons from the owner-supplied PNG.

Requires ImageMagick (`magick`) only when regenerating icons, not when packaging.
"""
from pathlib import Path
import struct
import subprocess

root = Path(__file__).resolve().parent.parent
source = root / 'assets/brand/app-icon.png'
packaging = root / 'packaging'
# PNG-backed ICNS entries (16, 32, 64, 128, 256, 512 and 1024 pixels).
icon_types = {16: b'icp4', 32: b'icp5', 64: b'icp6', 128: b'ic07',
              256: b'ic08', 512: b'ic09', 1024: b'ic10'}
entries = []
for size, icon_type in icon_types.items():
    # Use the same scaled pixels for both platforms; preserve source alpha and aspect.
    result = subprocess.run(['magick', str(source), '-strip', '-filter', 'Lanczos',
                             '-resize', f'{size}x{size}', f'PNG32:-'],
                            stdout=subprocess.PIPE, check=True)
    png = result.stdout
    if png[:8] != b'\x89PNG\r\n\x1a\n' or struct.unpack('>II', png[16:24]) != (size, size):
        raise RuntimeError(f'failed to generate {size}px icon')
    entries.append(icon_type + struct.pack('>I', len(png) + 8) + png)
    if size in (128, 256, 512):
        (packaging / f'pipkin-{size}.png').write_bytes(png)
body = b''.join(entries)
(packaging / 'pipkin.icns').write_bytes(b'icns' + struct.pack('>I', len(body) + 8) + body)
print('Generated Linux PNGs and macOS ICNS from assets/brand/app-icon.png')
