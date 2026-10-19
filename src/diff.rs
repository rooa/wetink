//! Block-level change tracking against a baseline.
//!
//! The baseline is the document as it was when opened (or when marks were
//! last cleared). Each current block keeps a link to its baseline block, so
//! marks and diffs always describe the net change since the baseline.

use crate::render::Block;
use similar::{Algorithm, DiffOp, capture_diff_slices};
use std::time::Instant;

/// Minimum word similarity for a new block to count as an edit of an old one.
const SIMILAR: f32 = 0.4;
/// How far ahead in a replaced region to look for an old block to pair with.
const WINDOW: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
}

#[derive(Clone, Debug, Default)]
pub struct Mark {
    pub change: Option<Change>,
    /// When the block last changed (drives the flash).
    pub at: Option<Instant>,
    /// The block's counterpart in the baseline; `None` for blocks added since.
    pub origin: Option<usize>,
    /// Baseline blocks removed just before this one.
    pub removed: Vec<usize>,
    /// Whether the diff panel under the block is open.
    pub open: bool,
    /// Whether the panel under the removal placeholder before the block is open.
    pub ghost_open: bool,
}

#[derive(Default)]
pub struct Changes {
    /// Source of each baseline block.
    pub baseline: Vec<String>,
    /// What the baseline is: "opened" or "cleared".
    pub since: &'static str,
    pub marks: Vec<Mark>,
    /// Baseline blocks removed after the last block.
    pub removed_end: Vec<usize>,
    pub end_open: bool,
}

pub struct Update {
    /// Old block index → nearest new block index (may equal the new length).
    pub map: Vec<usize>,
    pub first_change: Option<usize>,
}

fn changed(change: Change, now: Instant) -> Mark {
    Mark {
        change: Some(change),
        at: Some(now),
        ..Default::default()
    }
}

/// Baseline blocks that disappear with an old block: itself (unless added
/// since the baseline) and any removals already attached to it.
fn lineage(m: &Mark) -> impl Iterator<Item = usize> + '_ {
    m.origin.into_iter().chain(m.removed.iter().copied())
}

/// Words are ASCII alphanumeric runs; other non-space, non-punctuation characters (CJK) count alone.
fn words(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        if c.is_ascii_alphanumeric() {
            start.get_or_insert(i);
            continue;
        }
        if let Some(st) = start.take() {
            out.push(&s[st..i]);
        }
        if !c.is_whitespace() && !c.is_ascii_punctuation() {
            out.push(&s[i..i + c.len_utf8()]);
        }
    }
    out.extend(start.map(|st| &s[st..]));
    out
}

fn similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (words(a), words(b));
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let same: usize = capture_diff_slices(Algorithm::Myers, &a, &b)
        .iter()
        .map(|op| match op {
            DiffOp::Equal { len, .. } => *len,
            _ => 0,
        })
        .sum();
    2.0 * same as f32 / (a.len() + b.len()) as f32
}

impl Changes {
    /// Start tracking with `blocks` as the baseline.
    pub fn new(blocks: &[Block], since: &'static str) -> Self {
        Changes {
            baseline: blocks.iter().map(|b| b.src.clone()).collect(),
            since,
            marks: (0..blocks.len())
                .map(|i| Mark {
                    origin: Some(i),
                    ..Default::default()
                })
                .collect(),
            removed_end: Vec::new(),
            end_open: false,
        }
    }

    pub fn removed(&self) -> usize {
        self.removed_end.len() + self.marks.iter().map(|m| m.removed.len()).sum::<usize>()
    }

    /// Carry marks from `old` (the previous version) over to `new`.
    pub fn update(&mut self, old: &[Block], new: &[Block], now: Instant) -> Update {
        let old_h: Vec<u64> = old.iter().map(|b| b.hash).collect();
        let new_h: Vec<u64> = new.iter().map(|b| b.hash).collect();
        let marks = &self.marks;
        let mut out = vec![Mark::default(); new.len()];
        let mut map = vec![0; old.len()];
        let mut carry: Vec<usize> = Vec::new();
        let mut put = |i: usize, mut m: Mark, carry: &mut Vec<usize>| {
            if !carry.is_empty() {
                carry.append(&mut m.removed);
                carry.sort_unstable();
                m.removed = std::mem::take(carry);
            }
            out[i] = m;
        };
        for op in capture_diff_slices(Algorithm::Myers, &old_h, &new_h) {
            match op {
                DiffOp::Equal {
                    old_index,
                    new_index,
                    len,
                } => {
                    for k in 0..len {
                        map[old_index + k] = new_index + k;
                        put(new_index + k, marks[old_index + k].clone(), &mut carry);
                    }
                }
                DiffOp::Delete {
                    old_index,
                    old_len,
                    new_index,
                } => {
                    for k in 0..old_len {
                        map[old_index + k] = new_index;
                        carry.extend(lineage(&marks[old_index + k]));
                    }
                }
                DiffOp::Insert {
                    new_index, new_len, ..
                } => {
                    for k in 0..new_len {
                        put(new_index + k, changed(Change::Added, now), &mut carry);
                    }
                }
                DiffOp::Replace {
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                } => {
                    // Pair each new block with the most similar old block ahead of the
                    // last pairing; unpaired new blocks are added, skipped old ones removed.
                    let (mut i, end) = (old_index, old_index + old_len);
                    for (j, nb) in new.iter().enumerate().skip(new_index).take(new_len) {
                        let best = (i..end.min(i + WINDOW))
                            .map(|o| (o, similarity(&old[o].src, &nb.src)))
                            .filter(|&(_, s)| s >= SIMILAR)
                            .max_by(|a, b| a.1.total_cmp(&b.1));
                        let Some((o, _)) = best else {
                            put(j, changed(Change::Added, now), &mut carry);
                            continue;
                        };
                        for d in i..o {
                            map[d] = j;
                            carry.extend(lineage(&marks[d]));
                        }
                        map[o] = j;
                        let m = &marks[o];
                        let kind = if m.change == Some(Change::Added) {
                            Change::Added
                        } else {
                            Change::Modified
                        };
                        carry.extend(m.removed.iter().copied());
                        let mark = Mark {
                            origin: m.origin,
                            open: m.open,
                            ghost_open: m.ghost_open,
                            ..changed(kind, now)
                        };
                        put(j, mark, &mut carry);
                        i = o + 1;
                    }
                    for d in i..end {
                        map[d] = new_index + new_len;
                        carry.extend(lineage(&marks[d]));
                    }
                }
            }
        }
        // A block edited back to its baseline text is no longer changed.
        for (m, b) in out.iter_mut().zip(new) {
            if m.change == Some(Change::Modified)
                && m.origin.is_some_and(|o| self.baseline[o] == b.src)
            {
                m.change = None;
                m.open = false;
            }
        }
        let first_change = out
            .iter()
            .position(|m| m.at == Some(now) && m.change.is_some());
        self.marks = out;
        self.removed_end.extend(carry);
        self.removed_end.sort_unstable();
        Update { map, first_change }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;
    use crate::render::parse;

    fn doc(src: &str) -> Vec<Block> {
        parse(src, &mut Highlighter::default())
    }

    fn kinds(c: &Changes) -> Vec<Option<Change>> {
        c.marks.iter().map(|m| m.change).collect()
    }

    #[test]
    fn insert_modify_delete() {
        let now = Instant::now();
        let old = doc("a\n\nb\n\nc\n");
        let mut c = Changes::new(&old, "opened");
        let u = c.update(&old, &doc("a\n\nb b\n\nc\n\nd\n"), now);
        assert_eq!(
            kinds(&c),
            [None, Some(Change::Modified), None, Some(Change::Added)]
        );
        assert_eq!(c.marks[1].origin, Some(1));
        assert_eq!(c.marks[3].origin, None);
        assert_eq!(u.first_change, Some(1));

        let mut c = Changes::new(&old, "opened");
        let u = c.update(&old, &doc("a\n\nc\n"), now);
        assert_eq!(kinds(&c), [None, None]);
        assert_eq!(c.marks[1].removed, [1]);
        assert_eq!(u.map, [0, 1, 1]);
        assert_eq!(u.first_change, None);

        let mut c = Changes::new(&old, "opened");
        c.update(&old, &doc("a\n\nb\n"), now);
        assert_eq!(c.removed_end, [2]);
    }

    #[test]
    fn replaced_regions_pair_by_similarity() {
        // Inserting a paragraph and editing the next block shows one added, one modified.
        let old = doc("## Lists\n\n- first item\n- second\n");
        let new = doc("## Lists\n\nA brand new paragraph.\n\n- first item, edited\n- second\n");
        let mut c = Changes::new(&old, "opened");
        c.update(&old, &new, Instant::now());
        assert_eq!(
            kinds(&c),
            [None, Some(Change::Added), Some(Change::Modified), None]
        );
        assert_eq!(c.marks[2].origin, Some(1));
        // Rewriting a block entirely reads as remove + add.
        let mut c = Changes::new(&old, "opened");
        c.update(
            &old,
            &doc("## Lists\n\n- totally different words\n- second\n"),
            Instant::now(),
        );
        assert_eq!(kinds(&c), [None, Some(Change::Added), None]);
        assert_eq!(c.marks[2].removed, [1]);
    }

    #[test]
    fn marks_track_the_baseline_across_updates() {
        let t1 = Instant::now();
        let v1 = doc("one two\n\nthree four\n");
        let mut c = Changes::new(&v1, "opened");
        let v2 = doc("one two\n\nthree four\n\nfive six\n");
        c.update(&v1, &v2, t1);
        let t2 = t1 + std::time::Duration::from_millis(10);
        // Edit the new block and block 0: the new block stays "added", block 0 keeps its origin.
        let v3 = doc("one two 2\n\nthree four\n\nfive six 6\n");
        c.update(&v2, &v3, t2);
        assert_eq!(
            kinds(&c),
            [Some(Change::Modified), None, Some(Change::Added)]
        );
        assert_eq!(c.marks[0].origin, Some(0));
        // Removing a block that was not in the baseline leaves no ghost.
        let v4 = doc("one two 2\n\nthree four\n");
        c.update(&v3, &v4, t2);
        assert!(c.removed_end.is_empty());
        // Editing a block back to its baseline text clears its mark.
        let v5 = doc("one two\n\nthree four\n");
        c.update(&v4, &v5, t2);
        assert_eq!(kinds(&c), [None, None]);
    }

    #[test]
    fn removed_blocks_remember_their_baseline_index() {
        let v1 = doc("a\n\nb\n\nc\n\nd\n");
        let mut c = Changes::new(&v1, "opened");
        let v2 = doc("a\n\nd\n");
        c.update(&v1, &v2, Instant::now());
        assert_eq!(c.marks[1].removed, [1, 2]);
        assert_eq!(c.removed(), 2);
    }

    #[test]
    fn words_split_ascii_runs_and_cjk_chars() {
        assert_eq!(words("- item, now 日本"), ["item", "now", "日", "本"]);
    }
}
