//! Header bar and tab bar rendering helpers for the aetherdz TUI.
//!
//! Pure functions over [`ratatui::text::Line`] and [`ratatui::text::Span`] —
//! no terminal access, so everything here is unit-testable in isolation. The
//! widgets in `lib.rs` consume these lines directly.
//!
//! Palette (Modern Dark, from the ui-ux-pro-max design system):
//! - brand  → green   `#22c55e`
//! - screen → cyan    `#38bdf8`
//! - dim    → slate   `#64748b`
//! - accent → indigo  `#818cf8`

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Brand green.
const GREEN: Color = Color::Rgb(34, 197, 94);
/// Screen cyan.
const CYAN: Color = Color::Rgb(56, 189, 248);
/// Muted slate for secondary text.
const DIM: Color = Color::Rgb(100, 116, 139);
/// Indigo accent for the selected tab.
const ACCENT: Color = Color::Rgb(129, 140, 248);

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
        Style::new().fg(GREEN).add_modifier(Modifier::BOLD),
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
    let pad = (total_width as usize).saturating_sub(left_width + right_width);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    spans.extend([model, version, sessions]);
    Line::from(spans)
}

/// Render a tab bar from `(label, selected)` tuples.
///
/// The selected tab is drawn as `▍{label}` in bold indigo; unselected tabs are
/// ` {label}` in dim slate. Tabs are joined with a single space and the whole
/// line is padded with trailing spaces out to `total_width` (a floor, never a
/// truncation point).
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
                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
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
    fn total_width_is_a_floor_not_a_truncation() {
        let line = render_header(&sample_data(), 4);
        assert!(line.width() >= 4);
        assert!(line.to_string().contains("aetherdz"));
    }
}
