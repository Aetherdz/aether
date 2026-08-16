#!/usr/bin/env python3
"""
Aether logo exploration generator.

Pure-python, zero-dependency geometric SVG builder. Regenerate any direction
with different parameters instead of hand-editing SVGs:

    python3 generate.py --direction wind --stroke 8 --negative 0.62 --color rust
    python3 generate.py --all

Three distinct directions (not color variants of one idea):
  1. wind   — "Terminal Wind": a bold `>` prompt cursor whose apex trails into
              wind streaks. The cursor that quietly fills your terminal, like
              aether fills the sky. Reads as a single-color glyph at 16px.
  2. hex    — "Hex Crystal": a regular hexagon (Rust's compiled single binary)
              with a negative-space terminal cursor cut out of its core.
              Solid, self-contained, binary.
  3. aether — "Ethereal A": the letter A where the right leg is solid and the
              left leg dissolves into wind lines — negative space = the sky
              the aether occupies. The name, literally.

Every direction exports full-color (2 colors max) AND single-color mono.
Palettes: rust  (#E05D33 / #0A0A0A) or term-green (#00C853 / #0A0A0A).
"""

import argparse
import os
import sys

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "out")

PALETTES = {
    "rust": {"mark": "#E05D33", "dark": "#0A0A0A", "light": "#F5F5F5"},
    "term": {"mark": "#00C853", "dark": "#0A0A0A", "light": "#F5F5F5"},
}


def svg_wrap(inner: str, bg: str | None) -> str:
    bg_rect = ""
    if bg:
        bg_rect = f'<rect width="64" height="64" fill="{bg}"/>'
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" '
        'shape-rendering="geometricPrecision">'
        f"{bg_rect}{inner}</svg>"
    )


# ---------------------------------------------------------------- direction 1
def direction_wind(stroke: float, negative: float, mark: str, bg: str | None) -> str:
    """Bold `>` cursor with wind streaks trailing off its apex."""
    s = stroke
    # chevron: two strokes meeting at apex (20,32)
    arm_len = 20.0
    spread = 14.0 * negative
    apex_x, apex_y = 24.0, 32.0
    p1 = (apex_x - arm_len, apex_y - spread)
    p2 = (apex_x, apex_y)
    p3 = (apex_x - arm_len, apex_y + spread)
    chev = (
        f'<path d="M {p1[0]:.1f} {p1[1]:.1f} L {p2[0]:.1f} {p2[1]:.1f} '
        f'L {p3[0]:.1f} {p3[1]:.1f}" fill="none" stroke="{mark}" '
        f'stroke-width="{s:.1f}" stroke-linecap="round" stroke-linejoin="round"/>'
    )
    # wind streaks: 3 horizontal lines from apex, decreasing length/width
    streaks = ""
    for i, (y_off, w_mult, length, op) in enumerate(
        [
            (0.0, 0.95, 26.0, 1.0),
            (-spread * 0.62, 0.55, 18.0, 0.8),
            (spread * 0.62, 0.55, 18.0, 0.8),
        ]
    ):
        y = apex_y + y_off
        x0 = apex_x + 2.5
        x1 = apex_x + 2.5 + length * negative
        streaks += (
            f'<line x1="{x0:.1f}" y1="{y:.1f}" x2="{x1:.1f}" y2="{y:.1f}" '
            f'stroke="{mark}" stroke-width="{s * w_mult:.1f}" '
            f'stroke-linecap="round" opacity="{op}"/>'
        )
    return chev + streaks


# ---------------------------------------------------------------- direction 2
def direction_hex(stroke: float, negative: float, mark: str, bg: str | None) -> str:
    """Hexagon (single binary) with a negative-space `>` cursor cut from core."""
    s = stroke
    cx, cy = 32.0, 32.0
    r = 25.0
    # hexagon FILLED with mark color; cursor cut out in bg color = real negative space
    import math

    pts = []
    for k in range(6):
        ang = math.radians(30 + 60 * k)
        pts.append((cx + r * math.cos(ang), cy + r * math.sin(ang)))
    hex_path = "M " + " L ".join(f"{x:.1f} {y:.1f}" for x, y in pts) + " Z"
    body = (
        f'<path d="{hex_path}" fill="{mark}" stroke="{mark}" stroke-width="{s:.1f}" '
        'stroke-linejoin="round"/>'
    )
    # negative-space cursor: chevron drawn in bg color ON TOP of the filled hexagon
    cut = ""
    if bg:
        w = 9.0 * negative
        x0, x1 = cx - 6.5, cx + 6.0
        cut = (
            f'<path d="M {x0:.1f} {cy - w:.1f} L {x1:.1f} {cy:.1f} '
            f'L {x0:.1f} {cy + w:.1f}" fill="none" stroke="{bg}" '
            f'stroke-width="{s:.1f}" stroke-linecap="round" '
            'stroke-linejoin="round"/>'
        )
    return body + cut


# ---------------------------------------------------------------- direction 3
def direction_aether(stroke: float, negative: float, mark: str, bg: str | None) -> str:
    """A whose right leg is solid and left leg dissolves into wind lines."""
    s = stroke
    base_y = 52.0
    apex_x, apex_y = 32.0, 13.0
    # right leg: solid diagonal
    right = (
        f'<line x1="{apex_x:.1f}" y1="{apex_y:.1f}" x2="44.0" y2="{base_y:.1f}" '
        f'stroke="{mark}" stroke-width="{s:.1f}" stroke-linecap="round"/>'
    )
    # left leg: 4 wind segments, dissolving upward (shorter + fainter near apex)
    segs = ""
    n = 4
    for i in range(n):
        t0 = i / n
        t1 = (i + 1) / n
        x0 = apex_x + (20.0 - apex_x) * t0
        y0 = apex_y + (base_y - apex_y) * t0
        x1 = apex_x + (20.0 - apex_x) * t1
        y1 = apex_y + (base_y - apex_y) * t1
        op = 0.45 + 0.55 * ((i + 1) / n)
        segs += (
            f'<line x1="{x0:.1f}" y1="{y0:.1f}" x2="{x1:.1f}" y2="{y1:.1f}" '
            f'stroke="{mark}" stroke-width="{s * (0.5 + 0.5 * t1):.1f}" '
            f'stroke-linecap="round" opacity="{op:.2f}"/>'
        )
    # crossbar: short wind line at mid-height
    cross_y = 34.0
    cross = (
        f'<line x1="25.0" y1="{cross_y:.1f}" x2="37.0" y2="{cross_y:.1f}" '
        f'stroke="{mark}" stroke-width="{s * 0.7:.1f}" stroke-linecap="round"/>'
    )
    return right + segs + cross


DIRECTIONS = {
    "wind": direction_wind,
    "hex": direction_hex,
    "aether": direction_aether,
}

RATIONALES = {
    "wind": "A `>` prompt cursor whose apex trails into wind streaks — the tool that "
    "fills your terminal quietly, the way aether fills the sky. Reads at 16px.",
    "hex": "A hexagon (Rust's compiled single binary) with a terminal cursor cut from "
    "its core in negative space — solid, self-contained, binary.",
    "aether": "The letter A: solid right leg, left leg dissolving into wind lines — "
    "negative space is the sky the aether occupies. The name, literally.",
}


def build(name: str, palette: str, stroke: float, negative: float, mono: bool) -> str:
    pal = PALETTES[palette]
    fn = DIRECTIONS[name]
    if mono:
        mark = pal["light"]
        inner = fn(stroke, negative, mark, None)
        return svg_wrap(inner, None)
    inner = fn(stroke, negative, pal["mark"], pal["dark"])
    return svg_wrap(inner, pal["dark"])


def main() -> None:
    ap = argparse.ArgumentParser(description="Aether logo exploration generator")
    ap.add_argument("--direction", choices=list(DIRECTIONS), default="wind")
    ap.add_argument("--all", action="store_true", help="generate every direction")
    ap.add_argument("--palette", choices=list(PALETTES), default="rust")
    ap.add_argument("--stroke", type=float, default=8.0, help="stroke width (px)")
    ap.add_argument("--negative", type=float, default=0.6, help="negative-space ratio 0.2-1.0")
    ap.add_argument("--mono", action="store_true", help="single-color (no bg tile)")
    args = ap.parse_args()

    os.makedirs(OUT, exist_ok=True)
    dirs = list(DIRECTIONS) if args.all else [args.direction]
    written = []
    for d in dirs:
        color = build(d, args.palette, args.stroke, args.negative, mono=False)
        mono = build(d, args.palette, args.stroke, args.negative, mono=True)
        cf, mf = f"aether-{d}-color.svg", f"aether-{d}-mono.svg"
        for fname, content in ((cf, color), (mf, mono)):
            with open(os.path.join(OUT, fname), "w") as f:
                f.write(content)
            written.append(fname)
    print(f"wrote {len(written)} files to {OUT}:")
    for f in written:
        print("  ", f)


if __name__ == "__main__":
    sys.exit(main())