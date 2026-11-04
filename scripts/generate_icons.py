#!/usr/bin/env python3
"""Build the Huda icon set from the approved artwork. Requires Pillow only when regenerating assets.

source.png stays the untouched, full-bleed master. The platform shape is applied here, so it is
exact and can change without new artwork:
- 128px and up, and every ICNS entry, follow the macOS grid: an 824/1024 continuous-corner body
  with a soft shadow on a transparent canvas.
- 64px and below (tray, title bar, taskbar) fill the canvas and zoom in on the mark.
"""
from math import copysign, cos, sin, tau
from pathlib import Path
import hashlib
import json
from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / 'assets/icons'
SIZES = (16, 24, 32, 48, 64, 128, 256, 512, 1024)
LARGEST_SMALL = 64
ICNS_SIZES = (32, 64, 128, 256, 512)
WORK = 2048  # Working canvas; matches the master so the grid body is not resampled twice.
BODY = WORK * 824 // 1024
CENTER = (1023.5, 1020)  # Centre of the H in source.png, measured.
GRID_CROP = 1800  # Master pixels shown in the grid body.
SMALL_CROP = 1414  # Puts the H's pillars and gap on whole pixels at 16px, 32px and 64px.
SHADOW_OFFSET, SHADOW_BLUR, SHADOW_OPACITY = 24, 28, 0.4


def squircle(size):
    """Antialiased continuous-corner mask: a superellipse, the usual match for Apple's shape."""
    radius = size * 2
    exponent = 2 / 5

    def axis(value):
        return radius + radius * copysign(abs(value) ** exponent, value)

    steps = (index * tau / 1440 for index in range(1440))
    outline = [(axis(cos(step)), axis(sin(step))) for step in steps]
    mask = Image.new('L', (size * 4, size * 4))
    ImageDraw.Draw(mask).polygon(outline, fill=255)
    return mask.resize((size, size), Image.Resampling.LANCZOS)


def tile(source, crop, size):
    half = crop / 2
    box = (CENTER[0] - half, CENTER[1] - half, CENTER[0] + half, CENTER[1] + half)
    image = source.resize((size, size), Image.Resampling.LANCZOS, box=box)
    image.putalpha(squircle(size))
    return image


def grid_master(source):
    body = tile(source, GRID_CROP, BODY)
    inset = (WORK - BODY) // 2
    shadow = Image.new('L', (WORK, WORK))
    shadow.paste(body.getchannel('A').point(lambda alpha: alpha * SHADOW_OPACITY),
                 (inset, inset + SHADOW_OFFSET))
    canvas = Image.new('RGBA', (WORK, WORK))
    canvas.putalpha(shadow.filter(ImageFilter.GaussianBlur(SHADOW_BLUR)))
    canvas.alpha_composite(body, (inset, inset))
    return canvas


def main():
    with Image.open(ICONS / 'source.png') as master:
        if master.width != master.height:
            raise ValueError('Approved master must be square; do not silently crop it.')
        source = master.convert('RGBA')
    grid = grid_master(source)
    small = tile(source, SMALL_CROP, WORK)

    def render(image, size):
        return image.resize((size, size), Image.Resampling.LANCZOS)

    icons = {size: render(small if size <= LARGEST_SMALL else grid, size) for size in SIZES}
    for size, image in icons.items():
        image.save(ICONS / f'huda-{size}.png', optimize=True)
    icons[1024].save(ICONS / 'huda.icns', append_images=[render(grid, n) for n in ICNS_SIZES])
    ico = [n for n in SIZES if n <= 256]
    icons[256].save(ICONS / 'huda.ico', sizes=[(n, n) for n in ico],
                    append_images=[icons[n] for n in ico])

    files = sorted(f for f in ICONS.iterdir() if f.suffix in ('.png', '.ico', '.icns'))
    manifest_path = ICONS / 'manifest.json'
    manifest = json.loads(manifest_path.read_text())
    manifest['sha256'] = {f.name: hashlib.sha256(f.read_bytes()).hexdigest() for f in files}
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
