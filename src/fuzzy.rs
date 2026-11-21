//! Fuzzy matching for the quick pickers (Cmd+P files, branches).

/// A match of `query` in a haystack: its score and the matched byte offsets.
#[derive(Clone, Debug, PartialEq)]
pub struct FuzzyMatch {
    pub score: i64,
    pub positions: Vec<usize>,
}

/// Case-insensitive subsequence match. Consecutive characters, characters at
/// the start of a word or path segment, and characters inside the file name
/// (from `name_start` on) score higher. Returns `None` when `query` is not a
/// subsequence of `haystack`. An empty query matches everything with score 0.
pub fn fuzzy_match(query: &str, haystack: &str, name_start: usize) -> Option<FuzzyMatch> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: vec![],
        });
    }
    let chars: Vec<(usize, char)> = haystack.char_indices().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|(_, c)| c.to_lowercase().next().unwrap_or(*c))
        .collect();

    // Greedy pass from the right, so matches prefer the file name over folders,
    // then a left-to-right pass from the first possible start for tighter runs.
    let mut best: Option<FuzzyMatch> = None;
    for start in 0..chars.len() {
        if lower[start] != query[0] {
            continue;
        }
        let mut positions = Vec::with_capacity(query.len());
        let mut qi = 0;
        let mut i = start;
        while i < chars.len() && qi < query.len() {
            if lower[i] == query[qi] {
                positions.push(i);
                qi += 1;
            }
            i += 1;
        }
        if qi < query.len() {
            break; // no later start can match either
        }
        let score = score(&positions, &chars, name_start);
        if best.as_ref().is_none_or(|b| score > b.score) {
            best = Some(FuzzyMatch {
                score,
                positions: positions.iter().map(|&p| chars[p].0).collect(),
            });
        }
    }
    best
}

fn score(positions: &[usize], chars: &[(usize, char)], name_start: usize) -> i64 {
    let mut score: i64 = 0;
    for (k, &p) in positions.iter().enumerate() {
        score += 10;
        if k > 0 && positions[k - 1] + 1 == p {
            score += 15;
        }
        let boundary = p == 0 || matches!(chars[p - 1].1, '/' | '\\' | '_' | '-' | '.' | ' ');
        let camel = p > 0 && chars[p].1.is_uppercase() && chars[p - 1].1.is_lowercase();
        if boundary || camel {
            score += 12;
        }
        if chars[p].0 >= name_start {
            score += 8;
        }
    }
    // Prefer shorter haystacks and earlier matches.
    score - chars.len() as i64 / 4 - positions.first().copied().unwrap_or(0) as i64 / 8
}

#[cfg(test)]
mod tests {
    use super::fuzzy_match;

    fn name_start(path: &str) -> usize {
        path.rfind('/').map_or(0, |i| i + 1)
    }

    fn rank<'a>(query: &str, paths: &[&'a str]) -> Vec<&'a str> {
        let mut scored: Vec<(i64, &str)> = paths
            .iter()
            .filter_map(|p| fuzzy_match(query, p, name_start(p)).map(|m| (m.score, *p)))
            .collect();
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored.into_iter().map(|(_, p)| p).collect()
    }

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_match("xyz", "src/main.rs", 4).is_none());
        assert_eq!(
            fuzzy_match("smr", "src/main.rs", 4).unwrap().positions,
            vec![0, 4, 9]
        );
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(fuzzy_match("", "anything", 0).unwrap().score, 0);
    }

    #[test]
    fn prefers_file_name_and_word_starts() {
        let paths = [
            "src/ui/diff_pane.rs",
            "src/diff.rs",
            "docs/different/notes.md",
        ];
        assert_eq!(rank("diff", &paths)[0], "src/diff.rs");
        assert_eq!(rank("dpane", &paths), ["src/ui/diff_pane.rs"]);
    }

    #[test]
    fn case_insensitive() {
        assert!(fuzzy_match("CARGO", "Cargo.toml", 0).is_some());
    }
}
