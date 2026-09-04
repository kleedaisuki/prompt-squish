//! XML-aware whitespace normalization for `xmlsquish`.
//!
//! This crate deliberately implements a lexical finite-state machine instead of
//! parsing XML into a tree. Markup is copied byte-for-byte; only XML whitespace
//! between lexical atoms is normalized.

use std::error::Error;
use std::fmt;

/// Statistics for whitespace handled by [`squish`].
///
/// All counters are Unicode scalar-value counts. XML whitespace is ASCII, so
/// every recognized input whitespace character is also exactly one byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WhitespaceStats {
    /// XML whitespace characters found anywhere in the input.
    pub recognized: usize,
    /// Excess input whitespace characters eliminated from the character count.
    ///
    /// A one-character separator such as a tab is canonicalized to a space but
    /// is not counted as removed because its position is reused.
    pub removed: usize,
    /// ASCII spaces added where adjacent atoms had no input separator to reuse.
    pub inserted: usize,
}

/// Successful squishing result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquishOutput {
    pub output: String,
    pub stats: WhitespaceStats,
}

/// The kind of markup construct which reached end-of-input before closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SquishErrorKind {
    Tag,
    Comment,
    Cdata,
    ProcessingInstruction,
    Doctype,
}

impl SquishErrorKind {
    const fn description(self) -> &'static str {
        match self {
            Self::Tag => "tag",
            Self::Comment => "comment",
            Self::Cdata => "CDATA section",
            Self::ProcessingInstruction => "processing instruction",
            Self::Doctype => "DOCTYPE declaration",
        }
    }
}

/// A lexical error found while scanning markup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquishError {
    /// The unterminated construct.
    pub kind: SquishErrorKind,
    /// Byte offset of the construct's opening `<` in the original input.
    pub offset: usize,
}

impl fmt::Display for SquishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unterminated {} at byte offset {}",
            self.kind.description(),
            self.offset
        )
    }
}

impl Error for SquishError {}

/// Normalize XML whitespace between lexical atoms.
///
/// An atom is either a maximal run of non-XML-whitespace text or one complete
/// markup construct. Markup bytes (including whitespace inside markup) are
/// preserved exactly. Leading and trailing XML whitespace is removed, and every
/// adjacent atom pair is separated by exactly one ASCII space.
pub fn squish(input: &str) -> Result<SquishOutput, SquishError> {
    Scanner::new(input).run()
}

struct Scanner<'a> {
    input: &'a str,
    cursor: usize,
    output: String,
    atoms: usize,
    recognized: usize,
    removed: usize,
    inserted: usize,
    pending_whitespace: usize,
}

impl<'a> Scanner<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            cursor: 0,
            output: String::with_capacity(input.len()),
            atoms: 0,
            recognized: 0,
            removed: 0,
            inserted: 0,
            pending_whitespace: 0,
        }
    }

    fn run(mut self) -> Result<SquishOutput, SquishError> {
        while self.cursor < self.input.len() {
            if is_xml_space(self.byte(self.cursor)) {
                self.consume_whitespace();
                continue;
            }

            let start = self.cursor;
            let end = if self.byte(start) == b'<' {
                scan_markup(self.input.as_bytes(), start)?
            } else {
                self.scan_text()
            };
            self.push_atom(start, end);
            self.cursor = end;
        }

        // No atom follows the final run, so every trailing character is removed.
        self.removed += self.pending_whitespace;
        Ok(SquishOutput {
            output: self.output,
            stats: WhitespaceStats {
                recognized: self.recognized,
                removed: self.removed,
                inserted: self.inserted,
            },
        })
    }

    fn byte(&self, offset: usize) -> u8 {
        self.input.as_bytes()[offset]
    }

    fn consume_whitespace(&mut self) {
        while self.cursor < self.input.len() && is_xml_space(self.byte(self.cursor)) {
            self.recognized += 1;
            self.pending_whitespace += 1;
            self.cursor += 1;
        }
    }

    fn scan_text(&self) -> usize {
        let bytes = self.input.as_bytes();
        let mut end = self.cursor;
        while end < bytes.len() && bytes[end] != b'<' && !is_xml_space(bytes[end]) {
            end += 1;
        }
        end
    }

    fn push_atom(&mut self, start: usize, end: usize) {
        self.recognized += self.input.as_bytes()[start..end]
            .iter()
            .filter(|&&byte| is_xml_space(byte))
            .count();

        if self.atoms == 0 {
            self.removed += self.pending_whitespace;
        } else if self.pending_whitespace == 0 {
            self.output.push(' ');
            self.inserted += 1;
        } else {
            // Reuse one character's place, canonicalizing it to an ASCII space.
            self.output.push(' ');
            self.removed += self.pending_whitespace - 1;
        }
        self.pending_whitespace = 0;
        self.output.push_str(&self.input[start..end]);
        self.atoms += 1;
    }
}

const fn is_xml_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn scan_markup(bytes: &[u8], start: usize) -> Result<usize, SquishError> {
    debug_assert_eq!(bytes[start], b'<');

    if starts_with(bytes, start, b"<!--") {
        scan_delimited(bytes, start, start + 4, b"-->", SquishErrorKind::Comment)
    } else if starts_with(bytes, start, b"<![CDATA[") {
        scan_delimited(bytes, start, start + 9, b"]]>", SquishErrorKind::Cdata)
    } else if starts_with(bytes, start, b"<?") {
        scan_delimited(
            bytes,
            start,
            start + 2,
            b"?>",
            SquishErrorKind::ProcessingInstruction,
        )
    } else if is_doctype_start(bytes, start) {
        scan_doctype(bytes, start)
    } else {
        scan_tag(bytes, start)
    }
}

fn starts_with(bytes: &[u8], offset: usize, prefix: &[u8]) -> bool {
    bytes.get(offset..offset.saturating_add(prefix.len())) == Some(prefix)
}

fn is_doctype_start(bytes: &[u8], start: usize) -> bool {
    const PREFIX: &[u8] = b"<!DOCTYPE";
    if !starts_with(bytes, start, PREFIX) {
        return false;
    }
    matches!(
        bytes.get(start + PREFIX.len()),
        None | Some(b' ' | b'\t' | b'\r' | b'\n' | b'[' | b'>')
    )
}

fn scan_delimited(
    bytes: &[u8],
    construct_start: usize,
    mut cursor: usize,
    delimiter: &[u8],
    kind: SquishErrorKind,
) -> Result<usize, SquishError> {
    while cursor < bytes.len() {
        if starts_with(bytes, cursor, delimiter) {
            return Ok(cursor + delimiter.len());
        }
        cursor += 1;
    }
    Err(SquishError {
        kind,
        offset: construct_start,
    })
}

fn scan_tag(bytes: &[u8], start: usize) -> Result<usize, SquishError> {
    #[derive(Clone, Copy)]
    enum State {
        Content,
        SingleQuoted,
        DoubleQuoted,
    }

    let mut state = State::Content;
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        state = match (state, bytes[cursor]) {
            (State::Content, b'>') => return Ok(cursor + 1),
            (State::Content, b'\'') => State::SingleQuoted,
            (State::Content, b'"') => State::DoubleQuoted,
            (State::SingleQuoted, b'\'') | (State::DoubleQuoted, b'"') => State::Content,
            (state, _) => state,
        };
        cursor += 1;
    }
    Err(SquishError {
        kind: SquishErrorKind::Tag,
        offset: start,
    })
}

fn scan_doctype(bytes: &[u8], start: usize) -> Result<usize, SquishError> {
    #[derive(Clone, Copy)]
    enum State {
        Content,
        SingleQuoted,
        DoubleQuoted,
    }

    let mut state = State::Content;
    let mut bracket_depth = 0usize;
    let mut cursor = start + b"<!DOCTYPE".len();

    while cursor < bytes.len() {
        state = match (state, bytes[cursor]) {
            (State::SingleQuoted, b'\'') | (State::DoubleQuoted, b'"') => State::Content,
            (State::SingleQuoted, _) => State::SingleQuoted,
            (State::DoubleQuoted, _) => State::DoubleQuoted,
            (State::Content, b'\'') => State::SingleQuoted,
            (State::Content, b'"') => State::DoubleQuoted,
            (State::Content, _) if starts_with(bytes, cursor, b"<!--") => {
                cursor =
                    scan_delimited(bytes, cursor, cursor + 4, b"-->", SquishErrorKind::Comment)?;
                continue;
            }
            (State::Content, _) if starts_with(bytes, cursor, b"<?") => {
                cursor = scan_delimited(
                    bytes,
                    cursor,
                    cursor + 2,
                    b"?>",
                    SquishErrorKind::ProcessingInstruction,
                )?;
                continue;
            }
            (State::Content, b'[') => {
                bracket_depth += 1;
                State::Content
            }
            (State::Content, b']') => {
                bracket_depth = bracket_depth.saturating_sub(1);
                State::Content
            }
            (State::Content, b'>') if bracket_depth == 0 => return Ok(cursor + 1),
            (State::Content, _) => State::Content,
        };
        cursor += 1;
    }

    Err(SquishError {
        kind: SquishErrorKind::Doctype,
        offset: start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(input: &str) -> SquishOutput {
        squish(input).expect("input should be lexically complete")
    }

    #[test]
    fn empty_and_whitespace_only_inputs_become_empty() {
        assert_eq!(
            ok(""),
            SquishOutput {
                output: String::new(),
                stats: WhitespaceStats::default()
            }
        );
        assert_eq!(
            ok(" \t\r\n"),
            SquishOutput {
                output: String::new(),
                stats: WhitespaceStats {
                    recognized: 4,
                    removed: 4,
                    inserted: 0
                },
            }
        );
    }

    #[test]
    fn joins_every_atom_pair_with_one_ascii_space() {
        let result = ok(" \n<root>\t hello\r\nworld </root>  ");
        assert_eq!(result.output, "<root> hello world </root>");
        assert_eq!(
            result.stats,
            WhitespaceStats {
                recognized: 9,
                removed: 6,
                inserted: 0
            }
        );
    }

    #[test]
    fn adjacent_markup_and_text_are_still_separated() {
        let result = ok("<a><b>x</b></a>");
        assert_eq!(result.output, "<a> <b> x </b> </a>");
        assert_eq!(
            result.stats,
            WhitespaceStats {
                recognized: 0,
                removed: 0,
                inserted: 4
            }
        );
    }

    #[test]
    fn preserves_markup_interior_byte_for_byte() {
        let input = "<node  a = \"x > y\"\n b='z'>value</node   >";
        assert_eq!(
            ok(input).output,
            "<node  a = \"x > y\"\n b='z'> value </node   >"
        );
    }

    #[test]
    fn recognizes_only_xml_s_as_whitespace() {
        let result = ok("a\u{00a0}b\u{2003}c d");
        assert_eq!(result.output, "a\u{00a0}b\u{2003}c d");
        assert_eq!(
            result.stats,
            WhitespaceStats {
                recognized: 1,
                removed: 0,
                inserted: 0
            }
        );
    }

    #[test]
    fn copies_comments_cdata_and_processing_instructions_as_atoms() {
        let input = "<!-- a > b --> <![CDATA[ <x>  ]]><?pi a > b?>";
        let result = ok(input);
        assert_eq!(
            result.output,
            "<!-- a > b --> <![CDATA[ <x>  ]]> <?pi a > b?>"
        );
        assert_eq!(
            result.stats,
            WhitespaceStats {
                recognized: input
                    .as_bytes()
                    .iter()
                    .filter(|&&byte| is_xml_space(byte))
                    .count(),
                removed: 0,
                inserted: 1
            }
        );
    }

    #[test]
    fn doctype_handles_quotes_subset_depth_comments_and_pi() {
        let input = "<!DOCTYPE root [\n<!ENTITY gt '>'>\n<!-- ] > -->\n<?inside ] ?>\n<!ELEMENT root (#PCDATA)>\n]>\n<root>x</root>";
        let result = ok(input);
        let split = input.find("\n<root>").unwrap();
        assert_eq!(
            result.output,
            format!("{} <root> x </root>", &input[..split])
        );
        assert_eq!(
            result.stats.recognized,
            input
                .as_bytes()
                .iter()
                .filter(|&&byte| is_xml_space(byte))
                .count()
        );
        assert_eq!(result.stats.removed, 0);
        assert_eq!(result.stats.inserted, 2);
    }

    #[test]
    fn doctype_keyword_requires_a_boundary() {
        assert_eq!(ok("<!DOCTYPEfoo>bar").output, "<!DOCTYPEfoo> bar");
        assert_eq!(ok("<!DOCTYPE><r/>").output, "<!DOCTYPE> <r/>");
        assert_eq!(
            squish("<!DOCTYPE"),
            Err(SquishError {
                kind: SquishErrorKind::Doctype,
                offset: 0
            })
        );
    }

    #[test]
    fn unicode_text_is_sliced_on_utf8_boundaries_and_stats_are_char_counts() {
        let input = "  猫\t娘 <萌>✨</萌> ";
        let result = ok(input);
        assert_eq!(result.output, "猫 娘 <萌> ✨ </萌>");
        assert_eq!(
            result.stats,
            WhitespaceStats {
                recognized: 5,
                removed: 3,
                inserted: 2
            }
        );
        assert_eq!(
            result.output.chars().count(),
            input.chars().count() - result.stats.removed + result.stats.inserted
        );
    }

    #[test]
    fn reports_each_unterminated_top_level_construct_at_its_byte_offset() {
        let cases = [
            ("猫 <!--no", SquishErrorKind::Comment, 4),
            ("x <![CDATA[no", SquishErrorKind::Cdata, 2),
            ("x <?no", SquishErrorKind::ProcessingInstruction, 2),
            ("x <!DOCTYPE root [", SquishErrorKind::Doctype, 2),
            ("x <tag attr='>'", SquishErrorKind::Tag, 2),
        ];
        for (input, kind, offset) in cases {
            assert_eq!(
                squish(input),
                Err(SquishError { kind, offset }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn reports_unterminated_nested_doctype_construct_at_nested_offset() {
        let comment = "<!DOCTYPE r [<!-- nope";
        assert_eq!(
            squish(comment),
            Err(SquishError {
                kind: SquishErrorKind::Comment,
                offset: 13
            })
        );
        let pi = "<!DOCTYPE r [<?nope";
        assert_eq!(
            squish(pi),
            Err(SquishError {
                kind: SquishErrorKind::ProcessingInstruction,
                offset: 13
            })
        );
    }

    #[test]
    fn output_character_accounting_identity_always_holds_for_successes() {
        for input in [
            "",
            " ",
            "abc",
            "a b",
            "<a/>",
            "<a>x</a>",
            "\n<a  x=' '>\t猫\r</a>\n",
        ] {
            let result = ok(input);
            assert_eq!(
                result.output.chars().count(),
                input.chars().count() - result.stats.removed + result.stats.inserted,
                "{input:?}"
            );
        }
    }

    #[test]
    fn distinguishes_reused_removed_and_inserted_whitespace() {
        assert_eq!(
            ok("a b").stats,
            WhitespaceStats {
                recognized: 1,
                removed: 0,
                inserted: 0
            }
        );
        let tab = ok("a\tb");
        assert_eq!(tab.output, "a b");
        assert_eq!(
            tab.stats,
            WhitespaceStats {
                recognized: 1,
                removed: 0,
                inserted: 0
            }
        );
        assert_eq!(
            ok("a  b").stats,
            WhitespaceStats {
                recognized: 2,
                removed: 1,
                inserted: 0
            }
        );
        assert_eq!(
            ok("<a><b>").stats,
            WhitespaceStats {
                recognized: 0,
                removed: 0,
                inserted: 1
            }
        );
    }

    #[test]
    fn protected_markup_whitespace_is_recognized_but_not_removed() {
        let input = "<a  x=' \t'>";
        let result = ok(input);
        assert_eq!(result.output, input);
        assert_eq!(
            result.stats,
            WhitespaceStats {
                recognized: 4,
                removed: 0,
                inserted: 0
            }
        );
    }
}
