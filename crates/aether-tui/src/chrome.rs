//! Header bar and tab bar rendering helpers for the aetherdz TUI.
//!
//! Pure functions over [`ratatui::text::Line`] and [`ratatui::text::Span`] —
//! no terminal access, so everything here is unit-testable in isolation. The
//! widgets in `lib.rs` consume these lines directly.
//!
//! Palette (Modern Dark, from the ui-ux-pro-max design system). Semantics
//! match `theme` in `lib.rs` — one accent concept across the app:
//! - accent → green   `#22c55e` (brand — the single accent)
//! - screen → cyan    `#38bdf8`
//! - dim    → slate   `#64748b`
//! - ai     → indigo  `#818cf8` (AI/agent)

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Brand green — the single accent (matches `theme::ACCENT` in lib.rs).
const ACCENT: Color = Color::Rgb(34, 197, 94);
/// Screen cyan.
const CYAN: Color = Color::Rgb(56, 189, 248);
/// Muted slate for secondary text.
const DIM: Color = Color::Rgb(100, 116, 139);
/// AI/agent indigo (matches `theme::AI` in lib.rs).
const AI: Color = Color::Rgb(129, 140, 248);

/// Everything the header bar needs to render, gathered in one place.
#[derive(Clone, Debug, Default)]
pub struct HeaderData<'a> {
    /// Product brand, drawn bold green on the left.
    pub brand: &'a str,
    /// Current screen name, drawn cyan after the brand.
    pub screen: &'a str,
    /// Active model name (right-aligned group).
    pub model: &'a str,
    /// Provider name (right-aligned group).
    pub provider: &'a str,
    /// App version, prefixed with `v`.
    pub version: &'a str,
    /// Number of open sessions.
    pub session_count: usize,
}

/// Render the one-line header bar.
///
/// Left group: `{brand}` bold green followed by ` · {screen}` cyan. Right
/// group: `{model} · {provider}`, ` v{version}`, and ` · {n} sessions`, all
/// dim. The right group is right-aligned: the left spans are padded with
/// spaces so the whole line reaches `total_width`. `total_width` is a floor —
/// content wider than it is never truncated.
pub fn render_header(data: &HeaderData, total_width: u16) -> Line<'static> {
    let brand = Span::styled(
        data.brand.to_string(),
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    );
    let screen = Span::styled(format!(" · {}", data.screen), Style::new().fg(CYAN));
    let model = Span::styled(
        format!("{} · {}", data.model, data.provider),
        Style::new().fg(DIM),
    );
    let version = Span::styled(format!(" v{}", data.version), Style::new().fg(DIM));
    let sessions = Span::styled(
        format!(" · {} sessions", data.session_count),
        Style::new().fg(DIM),
    );

    // Visible width in display columns; each char counts as 1 (unicode-safe).
    let left_width = brand.content.chars().count() + screen.content.chars().count();
    let right_width = model.content.chars().count()
        + version.content.chars().count()
        + sessions.content.chars().count();

    let mut spans: Vec<Span<'static>> = vec![brand, screen];
    let pad = (total_width as usize).saturating_sub(left_width);
    if right_width <= pad {
        spans.push(Span::raw(" ".repeat(pad - right_width)));
        spans.extend([model, version, sessions]);
    } else if pad > 0 {
        // The right group doesn't fit: truncate it gracefully with a
        // trailing `…` instead of letting the terminal hard-clip it.
        let right_text = format!("{}{}{}", model.content, version.content, sessions.content);
        spans.push(Span::styled(
            truncate(&right_text, pad.saturating_sub(1)),
            Style::new().fg(DIM),
        ));
    }
    Line::from(spans)
}

/// Render a tab bar from `(label, selected)` tuples.
///
/// The selected tab is drawn as `▍{label}` in bold underlined brand green;
/// unselected tabs are ` {label}` in dim slate. Tabs are joined with a single
/// space and the whole line is padded with trailing spaces out to
/// `total_width` (a floor, never a truncation point).
pub fn render_tabs(tabs: &[(&'static str, bool)], total_width: u16) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut width = 0usize;

    for (i, (label, selected)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
            width += 1;
        }
        if *selected {
            let text = format!("▍{label}");
            width += text.chars().count();
            spans.push(Span::styled(
                text,
                Style::new()
                    .fg(ACCENT)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ));
        } else {
            let text = format!(" {label}");
            width += text.chars().count();
            spans.push(Span::styled(text, Style::new().fg(DIM)));
        }
    }

    let pad = (total_width as usize).saturating_sub(width);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    Line::from(spans)
}

/// Format a token count like opencode's status bar: 999 → `999`, 1500 →
/// `1.5K`, 12000 → `12K` (`.0` dropped), 1_200_000 → `1.2M`. `0` renders as
/// `0`. Always compact.
pub fn format_tokens(tokens: u64) -> String {
    match tokens {
        0..=999 => format!("{tokens}"),
        1_000..=999_999 => {
            let s = format!("{:.1}K", tokens as f64 / 1_000.0);
            s.replace(".0K", "K")
        }
        _ => {
            let s = format!("{:.1}M", tokens as f64 / 1_000_000.0);
            s.replace(".0M", "M")
        }
    }
}

/// Format the usage line for the sidebar, opencode-style:
/// `"in 1.2K · out 3.4K · total 4.6K · 12% of ctx"`. The context-window
/// percentage is omitted when `context_window` is 0.
pub fn usage_line(input: u64, output: u64, context_window: u64) -> String {
    let total = input + output;
    let base = format!(
        "in {} · out {} · total {}",
        format_tokens(input),
        format_tokens(output),
        format_tokens(total)
    );
    if context_window == 0 {
        return base;
    }
    let pct = total as f64 / context_window as f64 * 100.0;
    format!("{base} · {pct:.0}% of ctx")
}

/// Render the aether brand mark: an ascending triangle — the
/// plan → build → route flow. Each line is a `&str`; callers may join them
/// vertically for a sidebar or print one line per row.
pub const LOGO: [&str; 5] = [
    "      ▲",
    "     ▲ ▲",
    "    ▲   ▲",
    "   ▲     ▲",
    "  ▲ ▲ ▲ ▲ ▲",
];

/// Render the full logo as one multi-line `Line` per row, with the apex in
/// brand green and the base in cyan, ending with the wordmark.
pub fn render_logo() -> Vec<Line<'static>> {
    let mut out = Vec::with_capacity(LOGO.len() + 1);
    for (i, row) in LOGO.iter().enumerate() {
        let color = if i == 0 {
            ACCENT
        } else if i == LOGO.len() - 1 {
            AI
        } else {
            CYAN
        };
        out.push(Line::from(Span::styled(*row, Style::new().fg(color))));
    }
    out.push(Line::from(Span::styled(
        "aether",
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));
    out
}

/// Truncate `s` to at most `max` characters, appending a single `…` when
/// anything was cut. Char boundaries are respected — multi-byte characters are
/// never split.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> HeaderData<'static> {
        HeaderData {
            brand: "aetherdz",
            screen: "sessions",
            model: "gpt-4o-mini",
            provider: "openai",
            version: "0.3.2",
            session_count: 4,
        }
    }

    #[test]
    fn header_contains_brand_and_screen() {
        let line = render_header(&sample_data(), 120);
        let text = line.to_string();
        assert!(text.contains("aetherdz"));
        assert!(text.contains("sessions"));
    }

    #[test]
    fn header_contains_right_aligned_meta() {
        let line = render_header(&sample_data(), 120);
        let text = line.to_string();
        assert!(text.contains("gpt-4o-mini · openai"));
        assert!(text.contains("v0.3.2"));
        assert!(text.contains("4 sessions"));
    }

    #[test]
    fn tabs_mark_exactly_one_selected_bold() {
        let tabs: [(&'static str, bool); 3] =
            [("chat", false), ("sessions", true), ("settings", false)];
        let line = render_tabs(&tabs, 60);
        let bold: Vec<&Span<'static>> = line
            .spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
            .collect();
        assert_eq!(bold.len(), 1, "exactly one tab is bold");
        assert_eq!(bold[0].style.fg, Some(ACCENT));
        assert_eq!(bold[0].content.as_ref(), "▍sessions");
    }

    #[test]
    fn truncate_keeps_short_input_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn truncate_cuts_long_input_with_ellipsis() {
        let out = truncate("hello world", 5);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= 5 + 1);
        assert!(out.starts_with("hello"));
    }

    #[test]
    fn header_pads_to_total_width() {
        let line = render_header(&sample_data(), 90);
        assert!(
            line.width() >= 90,
            "line pads to at least the requested width"
        );
    }

    #[test]
    fn tabs_pad_to_total_width() {
        let tabs: [(&'static str, bool); 2] = [("chat", true), ("history", false)];
        let line = render_tabs(&tabs, 50);
        assert!(
            line.width() >= 50,
            "tab line pads to at least the requested width"
        );
    }

    #[test]
    fn header_truncates_right_group_on_narrow_width() {
        let line = render_header(&sample_data(), 30);
        let text = line.to_string();
        assert!(text.contains("aetherdz"));
        assert!(
            text.ends_with('…'),
            "the right group must end with an ellipsis, got: {text:?}"
        );
        assert!(
            line.width() <= 30,
            "a truncated header must fit the requested width, got {}",
            line.width()
        );
    }

    #[test]
    fn header_omits_right_group_when_no_room() {
        let line = render_header(&sample_data(), 10);
        let text = line.to_string();
        assert!(text.contains("aetherdz"));
        assert!(
            !text.contains("gpt-4o-mini"),
            "the right group must be omitted when it cannot fit, got: {text:?}"
        );
    }

    #[test]
    fn selected_tab_is_underlined() {
        let tabs: [(&'static str, bool); 2] = [("chat", true), ("sessions", false)];
        let line = render_tabs(&tabs, 40);
        let selected = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "▍chat")
            .expect("selected tab must render with the ▍ glyph");
        assert!(
            selected.style.add_modifier.contains(Modifier::UNDERLINED),
            "the selected tab must be underlined"
        );
    }

    #[test]
    fn total_width_is_a_floor_not_a_truncation() {
        let line = render_header(&sample_data(), 4);
        assert!(line.width() >= 4);
        assert!(line.to_string().contains("aetherdz"));
    }

    #[test]
    fn logo_renders_five_art_rows_plus_wordmark() {
        let lines = render_logo();
        assert_eq!(lines.len(), LOGO.len() + 1);
        assert!(lines[0].to_string().contains('▲'));
        assert!(lines[lines.len() - 1].to_string().trim() == "aether");
    }

    #[test]
    fn logo_rows_are_colored_alternating() {
        let lines = render_logo();
        let apex = &lines[0].spans[0].style.fg;
        let base = &lines[LOGO.len() - 1].spans[0].style.fg;
        assert_eq!(*apex, Some(ACCENT));
        assert_eq!(*base, Some(AI));
        assert!(
            lines[lines.len() - 1].spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn format_tokens_uses_compact_units() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_500), "1.5K");
        assert_eq!(format_tokens(1_200_000), "1.2M");
    }

    #[test]
    fn format_tokens_drops_point_zero_suffix() {
        assert_eq!(format_tokens(12_000), "12K");
        assert_eq!(format_tokens(3_400_000), "3.4M");
    }

    #[test]
    fn usage_line_omits_percentage_without_context() {
        assert_eq!(
            usage_line(1_200, 3_400, 0),
            "in 1.2K · out 3.4K · total 4.6K"
        );
    }

    #[test]
    fn usage_line_appends_context_percentage() {
        let line = usage_line(12_000, 34_000, 100_000);
        assert!(line.starts_with("in 12K · out 34K · total 46K"));
        assert!(line.ends_with("% of ctx"));
    }
}
