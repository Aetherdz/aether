//! Pure, unit-testable text helpers — the Rust port of `aether-cli/src/render.ts`.
//!
//! These functions never touch the terminal; the ratatui widgets in `lib.rs`
//! consume them. Box-drawing characters match the TS original exactly.

/// Visible length of a string with all ANSI SGR sequences (`\x1b[...m`) stripped.
pub fn visible_length(text: &str) -> usize {
    strip_ansi(text).chars().count()
}

/// Remove every ANSI SGR escape sequence, mirroring the TS `/\\x1b\\[[0-9;]*m/g`.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c2 in chars.by_ref() {
                if c2 == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Word-wrap `text` to at most `width` columns. Blank lines are preserved.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut current = String::new();
        for word in para.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.chars().count() + word.chars().count() + 1 > width {
                out.push(current);
                current = word.to_string();
            } else {
                current.push(' ');
                current.push_str(word);
            }
        }
        if !current.is_empty() || para.is_empty() {
            out.push(current);
        }
    }
    out
}

/// A horizontal rule of `char`, `width` columns wide.
pub fn line(char: char, width: usize) -> String {
    std::iter::repeat_n(char, width).collect()
}

/// A box-drawn frame around `lines` with an optional centred title.
pub fn boxed(title: Option<&str>, lines: &[String], width: usize) -> Vec<String> {
    const PAD: usize = 2;
    let title_len = title.map_or(0, |t| visible_length(t) + 2);
    let longest = lines.iter().map(|l| visible_length(l)).max().unwrap_or(0);
    let inner = if width > 0 {
        width.saturating_sub(PAD * 2).max(title_len + 2)
    } else {
        longest.max(title_len + 2).min(160)
    };
    let mut out = Vec::new();
    match title {
        Some(t) => {
            let padded = format!(" {t} ");
            let dashes = inner.saturating_sub(visible_length(&padded));
            let left = dashes / 2;
            let right = dashes - left;
            out.push(format!("┌{}┐", line('─', left) + &padded + &line('─', right)));
        }
        None => out.push(format!("┌{}┐", line('─', inner))),
    }
    for l in lines {
        let len = visible_length(l);
        let padded = if len > inner {
            let mut s: String = l.chars().take(inner).collect();
            s.push('…');
            s
        } else {
            l.clone()
        };
        let pad = inner.saturating_sub(visible_length(&padded));
        out.push(format!("│{} {} {}│", " ".repeat(PAD), padded, " ".repeat(pad)));
    }
    out.push(format!("└{}┘", line('─', inner)));
    out
}

/// A divider line with an optional dimmed label, exactly 72 columns wide.
pub fn divider(label: Option<&str>) -> String {
    match label {
        Some(l) => {
            let left = 72usize.saturating_sub(visible_length(l) + 3);
            format!("{} {l} {}", line('─', left), line('─', 1))
        }
        None => line('─', 72),
    }
}

/// Token usage summary line: `"N in  M out  T total"` (empty when unknown).
pub fn usage_summary(input: Option<u64>, output: Option<u64>, total: Option<u64>) -> String {
    let mut parts = Vec::new();
    if let Some(i) = input {
        parts.push(format!("{i} in"));
    }
    if let Some(o) = output {
        parts.push(format!("{o} out"));
    }
    if let Some(t) = total {
        parts.push(format!("{t} total"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("tokens: {}", parts.join("  "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_respects_width_and_blank_lines() {
        let lines = wrap("hello world foo", 10);
        assert_eq!(lines, vec!["hello", "world foo"]);
        let blank = wrap("a\n\nb", 10);
        assert_eq!(blank, vec!["a", "", "b"]);
    }

    #[test]
    fn visible_length_ignores_ansi() {
        assert_eq!(visible_length("\x1b[36mhello\x1b[0m"), 5);
    }

    #[test]
    fn boxed_has_borders_and_padded_rows() {
        let rows = boxed(Some("title"), &["ab".to_string()], 0);
        assert_eq!(rows.len(), 3);
        assert!(rows[0].starts_with("┌") && rows[0].contains("title"));
        assert!(rows[1].starts_with("│"));
        assert!(rows[2].starts_with("└"));
    }

    #[test]
    fn boxed_truncates_overflow_rows() {
        let long = "x".repeat(300);
        let rows = boxed(None, &[long], 0);
        assert!(rows[1].ends_with('│'));
        assert!(rows[1].contains('…'));
    }

    #[test]
    fn divider_label_pads_to_72() {
        let d = divider(Some("sync"));
        assert_eq!(d.chars().count(), 72);
        assert_eq!(divider(None).chars().count(), 72);
    }

    #[test]
    fn usage_summary_renders_only_known_parts() {
        assert_eq!(usage_summary(Some(10), Some(5), None), "tokens: 10 in  5 out");
        assert_eq!(usage_summary(None, None, None), "");
    }
}
