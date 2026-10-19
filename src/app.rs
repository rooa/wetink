//! The interactive viewer: watches the file, tracks changes, draws.

use crate::diff::{Change, Changes};
use crate::highlight::Highlighter;
use crate::layout::{self, Part, Row};
use crate::render::{self, Block};
use crate::theme::t;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

const GUTTER: usize = 2;
const FLASH: Duration = Duration::from_millis(1800);
/// Longest a change waits to settle, so a file written continuously still updates.
const SETTLE_MAX: Duration = Duration::from_millis(500);

type Stamp = Option<(SystemTime, u64)>;

pub struct App {
    path: PathBuf,
    max_width: usize,
    hl: Highlighter,
    src: Option<String>,
    blocks: Vec<Block>,
    changes: Changes,
    rows: Vec<Row>,
    starts: Vec<usize>,
    width: usize,
    height: usize,
    top: usize,
    follow: bool,
    flash_until: Option<Instant>,
    /// The change last selected with n/N: its block, and whether it is the
    /// removal placeholder before that block (`None` block: after the last one).
    cursor: Option<(Option<usize>, bool)>,
    stamp: Stamp,
    pending: Stamp,
    pending_since: Instant,
}

fn stamp(path: &PathBuf) -> Stamp {
    let m = fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Read the file, replacing invalid UTF-8 rather than failing on it.
fn read(path: &PathBuf) -> Option<String> {
    fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

impl App {
    pub fn new(path: PathBuf, max_width: usize) -> Self {
        let mut app = App {
            stamp: stamp(&path),
            path,
            max_width,
            hl: Highlighter::default(),
            src: None,
            blocks: Vec::new(),
            changes: Changes::default(),
            rows: Vec::new(),
            starts: Vec::new(),
            width: 0,
            height: 0,
            top: 0,
            follow: true,
            flash_until: None,
            cursor: None,
            pending: None,
            pending_since: Instant::now(),
        };
        if let Some(text) = read(&app.path) {
            app.load(text, Instant::now());
        }
        app
    }

    pub fn flashing(&self, now: Instant) -> bool {
        self.flash_until.is_some_and(|t| now < t)
    }

    /// Check the file; reload once its size and mtime have held for one poll,
    /// so half-written files are not diffed, or after `SETTLE_MAX` while it
    /// keeps changing. Returns whether anything changed.
    pub fn poll(&mut self, now: Instant) -> bool {
        let s = stamp(&self.path);
        if s == self.stamp {
            self.pending = None;
            return false;
        }
        if s != self.pending {
            if self.pending.is_none() {
                self.pending_since = now;
            }
            self.pending = s;
            if now.saturating_duration_since(self.pending_since) < SETTLE_MAX {
                return false;
            }
        }
        self.stamp = s;
        self.pending = None;
        // A vanished file (e.g. mid atomic rename) keeps showing the last version.
        match s.and_then(|_| read(&self.path)) {
            Some(text) => self.load(text, now),
            None => false,
        }
    }

    fn load(&mut self, text: String, now: Instant) -> bool {
        if self.src.as_deref() == Some(text.as_str()) {
            return false;
        }
        let blocks = render::parse(&text, &mut self.hl);
        let anchor = self.anchor();
        let first_load = self.src.is_none();
        let u = self.changes.update(&self.blocks, &blocks, now);
        self.cursor = None;
        self.blocks = blocks;
        self.src = Some(text);
        if first_load {
            self.changes = Changes::new(&self.blocks, "opened");
        }
        self.relayout(anchor.map(|(b, off)| (u.map[b], off)));
        if let Some(i) = u.first_change.filter(|_| !first_load) {
            self.flash_until = Some(now + FLASH);
            if self.follow {
                self.reveal(i);
            }
        }
        true
    }

    /// Topmost visible block and the row offset of the viewport from its start.
    fn anchor(&self) -> Option<(usize, isize)> {
        let row = self.rows.get(self.top..)?.iter().find_map(|r| r.block)?;
        Some((row, self.top as isize - self.starts[row] as isize))
    }

    fn relayout(&mut self, anchor: Option<(usize, isize)>) {
        if self.width == 0 {
            return;
        }
        let l = layout::layout(&self.blocks, &self.changes, self.width);
        self.rows = l.rows;
        self.starts = l.starts;
        self.top = match anchor {
            Some((b, off)) if !self.starts.is_empty() => {
                let start = self.starts[b.min(self.starts.len() - 1)];
                (start as isize + off).max(0) as usize
            }
            _ => 0,
        };
        self.clamp();
    }

    fn max_top(&self) -> usize {
        self.rows.len().saturating_sub(self.height)
    }

    fn clamp(&mut self) {
        self.top = self.top.min(self.max_top());
    }

    fn scroll(&mut self, d: isize) {
        self.top = (self.top as isize + d).clamp(0, self.max_top() as isize) as usize;
    }

    /// Scroll so block `i` is in view, if it is not already.
    fn reveal(&mut self, i: usize) {
        let start = self.starts[i];
        let end = self.starts.get(i + 1).copied().unwrap_or(self.rows.len());
        if start < self.top || end > self.top + self.height {
            self.top = start.saturating_sub(self.height / 4);
            self.clamp();
        }
    }

    /// Rows where a change starts: changed blocks and removal placeholders.
    fn targets(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(n, r)| match (r.part, r.block) {
                (Part::Body, Some(b))
                    if self.starts[b] == n && self.changes.marks[b].change.is_some() =>
                {
                    Some(n)
                }
                (Part::Ghost, _) => Some(n),
                _ => None,
            })
    }

    fn visible(&self, n: usize) -> bool {
        n >= self.top && n < self.top + self.height
    }

    /// Whether row `r` belongs to the selected change.
    fn selected(&self, r: &Row) -> bool {
        self.cursor.is_some_and(|(block, ghost)| {
            r.block == block && matches!(r.part, Part::Ghost | Part::GhostDiff) == ghost
        })
    }

    /// First row of the selected change, if it is on screen.
    fn cursor_row(&self) -> Option<usize> {
        let (block, ghost) = self.cursor?;
        let part = if ghost { Part::Ghost } else { Part::Body };
        let n = self
            .rows
            .iter()
            .position(|r| r.block == block && r.part == part)?;
        self.visible(n).then_some(n)
    }

    /// Select the next or previous change: after the selected one if it is on
    /// screen, otherwise the first one from the top of the screen down.
    fn jump(&mut self, forward: bool) {
        let (top, bottom) = (self.top, self.top + self.height);
        let target = match (self.cursor_row(), forward) {
            (Some(c), true) => self.targets().find(|&n| n > c),
            (Some(c), false) => self.targets().filter(|&n| n < c).last(),
            (None, true) => self.targets().find(|&n| n >= top),
            (None, false) => self.targets().filter(|&n| n < bottom).last(),
        };
        if let Some(n) = target {
            let r = &self.rows[n];
            self.cursor = Some((r.block, r.part == Part::Ghost));
            if !self.visible(n) {
                self.top = n.saturating_sub(self.height / 4);
                self.clamp();
            }
        }
    }

    /// Accept the current version: it becomes the new baseline.
    fn clear(&mut self) {
        let anchor = self.anchor();
        self.changes = Changes::new(&self.blocks, "cleared");
        self.flash_until = None;
        self.cursor = None;
        self.relayout(anchor);
    }

    /// Open or close the diff panel of the change at row `n`.
    fn toggle(&mut self, n: usize) {
        let Some(&Row { block, part, .. }) = self.rows.get(n) else {
            return;
        };
        let ghost = matches!(part, Part::Ghost | Part::GhostDiff);
        let ch = &mut self.changes;
        let open = match block {
            Some(b) if ghost => &mut ch.marks[b].ghost_open,
            None if ghost => &mut ch.end_open,
            Some(b) if ch.marks[b].change.is_some() => &mut ch.marks[b].open,
            _ => return,
        };
        *open = !*open;
        let opened = *open;
        let anchor = self.anchor();
        self.relayout(anchor);
        if opened {
            // Scroll just enough to show the panel, keeping the change itself in view.
            let rows = self.rows.iter().enumerate().filter(|(_, r)| {
                r.block == block && matches!(r.part, Part::Ghost | Part::GhostDiff) == ghost
            });
            let (start, end) =
                rows.fold((usize::MAX, 0), |(s, e), (i, _)| (s.min(i), e.max(i + 1)));
            if end > self.top + self.height {
                self.top = (end - self.height).min(start);
            }
            self.top = self.top.min(start);
            self.clamp();
        }
    }

    /// Toggle the diff of the selected change, or of the visible change nearest
    /// the focus line when none is selected on screen.
    fn toggle_nearest(&mut self) {
        if let Some(n) = self.cursor_row() {
            self.toggle(n);
            return;
        }
        let focus = self.top + self.height / 4;
        let (top, bottom) = (self.top, self.top + self.height);
        let nearest = self
            .targets()
            .filter(|&n| n >= top && n < bottom)
            .min_by_key(|&n| n.abs_diff(focus));
        if let Some(n) = nearest {
            self.toggle(n);
        }
    }

    /// A left click on screen row `y`.
    pub fn click(&mut self, y: u16) {
        if (y as usize) < self.height {
            self.toggle(self.top + y as usize);
        }
    }

    pub fn wheel(&mut self, down: bool) {
        self.scroll(if down { 3 } else { -3 });
    }

    /// Handle a key; returns true to quit.
    pub fn key(&mut self, k: KeyEvent) -> bool {
        let page = self.height.max(1) as isize;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('c') if ctrl => return true,
            KeyCode::Char('d') if ctrl => self.scroll(page / 2),
            KeyCode::Char('u') if ctrl => self.scroll(-page / 2),
            KeyCode::Char('f') if ctrl => self.scroll(page),
            KeyCode::Char('b') if ctrl => self.scroll(-page),
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Enter => self.scroll(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll(-1),
            KeyCode::Char(' ') | KeyCode::PageDown => self.scroll(page),
            KeyCode::Char('b') | KeyCode::PageUp => self.scroll(-page),
            KeyCode::Char('g') | KeyCode::Home => self.top = 0,
            KeyCode::Char('G') | KeyCode::End => self.top = self.max_top(),
            KeyCode::Char('n') => self.jump(true),
            KeyCode::Char('N') | KeyCode::Char('p') => self.jump(false),
            KeyCode::Char('d') => self.toggle_nearest(),
            KeyCode::Char('c') => self.clear(),
            KeyCode::Char('f') => self.follow = !self.follow,
            _ => {}
        }
        false
    }

    pub fn draw(&mut self, frame: &mut Frame, now: Instant) {
        let area = frame.area();
        self.height = area.height.saturating_sub(1) as usize;
        let width = (area.width as usize)
            .saturating_sub(GUTTER)
            .min(self.max_width);
        if width != self.width {
            let anchor = self.anchor();
            self.width = width;
            self.relayout(anchor);
        }
        self.clamp();
        let buf = frame.buffer_mut();
        if self.src.is_none() {
            let msg = format!("waiting for {} …", self.path.display());
            buf.set_string(
                area.x + GUTTER as u16,
                area.y,
                msg,
                Style::new().fg(t().dim),
            );
        }
        for (y, row) in self.rows[self.top..].iter().take(self.height).enumerate() {
            let y = area.y + y as u16;
            let (bar, tint) = self.marker(row, now);
            if let Some(bg) = tint {
                buf.set_style(Rect::new(area.x, y, area.width, 1), Style::new().bg(bg));
            }
            if let Some(fg) = bar {
                let glyph = if self.selected(row) { "▌" } else { "▎" };
                buf.set_string(area.x, y, glyph, Style::new().fg(fg));
            }
            buf.set_line(
                area.x + GUTTER as u16,
                y,
                &row.line,
                area.width.saturating_sub(GUTTER as u16),
            );
        }
        self.status(
            frame,
            Rect::new(area.x, area.y + self.height as u16, area.width, 1),
        );
    }

    /// Gutter color and flash background for a row.
    fn marker(&self, row: &Row, now: Instant) -> (Option<Color>, Option<Color>) {
        if matches!(row.part, Part::Ghost | Part::GhostDiff) {
            return (Some(t().removed), None);
        }
        let Some(m) = row.block.map(|b| &self.changes.marks[b]) else {
            return (None, None);
        };
        let Some(change) = m.change else {
            return (None, None);
        };
        let (bar, tint) = match change {
            Change::Added => (t().added, t().added_tint),
            Change::Modified => (t().modified, t().modified_tint),
        };
        let flash = m.at.filter(|_| row.part == Part::Body).and_then(|at| {
            let k = 1.0 - now.saturating_duration_since(at).as_secs_f32() / FLASH.as_secs_f32();
            (k > 0.0).then(|| t().fade(tint, k))
        });
        (Some(bar), flash)
    }

    fn status(&self, frame: &mut Frame, area: Rect) {
        let buf = frame.buffer_mut();
        buf.set_style(area, Style::new().bg(t().bar_bg).fg(t().bar_fg));
        let name = self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let mut left = vec![
            Span::styled(" wetink ", Style::new().fg(t().h1_fg).bg(t().h1_bg).bold()),
            Span::raw(format!(" {name} ")),
        ];
        let added = self
            .changes
            .marks
            .iter()
            .filter(|m| m.change == Some(Change::Added))
            .count();
        let modified = self
            .changes
            .marks
            .iter()
            .filter(|m| m.change == Some(Change::Modified))
            .count();
        let removed = self.changes.removed();
        for (n, label, color) in [
            (added, "added", t().added),
            (modified, "modified", t().modified),
            (removed, "removed", t().removed),
        ] {
            if n > 0 {
                left.push(Span::styled(
                    format!(" ▎{n} {label}"),
                    Style::new().fg(color),
                ));
            }
        }
        let left = Line::from(left);
        let used = left.width();
        buf.set_line(area.x, area.y, &left, area.width);
        let pct = if self.max_top() == 0 {
            100
        } else {
            self.top * 100 / self.max_top()
        };
        let follow = if self.follow { "follow" } else { "nofollow" };
        // Drop the key hints, then the rest, when they would overlap the counts.
        let full =
            format!(" {follow}  {pct:>3}%  n/N jump · d diff · c clear · f follow · q quit ");
        let short = format!(" {follow}  {pct:>3}% ");
        if let Some(right) = [full, short]
            .into_iter()
            .find(|r| used + r.chars().count() < area.width as usize)
        {
            let x = area.x + area.width - right.chars().count() as u16;
            buf.set_string(x, area.y, right, Style::new().fg(t().dim));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n_selects_changes_in_order_and_d_opens_the_selected_one() {
        let dir = std::env::temp_dir().join(format!("wetink-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.md");
        fs::write(&path, "a\n\nb\n\nc\n").unwrap();
        let mut app = App::new(path, 80);
        (app.width, app.height) = (80, 20);
        app.relayout(None);
        app.load("a 1\n\nb 2\n\nc\n".into(), Instant::now());

        // Changes at the very top are not skipped.
        app.jump(true);
        assert_eq!(app.cursor, Some((Some(0), false)));
        app.jump(true);
        assert_eq!(app.cursor, Some((Some(1), false)));
        app.jump(false);
        assert_eq!(app.cursor, Some((Some(0), false)));
        app.toggle_nearest();
        assert!(app.changes.marks[0].open && !app.changes.marks[1].open);
        // The selection survives the relayout that opening a panel causes.
        app.jump(true);
        assert_eq!(app.cursor, Some((Some(1), false)));
        fs::remove_dir_all(dir).ok();
    }
}
