use std::{fmt, ops::Range};

/// 源文件中的半开字节区间。 / A half-open byte range in the source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    /// 起始字节偏移。 / Inclusive byte offset.
    pub start: usize,
    /// 结束字节偏移。 / Exclusive byte offset.
    pub end: usize,
}

impl Span {
    pub(crate) const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    /// 转换为标准库区间。 / Converts to a standard range.
    pub const fn range(self) -> Range<usize> {
        self.start..self.end
    }
}

/// 无损词法记号类别。 / Kind of a lossless lexical token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    /// UTF-8 BOM。 / UTF-8 byte-order mark.
    Bom,
    /// 元素开始或空元素标签。 / Start or empty-element tag.
    StartTag,
    /// 元素结束标签。 / End tag.
    EndTag,
    /// 字符数据，包括纯空白。 / Character data, including whitespace-only data.
    CharacterData,
    /// XML 注释。 / XML comment.
    Comment,
    /// CDATA 节。 / CDATA section.
    Cdata,
    /// 处理指令，包括 XML 声明。 / Processing instruction, including the XML declaration.
    ProcessingInstruction,
    /// 文档类型声明。 / Document type declaration.
    Doctype,
}

/// 引用原始字节而不复制的词法记号。 / A lexical token referencing source bytes without copying.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    /// 记号类别。 / Token kind.
    pub kind: TokenKind,
    /// 记号的完整原始区间。 / Full raw token span.
    pub span: Span,
}

/// 稳定的格式化诊断代码。 / Stable formatting diagnostic code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticCode {
    /// 输入不是有效 UTF-8。 / Input is not valid UTF-8.
    InvalidUtf8,
    /// XML 构造未闭合或词法无效。 / An XML construct is unterminated or lexically invalid.
    InvalidSyntax,
    /// 元素闭合次序无效。 / Element closing order is invalid.
    UnbalancedElement,
    /// 文档没有且仅有一个根元素。 / Document does not have exactly one root element.
    InvalidDocument,
}

/// 带源码范围的结构化诊断。 / A structured diagnostic with a source span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    /// 稳定代码。 / Stable code.
    pub code: DiagnosticCode,
    /// 主要源码范围。 / Primary source span.
    pub span: Span,
    /// 面向人的英文消息；渲染和本地化由上层负责。 / Human English message; rendering/localization belongs upstream.
    pub message: &'static str,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at bytes {}..{}",
            self.message, self.span.start, self.span.end
        )
    }
}
impl std::error::Error for Diagnostic {}

#[derive(Clone, Debug)]
pub(crate) struct TagLayout {
    pub trivia: Vec<Span>,
    pub separators: Vec<bool>,
}

/// 保存精确源字节、记号和标签布局的无损 XML tape/CST。
/// / A lossless XML tape/CST retaining exact source bytes, tokens, and tag layouts.
#[derive(Clone, Debug)]
pub struct LosslessXml<'a> {
    source: &'a [u8],
    tokens: Vec<Token>,
    pub(crate) tags: Vec<TagLayout>,
    bom: bool,
    line_ending: Option<&'static str>,
}

impl<'a> LosslessXml<'a> {
    /// 迭代扫描完整 XML 文档。元素深度仅消耗堆内存，不递归调用 Rust 栈。
    /// / Iteratively scans a complete XML document. Element depth uses heap memory, not recursion.
    pub fn parse(source: &'a [u8]) -> Result<Self, Diagnostic> {
        Scanner::new(source)?.scan()
    }
    /// 返回原始文件字节。 / Returns the exact original file bytes.
    pub const fn source(&self) -> &'a [u8] {
        self.source
    }
    /// 返回词法记号。 / Returns lexical tokens.
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }
    /// 返回记号的原始字节。 / Returns a token's exact raw bytes.
    pub fn raw(&self, token: &Token) -> &'a [u8] {
        &self.source[token.span.range()]
    }
    /// 输入是否包含 UTF-8 BOM。 / Whether the input has a UTF-8 BOM.
    pub const fn has_bom(&self) -> bool {
        self.bom
    }
    /// 返回首次发现的 `LF`、`CRLF` 或裸 `CR` 换行风格。
    /// / Returns the first observed `LF`, `CRLF`, or bare `CR` line-ending style.
    pub const fn line_ending(&self) -> Option<&'static str> {
        self.line_ending
    }
}

struct Scanner<'a> {
    source: &'a [u8],
    pos: usize,
    tokens: Vec<Token>,
    tags: Vec<TagLayout>,
    stack: Vec<Range<usize>>,
    roots: usize,
    seen_doctype: bool,
    bom: bool,
    line_ending: Option<&'static str>,
}

impl<'a> Scanner<'a> {
    fn new(source: &'a [u8]) -> Result<Self, Diagnostic> {
        let bom = source.starts_with(&[0xef, 0xbb, 0xbf]);
        let offset = if bom { 3 } else { 0 };
        if let Err(error) = std::str::from_utf8(&source[offset..]) {
            let start = offset + error.valid_up_to();
            return Err(diag(
                DiagnosticCode::InvalidUtf8,
                start,
                (start + 1).min(source.len()),
                "source is not valid UTF-8",
            ));
        }
        if let Some((relative, character)) = std::str::from_utf8(&source[offset..])
            .expect("UTF-8 was checked above")
            .char_indices()
            .find(|(_, character)| !is_xml_char(*character))
        {
            let start = offset + relative;
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                start,
                start + character.len_utf8(),
                "source contains a character forbidden by XML 1.0",
            ));
        }
        let line_ending = source
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .map(|index| match source[index] {
                b'\r' if source.get(index + 1) == Some(&b'\n') => "\r\n",
                b'\r' => "\r",
                _ => "\n",
            });
        Ok(Self {
            source,
            pos: offset,
            tokens: Vec::new(),
            tags: Vec::new(),
            stack: Vec::new(),
            roots: 0,
            seen_doctype: false,
            bom,
            line_ending,
        })
    }

    fn scan(mut self) -> Result<LosslessXml<'a>, Diagnostic> {
        if self.bom {
            self.tokens.push(Token {
                kind: TokenKind::Bom,
                span: Span::new(0, 3),
            });
        }
        while self.pos < self.source.len() {
            if self.source[self.pos] != b'<' {
                self.text()?;
                continue;
            }
            if self.at(b"<!--") {
                self.delimited(TokenKind::Comment, b"-->", true)?;
            } else if self.at(b"<![CDATA[") {
                if self.stack.is_empty() {
                    return Err(diag(
                        DiagnosticCode::InvalidDocument,
                        self.pos,
                        (self.pos + 9).min(self.source.len()),
                        "CDATA is not allowed outside the root element",
                    ));
                }
                self.delimited(TokenKind::Cdata, b"]]>", false)?;
            } else if self.at(b"<?") {
                self.delimited(TokenKind::ProcessingInstruction, b"?>", false)?;
            } else if self.is_doctype() {
                if self.seen_doctype || self.roots != 0 {
                    return Err(diag(
                        DiagnosticCode::InvalidDocument,
                        self.pos,
                        (self.pos + 9).min(self.source.len()),
                        "document type declaration must occur once before the root element",
                    ));
                }
                self.seen_doctype = true;
                self.doctype()?;
            } else if self.at(b"</") {
                self.end_tag()?;
            } else {
                self.start_tag()?;
            }
        }
        if let Some(name) = self.stack.last() {
            return Err(diag(
                DiagnosticCode::UnbalancedElement,
                name.start,
                name.end,
                "element is not closed",
            ));
        }
        if self.roots != 1 {
            return Err(diag(
                DiagnosticCode::InvalidDocument,
                0,
                self.source.len(),
                "XML document must contain exactly one root element",
            ));
        }
        Ok(LosslessXml {
            source: self.source,
            tokens: self.tokens,
            tags: self.tags,
            bom: self.bom,
            line_ending: self.line_ending,
        })
    }

    fn text(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        while self.pos < self.source.len() && self.source[self.pos] != b'<' {
            self.pos += 1;
        }
        let raw = &self.source[start..self.pos];
        if raw.windows(3).any(|x| x == b"]]>") || !valid_refs(raw) {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                start,
                self.pos,
                "invalid character data or entity reference",
            ));
        }
        if self.stack.is_empty() && raw.iter().any(|b| !is_space(*b)) {
            return Err(diag(
                DiagnosticCode::InvalidDocument,
                start,
                self.pos,
                "character data is not allowed outside the root element",
            ));
        }
        self.tokens.push(Token {
            kind: TokenKind::CharacterData,
            span: Span::new(start, self.pos),
        });
        Ok(())
    }

    fn delimited(
        &mut self,
        kind: TokenKind,
        close: &[u8],
        comment: bool,
    ) -> Result<(), Diagnostic> {
        let start = self.pos;
        let content_start = match kind {
            TokenKind::Comment => start + 4,
            TokenKind::Cdata => start + 9,
            _ => start + 2,
        };
        let Some(relative) = find(&self.source[content_start..], close) else {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                start,
                self.source.len(),
                "unterminated XML construct",
            ));
        };
        let content_end = content_start + relative;
        if comment
            && self.source[content_start..content_end]
                .windows(2)
                .any(|x| x == b"--")
        {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                content_start,
                content_end,
                "XML comment contains '--'",
            ));
        }
        if comment && self.source[content_start..content_end].ends_with(b"-") {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                content_start,
                content_end,
                "XML comment content must not end with '-'",
            ));
        }
        if kind == TokenKind::ProcessingInstruction {
            self.validate_processing_instruction(start, content_start, content_end)?;
        }
        self.pos = content_end + close.len();
        self.tokens.push(Token {
            kind,
            span: Span::new(start, self.pos),
        });
        Ok(())
    }

    fn doctype(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        let mut quote = None;
        let mut brackets = 0usize;
        self.pos += 9;
        self.spaces();
        self.name()?;
        while self.pos < self.source.len() {
            let byte = self.source[self.pos];
            if let Some(q) = quote {
                if byte == q {
                    quote = None;
                }
            } else {
                match byte {
                    b'\'' | b'"' => quote = Some(byte),
                    b'[' => brackets += 1,
                    b']' => brackets = brackets.saturating_sub(1),
                    b'>' if brackets == 0 => {
                        self.pos += 1;
                        self.tokens.push(Token {
                            kind: TokenKind::Doctype,
                            span: Span::new(start, self.pos),
                        });
                        return Ok(());
                    }
                    _ => {}
                }
            }
            self.pos += 1;
        }
        Err(diag(
            DiagnosticCode::InvalidSyntax,
            start,
            self.pos,
            "unterminated document type declaration",
        ))
    }

    fn start_tag(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        self.pos += 1;
        let name = self.name()?;
        if self.stack.is_empty() {
            self.roots += 1;
            if self.roots > 1 {
                return Err(diag(
                    DiagnosticCode::InvalidDocument,
                    start,
                    self.pos,
                    "multiple root elements",
                ));
            }
        }
        let mut trivia = Vec::new();
        let mut separators = Vec::new();
        let mut attrs: Vec<Range<usize>> = Vec::new();
        let empty;
        loop {
            let ws = self.spaces();
            if ws.start != ws.end {
                trivia.push(Span::new(ws.start, ws.end));
                separators.push(!(self.at(b"/>") || self.at(b">")));
            }
            if self.at(b"/>") {
                self.pos += 2;
                empty = true;
                break;
            }
            if self.at(b">") {
                self.pos += 1;
                empty = false;
                break;
            }
            if ws.start == ws.end {
                return Err(diag(
                    DiagnosticCode::InvalidSyntax,
                    self.pos,
                    (self.pos + 1).min(self.source.len()),
                    "attribute must be separated by whitespace",
                ));
            }
            let attr = self.name()?;
            if attrs
                .iter()
                .any(|old| self.source[old.clone()] == self.source[attr.clone()])
            {
                return Err(diag(
                    DiagnosticCode::InvalidSyntax,
                    attr.start,
                    attr.end,
                    "duplicate attribute name",
                ));
            }
            attrs.push(attr);
            let pre_eq = self.spaces();
            if pre_eq.start != pre_eq.end {
                trivia.push(Span::new(pre_eq.start, pre_eq.end));
                separators.push(false);
            }
            if !self.at(b"=") {
                return Err(diag(
                    DiagnosticCode::InvalidSyntax,
                    self.pos,
                    (self.pos + 1).min(self.source.len()),
                    "attribute is missing '='",
                ));
            }
            self.pos += 1;
            let post_eq = self.spaces();
            if post_eq.start != post_eq.end {
                trivia.push(Span::new(post_eq.start, post_eq.end));
                separators.push(false);
            }
            self.attribute_value()?;
        }
        self.tokens.push(Token {
            kind: TokenKind::StartTag,
            span: Span::new(start, self.pos),
        });
        self.tags.push(TagLayout { trivia, separators });
        if !empty {
            self.stack.push(name);
        }
        Ok(())
    }

    fn end_tag(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        self.pos += 2;
        let name = self.name()?;
        let whitespace = self.spaces();
        if !self.at(b">") {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                self.pos,
                (self.pos + 1).min(self.source.len()),
                "invalid end tag",
            ));
        }
        self.pos += 1;
        let Some(open) = self.stack.pop() else {
            return Err(diag(
                DiagnosticCode::UnbalancedElement,
                start,
                self.pos,
                "closing tag has no open element",
            ));
        };
        if self.source[open] != self.source[name] {
            return Err(diag(
                DiagnosticCode::UnbalancedElement,
                start,
                self.pos,
                "closing tag does not match open element",
            ));
        }
        self.tokens.push(Token {
            kind: TokenKind::EndTag,
            span: Span::new(start, self.pos),
        });
        let trivia: Vec<_> = (whitespace.start != whitespace.end)
            .then(|| Span::new(whitespace.start, whitespace.end))
            .into_iter()
            .collect();
        let separators = vec![false; trivia.len()];
        self.tags.push(TagLayout { trivia, separators });
        Ok(())
    }

    fn name(&mut self) -> Result<Range<usize>, Diagnostic> {
        let start = self.pos;
        let Some((first, width)) = next_char(self.source, self.pos) else {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                start,
                start,
                "expected XML name",
            ));
        };
        if !name_start(first) {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                start,
                start + width,
                "invalid XML name start",
            ));
        }
        self.pos += width;
        while let Some((ch, width)) = next_char(self.source, self.pos) {
            if !name_continue(ch) {
                break;
            }
            self.pos += width;
        }
        Ok(start..self.pos)
    }

    fn spaces(&mut self) -> Range<usize> {
        let start = self.pos;
        while self.pos < self.source.len() && is_space(self.source[self.pos]) {
            self.pos += 1;
        }
        start..self.pos
    }

    fn attribute_value(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        let Some(&quote @ (b'\'' | b'"')) = self.source.get(self.pos) else {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                start,
                (start + 1).min(self.source.len()),
                "attribute value must be quoted",
            ));
        };
        self.pos += 1;
        let content = self.pos;
        while self.pos < self.source.len() && self.source[self.pos] != quote {
            if self.source[self.pos] == b'<' {
                return Err(diag(
                    DiagnosticCode::InvalidSyntax,
                    self.pos,
                    self.pos + 1,
                    "'<' is not allowed in an attribute value",
                ));
            }
            self.pos += 1;
        }
        if self.pos == self.source.len() {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                start,
                self.pos,
                "unterminated attribute value",
            ));
        }
        if !valid_refs(&self.source[content..self.pos]) {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                content,
                self.pos,
                "invalid entity reference in attribute value",
            ));
        }
        self.pos += 1;
        Ok(())
    }

    fn at(&self, bytes: &[u8]) -> bool {
        self.source[self.pos..].starts_with(bytes)
    }

    fn is_doctype(&self) -> bool {
        self.at(b"<!DOCTYPE")
            && self
                .source
                .get(self.pos + b"<!DOCTYPE".len())
                .is_some_and(|byte| is_space(*byte))
    }

    fn validate_processing_instruction(
        &self,
        start: usize,
        content_start: usize,
        content_end: usize,
    ) -> Result<(), Diagnostic> {
        let Some((first, width)) = next_char(self.source, content_start) else {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                content_start,
                content_end,
                "processing instruction is missing a target",
            ));
        };
        if !name_start(first) {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                content_start,
                content_start + width,
                "invalid processing instruction target",
            ));
        }
        let mut end = content_start + width;
        while let Some((character, width)) = next_char(self.source, end) {
            if !name_continue(character) {
                break;
            }
            end += width;
        }
        if end != content_end && !is_space(self.source[end]) {
            return Err(diag(
                DiagnosticCode::InvalidSyntax,
                end,
                (end + 1).min(content_end),
                "processing instruction target must be followed by whitespace",
            ));
        }
        let target = &self.source[content_start..end];
        if target.eq_ignore_ascii_case(b"xml") {
            let declaration_offset = if self.bom { 3 } else { 0 };
            if target != b"xml" || start != declaration_offset {
                return Err(diag(
                    DiagnosticCode::InvalidSyntax,
                    content_start,
                    end,
                    "the reserved 'xml' target is only valid in the leading XML declaration",
                ));
            }
            if !valid_xml_declaration(&self.source[end..content_end]) {
                return Err(diag(
                    DiagnosticCode::InvalidSyntax,
                    end,
                    content_end,
                    "malformed XML declaration",
                ));
            }
        }
        Ok(())
    }
}

fn next_char(source: &[u8], pos: usize) -> Option<(char, usize)> {
    let text = std::str::from_utf8(source.get(pos..)?).ok()?;
    let ch = text.chars().next()?;
    Some((ch, ch.len_utf8()))
}
fn name_start(ch: char) -> bool {
    matches!(ch, ':' | 'A'..='Z' | '_' | 'a'..='z')
        || ('\u{c0}'..='\u{d6}').contains(&ch)
        || ('\u{d8}'..='\u{f6}').contains(&ch)
        || ('\u{f8}'..='\u{2ff}').contains(&ch)
        || ('\u{370}'..='\u{37d}').contains(&ch)
        || ('\u{37f}'..='\u{1fff}').contains(&ch)
        || ('\u{200c}'..='\u{200d}').contains(&ch)
        || ('\u{2070}'..='\u{218f}').contains(&ch)
        || ('\u{2c00}'..='\u{2fef}').contains(&ch)
        || ('\u{3001}'..='\u{d7ff}').contains(&ch)
        || ('\u{f900}'..='\u{fdcf}').contains(&ch)
        || ('\u{fdf0}'..='\u{fffd}').contains(&ch)
        || ('\u{10000}'..='\u{effff}').contains(&ch)
}
fn name_continue(ch: char) -> bool {
    name_start(ch)
        || matches!(ch, '-' | '.' | '0'..='9' | '\u{b7}')
        || ('\u{300}'..='\u{36f}').contains(&ch)
        || ('\u{203f}'..='\u{2040}').contains(&ch)
}
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}
fn is_xml_char(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{a}' | '\u{d}')
        || ('\u{20}'..='\u{d7ff}').contains(&character)
        || ('\u{e000}'..='\u{fffd}').contains(&character)
        || ('\u{10000}'..='\u{10ffff}').contains(&character)
}
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|x| x == needle)
}

fn valid_refs(raw: &[u8]) -> bool {
    let mut pos = 0;
    while let Some(relative) = raw[pos..].iter().position(|b| *b == b'&') {
        let start = pos + relative + 1;
        let Some(end_relative) = raw[start..].iter().position(|b| *b == b';') else {
            return false;
        };
        let value = &raw[start..start + end_relative];
        if value.is_empty() {
            return false;
        }
        let valid = if let Some(hex) = value.strip_prefix(b"#x") {
            valid_numeric_ref(hex, 16)
        } else if let Some(dec) = value.strip_prefix(b"#") {
            valid_numeric_ref(dec, 10)
        } else {
            value.iter().enumerate().all(|(index, byte)| {
                if index == 0 {
                    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b':')
                } else {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'-' | b'.')
                }
            })
        };
        if !valid {
            return false;
        }
        pos = start + end_relative + 1;
    }
    true
}

fn valid_numeric_ref(digits: &[u8], radix: u32) -> bool {
    if digits.is_empty() {
        return false;
    }
    let Ok(text) = std::str::from_utf8(digits) else {
        return false;
    };
    u32::from_str_radix(text, radix)
        .ok()
        .and_then(char::from_u32)
        .is_some_and(is_xml_char)
}

fn valid_xml_declaration(raw: &[u8]) -> bool {
    let mut cursor = DeclCursor { raw, pos: 0 };
    if !cursor.spaces() || cursor.name() != Some(b"version".as_slice()) {
        return false;
    }
    let Some(version) = cursor.value() else {
        return false;
    };
    if !version.starts_with(b"1.")
        || version.len() == 2
        || !version[2..].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    let mut seen_encoding = false;
    let mut seen_standalone = false;
    loop {
        let separated = cursor.spaces();
        if cursor.pos == raw.len() {
            return true;
        }
        if !separated {
            return false;
        }
        match cursor.name() {
            Some(b"encoding") if !seen_encoding && !seen_standalone => {
                let Some(value) = cursor.value() else {
                    return false;
                };
                if value.is_empty()
                    || !value[0].is_ascii_alphabetic()
                    || !value[1..].iter().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                    })
                {
                    return false;
                }
                seen_encoding = true;
            }
            Some(b"standalone") if !seen_standalone => {
                let Some(value) = cursor.value() else {
                    return false;
                };
                if !matches!(value, b"yes" | b"no") {
                    return false;
                }
                seen_standalone = true;
            }
            _ => return false,
        }
    }
}

struct DeclCursor<'a> {
    raw: &'a [u8],
    pos: usize,
}
impl<'a> DeclCursor<'a> {
    fn spaces(&mut self) -> bool {
        let start = self.pos;
        while self.raw.get(self.pos).is_some_and(|byte| is_space(*byte)) {
            self.pos += 1;
        }
        self.pos != start
    }
    fn name(&mut self) -> Option<&'a [u8]> {
        let start = self.pos;
        while self.raw.get(self.pos).is_some_and(u8::is_ascii_alphabetic) {
            self.pos += 1;
        }
        (self.pos != start).then_some(&self.raw[start..self.pos])
    }
    fn value(&mut self) -> Option<&'a [u8]> {
        self.spaces();
        if self.raw.get(self.pos) != Some(&b'=') {
            return None;
        }
        self.pos += 1;
        self.spaces();
        let quote = *self.raw.get(self.pos)?;
        if !matches!(quote, b'\'' | b'"') {
            return None;
        }
        self.pos += 1;
        let start = self.pos;
        while self.raw.get(self.pos).is_some_and(|byte| *byte != quote) {
            self.pos += 1;
        }
        if self.pos == self.raw.len() {
            return None;
        }
        let value = &self.raw[start..self.pos];
        self.pos += 1;
        Some(value)
    }
}

fn diag(code: DiagnosticCode, start: usize, end: usize, message: &'static str) -> Diagnostic {
    Diagnostic {
        code,
        span: Span::new(start, end),
        message,
    }
}
