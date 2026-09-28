// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>

//! The managed block: a language-independent text engine.
//!
//! hyprtilt owns exactly the lines between a begin marker and an end marker
//! and nothing else. This module finds the markers, and replaces, inserts or
//! removes a block while copying every byte outside it verbatim. It knows
//! nothing about Lua or hyprlang; the backends decide what goes inside and
//! check that a marker line is really a comment in their language.
//!
//! Line endings follow the file: the first line ending found decides between
//! `\n` and `\r\n` for new lines. A file without a final newline keeps it
//! missing, unless the block is inserted at the very end.

use std::ops::Range;

/// The begin and end marker lines of a managed block, without line endings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Markers {
    /// The line that opens the block.
    pub begin: &'static str,
    /// The line that closes the block.
    pub end: &'static str,
}

/// Markers for Lua files.
pub const LUA_MARKERS: Markers = Markers {
    begin: "-- BEGIN hyprtilt (managed)",
    end: "-- END hyprtilt",
};

/// Markers for hyprlang files.
pub const HYPRLANG_MARKERS: Markers = Markers {
    begin: "# BEGIN hyprtilt (managed)",
    end: "# END hyprtilt",
};

/// Where a block is in the source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockLocation {
    /// The whole block: from the start of the begin line to the end of the
    /// end line, including the end line's line ending if it has one.
    pub span: Range<usize>,
    /// The content between the two marker lines (starts after the begin
    /// line's line ending, ends at the start of the end line).
    pub body: Range<usize>,
    /// Byte range of the begin marker line, without its line ending.
    pub begin_marker: Range<usize>,
    /// Byte range of the end marker line, without its line ending.
    pub end_marker: Range<usize>,
    /// 1-based line number of the begin marker.
    pub begin_line: usize,
    /// 1-based line number of the end marker.
    pub end_line: usize,
}

/// Why the markers of a file cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlockError {
    /// A begin marker without an end marker after it.
    #[error("line {line}: managed block is not closed (no end marker)")]
    Unclosed {
        /// Line of the begin marker.
        line: usize,
    },
    /// An end marker without a begin marker before it.
    #[error("line {line}: end marker without a begin marker")]
    UnexpectedEnd {
        /// Line of the end marker.
        line: usize,
    },
    /// A second begin marker, inside or after the first block.
    #[error("line {line}: more than one managed block (the first begins on line {first})")]
    Duplicate {
        /// Line of the second begin marker.
        line: usize,
        /// Line of the first begin marker.
        first: usize,
    },
}

/// One line of the source with its byte ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// 1-based line number.
    pub number: usize,
    /// The line's content, without the line ending.
    pub content: Range<usize>,
    /// The content plus the line ending, if there is one.
    pub full: Range<usize>,
}

/// Split `src` into lines. `\n` and `\r\n` both end a line; a lone `\r`
/// does not (neither Lua nor hyprlang treat it as a line break in practice).
///
/// # Examples
///
/// ```
/// use hyprtilt_core::block::lines;
///
/// let l = lines("a\r\nb");
/// assert_eq!(l.len(), 2);
/// assert_eq!(l[0].content, 0..1);
/// assert_eq!(l[0].full, 0..3);
/// assert_eq!(l[1].full, 3..4);
/// ```
#[must_use]
pub fn lines(src: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = src.as_bytes();
    let mut number = 1;
    while start < bytes.len() {
        let (content_end, full_end) = match src[start..].find('\n') {
            Some(i) => {
                let nl = start + i;
                let content_end = if nl > start && bytes[nl - 1] == b'\r' {
                    nl - 1
                } else {
                    nl
                };
                (content_end, nl + 1)
            }
            None => (bytes.len(), bytes.len()),
        };
        out.push(Line {
            number,
            content: start..content_end,
            full: start..full_end,
        });
        number += 1;
        start = full_end;
    }
    out
}

/// The line ending used for new lines: the first one in the file, `\n` if
/// the file has none.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::block::line_ending;
///
/// assert_eq!(line_ending("a\r\nb\n"), "\r\n");
/// assert_eq!(line_ending("a\nb\r\n"), "\n");
/// assert_eq!(line_ending("single line"), "\n");
/// ```
#[must_use]
pub fn line_ending(src: &str) -> &'static str {
    match src.find('\n') {
        Some(i) if i > 0 && src.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

/// The 1-based line number of a byte offset.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::block::line_of;
///
/// assert_eq!(line_of("a\nb\nc", 0), 1);
/// assert_eq!(line_of("a\nb\nc", 2), 2);
/// assert_eq!(line_of("a\nb\nc", 4), 3);
/// ```
// Called once per diagnostic, so a byte loop is fine and saves a dependency.
#[allow(clippy::naive_bytecount)]
#[must_use]
pub fn line_of(src: &str, offset: usize) -> usize {
    src.as_bytes()[..offset.min(src.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

/// Find the managed block. A marker is a line whose content, without
/// surrounding whitespace, equals the marker text exactly.
///
/// # Errors
///
/// Returns [`BlockError`] for an unclosed block, an end marker without a
/// begin marker, or more than one block.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::block::{locate, LUA_MARKERS};
///
/// let src = "x = 1\n-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"DP-1\" })\n-- END hyprtilt\n";
/// let block = locate(src, &LUA_MARKERS).unwrap().unwrap();
/// assert_eq!(block.begin_line, 2);
/// assert_eq!(&src[block.body.clone()], "hl.monitor({ output = \"DP-1\" })\n");
/// assert_eq!(&src[..block.span.start], "x = 1\n");
/// ```
pub fn locate(src: &str, markers: &Markers) -> Result<Option<BlockLocation>, BlockError> {
    let mut begin: Option<Line> = None;
    let mut found: Option<BlockLocation> = None;
    for line in lines(src) {
        let text = src[line.content.clone()].trim();
        if text == markers.begin {
            if let Some(open) = &begin {
                return Err(BlockError::Duplicate {
                    line: line.number,
                    first: open.number,
                });
            }
            if let Some(done) = &found {
                return Err(BlockError::Duplicate {
                    line: line.number,
                    first: done.begin_line,
                });
            }
            begin = Some(line);
        } else if text == markers.end {
            let Some(open) = begin.take() else {
                return Err(BlockError::UnexpectedEnd { line: line.number });
            };
            found = Some(BlockLocation {
                span: open.full.start..line.full.end,
                body: open.full.end..line.full.start,
                begin_marker: open.content.clone(),
                end_marker: line.content.clone(),
                begin_line: open.number,
                end_line: line.number,
            });
        }
    }
    if let Some(open) = begin {
        return Err(BlockError::Unclosed { line: open.number });
    }
    Ok(found)
}

/// Replace the body of an existing block. `body` is a sequence of complete
/// lines (each ending with a line ending) or empty. The marker lines, their
/// indentation and everything outside the block stay as they are.
///
/// # Panics
///
/// Panics if `location` does not describe `src` (ranges out of bounds).
///
/// # Examples
///
/// ```
/// use hyprtilt_core::block::{locate, replace_body, LUA_MARKERS};
///
/// let src = "a\n-- BEGIN hyprtilt (managed)\nold\n-- END hyprtilt\nb\n";
/// let loc = locate(src, &LUA_MARKERS).unwrap().unwrap();
/// let out = replace_body(src, &loc, "new\n");
/// assert_eq!(out, "a\n-- BEGIN hyprtilt (managed)\nnew\n-- END hyprtilt\nb\n");
/// ```
#[must_use]
pub fn replace_body(src: &str, location: &BlockLocation, body: &str) -> String {
    let mut out = String::with_capacity(src.len() + body.len());
    out.push_str(&src[..location.body.start]);
    out.push_str(body);
    out.push_str(&src[location.body.end..]);
    out
}

/// Insert a new block at byte offset `at`, which must be at the start of a
/// line or at the end of the file. `body` is a sequence of complete lines
/// using `\n`; they are converted to the file's line ending.
///
/// At the end of a file whose last line has no line ending, one is added
/// before the block. A blank line separates the block from non-blank text
/// before and after it, so that the block stands out; existing blank lines
/// are reused rather than doubled.
///
/// # Panics
///
/// Panics if `at` is not at a line start or the end of `src`.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::block::{insert_block, LUA_MARKERS};
///
/// let src = "x = 1\nreturn {}\n";
/// let out = insert_block(src, 6, "hl.monitor({ output = \"DP-1\" })\n", &LUA_MARKERS);
/// assert_eq!(
///     out,
///     "x = 1\n\n-- BEGIN hyprtilt (managed)\nhl.monitor({ output = \"DP-1\" })\n-- END hyprtilt\n\nreturn {}\n"
/// );
/// ```
#[must_use]
pub fn insert_block(src: &str, at: usize, body: &str, markers: &Markers) -> String {
    insert(src, at, body, markers, true)
}

/// Like [`insert_block`], but without blank lines around the block: used
/// when the block takes the place of lines that were there before, such as
/// rules adopted into it, so that a comment above them stays attached.
///
/// # Panics
///
/// Panics if `at` is not at a line start or the end of `src`.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::block::{insert_block_in_place, LUA_MARKERS};
///
/// let out = insert_block_in_place("-- screens\nx = 1\n", 11, "r\n", &LUA_MARKERS);
/// assert_eq!(out, "-- screens\n-- BEGIN hyprtilt (managed)\nr\n-- END hyprtilt\nx = 1\n");
/// ```
#[must_use]
pub fn insert_block_in_place(src: &str, at: usize, body: &str, markers: &Markers) -> String {
    insert(src, at, body, markers, false)
}

fn insert(src: &str, at: usize, body: &str, markers: &Markers, separate: bool) -> String {
    assert!(
        at == src.len() || at == 0 || src.as_bytes()[at - 1] == b'\n',
        "insertion point {at} is not at a line start"
    );
    let eol = line_ending(src);
    let before = &src[..at];
    let after = &src[at..];

    let mut out = String::with_capacity(src.len() + body.len() + 64);
    out.push_str(before);
    if !before.is_empty() && !before.ends_with('\n') {
        out.push_str(eol);
    }
    if separate && !before.is_empty() && !ends_with_blank_line(before) {
        out.push_str(eol);
    }
    out.push_str(markers.begin);
    out.push_str(eol);
    for line in body.lines() {
        out.push_str(line);
        out.push_str(eol);
    }
    out.push_str(markers.end);
    out.push_str(eol);
    if separate && !after.is_empty() && !starts_with_blank_line(after) {
        out.push_str(eol);
    }
    out.push_str(after);
    out
}

/// Remove the two marker lines of a block and keep its body as ordinary
/// text (`unmanage`).
///
/// # Examples
///
/// ```
/// use hyprtilt_core::block::{locate, remove_markers, LUA_MARKERS};
///
/// let src = "a\n-- BEGIN hyprtilt (managed)\nrule\n-- END hyprtilt\nb\n";
/// let loc = locate(src, &LUA_MARKERS).unwrap().unwrap();
/// assert_eq!(remove_markers(src, &loc), "a\nrule\nb\n");
/// ```
#[must_use]
pub fn remove_markers(src: &str, location: &BlockLocation) -> String {
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..location.span.start]);
    out.push_str(&src[location.body.clone()]);
    out.push_str(&src[location.span.end..]);
    out
}

/// Remove the byte range `span` from `src`. Used to take rules out of the
/// file when they are adopted into the block.
///
/// # Examples
///
/// ```
/// use hyprtilt_core::block::remove_range;
///
/// assert_eq!(remove_range("abcdef", &(1..3)), "adef");
/// ```
#[must_use]
pub fn remove_range(src: &str, span: &Range<usize>) -> String {
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..span.start]);
    out.push_str(&src[span.end..]);
    out
}

/// Convert a body written with `\n` to the file's line ending.
#[must_use]
pub fn with_line_ending(body: &str, eol: &str) -> String {
    if eol == "\n" {
        return body.to_owned();
    }
    body.replace("\r\n", "\n").replace('\n', eol)
}

fn ends_with_blank_line(s: &str) -> bool {
    let trimmed = s.trim_end_matches(['\n', '\r']);
    if trimmed.is_empty() {
        return true;
    }
    let tail = &s[trimmed.len()..];
    // Two line endings at the end mean the last line is blank.
    tail.matches('\n').count() >= 2 || trimmed.ends_with('\n')
}

fn starts_with_blank_line(s: &str) -> bool {
    let first = s.split('\n').next().unwrap_or("");
    first.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Markers = LUA_MARKERS;

    fn block_src(before: &str, body: &str, after: &str) -> String {
        format!("{before}{}\n{body}{}\n{after}", L.begin, L.end)
    }

    #[test]
    fn lines_handles_crlf_and_missing_final_newline() {
        let l = lines("a\nb\r\nc");
        assert_eq!(l.len(), 3);
        assert_eq!(l[1].content, 2..3);
        assert_eq!(l[1].full, 2..5);
        assert_eq!(l[2].full, 5..6);
        assert!(lines("").is_empty());
        assert_eq!(lines("\n").len(), 1);
    }

    #[test]
    fn locate_none_and_indented_markers() {
        assert_eq!(locate("x = 1\n", &L).unwrap(), None);
        let src = "  -- BEGIN hyprtilt (managed)  \nbody\n\t-- END hyprtilt\n";
        let loc = locate(src, &L).unwrap().unwrap();
        assert_eq!(&src[loc.body.clone()], "body\n");
        assert_eq!(loc.span, 0..src.len());
        assert_eq!((loc.begin_line, loc.end_line), (1, 3));
    }

    #[test]
    fn locate_rejects_malformed_blocks() {
        let unclosed = format!("{}\nx\n", L.begin);
        assert_eq!(locate(&unclosed, &L), Err(BlockError::Unclosed { line: 1 }));
        let stray_end = format!("x\n{}\n", L.end);
        assert_eq!(
            locate(&stray_end, &L),
            Err(BlockError::UnexpectedEnd { line: 2 })
        );
        let nested = format!("{b}\n{b}\n{e}\n{e}\n", b = L.begin, e = L.end);
        assert_eq!(
            locate(&nested, &L),
            Err(BlockError::Duplicate { line: 2, first: 1 })
        );
        let two = format!("{b}\n{e}\n{b}\n{e}\n", b = L.begin, e = L.end);
        assert_eq!(
            locate(&two, &L),
            Err(BlockError::Duplicate { line: 3, first: 1 })
        );
        assert_eq!(
            BlockError::Duplicate { line: 3, first: 1 }.to_string(),
            "line 3: more than one managed block (the first begins on line 1)"
        );
    }

    #[test]
    fn markers_must_match_the_whole_line() {
        let src = "-- BEGIN hyprtilt (managed) extra\n-- END hyprtilt!\n";
        assert_eq!(locate(src, &L).unwrap(), None);
        // hyprlang markers are not Lua markers.
        let h = format!("{}\n{}\n", HYPRLANG_MARKERS.begin, HYPRLANG_MARKERS.end);
        assert_eq!(locate(&h, &L).unwrap(), None);
        assert!(locate(&h, &HYPRLANG_MARKERS).unwrap().is_some());
    }

    #[test]
    fn replace_body_preserves_everything_outside() {
        let before = "-- mine\r\nlocal x = 1\n\n";
        let after = "\nreturn {\n  a = 1,\n}";
        let src = block_src(before, "old 1\nold 2\n", after);
        let loc = locate(&src, &L).unwrap().unwrap();
        let out = replace_body(&src, &loc, "new\n");
        assert!(out.starts_with(before));
        assert!(out.ends_with(after));
        let loc2 = locate(&out, &L).unwrap().unwrap();
        assert_eq!(&out[loc2.body.clone()], "new\n");
        assert_eq!(&out[..loc2.span.start], &src[..loc.span.start]);
        assert_eq!(&out[loc2.span.end..], &src[loc.span.end..]);
    }

    #[test]
    fn replace_body_with_empty_body() {
        let src = block_src("", "x\n", "");
        let loc = locate(&src, &L).unwrap().unwrap();
        assert_eq!(
            replace_body(&src, &loc, ""),
            format!("{}\n{}\n", L.begin, L.end)
        );
    }

    #[test]
    fn end_marker_on_last_line_without_newline() {
        let src = format!("{}\nx\n{}", L.begin, L.end);
        let loc = locate(&src, &L).unwrap().unwrap();
        assert_eq!(loc.span, 0..src.len());
        assert_eq!(
            replace_body(&src, &loc, "y\n"),
            format!("{}\ny\n{}", L.begin, L.end)
        );
    }

    #[test]
    fn insert_in_empty_file() {
        assert_eq!(
            insert_block("", 0, "r\n", &L),
            format!("{}\nr\n{}\n", L.begin, L.end)
        );
    }

    #[test]
    fn insert_at_end_without_final_newline() {
        let out = insert_block("x = 1", 5, "r\n", &L);
        assert_eq!(out, format!("x = 1\n\n{}\nr\n{}\n", L.begin, L.end));
    }

    #[test]
    fn insert_reuses_existing_blank_lines() {
        let out = insert_block("x = 1\n\nreturn {}\n", 7, "r\n", &L);
        assert_eq!(
            out,
            format!("x = 1\n\n{}\nr\n{}\n\nreturn {{}}\n", L.begin, L.end)
        );
        let out = insert_block("x = 1\n\n", 7, "r\n", &L);
        assert_eq!(out, format!("x = 1\n\n{}\nr\n{}\n", L.begin, L.end));
    }

    #[test]
    fn insert_uses_crlf_when_the_file_does() {
        let out = insert_block("x = 1\r\nreturn {}\r\n", 7, "a\nb\n", &L);
        assert_eq!(
            out,
            format!(
                "x = 1\r\n\r\n{}\r\na\r\nb\r\n{}\r\n\r\nreturn {{}}\r\n",
                L.begin, L.end
            )
        );
        assert_eq!(locate(&out, &L).unwrap().unwrap().begin_line, 3);
    }

    #[test]
    fn insert_at_start_of_file() {
        let out = insert_block("x = 1\n", 0, "r\n", &L);
        assert_eq!(out, format!("{}\nr\n{}\n\nx = 1\n", L.begin, L.end));
    }

    #[test]
    #[should_panic(expected = "not at a line start")]
    fn insert_in_the_middle_of_a_line_panics() {
        let _ = insert_block("abc\n", 1, "r\n", &L);
    }

    #[test]
    fn remove_markers_keeps_body_and_surroundings() {
        let src = block_src("a\n", "r1\nr2\n", "b\n");
        let loc = locate(&src, &L).unwrap().unwrap();
        assert_eq!(remove_markers(&src, &loc), "a\nr1\nr2\nb\n");
        let src = format!("{}\nr\n{}", L.begin, L.end);
        let loc = locate(&src, &L).unwrap().unwrap();
        assert_eq!(remove_markers(&src, &loc), "r\n");
    }

    #[test]
    fn line_helpers() {
        assert_eq!(line_of("", 0), 1);
        assert_eq!(line_of("a\nb", 99), 2);
        assert_eq!(with_line_ending("a\nb\n", "\r\n"), "a\r\nb\r\n");
        assert_eq!(with_line_ending("a\r\nb\n", "\r\n"), "a\r\nb\r\n");
        assert_eq!(with_line_ending("a\n", "\n"), "a\n");
        assert_eq!(remove_range("hello", &(0..5)), "");
    }
}
