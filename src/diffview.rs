//! Diff panels: how a block's Markdown source differs from the baseline.

use crate::layout::wrap;
use crate::render::Spans;
use crate::theme::t;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use similar::{ChangeTag, TextDiff};

fn prefix() -> Spans {
    vec![Span::styled("┊ ", t().border)]
}

fn header(text: String) -> Line<'static> {
    let mut spans = prefix();
    spans.push(Span::styled(text, Style::new().fg(t().dim).italic()));
    Line::from(spans)
}

/// Code and tables are diffed by line and keep their whitespace; prose by word.
fn code_like(src: &str) -> bool {
    ["```", "~~~", "|", "    ", "\t"]
        .iter()
        .any(|p| src.starts_with(p))
}

fn body(lines: &[Spans], hard: bool, width: usize) -> Vec<Line<'static>> {
    let p = prefix();
    lines
        .iter()
        .flat_map(|l| wrap(&p, &p, l, hard, width))
        .collect()
}

fn whole(src: &str, style: Style) -> Vec<Spans> {
    src.split('\n')
        .map(|l| vec![Span::styled(l.to_string(), style)])
        .collect()
}

pub fn modified(old: &str, new: &str, since: &str, width: usize) -> Vec<Line<'static>> {
    let del = Style::new()
        .fg(t().removed)
        .add_modifier(Modifier::CROSSED_OUT);
    let ins = Style::new().fg(t().added).bold();
    let same = Style::new().fg(t().dim);
    let style = |tag| match tag {
        ChangeTag::Delete => del,
        ChangeTag::Insert => ins,
        ChangeTag::Equal => same,
    };
    let hard = code_like(old) || code_like(new);
    let lines: Vec<Spans> = if hard {
        let diff = TextDiff::from_lines(old, new);
        diff.iter_all_changes()
            .map(|c| {
                let sign = match c.tag() {
                    ChangeTag::Delete => "- ",
                    ChangeTag::Insert => "+ ",
                    ChangeTag::Equal => "  ",
                };
                let text = format!("{sign}{}", c.value().trim_end_matches('\n'));
                let st = if c.tag() == ChangeTag::Delete {
                    Style::new().fg(t().removed)
                } else {
                    style(c.tag())
                };
                vec![Span::styled(text, st)]
            })
            .collect()
    } else {
        let diff = TextDiff::from_words(old, new);
        let mut lines: Vec<Spans> = vec![Vec::new()];
        for c in diff.iter_all_changes() {
            for (k, part) in c.value().split('\n').enumerate() {
                if k > 0 {
                    lines.push(Vec::new());
                }
                if !part.is_empty() {
                    lines
                        .last_mut()
                        .unwrap()
                        .push(Span::styled(part.to_string(), style(c.tag())));
                }
            }
        }
        lines
    };
    let mut out = vec![header(format!("modified since {since}"))];
    out.extend(body(&lines, hard, width));
    out
}

pub fn added(new: &str, since: &str, width: usize) -> Vec<Line<'static>> {
    let mut out = vec![header(format!("added since {since}"))];
    out.extend(body(
        &whole(new, Style::new().fg(t().added)),
        code_like(new),
        width,
    ));
    out
}

pub fn removed(old: &[&str], since: &str, width: usize) -> Vec<Line<'static>> {
    let mut out = vec![header(format!("removed since {since}"))];
    for (k, src) in old.iter().enumerate() {
        if k > 0 {
            out.push(Line::from(prefix()));
        }
        out.extend(body(
            &whole(src, Style::new().fg(t().removed)),
            code_like(src),
            width,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn styled(lines: &[Line], m: Modifier) -> String {
        lines
            .iter()
            .flat_map(|l| &l.spans)
            .filter(|s| s.style.add_modifier.contains(m))
            .map(|s| s.content.as_ref())
            .collect()
    }

    #[test]
    fn prose_gets_a_word_diff() {
        let l = modified(
            "- [ ] Run on staging",
            "- [x] Run on staging now",
            "opened",
            60,
        );
        assert_eq!(plain(&l)[0], "┊ modified since opened");
        assert_eq!(plain(&l)[1], "┊ - [ ][x] Run on staging now");
        assert_eq!(styled(&l, Modifier::CROSSED_OUT), "[ ]");
        assert_eq!(styled(&l, Modifier::BOLD), "[x] now");
    }

    #[test]
    fn code_and_tables_get_a_line_diff() {
        let old = "| a | 1 |\n| b | 2 |";
        let new = "| a | 1 |\n| b | 3 |";
        let l = modified(old, new, "opened", 60);
        assert_eq!(
            plain(&l)[1..],
            ["┊   | a | 1 |", "┊ - | b | 2 |", "┊ + | b | 3 |"]
        );
    }

    #[test]
    fn added_and_removed_show_whole_blocks() {
        assert_eq!(
            plain(&added("new para", "opened", 60)),
            ["┊ added since opened", "┊ new para"]
        );
        let r = removed(&["one", "two"], "cleared", 60);
        assert_eq!(
            plain(&r),
            ["┊ removed since cleared", "┊ one", "┊ ", "┊ two"]
        );
    }
}
