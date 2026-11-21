//! Copying one side of a change block onto the other ("merge" arrows in the diff).

use std::ops::Range;

/// Replaces lines `target_range` of `target` with lines `source_range` of
/// `source`. Ranges are 0-based line indices. The target's trailing-newline
/// state is kept (an empty target takes the source's).
pub fn splice_lines(
    target: &str,
    target_range: Range<usize>,
    source: &str,
    source_range: Range<usize>,
) -> String {
    let target_lines: Vec<&str> = target.lines().collect();
    let source_lines: Vec<&str> = source.lines().collect();
    let start = target_range.start.min(target_lines.len());
    let end = target_range.end.min(target_lines.len()).max(start);
    let src_start = source_range.start.min(source_lines.len());
    let src_end = source_range.end.min(source_lines.len()).max(src_start);

    let mut out: Vec<&str> = Vec::with_capacity(target_lines.len() + src_end - src_start);
    out.extend_from_slice(&target_lines[..start]);
    out.extend_from_slice(&source_lines[src_start..src_end]);
    out.extend_from_slice(&target_lines[end..]);
    if out.is_empty() {
        return String::new();
    }
    let trailing = if target.is_empty() {
        source.ends_with('\n') || source.is_empty()
    } else {
        target.ends_with('\n')
    };
    let mut text = out.join("\n");
    if trailing {
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::splice_lines;

    #[test]
    fn replaces_a_block() {
        assert_eq!(
            splice_lines("a\nX\nc\n", 1..2, "a\nb\nc\n", 1..2),
            "a\nb\nc\n"
        );
    }

    #[test]
    fn removes_inserted_lines() {
        // Reverting an insertion: copy an empty old range over the new lines.
        assert_eq!(splice_lines("a\nnew\nb\n", 1..2, "a\nb\n", 1..1), "a\nb\n");
    }

    #[test]
    fn restores_deleted_lines() {
        assert_eq!(splice_lines("a\nc\n", 1..1, "a\nb\nc\n", 1..2), "a\nb\nc\n");
    }

    #[test]
    fn keeps_missing_trailing_newline() {
        assert_eq!(splice_lines("a\nb", 1..2, "a\nB\n", 1..2), "a\nB");
    }

    #[test]
    fn fills_an_empty_target() {
        assert_eq!(splice_lines("", 0..0, "x\ny\n", 0..2), "x\ny\n");
        assert_eq!(splice_lines("x\n", 0..1, "", 0..0), "");
    }
}
