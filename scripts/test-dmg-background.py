#!/usr/bin/env python3
"""Validate committed TIFF representations/layout; native AppKit only on macOS.

No image converter, third-party Python library, display, owner profile, or Finder required.
This is not a visual acceptance test.
"""
import json
from pathlib import Path
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
path = ROOT / 'packaging/dmg-background.tiff'
data = path.read_bytes()
assert data[:2] in (b'II', b'MM')
order = '<' if data[:2] == b'II' else '>'
assert struct.unpack_from(order + 'H', data, 2)[0] == 42
next_offset = struct.unpack_from(order + 'I', data, 4)[0]
seen = set()
representations = []
while next_offset:
    assert next_offset not in seen, 'cyclic TIFF directories'
    seen.add(next_offset)
    count = struct.unpack_from(order + 'H', data, next_offset)[0]
    entries = {struct.unpack_from(order + 'H', data, next_offset + 2 + i * 12)[0]:
               next_offset + 2 + i * 12 for i in range(count)}

    def value(tag):
        entry = entries[tag]
        kind, length = struct.unpack_from(order + 'HI', data, entry + 2)
        assert length == 1
        if kind == 3:
            return struct.unpack_from(order + 'H', data, entry + 8)[0]
        if kind == 4:
            return struct.unpack_from(order + 'I', data, entry + 8)[0]
        assert kind == 5
        offset = struct.unpack_from(order + 'I', data, entry + 8)[0]
        numerator, denominator = struct.unpack_from(order + 'II', data, offset)
        assert denominator > 0
        return numerator / denominator

    assert value(296) == 2, 'TIFF must use pixels per inch'
    representations.append((value(256), value(257), value(282), value(283)))
    next_offset = struct.unpack_from(order + 'I', data, next_offset + 2 + count * 12)[0]
assert representations == [(800, 520, 72, 72), (1600, 1040, 144, 144)], representations
layout = json.loads((ROOT / 'packaging/dmg-layout.json').read_text())
assert layout['canvas'] == [800, 520]
for x, y in layout['icons'].values():
    half = layout['iconSize'] / 2
    assert x - half >= 0 and x + half <= 800
    assert y - half >= 240 and y + half + layout['textSize'] * 2 < 440
assert layout['icons']['Pipkin.app'][0] < layout['icons']['Applications'][0]

if sys.platform == 'darwin':
    # NSImage chooses representations by logical size/DPI, not just pixel count.
    # Test the actual decoder in the native package job without opening any window.
    swift = r'''
import AppKit
let image = NSImage(contentsOfFile: CommandLine.arguments[1])!
let reps = image.representations.sorted { $0.pixelsWide < $1.pixelsWide }
precondition(reps.count == 2, "missing native Retina representation")
for (index, rep) in reps.enumerated() {
    precondition(rep.pixelsWide == (index == 0 ? 800 : 1600))
    precondition(rep.pixelsHigh == (index == 0 ? 520 : 1040))
    precondition(abs(rep.size.width - 800) < 0.1 && abs(rep.size.height - 520) < 0.1)
}
print("AppKit decoded both Finder background representations at 800×520 logical points")
'''
    subprocess.run(['swift', '-', str(path)], input=swift, text=True, check=True)
print('DMG TIFF 1x/2x density, logical size and icon clearance checks passed; visual review not claimed')
