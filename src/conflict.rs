//! Merge conflict markers in a working-tree file, and resolving them the way
//! VS Code does: keep the current side, the incoming side, or both.

use std::ops::Range;
use std::sync::Arc;

use crate::diff::{LineKind, Row, Side};

/// One conflict: the 0-based line indices of its markers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// `<<<<<<< HEAD`
    pub start: usize,
    /// `||||||| base`, present with `merge.conflictStyle = diff3`.
    pub base: Option<usize>,
    /// `=======`
    pub mid: usize,
    /// `>>>>>>> branch`
    pub end: usize,
}

impl Conflict {
    /// Lines of the current change (ours).
    pub fn current(&self) -> Range<usize> {
        self.start + 1..self.base.unwrap_or(self.mid)
    }

    /// Lines of the incoming change (theirs).
    pub fn incoming(&self) -> Range<usize> {
        self.mid + 1..self.end
    }
}

/// Which side of a conflict to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Current,
    Incoming,
    Both,
}

/// The kind of conflict marker a line is, if any. Git writes markers as seven
/// characters, followed by a space and a label or by the end of the line.
fn marker(line: &str) -> Option<u8> {
    let b = line.as_bytes();
    let c = *b.first()?;
    if !matches!(c, b'<' | b'|' | b'=' | b'>') || b.len() < 7 || b[..7].iter().any(|&x| x != c) {
        return None;
    }
    match b.get(7) {
        None => Some(c),
        Some(b' ') if c != b'=' => Some(c),
        Some(b'\r') => Some(c),
        _ => None,
    }
}

/// Finds every well-formed conflict in `text`. A block with markers out of
/// order is skipped, as git never writes one.
pub fn parse(text: &str) -> Vec<Conflict> {
    let mut out = Vec::new();
    let (mut start, mut base, mut mid) = (None, None, None);
    for (i, line) in text.lines().enumerate() {
        match marker(line) {
            Some(b'<') => (start, base, mid) = (Some(i), None, None),
            Some(b'|') if start.is_some() && base.is_none() && mid.is_none() => base = Some(i),
            Some(b'=') if start.is_some() && mid.is_none() => mid = Some(i),
            Some(b'>') => {
                if let (Some(s), Some(m)) = (start, mid) {
                    out.push(Conflict {
                        start: s,
                        base,
                        mid: m,
                        end: i,
                    });
                }
                (start, base, mid) = (None, None, None);
            }
            _ => {}
        }
    }
    out
}

/// The label after a marker, e.g. `HEAD` in `<<<<<<< HEAD`.
pub fn label(line: &str) -> &str {
    line.get(7..).unwrap_or("").trim()
}

/// `text` with `conflict` replaced by the side(s) `how` keeps.
pub fn resolve(text: &str, conflict: &Conflict, how: Resolution) -> String {
    resolve_many(text, std::slice::from_ref(conflict), how)
}

/// `text` with every conflict resolved the same way.
pub fn resolve_all(text: &str, how: Resolution) -> String {
    resolve_many(text, &parse(text), how)
}

fn resolve_many(text: &str, conflicts: &[Conflict], how: Resolution) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut next = 0;
    for c in conflicts {
        if c.start < next || c.end >= lines.len() {
            continue;
        }
        out.extend_from_slice(&lines[next..c.start]);
        if how != Resolution::Incoming {
            out.extend_from_slice(&lines[c.current()]);
        }
        if how != Resolution::Current {
            out.extend_from_slice(&lines[c.incoming()]);
        }
        next = c.end + 1;
    }
    out.extend_from_slice(&lines[next.min(lines.len())..]);
    let mut s = out.join("\n");
    if !s.is_empty() && text.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// A conflicted file as shown in the conflict view: its lines (shared by the
/// rows, never copied) and where its conflicts are.
pub struct ConflictDoc {
    pub lines: Vec<Arc<str>>,
    pub conflicts: Vec<Conflict>,
    /// Characters in the longest line, for horizontal scrolling.
    pub max_cols: usize,
}

impl ConflictDoc {
    pub fn new(text: &str) -> Self {
        let lines = crate::diff::split_lines(text);
        let max_cols = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        Self {
            lines,
            conflicts: parse(text),
            max_cols,
        }
    }

    /// One row per line, with an action row ([`Row::Conflict`]) above each conflict.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::with_capacity(self.lines.len() + self.conflicts.len());
        let mut conflicts = self.conflicts.iter().enumerate().peekable();
        let mut current: Option<&Conflict> = None;
        for (i, text) in self.lines.iter().enumerate() {
            if let Some((ix, c)) = conflicts.next_if(|(_, c)| c.start == i) {
                rows.push(Row::Conflict { index: ix });
                current = Some(c);
            }
            let kind = match current {
                Some(c) if i == c.start => LineKind::CurrentMarker,
                Some(c) if i == c.end => LineKind::IncomingMarker,
                Some(c) if i == c.mid => LineKind::Separator,
                Some(c) if c.base.is_some_and(|b| i >= b && i < c.mid) => LineKind::Base,
                Some(c) if c.current().contains(&i) => LineKind::Current,
                Some(c) if c.incoming().contains(&i) => LineKind::Incoming,
                _ => LineKind::Context,
            };
            if current.is_some_and(|c| i >= c.end) {
                current = None;
            }
            rows.push(Row::Line {
                old: None,
                new: Some(i + 1),
                side: Side {
                    line: i + 1,
                    text: text.clone(),
                    kind,
                    changes: Vec::new(),
                },
                block: None,
            });
        }
        rows
    }

    /// The conflict that line `line` (1-based) is in or, failing that, the next one.
    pub fn at_or_after(&self, line: usize) -> Option<usize> {
        let ix = line.saturating_sub(1);
        self.conflicts.iter().position(|c| ix <= c.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "a\n<<<<<<< HEAD\nours 1\nours 2\n=======\ntheirs\n>>>>>>> origin/two\nb\n";

    #[test]
    fn finds_conflicts() {
        let c = parse(TEXT);
        assert_eq!(
            c,
            vec![Conflict {
                start: 1,
                base: None,
                mid: 4,
                end: 6
            }]
        );
        assert_eq!(c[0].current(), 2..4);
        assert_eq!(c[0].incoming(), 5..6);
        assert_eq!(label("<<<<<<< HEAD"), "HEAD");
        assert_eq!(label(">>>>>>> origin/two"), "origin/two");
    }

    #[test]
    fn resolves_each_way() {
        let c = &parse(TEXT)[0];
        assert_eq!(
            resolve(TEXT, c, Resolution::Current),
            "a\nours 1\nours 2\nb\n"
        );
        assert_eq!(resolve(TEXT, c, Resolution::Incoming), "a\ntheirs\nb\n");
        assert_eq!(
            resolve(TEXT, c, Resolution::Both),
            "a\nours 1\nours 2\ntheirs\nb\n"
        );
    }

    #[test]
    fn handles_diff3_and_several_conflicts() {
        let text = "<<<<<<< ours\nx\n||||||| base\no\n=======\ny\n>>>>>>> theirs\nmid\n<<<<<<< ours\n=======\nz\n>>>>>>> theirs";
        let c = parse(text);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].base, Some(2));
        assert_eq!(c[0].current(), 1..2);
        assert_eq!(resolve_all(text, Resolution::Current), "x\nmid");
        assert_eq!(resolve_all(text, Resolution::Incoming), "y\nmid\nz");
        assert_eq!(resolve_all(text, Resolution::Both), "x\ny\nmid\nz");
    }

    #[test]
    fn ignores_lookalikes_and_broken_blocks() {
        assert!(parse("========\n<<<<<<<< no\n").is_empty());
        assert!(parse("<<<<<<< a\nx\n>>>>>>> b\n").is_empty());
        assert!(parse("=======\n>>>>>>> b\n").is_empty());
    }

    #[test]
    fn rows_mark_each_part() {
        let doc = ConflictDoc::new(TEXT);
        let rows = doc.rows();
        assert_eq!(rows.len(), 9);
        assert!(matches!(rows[1], Row::Conflict { index: 0 }));
        let kinds: Vec<LineKind> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Line { side, .. } => Some(side.kind),
                _ => None,
            })
            .collect();
        use LineKind::*;
        assert_eq!(
            kinds,
            vec![
                Context,
                CurrentMarker,
                Current,
                Current,
                Separator,
                Incoming,
                IncomingMarker,
                Context
            ]
        );
        assert_eq!(doc.at_or_after(1), Some(0));
        assert_eq!(doc.at_or_after(8), None);
    }
}
