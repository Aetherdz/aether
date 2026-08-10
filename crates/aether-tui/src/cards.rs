//! Pure, unit-testable helpers for rendering "plan card" and "todos card"
//! styled text blocks in the aether agent TUI.
//!
//! This module is deliberately side-effect free: it only builds
//! [`ratatui::text::Line`] / [`ratatui::text::Span`] values from strings and
//! never touches the terminal, files, or any other I/O. Box-drawing and pip
//! characters follow the shared Modern Dark visual language (slate + accents).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Violet used for the plan-card title.
const VIOLET: Color = Color::Rgb(158, 135, 255);
/// Green used for completed todo pips.
const GREEN: Color = Color::Rgb(34, 197, 94);
/// Dim slate used for hollow pips and the empty-meter placeholder.
const DIM: Color = Color::Rgb(100, 116, 139);

/// A parsed ` ```plan ` fenced block, ready to be rendered as a card.
#[derive(Debug, PartialEq, Eq)]
pub struct PlanCard<'a> {
    /// Card title, shown in the top border. Falls back to `"Plan"`.
    pub title: &'a str,
    /// Body lines of the block (the heading line is excluded).
    pub lines: Vec<String>,
}

/// Find the FIRST fenced block whose fence opens with ` ```plan ` and return
/// its parsed title and body lines.
///
/// The opening fence is a line that is exactly `"```plan"` optionally
/// followed by whitespace; the closing fence is the next line starting with
/// `"```"`. The title is the first content line starting with `'#'` (with the
/// leading `"# "` stripped), falling back to `"Plan"` when no heading exists.
pub fn extract_plan_card(text: &str) -> Option<PlanCard<'_>> {
    let mut rest = text.lines();

    // Locate the opening fence.
    let mut found = false;
    for line in rest.by_ref() {
        if line.trim_end() == "```plan" {
            found = true;
            break;
        }
    }
    if !found {
        return None;
    }

    // Collect content until the closing fence (a line starting with "```").
    let mut content: Vec<&str> = Vec::new();
    for line in rest {
        if line.trim_start().starts_with("```") {
            break;
        }
        content.push(line);
    }

    // Title: first content line starting with '#', leading '#'s stripped.
    let heading_idx = content.iter().position(|l| l.trim_start().starts_with('#'));
    let title = match heading_idx {
        Some(i) => {
            let t = content[i].trim().trim_start_matches('#').trim();
            if t.is_empty() { "Plan" } else { t }
        }
        None => "Plan",
    };

    // Body: every content line except the heading, trailing empties removed.
    let mut lines: Vec<String> = content
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != heading_idx)
        .map(|(_, l)| (*l).to_string())
        .collect();
    while matches!(lines.last(), Some(l) if l.trim().is_empty()) {
        lines.pop();
    }

    Some(PlanCard { title, lines })
}

/// Render a [`PlanCard`] as a rounded-bordered text box:
///
/// ```text
/// ╭─{title}─╮
/// │ wrapped │
/// ╰─╯
/// ```
///
/// The title is drawn in violet bold on the top border. Body lines are wrapped
/// to the inner width (`width - 2`) and truncated with `…` when they still
/// overflow.
pub fn render_plan_card(card: &PlanCard, width: u16) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(2) as usize;
    let mut out: Vec<Line<'static>> = Vec::new();

    // Top border with the title in violet bold.
    let mut top = Line::default();
    top.push_span(Span::raw("╭─"));
    top.push_span(Span::styled(
        card.title.to_string(),
        Style::default().fg(VIOLET).add_modifier(Modifier::BOLD),
    ));
    top.push_span(Span::raw("─╮"));
    out.push(top);

    // Body rows, each padded to the inner width so the right border aligns.
    for body in &card.lines {
        for wrapped in wrap_to_width(body, inner) {
            let truncated = truncate_to_width(&wrapped, inner);
            let pad = inner.saturating_sub(truncated.chars().count());
            let padded = format!("{truncated}{}", " ".repeat(pad));
            let mut row = Line::default();
            row.push_span(Span::raw("│"));
            row.push_span(Span::raw(padded));
            row.push_span(Span::raw("│"));
            out.push(row);
        }
    }

    out.push(Line::from(Span::raw("╰─╯")));
    out
}

/// A pip meter: `{done}/{total}` followed by a `width`-long bar of `●`/`○`
/// pips. Filled pips are green, hollow pips are dim slate. When `total == 0`
/// a single dim `—` is rendered instead.
pub fn render_todos_pip_meter(done: usize, total: usize, width: u16) -> Line<'static> {
    if total == 0 {
        return Line::from(Span::styled("—".to_string(), Style::default().fg(DIM)));
    }
    let w = width as usize;
    let filled = done.min(w);
    let hollow = w - filled;
    let mut meter = Line::default();
    meter.push_span(Span::raw(format!("{done}/{total} ")));
    meter.push_span(Span::styled("●".repeat(filled), Style::default().fg(GREEN)));
    meter.push_span(Span::styled("○".repeat(hollow), Style::default().fg(DIM)));
    meter
}

/// Character-based wrap: each chunk is capped at `width` columns. When a
/// break is needed the chunk is split at the last space before the limit so
/// whole words survive; words longer than the limit are hard-broken on
/// character boundaries.
fn wrap_to_width(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut last_space: Option<usize> = None;
    for ch in text.chars() {
        // Collapse leading whitespace on a fresh chunk.
        if current.is_empty() && ch == ' ' {
            continue;
        }
        if ch == ' ' {
            last_space = Some(current.chars().count());
        }
        current.push(ch);
        if current.chars().count() >= width {
            match last_space {
                Some(0) => {
                    // Only a leading space fits: drop it and keep filling.
                    current.remove(0);
                    last_space = None;
                }
                Some(i) => {
                    // Break after the last space so the next word is intact.
                    let head: String = current.chars().take(i + 1).collect();
                    let rest: String = current.chars().skip(i + 1).collect();
                    out.push(head);
                    current = rest;
                    last_space = last_space_in(&current);
                }
                None => {
                    // No space to break at: hard character break.
                    out.push(current.clone());
                    current.clear();
                    last_space = None;
                }
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Index (in chars) of the last space in `s`, if any.
fn last_space_in(s: &str) -> Option<usize> {
    s.char_indices()
        .filter(|&(_, c)| c == ' ')
        .map(|(i, _)| s[..i].chars().count())
        .last()
}

/// Truncate `line` to at most `width` columns, appending `…` when it is cut.
fn truncate_to_width(line: &str, width: usize) -> String {
    let count = line.chars().count();
    if count <= width {
        return line.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let keep = width.saturating_sub(1);
    let mut out: String = line.chars().take(keep).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_finds_plan_fence_title_and_lines() {
        let text = "intro\n```plan   \n# Sprint 12\n- task one\n- task two\n\n```\ntail";
        let card = extract_plan_card(text).expect("plan block");
        assert_eq!(card.title, "Sprint 12");
        assert_eq!(card.lines, vec!["- task one", "- task two"]);
    }

    #[test]
    fn extract_returns_none_without_plan_fence() {
        assert!(extract_plan_card("no fence here").is_none());
        assert!(extract_plan_card("```bash\necho hi\n```").is_none());
        assert!(extract_plan_card("```planning\n# T\nbody\n```").is_none());
    }

    #[test]
    fn extract_falls_back_to_plan_title() {
        let card = extract_plan_card("```plan\n- a\n- b\n```").expect("block");
        assert_eq!(card.title, "Plan");
        assert_eq!(card.lines, vec!["- a", "- b"]);
    }

    #[test]
    fn extract_takes_first_plan_block() {
        let text = "```plan\n# First\nA\n```\n```plan\n# Second\nB\n```";
        let card = extract_plan_card(text).expect("block");
        assert_eq!(card.title, "First");
        assert_eq!(card.lines, vec!["A"]);
    }

    #[test]
    fn render_card_has_borders_and_title() {
        let card = PlanCard {
            title: "Plan",
            lines: vec!["alpha".to_string(), "beta".to_string()],
        };
        let out = render_plan_card(&card, 20);
        assert!(!out.is_empty());

        let top = out[0].to_string();
        assert!(top.starts_with('╭'));
        assert!(top.contains("Plan"));
        assert!(top.ends_with('╮'));

        for row in &out[1..out.len() - 1] {
            let s = row.to_string();
            assert!(s.starts_with('│'), "missing left border: {s:?}");
            assert!(s.ends_with('│'), "missing right border: {s:?}");
        }

        let bottom = out[out.len() - 1].to_string();
        assert!(bottom.starts_with('╰'));
        assert!(bottom.ends_with('╯'));
    }

    #[test]
    fn render_card_wraps_long_lines_without_splitting_words() {
        let card = PlanCard {
            title: "Plan",
            lines: vec!["one two three four five".to_string()],
        };
        let out = render_plan_card(&card, 10);
        let inner = 8usize;
        let bodies: Vec<String> = out[1..out.len() - 1]
            .iter()
            .map(|l| l.to_string())
            .collect();
        assert_eq!(bodies.len(), 4, "expected 4 wrapped rows, got {bodies:?}");
        for row in &bodies {
            let inside = row
                .strip_prefix('│')
                .and_then(|r| r.strip_suffix('│'))
                .expect("bordered row");
            assert_eq!(inside.chars().count(), inner);
        }
        // No word is split mid-way: every word appears intact somewhere.
        assert!(bodies.iter().any(|r| r.contains("one")));
        assert!(bodies.iter().any(|r| r.contains("two")));
        assert!(bodies.iter().any(|r| r.contains("three")));
        assert!(bodies.iter().any(|r| r.contains("four")));
        assert!(bodies.iter().any(|r| r.contains("five")));
    }

    #[test]
    fn render_empty_card_has_two_borders_only() {
        let card = PlanCard {
            title: "Plan",
            lines: Vec::new(),
        };
        let out = render_plan_card(&card, 20);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn pip_meter_fills_and_leaves_hollow() {
        let meter = render_todos_pip_meter(3, 5, 10).to_string();
        assert!(meter.contains("3/5"));
        assert!(meter.contains(&"●".repeat(3)));
        assert!(meter.contains(&"○".repeat(7)));
    }

    #[test]
    fn pip_meter_clamps_done_to_width() {
        let meter = render_todos_pip_meter(9, 10, 5).to_string();
        assert!(meter.contains(&"●".repeat(5)));
        assert!(!meter.contains('○'));
    }

    #[test]
    fn pip_meter_zero_total_renders_dash() {
        let meter = render_todos_pip_meter(0, 0, 8).to_string();
        assert_eq!(meter, "—");
    }
}
