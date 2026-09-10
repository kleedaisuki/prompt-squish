//! Lexical XML whitespace normalization without tree allocation.
//! 不构建树的 XML 词法空白规范化；保留标记字节。

use std::error::Error;
use std::fmt;

/// Statistics for whitespace handled by [`squish`].
/// 空白转换统计，计数单位为 Unicode 标量值。
///
/// All counters are Unicode scalar-value counts. XML whitespace is ASCII, so
/// every recognized input whitespace character is also exactly one byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WhitespaceStats {
    /// XML whitespace characters found anywhere in the input.
    /// 输入中所有 XML 空白字符数。
    pub recognized: usize,
    /// Excess input whitespace characters eliminated from the character count.
    /// 从字符总数中移除的多余输入空白。
    ///
    /// A one-character separator such as a tab is canonicalized to a space but
    /// is not counted as removed because its position is reused.
    pub removed: usize,
    /// ASCII spaces added where adjacent atoms had no input separator to reuse.
    /// 相邻原子没有可复用分隔符时补入的 ASCII 空格数。
    pub inserted: usize,
}

/// Successful squishing result. / 成功的空白转换结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquishOutput {
    /// Normalized text. / 规范化后的文本。
    pub output: String,
    /// Whitespace accounting for this transformation. / 本次转换的空白统计。
    pub stats: WhitespaceStats,
}

/// The kind of markup construct which reached end-of-input before closing.
/// 到达输入结尾时尚未闭合的标记类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SquishErrorKind {
    /// Element tag / 元素标签。
    Tag,
    /// XML comment / XML 注释。
    Comment,
    /// CDATA section / CDATA 节。
    Cdata,
    /// Processing instruction / 处理指令。
    ProcessingInstruction,
    /// Document type declaration / 文档类型声明。
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

/// A lexical error found while scanning markup. / 扫描标记时发现的词法错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SquishError {
    /// The unterminated construct. / 未闭合的标记类型。
    pub kind: SquishErrorKind,
    /// Byte offset of the construct's opening `<` in the original input.
    /// 起始 `<` 在原输入中的字节偏移。
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
/// 规范化词法原子之间的 XML 空白；标记字节不变，首尾空白移除。
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
#[path = "squish.test.rs"]
mod tests;
