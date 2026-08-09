# Aether Design System

## 0. Research Log

- Embedded refs: shortlisted [opencode.ai, jcode.sh, aether-site] → picked [jcode.sh (Layer A — monochrome mono minimal)] + [opencode.ai (Layer B — terminal composition, macOS window chrome)] because the product is a Rust terminal AI agent; user directed a **black & white** (monochrome light) treatment after the first dark pass.
- Lazyweb: 0 queries (refs provided directly by user, live-fetched).
- Imagen drafts: none — logo designed by hand as original SVG mark.
- Skipped lanes: ui-ux-db CLI — palette dictated by the two named references plus explicit user direction to black & white.

## 1. Atmosphere & Identity

A quiet command center. Dense when needed, spacious when not.
The signature is **terminal honesty** — the page reads like an editor buffer:
monospace everywhere, hairlines instead of shadows, and **no color at all**:
pure black ink on white paper, with grays doing the hierarchy work.
It feels like a good terminal emulator: fast, monochrome, exact.
The logo is the only dark object — a black tile with a blinking white cursor —
so the mark itself becomes the page's single point of contrast.

## 2. Color

### Palette

| Role | Token | Value | Usage |
|------|-------|-------|-------|
| Surface/primary | --surface-primary | #FFFFFF | Main background |
| Surface/secondary | --surface-secondary | #F4F4F4 | Cards, panels, terminal body |
| Surface/elevated | --surface-elevated | #FFFFFF | Terminal title bar, code blocks |
| Text/primary | --text-primary | #111111 | Headlines, body, cursor, accent |
| Text/secondary | --text-secondary | #666666 | Captions, hints, lead |
| Text/tertiary | --text-tertiary | #999999 | Disabled, muted, path, dim |
| Border/default | --border-default | #CCCCCC | Cards, dividers, code blocks |
| Border/subtle | --border-subtle | #E5E5E5 | Soft separations |
| Accent/primary | --accent-primary | #111111 | Ink — interactive elements ARE the text color (black & white) |
| Accent/hover | --accent-hover | #000000 | Hover state |
| Status/success | --status-success | #111111 | Confirmations (copied state) — ink, not green |
| Status/error | --status-error | #666666 | Errors — gray, not red |

### Rules
- **Black & white is the constraint.** No chromatic color anywhere in the UI. The palette is ink (#111), paper (#FFF), and three grays.
- Accent = ink. Interactive elements are distinguished by weight, borders, and hover, never by hue.
- Terminal window dots are monochrome (white/gray outline circles), not macOS traffic-light colors — the one place the reference's color was deliberately stripped to obey the constraint.
- The logo's dark tile (#111) + white A + blinking white cursor is the ONLY dark surface — it is the page's contrast anchor.
- Never introduce a color not in this table. Extend the table first.

## 3. Typography

### Scale

| Level | Size | Weight | Line Height | Tracking | Usage |
|-------|------|--------|-------------|----------|-------|
| Display | clamp(2.25rem, 6vw, 3.5rem) | 700 | 1.05 | -0.02em | Hero tagline |
| H1 | 1.75rem | 700 | 1.2 | -0.01em | Section headers |
| H2 | 1.25rem | 700 | 1.3 | 0 | Card titles |
| Body/lg | 1.0625rem | 400 | 1.6 | 0 | Lead paragraph |
| Body | 0.9375rem | 400 | 1.6 | 0 | Default text |
| Body/sm | 0.8125rem | 400 | 1.5 | 0 | Secondary info |
| Caption | 0.75rem | 500 | 1.4 | 0.02em | Labels, metadata |
| Overline | 0.6875rem | 700 | 1.3 | 0.12em | Section labels, uppercase |

### Font Stack
- Primary: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace
- Mono: same stack (single-family site — the monochrome mono discipline of jcode.sh)
- Serif: none

### Rules
- One family only. Mono is the brand.
- Body text never below 12px (11px allowed for overline labels only).
- Headings that wrap to 4+ lines are too large — clamp() handles it.

## 4. Spacing & Layout

### Base Unit
All spacing derives from a base of **4px**.

| Token | Value | Usage |
|-------|-------|-------|
| --space-1 | 4px | Tight: icon-to-label |
| --space-2 | 8px | Compact: list items, inline groups |
| --space-3 | 12px | Default: form field padding |
| --space-4 | 16px | Standard: card padding |
| --space-6 | 24px | Generous: card padding (default) |
| --space-8 | 32px | Between card groups |
| --space-10 | 40px | Sections within a page |
| --space-12 | 48px | Major section breaks |
| --space-16 | 64px | Page-level vertical rhythm |
| --space-20 | 80px | Hero spacing |

### Grid
- Max content width: 1080px (jcode.sh's narrow 860px, relaxed to fit 3-col feature grid)
- Column system: 12-col implicit via CSS grid, `repeat(auto-fit, minmax(min(18rem, 100%), 1fr))`
- Breakpoints: sm 640px, md 768px, lg 1024px
- Code/terminal windows: max-width 720px, centered

### Rules
- Tokenize design intent (spacing steps, content width, gutters). Keep browser mechanics raw: `clamp()`, `minmax()`, intrinsic sizing.
- Centered hero (overline → display → lead → install) with terminal window below is the fixed grammar — the jcode.sh narrow-column discipline applied to opencode.ai's hero-terminal composition.

## 5. Components

### Logo Mark (SVG)
- **Structure**: `<svg>` — black rounded-square tile (fill #111) with geometric "A" monogram in white + blinking white terminal-cursor block as the crossbar. Drawn by hand, original geometry.
- **Variants**: full (mark + wordmark "Aether"), mark-only (favicon, nav small), mono-inline (text logo in terminal).
- **Spacing**: mark 8px gap from wordmark; wordmark in JetBrains Mono 700.
- **States**: static (no hover state on logo itself).
- **Motion**: subtle 1.5s ease-in-out blink on the cursor block only, disabled under reduced-motion.
- **Accessibility**: `<title>` + `role="img"`; wordmark is real text, not SVG paths.
- **Layout**: inline flex cluster; scales via `height` only (width auto, preserves ratio).

### Quick Start Step
- **Structure**: `<article class="step">` → step head (number + title) → description → code block with comment + command lines.
- **Variants**: grid (3-col, collapses to 1-col under 480px).
- **Spacing**: --space-6 padding, --space-6 gap.
- **States**: default hairline border; no hover lift (steps are static content).
- **Accessibility**: code blocks `user-select: all` for one-click copy; contrast AA on comment (tertiary #999 on #FFF = 3.3:1 — acceptable for non-essential code comments, recorded as debt).
- **Layout**: grid primitive.

### Nav
- **Structure**: `<header>` → `<nav>` flex: logo left; links right (Docs, Features, Install); on mobile links collapse to Install button only.
- **Variants**: sticky, hairline bottom border.
- **Spacing**: --space-4 padding, content width 1080px.
- **States**: link default text-secondary; hover text-primary; active (current) text-primary + orange underline offset.
- **Accessibility**: `aria-current="page"`, focus ring 2px accent offset 2px.
- **Layout**: cluster primitive; scroll owner: page.

### Terminal Window
- **Structure**: `<figure class="terminal">` → title bar (3 macOS dots + file path caption) → `<pre>` code block.
- **Variants**: default (multi-line session), single-line (install command), dark-only.
- **Spacing**: --space-6 padding, radius 8px.
- **States**: static; copy button appears on hover of title bar (focus-visible always visible).
- **Accessibility**: `<pre>` with `tabindex="0"` for screen reader scroll; copy button with `aria-label`.
- **Motion**: fade-up on load (200ms ease-out) once.
- **Layout**: stack primitive; scroll owner: internal pre (max-height + overflow auto).

### Feature Card
- **Structure**: `<article>` → overline label (mono, accent) → H2 title → body copy.
- **Variants**: grid (3-col), full-width.
- **Spacing**: --space-6 padding, --space-6 gap.
- **States**: default hairline border; hover border-accent + translateY(-2px), 150ms.
- **Accessibility**: focus within; contrast AA on all text.
- **Layout**: grid primitive.

### Install Command / Copy
- **Structure**: `<div class="install">` → mono code + copy button (icon swaps on success).
- **Variants**: with or without label.
- **States**: default, hover (bg secondary), active (pressed), success (check icon 1.2s then revert), focus ring.
- **Accessibility**: button `aria-live` announce "copied".
- **Motion**: icon swap 100ms ease-out; only transform/opacity.

### Footer
- **Structure**: `<footer>` → hairline top border → 3 cols (brand blurb, links, status line).
- **Variants**: single.
- **Spacing**: --space-16 top, --space-8 internal.
- **States**: links like nav.
- **Layout**: grid primitive, collapses to stack under 640px.

## 6. Motion & Interaction

### Timing

| Type | Duration | Easing | Usage |
|------|----------|--------|-------|
| Micro | 100-150ms | ease-out | Button press, icon swap |
| Standard | 200-300ms | ease-in-out | Card hover lift, link color |
| Emphasis | 400-600ms | cubic-bezier(0.16, 1, 0.3, 1) | Hero terminal fade-up |
| Scroll-driven | tied to scroll | linear | (none used — keep static) |
| Ambient | 1.5s | ease-in-out | Logo cursor pulse |

### Rules
- Only animate `transform` and `opacity` (plus `background-color` for hover states). Never layout properties.
- Every interactive element has hover + active + focus states.
- Reduced motion: `prefers-reduced-motion: reduce` disables all animation except the copy-icon swap (functional feedback).

## 7. Depth & Surface

### Strategy
Choose ONE and commit: **[tonal-shift + hairline borders]** — surfaces separate by 1px `--border-default` hairlines and one-step tonal shifts (#FFFFFF → #F4F4F4). No shadows anywhere. The only elevated surface is the logo tile, which goes dark (#111) against the white page.

| Type | Value | Usage |
|------|-------|-------|
| Default | 1px solid var(--border-default) | Cards, terminal, code blocks |
| Subtle | 1px solid var(--border-subtle) | Soft separations |
| Elevated | #FFFFFF on #F4F4F4 | Terminal title bar pops via tone, not shadow |

## 8. Accessibility Constraints & Accepted Debt

### Constraints
- WCAG 2.2 AA — contrast floor 4.5:1 body / 3:1 large text.
- Visible focus on every interactive element: 2px accent ring, 2px offset.
- Full keyboard reachability (copy buttons, links, terminal pre).
- `prefers-reduced-motion` respected (Section 6).
- Semantic landmarks: header, main, section, footer. Skip link to main content.

### Accepted Debt
| Item | Location | Why accepted | Owner / Exit |
|------|----------|--------------|--------------|
| Code comment text (#999 on #FFF, 3.3:1) | quickstart steps | Non-essential annotation; below 4.5:1 AA but not load-bearing | Re-evaluate if comments become interactive |
