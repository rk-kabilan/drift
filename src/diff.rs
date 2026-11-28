//! Line diff model for the diff pane: hunks, folded unchanged regions and
//! intra-line change ranges, ready for side-by-side or inline display.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use similar::{ChangeTag, DiffOp, TextDiff};

/// Lines of unchanged context kept around each change.
pub const CONTEXT: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Removed,
    Added,
    // Parts of a merge conflict, in the conflict view.
    /// `<<<<<<< HEAD`
    CurrentMarker,
    /// A line of the current change (ours).
    Current,
    /// The `|||||||` marker and the common ancestor's lines (diff3 style).
    Base,
    /// `=======`
    Separator,
    /// A line of the incoming change (theirs).
    Incoming,
    /// `>>>>>>> branch`
    IncomingMarker,
}

/// One line on one side of the diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    /// 1-based line number in its file.
    pub line: usize,
    /// Shared with the diff model and every row showing this line, so
    /// building rows and rendering never copy line text.
    pub text: Arc<str>,
    pub kind: LineKind,
    /// Byte ranges of `text` that changed relative to the paired line.
    pub changes: Vec<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// `@@ -a,b +c,d @@` header at the start of a hunk.
    Hunk { header: String },
    /// Side by side: the old line on the left, the new line on the right.
    /// Either side is `None` where the other side has no counterpart.
    Pair {
        left: Option<Side>,
        right: Option<Side>,
        /// Set on the first row of a change block: its index in [`FileDiff::blocks`].
        block: Option<usize>,
    },
    /// Inline: a single line, which carries the old and/or new line number.
    Line {
        old: Option<usize>,
        new: Option<usize>,
        side: Side,
        /// Set on the first row of a change block: its index in [`FileDiff::blocks`].
        block: Option<usize>,
    },
    /// A folded run of unchanged lines; clicking it expands fold `id`.
    Fold { id: usize, hidden: usize },
    /// The actions above merge conflict `index` (Accept Current | Accept Incoming | …).
    Conflict { index: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    SideBySide,
    Inline,
}

enum Block {
    Equal {
        old_start: usize,
        new_start: usize,
        len: usize,
    },
    Change {
        old_start: usize,
        new_start: usize,
        dels: Vec<Side>,
        adds: Vec<Side>,
    },
}

/// The line ranges (0-based, end-exclusive) a change block covers on each side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeBlock {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// A computed diff between two texts.
pub struct FileDiff {
    old_lines: Vec<Arc<str>>,
    blocks: Vec<Block>,
    pub added: usize,
    pub removed: usize,
    /// Characters in the longest line on either side, for horizontal scrolling.
    pub max_cols: usize,
}

impl FileDiff {
    pub fn new(old: &str, new: &str) -> Self {
        let old_lines: Vec<Arc<str>> = split_lines(old);
        let new_lines: Vec<Arc<str>> = split_lines(new);
        let max_cols = old_lines
            .iter()
            .chain(&new_lines)
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        let old_refs: Vec<&str> = old_lines.iter().map(|l| &**l).collect();
        let new_refs: Vec<&str> = new_lines.iter().map(|l| &**l).collect();
        let diff = TextDiff::configure()
            .algorithm(similar::Algorithm::Patience)
            .diff_slices(&old_refs, &new_refs);

        let mut blocks: Vec<Block> = Vec::new();
        let (mut added, mut removed) = (0, 0);
        for op in diff.ops() {
            match *op {
                DiffOp::Equal {
                    old_index,
                    new_index,
                    len,
                } => blocks.push(Block::Equal {
                    old_start: old_index,
                    new_start: new_index,
                    len,
                }),
                DiffOp::Delete {
                    old_index, old_len, ..
                } => {
                    removed += old_len;
                    push_change(
                        &mut blocks,
                        (old_index, op.new_range().start),
                        side_lines(&old_lines, old_index, old_len, LineKind::Removed),
                        vec![],
                    );
                }
                DiffOp::Insert {
                    new_index, new_len, ..
                } => {
                    added += new_len;
                    push_change(
                        &mut blocks,
                        (op.old_range().start, new_index),
                        vec![],
                        side_lines(&new_lines, new_index, new_len, LineKind::Added),
                    );
                }
                DiffOp::Replace {
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                } => {
                    removed += old_len;
                    added += new_len;
                    push_change(
                        &mut blocks,
                        (old_index, new_index),
                        side_lines(&old_lines, old_index, old_len, LineKind::Removed),
                        side_lines(&new_lines, new_index, new_len, LineKind::Added),
                    );
                }
            }
        }
        for block in &mut blocks {
            if let Block::Change { dels, adds, .. } = block {
                for (d, a) in dels.iter_mut().zip(adds.iter_mut()) {
                    if let Some((dr, ar)) = intra_line(&d.text, &a.text) {
                        d.changes = dr;
                        a.changes = ar;
                    }
                }
            }
        }
        // Keep the new side's text for context lines in `old_lines`'s place: they are equal.
        Self {
            old_lines,
            blocks,
            added,
            removed,
            max_cols,
        }
    }

    /// Every change block, in order; `Row`s refer to these by index.
    pub fn blocks(&self) -> Vec<ChangeBlock> {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Change {
                    old_start,
                    new_start,
                    dels,
                    adds,
                } => Some(ChangeBlock {
                    old: *old_start..old_start + dels.len(),
                    new: *new_start..new_start + adds.len(),
                }),
                Block::Equal { .. } => None,
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0
    }

    /// Display rows for `layout`, with every fold collapsed unless its id is in `expanded`.
    pub fn rows(&self, layout: Layout, expanded: &HashSet<usize>) -> Vec<Row> {
        // First pass: a flat list of line rows and folds.
        let mut items: Vec<Row> = Vec::new();
        let mut fold_id = 0;
        let mut change_ix = 0;
        let last = self.blocks.len().saturating_sub(1);
        for (bi, block) in self.blocks.iter().enumerate() {
            match block {
                Block::Equal {
                    old_start,
                    new_start,
                    len,
                } => {
                    let (old_start, new_start, len) = (*old_start, *new_start, *len);
                    let keep_head = if bi == 0 { 0 } else { CONTEXT };
                    let keep_tail = if bi == last { 0 } else { CONTEXT };
                    let foldable = len > keep_head + keep_tail + 1 && !self.is_empty();
                    let this_fold = fold_id;
                    if foldable {
                        fold_id += 1;
                    }
                    for i in 0..len {
                        if foldable
                            && !expanded.contains(&this_fold)
                            && i >= keep_head
                            && i < len - keep_tail
                        {
                            if i == keep_head {
                                items.push(Row::Fold {
                                    id: this_fold,
                                    hidden: len - keep_head - keep_tail,
                                });
                            }
                            continue;
                        }
                        let text = self.old_lines[old_start + i].clone();
                        let side = |line| Side {
                            line,
                            text: text.clone(),
                            kind: LineKind::Context,
                            changes: vec![],
                        };
                        items.push(match layout {
                            Layout::SideBySide => Row::Pair {
                                left: Some(side(old_start + i + 1)),
                                right: Some(side(new_start + i + 1)),
                                block: None,
                            },
                            Layout::Inline => Row::Line {
                                old: Some(old_start + i + 1),
                                new: Some(new_start + i + 1),
                                side: side(new_start + i + 1),
                                block: None,
                            },
                        });
                    }
                }
                Block::Change { dels, adds, .. } => {
                    let this_block = change_ix;
                    change_ix += 1;
                    let first = |i: usize| (i == 0).then_some(this_block);
                    match layout {
                        Layout::SideBySide => {
                            for i in 0..dels.len().max(adds.len()) {
                                items.push(Row::Pair {
                                    left: dels.get(i).cloned(),
                                    right: adds.get(i).cloned(),
                                    block: first(i),
                                });
                            }
                        }
                        Layout::Inline => {
                            for (i, side) in dels.iter().chain(adds.iter()).enumerate() {
                                let removed = side.kind == LineKind::Removed;
                                items.push(Row::Line {
                                    old: removed.then_some(side.line),
                                    new: (!removed).then_some(side.line),
                                    side: side.clone(),
                                    block: first(i),
                                });
                            }
                        }
                    }
                }
            }
        }

        // Second pass: put a hunk header before each run of lines between folds.
        let mut rows = Vec::with_capacity(items.len() + 8);
        let mut start = 0;
        while start < items.len() {
            if let Row::Fold { .. } = items[start] {
                rows.push(items[start].clone());
                start += 1;
                continue;
            }
            let end = items[start..]
                .iter()
                .position(|r| matches!(r, Row::Fold { .. }))
                .map_or(items.len(), |p| start + p);
            if !self.is_empty() {
                rows.push(Row::Hunk {
                    header: hunk_header(&items[start..end]),
                });
            }
            rows.extend_from_slice(&items[start..end]);
            start = end;
        }
        rows
    }
}

pub(crate) fn split_lines(text: &str) -> Vec<Arc<str>> {
    text.lines().map(|l| Arc::from(&*expand_tabs(l))).collect()
}

fn expand_tabs(line: &str) -> std::borrow::Cow<'_, str> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if !line.contains('\t') {
        return line.into();
    }
    let mut out = String::with_capacity(line.len() + 8);
    let mut col = 0;
    for ch in line.chars() {
        if ch == '\t' {
            let n = 4 - col % 4;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out.into()
}

fn side_lines(lines: &[Arc<str>], start: usize, len: usize, kind: LineKind) -> Vec<Side> {
    (start..start + len)
        .map(|i| Side {
            line: i + 1,
            text: lines[i].clone(),
            kind,
            changes: vec![],
        })
        .collect()
}

fn push_change(
    blocks: &mut Vec<Block>,
    starts: (usize, usize),
    mut d: Vec<Side>,
    mut a: Vec<Side>,
) {
    if let Some(Block::Change { dels, adds, .. }) = blocks.last_mut() {
        dels.append(&mut d);
        adds.append(&mut a);
    } else {
        blocks.push(Block::Change {
            old_start: starts.0,
            new_start: starts.1,
            dels: d,
            adds: a,
        });
    }
}

fn hunk_header(items: &[Row]) -> String {
    let (mut old_first, mut old_count, mut new_first, mut new_count) = (None, 0, None, 0);
    for item in items {
        let (o, n) = match item {
            Row::Pair { left, right, .. } => (
                left.as_ref().map(|s| s.line),
                right.as_ref().map(|s| s.line),
            ),
            Row::Line { old, new, .. } => (*old, *new),
            _ => (None, None),
        };
        if let Some(o) = o {
            old_first.get_or_insert(o);
            old_count += 1;
        }
        if let Some(n) = n {
            new_first.get_or_insert(n);
            new_count += 1;
        }
    }
    format!(
        "@@ -{},{} +{},{} @@",
        old_first.unwrap_or(0),
        old_count,
        new_first.unwrap_or(0),
        new_count
    )
}

/// Byte ranges of a line that changed.
pub type ChangedRanges = Vec<Range<usize>>;

/// Word-level changed ranges between two versions of a line. Returns `None`
/// when the lines are too different for a highlight to be useful.
pub fn intra_line(old: &str, new: &str) -> Option<(ChangedRanges, ChangedRanges)> {
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);
    let diff = TextDiff::configure()
        .algorithm(similar::Algorithm::Myers)
        .diff_slices(&old_tokens, &new_tokens);
    let (mut o, mut n) = (0usize, 0usize);
    let (mut old_ranges, mut new_ranges): (ChangedRanges, ChangedRanges) = (vec![], vec![]);
    let mut same = 0usize;
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                if !change.value().trim().is_empty() {
                    same += len;
                }
                o += len;
                n += len;
            }
            ChangeTag::Delete => {
                push_range(&mut old_ranges, o..o + len);
                o += len;
            }
            ChangeTag::Insert => {
                push_range(&mut new_ranges, n..n + len);
                n += len;
            }
        }
    }
    let longest = old.trim().len().max(new.trim().len()).max(1);
    if same * 10 < longest * 3 {
        return None;
    }
    Some((old_ranges, new_ranges))
}

/// Splits a line into identifier/number runs, whitespace runs and single
/// punctuation characters, so changes highlight at a natural granularity.
fn tokenize(line: &str) -> Vec<&str> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Punct,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Punct
        }
    };
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<Class> = None;
    for (i, c) in line.char_indices() {
        let k = class(c);
        let split = match &prev {
            None => false,
            Some(p) => *p != k || k == Class::Punct,
        };
        if split {
            out.push(&line[start..i]);
            start = i;
        }
        prev = Some(k);
    }
    if start < line.len() {
        out.push(&line[start..]);
    }
    out
}

fn push_range(ranges: &mut Vec<Range<usize>>, r: Range<usize>) {
    if let Some(last) = ranges.last_mut() {
        if last.end == r.start {
            last.end = r.end;
            return;
        }
    }
    ranges.push(r);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(rows: &[Row], f: impl Fn(&Row) -> bool) -> usize {
        rows.iter().filter(|r| f(r)).count()
    }

    #[test]
    fn identical_texts_have_no_changes() {
        let d = FileDiff::new("a\nb\n", "a\nb\n");
        assert!(d.is_empty());
        let rows = d.rows(Layout::SideBySide, &HashSet::new());
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn replace_pairs_lines_and_marks_words() {
        let d = FileDiff::new("let x = 1;\n", "let x = 2;\n");
        assert_eq!((d.added, d.removed), (1, 1));
        let rows = d.rows(Layout::SideBySide, &HashSet::new());
        let Row::Pair {
            left: Some(l),
            right: Some(r),
            ..
        } = &rows[1]
        else {
            panic!("{rows:?}")
        };
        assert_eq!(l.kind, LineKind::Removed);
        assert_eq!(r.kind, LineKind::Added);
        assert_eq!(&l.text[l.changes[0].clone()], "1");
        assert_eq!(&r.text[r.changes[0].clone()], "2");
    }

    #[test]
    fn long_unchanged_runs_fold_and_expand() {
        let old: String = (1..=40).map(|i| format!("line {i}\n")).collect();
        let new = old.replace("line 20\n", "line twenty\n");
        let d = FileDiff::new(&old, &new);
        let rows = d.rows(Layout::SideBySide, &HashSet::new());
        assert_eq!(count(&rows, |r| matches!(r, Row::Fold { .. })), 2);
        assert_eq!(count(&rows, |r| matches!(r, Row::Hunk { .. })), 1);
        assert!(matches!(&rows[1], Row::Hunk { header } if header == "@@ -17,7 +17,7 @@"));
        let all: HashSet<usize> = [0, 1].into_iter().collect();
        let rows = d.rows(Layout::SideBySide, &all);
        assert_eq!(count(&rows, |r| matches!(r, Row::Pair { .. })), 40);
    }

    #[test]
    fn inline_lists_removed_before_added() {
        let d = FileDiff::new("a\nb\nc\n", "a\nB\nc\n");
        let rows = d.rows(Layout::Inline, &HashSet::new());
        let kinds: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Line { side, .. } => Some(side.kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            [
                LineKind::Context,
                LineKind::Removed,
                LineKind::Added,
                LineKind::Context
            ]
        );
    }

    #[test]
    fn blocks_report_line_ranges() {
        let d = FileDiff::new("a\nb\nc\nd\n", "a\nB\nc\nd\ne\n");
        assert_eq!(
            d.blocks(),
            [
                ChangeBlock {
                    old: 1..2,
                    new: 1..2
                },
                ChangeBlock {
                    old: 4..4,
                    new: 4..5
                }
            ]
        );
        let rows = d.rows(Layout::SideBySide, &HashSet::new());
        let starts: Vec<usize> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Pair { block, .. } => *block,
                _ => None,
            })
            .collect();
        assert_eq!(starts, [0, 1]);
    }

    #[test]
    fn tabs_expand_to_four_columns() {
        assert_eq!(expand_tabs("\tx"), "    x");
        assert_eq!(expand_tabs("ab\tx"), "ab  x");
    }
}
