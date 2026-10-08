#!/usr/bin/env python3
"""Regenerate the committed Retina Finder background using approved Pipkin art/fonts.

ImageMagick is required only for asset regeneration, never for source install or packaging.
The real app/Applications icons are Finder items, not painted look-alikes in this image.
"""
import json
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def run(*args):
    subprocess.run(['magick', *map(str, args)], check=True)


def set_retina_density(path):
    # ImageMagick's TIFF writer applies one density to every page. Give each
    # representation its own rational tags so NSImage sees the same 800×520pt size.
    data = bytearray(path.read_bytes())
    order = '<' if data[:2] == b'II' else '>' if data[:2] == b'MM' else None
    if order is None or struct.unpack_from(order + 'H', data, 2)[0] != 42:
        raise RuntimeError('expected a classic TIFF')
    offset = struct.unpack_from(order + 'I', data, 4)[0]
    for width, height, dpi in [(800, 520, 72), (1600, 1040, 144)]:
        if not offset: raise RuntimeError('missing Retina TIFF representation')
        count = struct.unpack_from(order + 'H', data, offset)[0]
        entries = {struct.unpack_from(order + 'H', data, offset + 2 + i * 12)[0]: offset + 2 + i * 12 for i in range(count)}
        for tag, dimension in [(256, width), (257, height)]:
            if struct.unpack_from(order + 'I', data, entries[tag] + 8)[0] != dimension:
                raise RuntimeError('unexpected TIFF representation dimensions')
        for tag in (282, 283):
            entry = entries[tag]
            if struct.unpack_from(order + 'HI', data, entry + 2) != (5, 1):
                raise RuntimeError('expected TIFF rational resolution')
            if len(data) % 2: data.append(0)
            struct.pack_into(order + 'I', data, entry + 8, len(data))
            data.extend(struct.pack(order + 'II', dpi, 1))
        offset = struct.unpack_from(order + 'I', data, offset + 2 + count * 12)[0]
    if offset: raise RuntimeError('unexpected extra TIFF representation')
    path.write_bytes(data)
    path.chmod(0o644)


def main():
    layout = json.loads((ROOT / 'packaging/dmg-layout.json').read_text())
    if layout['canvas'] != [800, 520] or layout['icons'] != {
        'Pipkin.app': [260, 322], 'Applications': [580, 322]
    }:
        raise SystemExit('Update the artwork composition alongside Finder layout changes')
    regular = ROOT / 'assets/fonts/Poppins-Regular.ttf'
    semibold = ROOT / 'assets/fonts/Poppins-SemiBold.ttf'
    with tempfile.TemporaryDirectory(prefix='pipkin-dmg-art-') as temporary:
        temp = Path(temporary)
        # Render at 2x. The mascot remains its own unaltered plate: only proportional
        # Lanczos scaling and alpha compositing are applied, never a colour transform.
        mascot = temp / 'mascot.png'
        run(ROOT / 'assets/brand/mascot.png', '-filter', 'Lanczos', '-resize', '336x348', mascot)
        shadow = temp / 'speech-shadow.png'
        run('-size', '1600x1040', 'xc:none', '-fill', '#19264b18',
            '-draw', 'roundrectangle 432,110 1512,442 48,48', '-blur', '0x18', shadow)
        high = temp / 'background@2x.png'
        run('-size', '1600x1040', 'xc:#f4f6fc', shadow, '-compose', 'over', '-composite',
            '-fill', '#ffffff', '-stroke', '#dce2f2', '-strokewidth', '2',
            '-draw', 'roundrectangle 432,96 1512,432 48,48',
            '-draw', 'path "M 432,252 L 400,280 L 432,304 Z"',
            # Hide the seam between the speech tail and its balloon.
            '-stroke', 'none', '-fill', '#ffffff', '-draw', 'rectangle 429,252 440,304',
            mascot, '-geometry', '+60+88', '-composite',
            '-font', semibold, '-pointsize', '64', '-fill', '#18223d',
            '-draw', 'text 512,218 "Drag Pipkin into"',
            '-fill', '#3454d1', '-draw', 'text 512,302 "Applications."',
            '-font', regular, '-pointsize', '28', '-fill', '#46536d',
            '-draw', 'text 512,378 "Then open Pipkin from Applications."',
            # One directional mark joins the two real 128pt Finder icons. Its entire
            # stroke stays outside both icon/label hit regions.
            '-fill', 'none', '-stroke', '#3454d1', '-strokewidth', '7',
            '-draw', 'path "M 728,644 L 952,644 M 914,606 L 952,644 L 914,682"',
            '-stroke', 'none', '-fill', '#46536d', '-font', regular, '-pointsize', '28',
            '-gravity', 'South', '-annotate', '+0+66', 'Once copied, eject this disk image.',
            '-strip', '-units', 'PixelsPerInch', '-density', '144', high)
        low = temp / 'background.png'
        run(high, '-filter', 'Lanczos', '-resize', '800x520!', '-strip',
            '-units', 'PixelsPerInch', '-density', '72', low)
        # Separate 72/144dpi representations have the same logical size. Finder can
        # choose a native-resolution representation instead of stretching a 1x PNG.
        output = ROOT / 'packaging/dmg-background.tiff'
        run(low, high, '-units', 'PixelsPerInch', '-density', '144', '-compress', 'LZW', output)
        set_retina_density(output)
    print('Generated 800×520 / 1600×1040 Finder background from approved Pipkin artwork')


if __name__ == '__main__':
    main()
