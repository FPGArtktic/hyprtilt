// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! Splitting hyprlang source into logical lines, following hyprlang 0.6's
//! text rules (`docs/hyprland-lua-api.md`, section 8.3):
//!
//! - a line ending in `\` is joined with the next one, before comments are
//!   handled, with the whitespace before the `\` removed;
//! - a line whose first non-blank character is `#` is a comment
//!   (`# hyprlang ...` is a directive);
//! - elsewhere `#` starts a comment, unless it is doubled: `##` is a
//!   literal `#`;
//! - a line is `lhs = rhs`, `name {`, or `}`.

use std::ops::Range;

use crate::block;

/// What a logical line is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LineKind {
    /// Nothing but whitespace.
    Blank,
    /// A comment line.
    Comment,
    /// `# hyprlang if ...`.
    If,
    /// `# hyprlang endif`.
    EndIf,
    /// Another `# hyprlang ...` directive.
    Directive,
    /// `name {`: a category opens.
    Open(String),
    /// `}`: a category closes.
    Close,
    /// `lhs = rhs`, both trimmed.
    Assign {
        /// The key.
        lhs: String,
        /// The value, comment removed and `##` turned into `#`.
        rhs: String,
    },
    /// Anything else (a hyprlang error).
    Other,
}

/// Physical lines joined by trailing backslashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Logical {
    /// The physical lines, line endings included.
    pub(crate) full: Range<usize>,
    /// 1-based number of the first physical line.
    pub(crate) line: usize,
    /// Whether the line was joined from several physical lines.
    pub(crate) joined: bool,
    /// The statement or comment without surrounding whitespace and, for a
    /// single physical line, without a trailing comment.
    pub(crate) stmt: Range<usize>,
    /// What the line is.
    pub(crate) kind: LineKind,
}

/// Split `src` into logical lines. The second value tells whether the last
/// physical line ends with a backslash, which Hyprland reports as an error
/// and which would swallow anything appended after it.
pub(crate) fn logical_lines(src: &str) -> (Vec<Logical>, bool) {
    let lines = block::lines(src);
    let mut out = Vec::new();
    let mut dangling = false;
    let mut i = 0;
    while i < lines.len() {
        let first = i;
        let mut text = String::new();
        loop {
            let content = &src[lines[i].content.clone()];
            if let Some(stripped) = content.strip_suffix('\\') {
                if i + 1 < lines.len() {
                    text.push_str(stripped.trim_end());
                    i += 1;
                    continue;
                }
                dangling = true;
            }
            text.push_str(content);
            break;
        }
        let joined = i > first;
        let content = lines[first].content.clone();
        let (kind, code_len) = classify(&text);
        let lead =
            content.start + (src[content.clone()].len() - src[content.clone()].trim_start().len());
        let stmt_end = if joined {
            let last = &lines[i].content;
            last.start + src[last.clone()].trim_end().len()
        } else {
            let code = &src[content.start..content.start + code_len.min(content.len())];
            content.start + code.trim_end().len()
        };
        out.push(Logical {
            full: lines[first].full.start..lines[i].full.end,
            line: lines[first].number,
            joined,
            stmt: lead.min(stmt_end)..stmt_end,
            kind,
        });
        i += 1;
    }
    (out, dangling)
}

/// Classify a joined line; the second value is the length of the text
/// before a trailing comment.
fn classify(text: &str) -> (LineKind, usize) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return (LineKind::Blank, 0);
    }
    if trimmed.starts_with('#') {
        let directive = trimmed.strip_prefix('#').unwrap_or("").trim_start();
        let kind = match directive.strip_prefix("hyprlang") {
            Some(rest) if rest.starts_with(char::is_whitespace) => {
                let word = rest.split_whitespace().next().unwrap_or("");
                match word {
                    "if" => LineKind::If,
                    "endif" => LineKind::EndIf,
                    _ => LineKind::Directive,
                }
            }
            _ => LineKind::Comment,
        };
        return (kind, text.len());
    }
    let (code, comment_at) = strip_comment(text);
    let code_len = comment_at.unwrap_or(text.len());
    let code = code.trim();
    let kind = if code == "}" {
        LineKind::Close
    } else if let Some((lhs, rhs)) = code.split_once('=') {
        LineKind::Assign {
            lhs: lhs.trim().to_owned(),
            rhs: rhs.trim().to_owned(),
        }
    } else if let Some(name) = code.strip_suffix('{') {
        LineKind::Open(name.trim().to_owned())
    } else {
        LineKind::Other
    };
    (kind, code_len)
}

/// Remove a trailing comment and turn `##` into `#`. Returns the text and
/// the byte offset in `text` where the comment starts.
pub(crate) fn strip_comment(text: &str) -> (String, Option<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '#' {
            if chars.peek().is_some_and(|&(_, n)| n == '#') {
                chars.next();
                out.push('#');
                continue;
            }
            return (out, Some(i));
        }
        out.push(c);
    }
    (out, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<LineKind> {
        logical_lines(src).0.into_iter().map(|l| l.kind).collect()
    }

    #[test]
    fn classification() {
        let src = "# c\n\n$v = 1\nmonitor=a,b # note\ngeneral {\n}\nstray\n# hyprlang if X\n# hyprlang endif\n# hyprlang noerror true\n#hyprlangish\n";
        assert_eq!(
            kinds(src),
            [
                LineKind::Comment,
                LineKind::Blank,
                LineKind::Assign {
                    lhs: "$v".to_owned(),
                    rhs: "1".to_owned()
                },
                LineKind::Assign {
                    lhs: "monitor".to_owned(),
                    rhs: "a,b".to_owned()
                },
                LineKind::Open("general".to_owned()),
                LineKind::Close,
                LineKind::Other,
                LineKind::If,
                LineKind::EndIf,
                LineKind::Directive,
                LineKind::Comment,
            ]
        );
    }

    #[test]
    fn doubled_hashes_are_literal() {
        assert_eq!(strip_comment("a ## b # c"), ("a # b ".to_owned(), Some(7)));
        assert_eq!(strip_comment("no comment"), ("no comment".to_owned(), None));
        assert_eq!(
            kinds("x = a##b\n"),
            [LineKind::Assign {
                lhs: "x".to_owned(),
                rhs: "a#b".to_owned()
            }]
        );
    }

    #[test]
    fn statement_spans_exclude_comments() {
        let src = "  monitor = DP-1, preferred, auto, 1   # fast\n";
        let (lines, dangling) = logical_lines(src);
        assert!(!dangling);
        assert_eq!(
            &src[lines[0].stmt.clone()],
            "monitor = DP-1, preferred, auto, 1"
        );
        let src = "# only a comment  \n";
        let (lines, _) = logical_lines(src);
        assert_eq!(&src[lines[0].stmt.clone()], "# only a comment");
    }

    #[test]
    fn backslashes_join_lines() {
        let src = "monitor = DP-1, \\\n  preferred, auto, 1\n# c \\\nswallowed = 1\nlast \\";
        let (lines, dangling) = logical_lines(src);
        assert!(dangling);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].joined);
        assert_eq!(
            lines[0].kind,
            LineKind::Assign {
                lhs: "monitor".to_owned(),
                rhs: "DP-1,  preferred, auto, 1".to_owned()
            }
        );
        assert_eq!(
            &src[lines[0].stmt.clone()],
            "monitor = DP-1, \\\n  preferred, auto, 1"
        );
        assert_eq!(lines[0].full, 0..src.find("# c").unwrap());
        // A comment ending in a backslash swallows the next line.
        assert_eq!(lines[1].kind, LineKind::Comment);
        assert_eq!(lines[1].line, 3);
        assert_eq!(lines[2].line, 5);
    }
}
