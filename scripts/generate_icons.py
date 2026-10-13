#!/usr/bin/env python3
"""Resize the approved Huda artwork. Requires Pillow only when regenerating assets."""
from pathlib import Path
import hashlib
import json
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / 'assets/icons'
SIZES = (16, 24, 32, 48, 64, 128, 256, 512, 1024)

def main():
    with Image.open(ICONS / 'source.png') as source:
        if source.width != source.height:
            raise ValueError('Approved master must be square; do not silently crop it.')
        image = source.convert('RGBA')
        for size in SIZES:
            image.resize((size, size), Image.Resampling.LANCZOS).save(ICONS / f'huda-{size}.png')
        image.resize((1024, 1024), Image.Resampling.LANCZOS).save(ICONS / 'huda.icns')
        image.save(ICONS / 'huda.ico', sizes=[(n, n) for n in SIZES if n <= 256])
    files = sorted(f for f in ICONS.iterdir() if f.suffix in ('.png', '.ico', '.icns'))
    manifest = {'source': 'source.png', 'transformation': 'Lanczos resize only; no artwork replacement',
                'sha256': {f.name: hashlib.sha256(f.read_bytes()).hexdigest() for f in files}}
    (ICONS / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

if __name__ == '__main__':
    main()
