#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Turns raw phone captures into the images under docs/screenshots.

Removes the system panels of the 720 x 1600 FLX1s screen, reduces the palette
and writes no metadata. Requires Pillow.

Usage: prepare_screenshots.py <directory with captures> <output directory>
"""
import os
import sys

from PIL import Image

PANELS_REMOVED = (0, 56, 720, 1573)
SELECTION = {
    "today-light-en": "today-light",
    "history-light-en": "history-light",
    "achievements-light-en": "achievements-light",
    "today-dark-en": "today-dark",
    "history-dark-en": "history-dark",
    "achievements-dark-en": "achievements-dark",
    "preferences-light-en": "preferences-light",
    "today-light-de": "today-german",
    "today-light-ru": "today-russian",
}


def main():
    source, output = sys.argv[1:]
    os.makedirs(output, exist_ok=True)
    for capture, name in SELECTION.items():
        with Image.open(os.path.join(source, f"{capture}.png")) as image:
            cropped = image.convert("RGB").crop(PANELS_REMOVED)
        reduced = cropped.quantize(colors=256, dither=Image.Dither.NONE)
        reduced.save(os.path.join(output, f"{name}.png"), optimize=True)


if __name__ == "__main__":
    main()
