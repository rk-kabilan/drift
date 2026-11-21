//! A small, language-agnostic highlighter for diff lines. It recognises
//! comments, strings, numbers, keywords, type-like names and function calls,
//! which covers the common C-family, Rust, Python, JS/TS, Go and shell syntax
//! without a full parser.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Comment,
    Type,
    Number,
    Function,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    /// `//` comments, `#[...]` attributes.
    CLike,
    /// `#` comments.
    Hash,
    /// No highlighting (plain text, markdown, data files).
    Plain,
}

impl Lang {
    pub fn for_path(path: &str) -> Self {
        let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "rs" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "java" | "kt" | "swift" | "go"
            | "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "scala" | "dart" | "zig" | "css"
            | "scss" | "json" | "jsonc" => Lang::CLike,
            "py" | "rb" | "sh" | "bash" | "zsh" | "toml" | "yaml" | "yml" | "r" | "pl" | "nix"
            | "mk" | "cmake" => Lang::Hash,
            _ if path.ends_with("Makefile") || path.ends_with("Dockerfile") => Lang::Hash,
            _ => Lang::Plain,
        }
    }

    pub fn name_for_path(path: &str) -> &'static str {
        let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "rs" => "Rust",
            "toml" => "TOML",
            "md" => "Markdown",
            "json" => "JSON",
            "js" | "mjs" | "cjs" | "jsx" => "JavaScript",
            "ts" | "tsx" => "TypeScript",
            "py" => "Python",
            "go" => "Go",
            "c" | "h" => "C",
            "cc" | "cpp" | "hpp" => "C++",
            "yml" | "yaml" => "YAML",
            "svg" => "SVG",
            "html" => "HTML",
            "css" => "CSS",
            "sh" | "bash" | "zsh" => "Shell",
            _ => "Plain Text",
        }
    }
}

const KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "crate",
    "def",
    "default",
    "defer",
    "del",
    "do",
    "dyn",
    "elif",
    "else",
    "enum",
    "except",
    "export",
    "extends",
    "extern",
    "false",
    "final",
    "finally",
    "fn",
    "for",
    "from",
    "func",
    "function",
    "go",
    "if",
    "impl",
    "import",
    "in",
    "interface",
    "is",
    "lambda",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "namespace",
    "nil",
    "None",
    "not",
    "null",
    "or",
    "and",
    "package",
    "pass",
    "private",
    "protected",
    "pub",
    "public",
    "raise",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "switch",
    "this",
    "throw",
    "trait",
    "true",
    "True",
    "False",
    "try",
    "type",
    "typeof",
    "unsafe",
    "use",
    "using",
    "var",
    "void",
    "where",
    "while",
    "with",
    "yield",
];

/// Token ranges (byte offsets) for one line, in order and non-overlapping.
pub fn highlight(line: &str, lang: Lang) -> Vec<(Range<usize>, Token)> {
    let mut out = Vec::new();
    if lang == Lang::Plain {
        return out;
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let rest = &line[i..];
        // Comments run to the end of the line.
        let comment = match lang {
            Lang::CLike => {
                rest.starts_with("//")
                    || rest.starts_with("/*")
                    || (rest.starts_with('*') && line[..i].trim().is_empty())
            }
            Lang::Hash => c == b'#',
            Lang::Plain => false,
        };
        if comment {
            out.push((i..line.len(), Token::Comment));
            break;
        }
        if lang == Lang::CLike && rest.starts_with("#[") {
            let end = rest.find(']').map_or(line.len(), |p| i + p + 1);
            out.push((i..end, Token::Comment));
            i = end;
            continue;
        }
        if c == b'"' || c == b'\'' || c == b'`' {
            // Rust lifetimes ('a) look like char literals; only treat ' as a quote when it closes nearby.
            if c == b'\'' && lang == Lang::CLike {
                let close = rest[1..].find('\'');
                if !matches!(close, Some(p) if p <= 2 || rest[1..].starts_with('\\')) {
                    i += 1;
                    continue;
                }
            }
            let mut j = i + 1;
            while j < bytes.len() {
                if bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if bytes[j] == c {
                    j += 1;
                    break;
                }
                j += 1;
            }
            let j = j.min(bytes.len());
            out.push((i..j, Token::String));
            i = j;
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'.' || bytes[j] == b'_')
            {
                j += 1;
            }
            out.push((i..j, Token::Number));
            i = j;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let word = &line[i..j];
            let next = line[j..].trim_start().chars().next();
            let token = if KEYWORDS.contains(&word) {
                Some(Token::Keyword)
            } else if word.starts_with(|ch: char| ch.is_ascii_uppercase()) {
                Some(Token::Type)
            } else if next == Some('(') || line[j..].starts_with('!') {
                Some(Token::Function)
            } else {
                None
            };
            if let Some(t) = token {
                out.push((i..j, t));
            }
            i = j;
            continue;
        }
        // Advance by a whole UTF-8 character.
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str, lang: Lang) -> Vec<(&str, Token)> {
        highlight(line, lang)
            .into_iter()
            .map(|(r, t)| (&line[r], t))
            .collect()
    }

    #[test]
    fn rust_line() {
        assert_eq!(
            kinds("pub fn new(x: u8) -> Self { \"hi\" } // done", Lang::CLike),
            vec![
                ("pub", Token::Keyword),
                ("fn", Token::Keyword),
                ("new", Token::Function),
                ("Self", Token::Keyword),
                ("\"hi\"", Token::String),
                ("// done", Token::Comment),
            ]
        );
    }

    #[test]
    fn lifetimes_are_not_strings() {
        assert!(kinds("fn f<'a>(x: &'a str)", Lang::CLike)
            .iter()
            .all(|(_, t)| *t != Token::String));
    }

    #[test]
    fn hash_comments() {
        assert_eq!(
            kinds("x = 1 # note", Lang::Hash),
            vec![("1", Token::Number), ("# note", Token::Comment)]
        );
    }
}
