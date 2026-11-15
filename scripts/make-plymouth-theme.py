#!/usr/bin/env python3
"""Draw the pictures of gevrek's boot splash (resources/plymouth/README.md).

    scripts/make-plymouth-theme.py

Writes resources/plymouth/gevrek/*.png from the brand's own files (the mark
and the lock, assets/brand). The pictures are kept in the repository, so a
package needs none of this; run it again when the mark changes.

Needs ImageMagick (`magick`, for the SVGs) and Pillow.

What plymouth's two-step module makes of them:

    throbber-NNNN.png   shown in a loop while the system starts
    animation-NNNN.png  shown once when the splash is told to end: the
                        picture goes into black, which is what the login
                        begins with
    lock, entry, bullet, capslock
                        the field for a disk's password
"""

import math
import pathlib
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw

REPO = pathlib.Path(__file__).resolve().parent.parent
BRAND = REPO / "assets" / "brand"
OUT = REPO / "resources" / "plymouth" / "gevrek"

# The brand's colours (assets/brand/USAGE.md).
CRUST = (0xD9, 0x70, 0x1F)
SESAME = (0xF4, 0xE2, 0xBC)
SURFACE = (0x2E, 0x1A, 0x10)
MUTED = (0xC4, 0xA4, 0x82)

# The mark, and a quarter of it free on each side (the brand's clear space).
MARK = 192
CLEAR = MARK // 4
WIDTH = MARK + 2 * CLEAR
# Under the mark: five bars, as the mark has five panes, and one of them lit.
BARS = 5
BAR_WIDTH = 18
BAR_HEIGHT = 4
BAR_GAP = 8
BARS_BELOW = CLEAR + 12
HEIGHT = MARK + BARS_BELOW + BAR_HEIGHT + 4
# A bar that is not lit: the crust, this much of it.
DIM = 0.38
# Drawn this much larger and brought down, for smooth edges.
OVER = 4

# The loop (plymouth spreads it over a fixed time, whatever its length).
LOOP = 30
# The end: 30 pictures a second.
END = 20


def raster(svg: pathlib.Path, size: int) -> Image.Image:
    """An SVG as a picture `size` high, on nothing."""
    with tempfile.TemporaryDirectory() as folder:
        png = pathlib.Path(folder) / "out.png"
        subprocess.run(
            ["magick", "-background", "none", "-density", "1200", str(svg),
             "-resize", f"{size}x{size}", f"PNG32:{png}"],
            check=True,
        )
        return Image.open(png).convert("RGBA").copy()


def lit(bar: int, place: float) -> float:
    """How lit a bar is when the light is at `place` (it goes round)."""
    distance = abs(bar - place % BARS)
    distance = min(distance, BARS - distance)
    return math.exp(-(distance * distance) / (2 * 0.62 * 0.62))


def bars(place: float) -> Image.Image:
    """The five bars with the light at `place`."""
    width = BARS * BAR_WIDTH + (BARS - 1) * BAR_GAP
    large = Image.new("RGBA", (width * OVER, BAR_HEIGHT * OVER), (0, 0, 0, 0))
    draw = ImageDraw.Draw(large)
    for bar in range(BARS):
        much = lit(bar, place)
        colour = tuple(round(CRUST[part] + (SESAME[part] - CRUST[part]) * much) for part in range(3))
        alpha = round(255 * (DIM + (1 - DIM) * much))
        left = bar * (BAR_WIDTH + BAR_GAP) * OVER
        draw.rounded_rectangle(
            [left, 0, left + BAR_WIDTH * OVER - 1, BAR_HEIGHT * OVER - 1],
            radius=BAR_HEIGHT * OVER // 2,
            fill=colour + (alpha,),
        )
    return large.resize((width, BAR_HEIGHT), Image.LANCZOS)


def frame(mark: Image.Image, step: int, there: float = 1.0) -> Image.Image:
    """One picture: the mark, the bars at this step, `there` of both."""
    picture = Image.new("RGBA", (WIDTH, HEIGHT), (0, 0, 0, 0))
    picture.alpha_composite(mark, (CLEAR, 0))
    row = bars(BARS * step / LOOP)
    picture.alpha_composite(row, ((WIDTH - row.width) // 2, MARK + BARS_BELOW))
    if there < 1.0:
        alpha = picture.getchannel("A").point(lambda value: round(value * there))
        picture.putalpha(alpha)
    return picture


def eased(part: float) -> float:
    part = min(max(part, 0.0), 1.0)
    return part * part * (3 - 2 * part)


def entry() -> Image.Image:
    """The field a disk's password is typed into."""
    width, height = 300, 40
    large = Image.new("RGBA", (width * OVER, height * OVER), (0, 0, 0, 0))
    draw = ImageDraw.Draw(large)
    draw.rounded_rectangle(
        [0, 0, width * OVER - 1, height * OVER - 1],
        radius=10 * OVER,
        fill=SURFACE + (255,),
        outline=MUTED + (110,),
        width=OVER,
    )
    return large.resize((width, height), Image.LANCZOS)


def bullet() -> Image.Image:
    size = 10
    large = Image.new("RGBA", (size * OVER, size * OVER), (0, 0, 0, 0))
    ImageDraw.Draw(large).ellipse([OVER, OVER, (size - 1) * OVER - 1, (size - 1) * OVER - 1], fill=SESAME + (255,))
    return large.resize((size, size), Image.LANCZOS)


def capslock() -> Image.Image:
    """An arrow up over a bar: the capitals are locked."""
    width, height = 24, 28
    large = Image.new("RGBA", (width * OVER, height * OVER), (0, 0, 0, 0))
    draw = ImageDraw.Draw(large)
    points = [(12, 3), (21, 13), (16, 13), (16, 19), (8, 19), (8, 13), (3, 13)]
    stroke = round(1.75 * OVER)
    scaled = [(x * OVER, y * OVER) for x, y in points]
    draw.line(scaled + [scaled[0]], fill=SESAME + (255,), width=stroke, joint="curve")
    for x, y in scaled:
        draw.ellipse([x - stroke / 2, y - stroke / 2, x + stroke / 2, y + stroke / 2], fill=SESAME + (255,))
    draw.rounded_rectangle([8 * OVER, 23 * OVER, 16 * OVER, 25 * OVER], radius=OVER, fill=SESAME + (255,))
    return large.resize((width, height), Image.LANCZOS)


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    for old in OUT.glob("*.png"):
        old.unlink()
    mark = raster(BRAND / "logo" / "mark" / "gevrek-mark-on-dark.svg", MARK)
    for step in range(LOOP):
        frame(mark, step).save(OUT / f"throbber-{step + 1:04}.png", optimize=True)
    # The end goes on from where the loop stops, into nothing.
    for step in range(END):
        there = 1.0 - eased((step + 1) / END)
        frame(mark, LOOP + step, there).save(OUT / f"animation-{step + 1:04}.png", optimize=True)
    raster(BRAND / "icons" / "color" / "gevrek-lock.svg", 36).save(OUT / "lock.png", optimize=True)
    entry().save(OUT / "entry.png", optimize=True)
    bullet().save(OUT / "bullet.png", optimize=True)
    capslock().save(OUT / "capslock.png", optimize=True)
    files = sorted(OUT.glob("*.png"))
    print(f"{len(files)} pictures in {OUT.relative_to(REPO)} ({sum(file.stat().st_size for file in files) // 1024} KiB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
