# aetherdz Design System

## 0. Research Log

- Embedded refs: shortlisted [opencode.ai, jcode.sh, aether-site] → picked [opencode.ai (Layer A — dark terminal aesthetic)] + [jcode.sh (Layer B — mono monochrome minimal)] because the product is a Rust terminal AI agent (like jcode.sh's Rust heritage) with a modern dark IDE identity (like opencode.ai's). Combined: dark base + mono type + orange accent + hairline borders.
- Lazyweb: 0 queries (refs provided directly by user, live-fetched).
- Imagen drafts: none — logo designed by hand as original SVG mark.
- Skipped lanes: ui-ux-db CLI — palette is dictated by the two named references; no additional search needed.

## 1. Atmosphere & Identity

A quiet command center. Dense when needed, spacious when not.
The signature is **terminal honesty** — the page reads like an editor buffer:
monospace everywhere, hairlines instead of shadows, and a single ember of
orange (the Aether glow) that appears only where the user can act.
It feels like a good terminal emulator: fast, monochrome, exact.

## 2. Color

### Palette

| Role | Token | Light | Dark | Usage |
|------|-------|-------|------|-------|
| Surface/primary | --surface-primary | #FFFFFF | #090909 | Main background |
| Surface/secondary | --surface-secondary | #F4F4F4 | #111111 | Cards, panels |
| Surface/elevated | --surface-elevated | #FFFFFF | #1A1A1A | Terminal window, popovers |
| Text/primary | --text-primary | #111111 | #F2F1F0 | Headlines, body |
| Text/secondary | --text-secondary | #666666 | #9C9C9C | Captions, hints |
| Text/tertiary | --text-tertiary | #999999 | #666666 | Disabled, muted |
| Border/default | --border-default | #CCCCCC | #2A2A2A | Dividers, outlines |
| Border/subtle | --border-subtle | #E5E5E5 | #1E1E1E | Soft separations |
| Accent/primary | --accent-primary | #FF9F0A | #FF9F0A | CTAs, links, focus, logo glow |
| Accent/hover | --accent-hover | #E88E00 | #FFB340 | Hover state |
| Status/success | --status-success | #16A34A | #30D158 | Confirmations, prompt $ |
| Status/error | --status-error | #DC2626 | #FF453A | Errors, terminal dots |

### Rules
- Surface hierarchy creates depth without shadows. Depth = tonal shift only.
- Accent (orange #FF9F0A) is used ONLY for interactive elements, the logo mark, and status cursors. Never decorative backgrounds.
- Terminal window dots (red #FF453A / yellow #FF9F0A / green #30D158) are the one decorative exception — inherited from macOS window chrome in the reference.
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
- **Structure**: `<svg>` — rounded-square tile (32px → 96px) with geometric "A" monogram + terminal cursor block. Drawn by hand, original geometry.
- **Variants**: full (mark + wordmark "aetherdz"), mark-only (favicon, nav small), mono-inline (text logo in terminal).
- **Spacing**: mark 8px gap from wordmark; wordmark in JetBrains Mono 700.
- **States**: static (no hover state on logo itself).
- **Motion**: subtle 1.5s ease-in-out glow pulse on the cursor block only, disabled under reduced-motion.
- **Accessibility**: `<title>` + `role="img"`; wordmark is real text, not SVG paths.
- **Layout**: inline flex cluster; scales via `height` only (width auto, preserves ratio).

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
Choose ONE and commit: **[tonal-shift + hairline borders]** — surfaces separate by 1px `--border-default` hairlines and one-step tonal shifts (#090909 → #111111 → #1A1A1A). No shadows anywhere.

| Type | Value | Usage |
|------|-------|-------|
| Default | 1px solid var(--border-default) | Cards, terminal, dividers |
| Subtle | 1px solid var(--border-subtle) | Soft separations |
| Elevated | #1A1A1A on #090909 | Terminal window pops via tone, not shadow |

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
| Terminal text is illustrative, not live | hero figure | Static marketing page; live terminal is product scope | Replace with real session recording when TUI ships video capture |
