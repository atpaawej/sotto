#!/usr/bin/env python3
"""Draw Sotto's app and tray icons.

The mark is three soft bars of a voice level, the middle one in signal red.
Run from the repo root: python3 tools/make-icons.py
"""

from pathlib import Path

from PIL import Image, ImageDraw

INK = (27, 28, 25, 255)
PAPER = (237, 235, 227, 255)
LIVE = (194, 69, 58, 255)

OUT = Path("src-tauri/icons")


def draw_mark(size: int, background, colours) -> Image.Image:
    scale = size / 512
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    if background is not None:
        draw.rounded_rectangle(
            [(0, 0), (size - 1, size - 1)],
            radius=int(112 * scale),
            fill=background,
        )

    bar_width = int(52 * scale)
    gap = int(40 * scale)
    centre = size / 2
    heights = [150, 250, 106]

    for index, height in enumerate(heights):
        offset = (index - 1) * (bar_width + gap)
        left = centre + offset - bar_width / 2
        top = centre - height * scale / 2
        draw.rounded_rectangle(
            [(left, top), (left + bar_width, top + height * scale)],
            radius=bar_width / 2,
            fill=colours[index],
        )

    return image


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)

    mark = (PAPER, LIVE, PAPER)
    draw_mark(512, INK, mark).save(OUT / "icon.png")
    draw_mark(256, INK, mark).save(OUT / "128x128@2x.png")
    draw_mark(128, INK, mark).save(OUT / "128x128.png")
    draw_mark(32, INK, mark).save(OUT / "32x32.png")

    # Tray: transparent and monochrome, so the panel decides the background.
    draw_mark(64, None, (PAPER, PAPER, PAPER)).save(OUT / "tray.png")


if __name__ == "__main__":
    main()
