//! Markdown source → top-level blocks of styled, not-yet-wrapped items.
//!
//! A block is a top-level element (heading, paragraph, code block, table, …)
//! or a single item of a top-level list. Each block carries a hash of its
//! source so successive versions of a document can be diffed block by block.

use crate::highlight::Highlighter;
use crate::theme::t;
use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag,
    TagEnd,
};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem::take;

pub type Spans = Vec<Span<'static>>;

pub enum Item {
    /// Wrappable text. `first` prefixes the first row, `rest` every continuation row.
    /// `hard` wraps at any character and keeps whitespace (code).
    Text {
        first: Spans,
        rest: Spans,
        spans: Spans,
        hard: bool,
    },
    Rule {
        prefix: Spans,
    },
    /// `rows[0]` is the header.
    Table {
        prefix: Spans,
        align: Vec<Alignment>,
        rows: Vec<Vec<Spans>>,
    },
}

pub struct Block {
    pub hash: u64,
    /// Source text, used to pair edited blocks with their previous version.
    pub src: String,
    pub gap_before: bool,
    pub items: Vec<Item>,
}

pub fn parse(src: &str, hl: &mut Highlighter) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
    hl.sweep();
    let mut r = Renderer {
        hl: Some(hl),
        ..Default::default()
    };
    let mut it = Parser::new_ext(src, opts).into_offset_iter().peekable();
    while let Some((ev, range)) = it.next() {
        match ev {
            Event::Start(tag) => {
                let top_item = r.top_list && r.depth == 1 && matches!(tag, Tag::Item);
                if r.depth == 0 && matches!(tag, Tag::List(_)) {
                    r.top_list = true;
                } else if r.depth == 0 || top_item {
                    r.begin(&src[range]);
                }
                r.depth += 1;
                r.start(tag, it.peek().map(|(e, _)| e));
            }
            Event::End(end) => {
                let item = matches!(end, TagEnd::Item);
                r.end(end);
                r.depth -= 1;
                if r.depth == 0 {
                    if r.top_list {
                        r.top_list = false;
                    } else {
                        r.finish();
                    }
                } else if r.depth == 1 && r.top_list && item {
                    r.finish();
                }
            }
            Event::Rule if r.depth == 0 => {
                r.begin(&src[range]);
                r.event(Event::Rule);
                r.finish();
            }
            ev => r.event(ev),
        }
    }
    r.blocks
}

enum Raw {
    Code(String),
    Html,
    Meta,
}

struct ListCtx {
    next: Option<u64>,
    loose: bool,
    count: usize,
}

struct Container {
    first: Spans,
    rest: Spans,
    used: bool,
    item: bool,
}

/// Column alignments, finished rows, and the row being built.
type TableBuild = (Vec<Alignment>, Vec<Vec<Spans>>, Vec<Spans>);

#[derive(Default)]
struct Renderer<'h> {
    hl: Option<&'h mut Highlighter>,
    blocks: Vec<Block>,
    cur: Option<Block>,
    depth: usize,
    top_list: bool,
    lists: Vec<ListCtx>,
    containers: Vec<Container>,
    inline: Spans,
    styles: Vec<Style>,
    /// Open links/images: destination, index into `inline` where the text starts, show URL.
    links: Vec<(String, usize, bool)>,
    raw: Option<(Raw, String)>,
    table: Option<TableBuild>,
    /// A block-level element just ended; the next one gets a blank line unless in a tight list.
    gap: bool,
}

impl Renderer<'_> {
    fn begin(&mut self, src: &str) {
        let src = src.trim_end();
        let mut h = DefaultHasher::new();
        src.hash(&mut h);
        self.cur = Some(Block {
            hash: h.finish(),
            src: src.to_string(),
            gap_before: true,
            items: Vec::new(),
        });
        self.gap = false;
    }

    fn finish(&mut self) {
        self.flush();
        if let Some(b) = self.cur.take().filter(|b| !b.items.is_empty()) {
            self.blocks.push(b);
        }
    }

    fn style(&self) -> Style {
        self.styles
            .iter()
            .fold(Style::new().fg(t().text), |a, s| a.patch(*s))
    }

    fn text(&mut self, t: &str) {
        let style = self.style();
        self.inline.push(Span::styled(t.to_string(), style));
    }

    fn push(&mut self, item: Item) {
        if let Some(b) = &mut self.cur {
            b.items.push(item);
        }
    }

    /// Prefixes for the next emitted line; marks every open container as started.
    fn prefixes(&mut self) -> (Spans, Spans) {
        let (mut first, mut rest) = (Vec::new(), Vec::new());
        for c in &mut self.containers {
            first.extend(if c.used { &c.rest } else { &c.first }.iter().cloned());
            rest.extend(c.rest.iter().cloned());
            c.used = true;
        }
        (first, rest)
    }

    fn emit(&mut self, spans: Spans, hard: bool, indent: &'static str) {
        let (mut first, mut rest) = self.prefixes();
        if !indent.is_empty() {
            first.push(Span::raw(indent));
            rest.push(Span::raw(indent));
        }
        self.push(Item::Text {
            first,
            rest,
            spans,
            hard,
        });
    }

    fn flush(&mut self) {
        if !self.inline.is_empty() {
            let spans = take(&mut self.inline);
            self.emit(spans, false, "");
        }
    }

    fn block_gap(&mut self) {
        let tight = self.lists.last().is_some_and(|l| !l.loose);
        if self.gap && !tight && self.cur.as_ref().is_some_and(|b| !b.items.is_empty()) {
            self.emit(Vec::new(), false, "");
        }
        self.gap = false;
    }

    fn start(&mut self, tag: Tag, next: Option<&Event>) {
        if matches!(
            tag,
            Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::BlockQuote(_)
                | Tag::CodeBlock(_)
                | Tag::HtmlBlock
                | Tag::MetadataBlock(_)
                | Tag::List(_)
                | Tag::Item
                | Tag::FootnoteDefinition(_)
                | Tag::Table(_)
        ) {
            self.flush();
            self.block_gap();
        }
        match tag {
            Tag::Heading { level, .. } => {
                let (style, prefix) = heading(level);
                self.styles.push(style);
                self.text(prefix);
            }
            Tag::BlockQuote(kind) => {
                let (color, label) = quote(kind);
                let bar = vec![Span::styled("│ ", color)];
                self.containers.push(Container {
                    first: bar.clone(),
                    rest: bar,
                    used: false,
                    item: false,
                });
                if let Some(label) = label {
                    self.emit(
                        vec![Span::styled(label, Style::new().fg(color).bold())],
                        false,
                        "",
                    );
                }
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split([' ', ',', '{']).next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.raw = Some((Raw::Code(lang), String::new()));
            }
            Tag::HtmlBlock => self.raw = Some((Raw::Html, String::new())),
            Tag::MetadataBlock(_) => self.raw = Some((Raw::Meta, String::new())),
            Tag::List(start) => self.lists.push(ListCtx {
                next: start,
                loose: false,
                count: 0,
            }),
            Tag::Item => {
                let loose = matches!(next, Some(Event::Start(Tag::Paragraph)));
                let depth = self
                    .lists
                    .iter()
                    .filter(|l| l.next.is_none())
                    .count()
                    .saturating_sub(1);
                let top = self.containers.is_empty();
                let Some(lc) = self.lists.last_mut() else {
                    return;
                };
                lc.loose = loose;
                if top
                    && lc.count > 0
                    && let Some(b) = self.cur.as_mut().filter(|b| b.items.is_empty())
                {
                    b.gap_before = loose;
                }
                lc.count += 1;
                let marker = match &mut lc.next {
                    Some(n) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    None => ["• ", "◦ ", "▪ "][depth.min(2)].to_string(),
                };
                let pad = " ".repeat(marker.chars().count());
                self.containers.push(Container {
                    first: vec![Span::styled(marker, t().dim)],
                    rest: vec![Span::raw(pad)],
                    used: false,
                    item: true,
                });
            }
            Tag::FootnoteDefinition(label) => self.containers.push(Container {
                first: vec![Span::styled(format!("[^{label}]: "), t().heading)],
                rest: vec![Span::raw("    ")],
                used: false,
                item: false,
            }),
            Tag::Table(align) => self.table = Some((align, Vec::new(), Vec::new())),
            Tag::Emphasis => self.styles.push(Style::new().italic()),
            Tag::Strong => self.styles.push(Style::new().bold()),
            Tag::Strikethrough => self.styles.push(Style::new().crossed_out()),
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => {
                self.styles.push(Style::new().fg(t().link_text).bold());
                let show = !matches!(link_type, LinkType::Autolink | LinkType::Email);
                self.links
                    .push((dest_url.to_string(), self.inline.len(), show));
            }
            Tag::Image { dest_url, .. } => {
                self.styles.push(Style::new().fg(t().image_text));
                self.text("Image: ");
                self.links
                    .push((dest_url.to_string(), self.inline.len(), true));
            }
            _ => {}
        }
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Paragraph => {
                self.flush();
                self.gap = true;
            }
            TagEnd::Heading(level) => {
                if level == HeadingLevel::H1 {
                    self.text("\u{a0}");
                }
                self.styles.pop();
                self.flush();
                self.gap = true;
            }
            TagEnd::BlockQuote(_) | TagEnd::Item | TagEnd::FootnoteDefinition => {
                self.flush();
                self.containers.pop();
                self.gap = true;
            }
            TagEnd::CodeBlock | TagEnd::HtmlBlock | TagEnd::MetadataBlock(_) => {
                if let Some((kind, text)) = self.raw.take() {
                    self.emit_raw(kind, &text);
                }
                self.gap = true;
            }
            TagEnd::List(_) => {
                self.lists.pop();
                self.gap = true;
            }
            TagEnd::TableCell => {
                let cell = take(&mut self.inline);
                if let Some(t) = &mut self.table {
                    t.2.push(cell);
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    let row = take(&mut t.2);
                    t.1.push(row);
                }
            }
            TagEnd::Table => {
                if let Some((align, rows, _)) = self.table.take() {
                    let (prefix, _) = self.prefixes();
                    self.push(Item::Table {
                        prefix,
                        align,
                        rows,
                    });
                }
                self.gap = true;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                if let Some((dest, start, true)) = self.links.pop() {
                    let text: String = self.inline[start..]
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect();
                    if !dest.is_empty() && text != dest {
                        self.text(" ");
                        self.inline.push(Span::styled(
                            dest,
                            Style::new().fg(t().link_url).underlined(),
                        ));
                    }
                }
            }
            TagEnd::Image => {
                self.styles.pop();
                if let Some((dest, ..)) = self.links.pop() {
                    self.inline.push(Span::styled(" → ", t().image_text));
                    self.inline.push(Span::styled(
                        dest,
                        Style::new().fg(t().image_url).underlined(),
                    ));
                }
            }
            _ => {}
        }
    }

    fn event(&mut self, ev: Event) {
        match ev {
            Event::Text(s) | Event::Html(s) if self.raw.is_some() => {
                if let Some((_, buf)) = &mut self.raw {
                    buf.push_str(&s);
                }
            }
            Event::Text(s) | Event::InlineMath(s) | Event::DisplayMath(s) => self.text(&s),
            Event::Code(s) => self.inline.push(Span::styled(
                format!("\u{a0}{s}\u{a0}"),
                Style::new().fg(t().code_fg).bg(t().code_bg),
            )),
            Event::Html(s) | Event::InlineHtml(s) => {
                self.inline.push(Span::styled(s.to_string(), t().dim))
            }
            Event::FootnoteReference(l) => self
                .inline
                .push(Span::styled(format!("[^{l}]"), t().heading)),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                self.block_gap();
                let (prefix, _) = self.prefixes();
                self.push(Item::Rule { prefix });
                self.gap = true;
            }
            Event::TaskListMarker(done) => {
                if let Some(c) = self
                    .containers
                    .iter_mut()
                    .rev()
                    .find(|c| c.item)
                    .filter(|c| !c.used)
                {
                    let (mark, color) = if done {
                        ("[✓] ", t().link_text)
                    } else {
                        ("[ ] ", t().dim)
                    };
                    c.first = vec![Span::styled(mark, color)];
                    c.rest = vec![Span::raw("    ")];
                }
            }
            Event::Start(_) | Event::End(_) => {}
        }
    }

    fn emit_raw(&mut self, kind: Raw, text: &str) {
        let text = text.strip_suffix('\n').unwrap_or(text);
        match kind {
            Raw::Code(lang) => {
                let code = text.replace('\t', "    ");
                let lines = self
                    .hl
                    .as_mut()
                    .map(|h| h.highlight(&lang, &code))
                    .unwrap_or_default();
                for spans in lines {
                    self.emit(spans, true, "  ");
                }
            }
            Raw::Html | Raw::Meta => {
                for line in text.lines() {
                    self.emit(vec![Span::styled(line.to_string(), t().dim)], true, "");
                }
            }
        }
    }
}

fn heading(level: HeadingLevel) -> (Style, &'static str) {
    let h = Style::new().fg(t().heading).bold();
    match level {
        HeadingLevel::H1 => (Style::new().fg(t().h1_fg).bg(t().h1_bg).bold(), "\u{a0}"),
        HeadingLevel::H2 => (h, "## "),
        HeadingLevel::H3 => (h, "### "),
        HeadingLevel::H4 => (h, "#### "),
        HeadingLevel::H5 => (h, "##### "),
        HeadingLevel::H6 => (Style::new().fg(t().h6), "###### "),
    }
}

fn quote(kind: Option<BlockQuoteKind>) -> (Color, Option<&'static str>) {
    match kind {
        None => (t().dim, None),
        Some(BlockQuoteKind::Note) => (t().heading, Some("Note")),
        Some(BlockQuoteKind::Tip) => (t().link_text, Some("Tip")),
        Some(BlockQuoteKind::Important) => (t().important, Some("Important")),
        Some(BlockQuoteKind::Warning) => (t().warning, Some("Warning")),
        Some(BlockQuoteKind::Caution) => (t().caution, Some("Caution")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(src: &str) -> Vec<Block> {
        parse(src, &mut Highlighter::default())
    }

    fn texts(b: &Block) -> Vec<String> {
        b.items
            .iter()
            .filter_map(|i| match i {
                Item::Text { first, spans, .. } => Some(
                    first
                        .iter()
                        .chain(spans)
                        .map(|s| s.content.as_ref())
                        .collect(),
                ),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn top_level_list_items_are_separate_blocks() {
        let b = blocks("# Title\n\npara\n\n- a\n- b\n\n1. x\n2. y\n");
        assert_eq!(b.len(), 6);
        assert_eq!(texts(&b[0]), ["\u{a0}Title\u{a0}"]);
        assert_eq!(texts(&b[2]), ["• a"]);
        assert!(!b[3].gap_before, "tight list items have no gap");
        assert_eq!(texts(&b[5]), ["2. y"]);
    }

    #[test]
    fn nested_lists_and_tasks_stay_in_their_item() {
        let b = blocks("- [x] done\n  - child\n- [ ] todo\n");
        assert_eq!(b.len(), 2);
        assert_eq!(texts(&b[0]), ["[✓] done", "    ◦ child"]);
        assert_eq!(texts(&b[1]), ["[ ] todo"]);
    }

    #[test]
    fn hash_depends_on_block_source_only() {
        let a = blocks("one\n\ntwo\n");
        let b = blocks("zero\n\none\n\ntwo\n");
        assert_eq!(a[0].hash, b[1].hash);
        assert_eq!(a[1].hash, b[2].hash);
    }

    #[test]
    fn quotes_code_and_tables() {
        let b = blocks("> [!NOTE]\n> hi\n\n```\nlet x;\n```\n\n| a | b |\n|---|--:|\n| 1 | 2 |\n");
        assert_eq!(texts(&b[0]), ["│ Note", "│ hi"]);
        assert_eq!(texts(&b[1]), ["  let x;"]);
        assert!(matches!(&b[2].items[0], Item::Table { rows, .. } if rows.len() == 2));
    }
}
