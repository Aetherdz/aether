# Aether logo exploration

Local, parameterized logo generator for **Aether** — a Rust terminal AI coding
agent. Three distinct directions (not color variants of one idea), each
exported full-color and single-color. Regenerate variants in seconds instead
of hand-editing SVGs.

## Why a generator (honest note)

Raw hand-written SVG from an LLM is often mediocre for logo work. This folder
is the fallback the design brief asks for: a small local environment to
iterate in, with geometry exposed as parameters, plus a preview page to
eyeball every variant at true favicon size (16px).

For production polish beyond this, the brief's external routes apply:
[Recraft.ai](https://www.recraft.ai) or [Looka](https://looka.com) for
AI-generated brand marks, or a geometric icon set (Tabler / Phosphor) in
Figma as a starting point that a designer adjusts.

## Quick start

```sh
# generate all three directions (color + mono each) -> out/
python3 generate.py --all

# iterate on one direction with your own parameters
python3 generate.py --direction wind --stroke 10 --negative 0.7
python3 generate.py --direction hex   --stroke 7  --negative 0.55 --palette term
python3 generate.py --direction aether --stroke 9 --negative 0.5

# view everything
open preview.html
```

## Parameters

| Flag | Default | What it controls |
|---|---|---|
| `--direction` | `wind` | `wind` · `hex` · `aether` |
| `--palette` | `rust` | `rust` (`#E05D33`) or `term` (green `#00C853`) |
| `--stroke` | `8.0` | stroke width in px (raise for 16px legibility) |
| `--negative` | `0.6` | negative-space ratio, 0.2–1.0 (lower = denser mark) |
| `--mono` | off | single-color on transparent (no bg tile) |

## Directions

| # | name | idea | reads at 16px |
|---|---|---|---|
| 1 | `wind` | `>` prompt cursor trailing into wind streaks — aether filling the sky, quietly | yes |
| 2 | `hex` | hexagon (Rust single binary) with a terminal cursor cut from its core in negative space | yes |
| 3 | `aether` | the letter A, right leg solid, left leg dissolving into wind lines | yes |

Each mono export is one color on a transparent background, so it works as a
terminal-prompt glyph or favicon on any background. No gradients, no shadows,
no embedded raster — flat editable vector paths only.

## Files

- `generate.py` — zero-dependency generator (pure python, stdlib only)
- `preview.html` — side-by-side viewer with 16px size test
- `out/*.svg` — the generated exports