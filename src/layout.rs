//! Styled blocks → screen rows at a given width.

use crate::diff::{Change, Changes};
use crate::diffview;
use crate::render::{Block, Item, Spans};
use crate::theme::t;
use pulldown_cmark::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Body,
    /// Diff panel under a changed block.
    Diff,
    /// Placeholder for removed blocks.
    Ghost,
    /// Panel listing what the placeholder removed.
    GhostDiff,
}

pub struct Row {
    /// The block a row belongs to. Ghost rows belong to the block they precede
    /// (`None` after the last block); gaps belong to none.
    pub block: Option<usize>,
    pub part: Part,
    pub line: Line<'static>,
}

pub struct Layout {
    pub rows: Vec<Row>,
    /// First row of each block.
    pub starts: Vec<usize>,
}

pub fn layout(blocks: &[Block], ch: &Changes, width: usize) -> Layout {
    let mut rows = Vec::new();
    let mut starts = Vec::with_capacity(blocks.len());
    let push = |rows: &mut Vec<Row>, lines: Vec<Line<'static>>, block, part| {
        rows.extend(lines.into_iter().map(|line| Row { block, part, line }));
    };
    let srcs = |idx: &[usize]| {
        idx.iter()
            .map(|&o| ch.baseline[o].as_str())
            .collect::<Vec<_>>()
    };
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 && b.gap_before {
            push(&mut rows, vec![Line::default()], None, Part::Body);
        }
        let mark = ch.marks.get(i);
        if let Some(m) = mark.filter(|m| !m.removed.is_empty()) {
            push(
                &mut rows,
                vec![ghost(m.removed.len())],
                Some(i),
                Part::Ghost,
            );
            if m.ghost_open {
                let lines = diffview::removed(&srcs(&m.removed), ch.since, width);
                push(&mut rows, lines, Some(i), Part::GhostDiff);
            }
        }
        starts.push(rows.len());
        for item in &b.items {
            push(&mut rows, item_lines(item, width), Some(i), Part::Body);
        }
        if let Some(m) = mark.filter(|m| m.open) {
            let lines = match (m.change, m.origin) {
                (Some(Change::Modified), Some(o)) => {
                    diffview::modified(&ch.baseline[o], &b.src, ch.since, width)
                }
                (Some(_), _) => diffview::added(&b.src, ch.since, width),
                (None, _) => Vec::new(),
            };
            push(&mut rows, lines, Some(i), Part::Diff);
        }
    }
    if !ch.removed_end.is_empty() {
        if !rows.is_empty() {
            push(&mut rows, vec![Line::default()], None, Part::Body);
        }
        push(
            &mut rows,
            vec![ghost(ch.removed_end.len())],
            None,
            Part::Ghost,
        );
        if ch.end_open {
            let lines = diffview::removed(&srcs(&ch.removed_end), ch.since, width);
            push(&mut rows, lines, None, Part::GhostDiff);
        }
    }
    Layout { rows, starts }
}

fn ghost(n: usize) -> Line<'static> {
    let text = format!("┄┄ {n} block{} removed ┄┄", if n == 1 { "" } else { "s" });
    Line::from(Span::styled(text, Style::new().fg(t().removed).italic()))
}

fn item_lines(item: &Item, width: usize) -> Vec<Line<'static>> {
    match item {
        Item::Text {
            first,
            rest,
            spans,
            hard,
        } => wrap(first, rest, spans, *hard, width),
        Item::Rule { prefix } => {
            let w = width.saturating_sub(spans_width(prefix));
            let mut spans = prefix.clone();
            spans.push(Span::styled("─".repeat(w), t().border));
            vec![Line::from(spans)]
        }
        Item::Table {
            prefix,
            align,
            rows,
        } => table(prefix, align, rows, width),
    }
}

/// A breakable space. No-break spaces (padding, `&nbsp;`) stay part of the word.
fn space(c: char) -> bool {
    c.is_whitespace() && c != '\u{a0}'
}

fn cw(c: char) -> usize {
    c.width().unwrap_or(0)
}

pub fn spans_width(spans: &[Span]) -> usize {
    spans
        .iter()
        .map(|s| s.content.chars().map(cw).sum::<usize>())
        .sum()
}

struct Wrapper {
    rows: Vec<Vec<(char, Style)>>,
    row: Vec<(char, Style)>,
    used: usize,
    avail: usize,
    rest_avail: usize,
}

impl Wrapper {
    fn newline(&mut self) {
        self.rows.push(std::mem::take(&mut self.row));
        self.used = 0;
        self.avail = self.rest_avail;
    }

    fn push(&mut self, c: char, st: Style) {
        let w = cw(c);
        if self.used + w > self.avail && !self.row.is_empty() {
            self.newline();
        }
        self.row.push((c, st));
        self.used += w;
    }
}

/// Wrap `spans` to `width` columns, `first`/`rest` prefixing the first/continuation rows.
pub fn wrap(
    first: &Spans,
    rest: &Spans,
    spans: &Spans,
    hard: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let chars: Vec<(char, Style)> = spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .collect();
    let mut w = Wrapper {
        rows: Vec::new(),
        row: Vec::new(),
        used: 0,
        avail: width.saturating_sub(spans_width(first)).max(1),
        rest_avail: width.saturating_sub(spans_width(rest)).max(1),
    };
    if hard {
        for &(c, st) in &chars {
            w.push(c, st);
        }
    } else {
        let mut pending: Option<(usize, usize)> = None;
        let mut i = 0;
        while i < chars.len() {
            let mut j = i + 1;
            if space(chars[i].0) {
                while j < chars.len() && space(chars[j].0) {
                    j += 1;
                }
                if !w.row.is_empty() {
                    pending = Some((i, j));
                }
                i = j;
                continue;
            }
            // A word runs to the next space; wide (CJK) characters break on their own.
            if cw(chars[i].0) < 2 {
                while j < chars.len() && !space(chars[j].0) && cw(chars[j].0) < 2 {
                    j += 1;
                }
            }
            let word: usize = chars[i..j].iter().map(|&(c, _)| cw(c)).sum();
            let space = pending.map_or(0, |(a, b)| b - a);
            if w.used + space + word <= w.avail {
                if let Some((a, b)) = pending {
                    for &(_, st) in &chars[a..b] {
                        w.push(' ', st);
                    }
                }
            } else if !w.row.is_empty() {
                w.newline();
            }
            for &(c, st) in &chars[i..j] {
                w.push(c, st);
            }
            pending = None;
            i = j;
        }
    }
    if !w.row.is_empty() || w.rows.is_empty() {
        w.rows.push(w.row);
    }
    w.rows
        .iter()
        .enumerate()
        .map(|(n, row)| {
            let mut spans = if n == 0 { first.clone() } else { rest.clone() };
            spans.extend(merge(row));
            Line::from(spans)
        })
        .collect()
}

fn merge(row: &[(char, Style)]) -> Spans {
    let mut out: Spans = Vec::new();
    for &(c, st) in row {
        // No-break spaces only steer wrapping; draw (and copy) them as spaces.
        let c = if c == '\u{a0}' { ' ' } else { c };
        match out.last_mut() {
            Some(s) if s.style == st => s.content.to_mut().push(c),
            _ => out.push(Span::styled(c.to_string(), st)),
        }
    }
    out
}

fn table(
    prefix: &Spans,
    align: &[Alignment],
    rows: &[Vec<Spans>],
    width: usize,
) -> Vec<Line<'static>> {
    let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if ncols == 0 {
        return Vec::new();
    }
    let mut cols = vec![1; ncols];
    for row in rows {
        for (j, cell) in row.iter().enumerate() {
            cols[j] = cols[j].max(spans_width(cell));
        }
    }
    // Each column takes its width plus one space on each side, separated by "│".
    let avail = width.saturating_sub(spans_width(prefix) + 3 * ncols - 1);
    while cols.iter().sum::<usize>() > avail {
        let (j, &max) = cols.iter().enumerate().max_by_key(|(_, w)| **w).unwrap();
        if max <= 3 {
            break;
        }
        cols[j] -= 1;
    }
    let border = Style::new().fg(t().border);
    let empty = Vec::new();
    let mut out = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let cells: Vec<Vec<Line>> = (0..ncols)
            .map(|j| {
                let mut cell = row.get(j).unwrap_or(&empty).clone();
                if r == 0 {
                    cell.iter_mut().for_each(|s| s.style = s.style.bold());
                }
                wrap(&empty, &empty, &cell, false, cols[j])
            })
            .collect();
        let height = cells.iter().map(Vec::len).max().unwrap_or(1);
        for k in 0..height {
            let mut spans = prefix.clone();
            for (j, lines) in cells.iter().enumerate() {
                if j > 0 {
                    spans.push(Span::styled("│", border));
                }
                let line = lines.get(k).map(|l| l.spans.clone()).unwrap_or_default();
                let pad = cols[j].saturating_sub(spans_width(&line));
                let left = match align.get(j) {
                    Some(Alignment::Right) => pad,
                    Some(Alignment::Center) => pad / 2,
                    _ => 0,
                };
                spans.push(Span::raw(" ".repeat(left + 1)));
                spans.extend(line);
                spans.push(Span::raw(" ".repeat(pad - left + 1)));
            }
            out.push(Line::from(spans));
        }
        if r == 0 {
            let mut spans = prefix.clone();
            let rule: Vec<String> = cols.iter().map(|w| "─".repeat(w + 2)).collect();
            spans.push(Span::styled(rule.join("┼"), border));
            out.push(Line::from(spans));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::Mark;
    use crate::highlight::Highlighter;
    use crate::render::parse;

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn render(src: &str, width: usize) -> Vec<String> {
        let blocks = parse(src, &mut Highlighter::default());
        layout(&blocks, &Changes::default(), width)
            .rows
            .iter()
            .map(|r| plain(&r.line))
            .collect()
    }

    #[test]
    fn words_wrap_within_width_with_hanging_indent() {
        let rows = render("- alpha beta gamma delta epsilon zeta eta theta\n", 20);
        assert_eq!(
            rows,
            ["• alpha beta gamma", "  delta epsilon zeta", "  eta theta"]
        );
    }

    #[test]
    fn padding_survives_at_row_edges() {
        let rows = render("# Title\n\n`code` here\n", 40);
        assert_eq!(rows[0], " Title ");
        assert!(rows[2].starts_with(" code "), "{rows:?}");
    }

    #[test]
    fn wide_characters_wrap_by_display_width() {
        let rows = render("日本語のテキストを折り返す\n", 10);
        assert!(
            rows.iter().all(|r| r.chars().map(cw).sum::<usize>() <= 10),
            "{rows:?}"
        );
        assert_eq!(rows.concat(), "日本語のテキストを折り返す");
    }

    #[test]
    fn long_words_and_code_are_split() {
        let rows = render("```\nabcdefghijkl\n```\n", 8);
        assert_eq!(rows, ["  abcdef", "  ghijkl"]);
    }

    #[test]
    fn tables_align_and_shrink_to_fit() {
        let rows = render("| name | n |\n|---|--:|\n| a | 10 |\n", 40);
        assert_eq!(rows, [" name │  n ", "──────┼────", " a    │ 10 "]);
        let wide = render(
            "| aaaaaaaaaaaaaaaaaaaa | bbbbbbbbbbbbbbbbbbbb |\n|---|---|\n| x | y |\n",
            30,
        );
        assert!(
            wide.iter().all(|r| r.chars().map(cw).sum::<usize>() <= 30),
            "{wide:#?}"
        );
    }

    #[test]
    fn ghosts_mark_removed_blocks_and_open_panels() {
        let blocks = parse("a\n\nb\n", &mut Highlighter::default());
        let mut ch = Changes {
            baseline: vec!["x".into(), "y".into(), "z".into()],
            since: "opened",
            marks: vec![
                Mark::default(),
                Mark {
                    removed: vec![0, 1],
                    ..Default::default()
                },
            ],
            removed_end: vec![2],
            end_open: false,
        };
        let l = layout(&blocks, &ch, 40);
        let rows: Vec<String> = l.rows.iter().map(|r| plain(&r.line)).collect();
        assert_eq!(
            rows,
            [
                "a",
                "",
                "┄┄ 2 blocks removed ┄┄",
                "b",
                "",
                "┄┄ 1 block removed ┄┄"
            ]
        );
        assert_eq!(l.starts, [0, 3]);
        assert_eq!((l.rows[2].block, l.rows[2].part), (Some(1), Part::Ghost));
        assert_eq!((l.rows[5].block, l.rows[5].part), (None, Part::Ghost));

        ch.marks[1].ghost_open = true;
        ch.end_open = true;
        let l = layout(&blocks, &ch, 40);
        let rows: Vec<String> = l.rows.iter().map(|r| plain(&r.line)).collect();
        assert_eq!(rows[3..7], ["┊ removed since opened", "┊ x", "┊ ", "┊ y"]);
        assert_eq!(rows[rows.len() - 2..], ["┊ removed since opened", "┊ z"]);
        assert_eq!(l.starts, [0, 7]);
    }
}
