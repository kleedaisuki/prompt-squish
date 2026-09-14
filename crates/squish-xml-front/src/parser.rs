//! 严格 XML 语法解析。 / Strict XML syntax parsing.

use crate::{DSL_NAMESPACE, ast::*};
use regex::Regex;
use roxmltree::{Document, Node as Xml};
use squish_ir::{DecodedSyntax, ExpandedName, ImportSpec};
use squish_protocol::{Diagnostic, DiagnosticId, Phase, Severity, Span};
use squish_source::SourceBlob;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn parse(source: &SourceBlob) -> Result<Unit, Box<Diagnostic>> {
    let bytes = source.bytes();
    let bom = usize::from(bytes.starts_with(&[0xef, 0xbb, 0xbf])) * 3;
    let text = std::str::from_utf8(&bytes[bom..]).map_err(|e| {
        diagnostic(
            source,
            "XS1000",
            Phase::Parse,
            "XML source must be UTF-8",
            e.valid_up_to() + bom,
            (e.valid_up_to() + 1).min(bytes.len() - bom) + bom,
        )
    })?;
    let doc = Document::parse(text).map_err(|e| {
        let offset = text_pos_offset(text, e.pos().row as usize, e.pos().col as usize) + bom;
        diagnostic(
            source,
            "XS1001",
            Phase::Parse,
            format!("XML: {e}"),
            offset,
            (offset + 1).min(bytes.len()),
        )
    })?;
    let root = doc.root_element();
    let mut p = Parser {
        source,
        bom,
        imports: Vec::new(),
    };
    if !builtin(root, "entry") && !builtin(root, "module") {
        return Err(p.error(
            root,
            "XS1100",
            "Syntax: source root must be xs:entry or xs:module",
        ));
    }
    p.attrs(root, &[])?;
    let parsed = if builtin(root, "entry") {
        p.entry(root)?
    } else {
        p.module(root)?
    };
    Ok(Unit {
        root: parsed,
        imports: p.imports,
    })
}

struct Parser<'a> {
    source: &'a SourceBlob,
    bom: usize,
    imports: Vec<Import>,
}
impl Parser<'_> {
    fn loc(&self, n: Xml<'_, '_>) -> Loc {
        let r = n.range();
        Loc {
            start: r.start,
            end: r.end,
        }
    }
    fn error(&self, n: Xml<'_, '_>, code: &str, message: impl Into<String>) -> Box<Diagnostic> {
        let l = self.loc(n);
        diagnostic(
            self.source,
            code,
            Phase::Analyze,
            message,
            l.start + self.bom,
            l.end + self.bom,
        )
    }
    fn attrs(&self, n: Xml<'_, '_>, allowed: &[&str]) -> Result<(), Box<Diagnostic>> {
        for a in n.attributes() {
            if a.namespace().is_some() || !allowed.contains(&a.name()) {
                return Err(self.error(
                    n,
                    "XS1101",
                    format!("Syntax: unexpected attribute {}", a.name()),
                ));
            }
        }
        Ok(())
    }
    fn attr(&self, n: Xml<'_, '_>, key: &str) -> Result<String, Box<Diagnostic>> {
        n.attribute(key)
            .map(str::to_owned)
            .ok_or_else(|| self.error(n, "XS1102", format!("Syntax: missing {key} attribute")))
    }
    fn empty(&self, n: Xml<'_, '_>) -> Result<(), Box<Diagnostic>> {
        if n.children()
            .any(|c| !c.is_text() || !c.text().unwrap_or("").chars().all(xml_space))
        {
            Err(self.error(n, "XS1103", "Syntax: directive must be empty"))
        } else {
            Ok(())
        }
    }
    fn local(&self, n: Xml<'_, '_>, key: &str) -> Result<String, Box<Diagnostic>> {
        let s = self.attr(n, key)?;
        if ncname(&s) {
            Ok(s)
        } else {
            Err(self.error(n, "XS1200", format!("Namespace: {key} must be an NCName")))
        }
    }
    fn qname(&self, n: Xml<'_, '_>, key: &str) -> Result<(ExpandedName, String), Box<Diagnostic>> {
        let s = self.attr(n, key)?;
        let (prefix, local) = s
            .split_once(':')
            .filter(|(p, l)| ncname(p) && ncname(l))
            .ok_or_else(|| {
                self.error(
                    n,
                    "XS1201",
                    "Namespace: macro name/reference must be a prefixed QName",
                )
            })?;
        let uri = n.lookup_namespace_uri(Some(prefix)).ok_or_else(|| {
            self.error(n, "XS1202", format!("Namespace: unbound prefix {prefix}"))
        })?;
        if uri == DSL_NAMESPACE {
            return Err(self.error(
                n,
                "XS1203",
                "Namespace: builtin namespace cannot identify user macros",
            ));
        }
        Ok((
            ExpandedName {
                namespace_uri: uri.into(),
                local_name: local.into(),
            },
            s,
        ))
    }
    fn import(&mut self, n: Xml<'_, '_>) -> Result<(), Box<Diagnostic>> {
        self.attrs(n, &["src"])?;
        self.empty(n)?;
        let raw = self.attr(n, "src")?;
        let spec =
            import_spec(&raw).map_err(|m| self.error(n, "XS1300", format!("Source: {m}")))?;
        self.imports.push(Import {
            loc: self.loc(n),
            spec,
            decoded: self.attr_map(n, "src", "spec"),
        });
        Ok(())
    }
    fn entry(&mut self, n: Xml<'_, '_>) -> Result<Root, Box<Diagnostic>> {
        let mut params = Vec::new();
        let mut body = Vec::new();
        let mut param_origins = Vec::new();
        let mut slots = BTreeMap::new();
        let mut content = false;
        for c in n.children() {
            if builtin(c, "import") || builtin(c, "param") {
                if content {
                    return Err(self.error(
                        c,
                        "XS1104",
                        "Syntax: entry declarations must precede output content",
                    ));
                }
                if builtin(c, "import") {
                    self.import(c)?
                } else {
                    let decoded = self.param(c, &mut params)?;
                    param_origins.push((self.loc(c), decoded));
                }
            } else {
                if !trivia(c) {
                    content = true
                }
                body.push(self.node(c, &mut slots)?);
            }
        }
        if !slots.is_empty() {
            return Err(self.error(n, "XS1400", "Signature: xs:entry cannot declare slots"));
        }
        Ok(Root::Entry {
            params,
            param_origins,
            body,
        })
    }
    fn module(&mut self, n: Xml<'_, '_>) -> Result<Root, Box<Diagnostic>> {
        let mut definitions = Vec::new();
        let mut symbols = BTreeSet::new();
        for c in n.children() {
            if trivia(c) || c.pi().is_some() {
                continue;
            }
            if builtin(c, "import") {
                self.import(c)?
            } else if builtin(c, "macro") {
                let d = self.definition(c)?;
                if !symbols.insert(d.symbol.clone()) {
                    return Err(self.error(
                        c,
                        "XS1401",
                        format!("Signature: duplicate macro {}", d.lexical),
                    ));
                }
                definitions.push(d)
            } else {
                return Err(self.error(
                    c,
                    "XS1105",
                    "Syntax: module contains only xs:import and explicit xs:macro declarations",
                ));
            }
        }
        Ok(Root::Module { definitions })
    }
    fn param(
        &self,
        n: Xml<'_, '_>,
        params: &mut Vec<String>,
    ) -> Result<RawDecodedValue, Box<Diagnostic>> {
        self.attrs(n, &["name"])?;
        self.empty(n)?;
        let name = self.local(n, "name")?;
        if params.contains(&name) {
            return Err(self.error(
                n,
                "XS1402",
                format!("Signature: duplicate parameter {name}"),
            ));
        }
        params.push(name);
        Ok(self.attr_map(n, "name", "name"))
    }
    fn definition(&mut self, n: Xml<'_, '_>) -> Result<Definition, Box<Diagnostic>> {
        self.attrs(n, &["name"])?;
        let (symbol, lexical) = self.qname(n, "name")?;
        let mut params = Vec::new();
        let mut slots = BTreeMap::new();
        let mut body = Vec::new();
        let mut decoded = vec![self.attr_map(n, "name", "symbol")];
        let mut content = false;
        for c in n.children() {
            if builtin(c, "param") {
                if content {
                    return Err(self.error(
                        c,
                        "XS1106",
                        "Syntax: parameters must precede macro content",
                    ));
                }
                let index = params.len();
                let mut value = self.param(c, &mut params)?;
                value.field = format!("signature.params.{index}");
                decoded.push(value);
            } else {
                if !trivia(c) {
                    content = true
                }
                body.push(self.node(c, &mut slots)?);
            }
        }
        Ok(Definition {
            loc: self.loc(n),
            lexical,
            symbol,
            params,
            slots,
            body,
            decoded,
        })
    }
    fn body(
        &mut self,
        n: Xml<'_, '_>,
        slots: &mut BTreeMap<String, bool>,
    ) -> Result<Vec<Node>, Box<Diagnostic>> {
        n.children().map(|c| self.node(c, slots)).collect()
    }
    fn get(&self, n: Xml<'_, '_>) -> Result<String, Box<Diagnostic>> {
        let s = self.attr(n, "get")?;
        if s.split_once('.').is_some_and(|(scope, name)| {
            (matches!(scope, "arg" | "match") && ncname(name))
                || (scope == "file" && matches!(name, "uri" | "dir" | "name"))
        }) {
            Ok(s)
        } else {
            Err(self.error(
                n,
                "XS1500",
                "Scalar: get must name file.uri, file.dir, file.name, arg.*, or match.* binding",
            ))
        }
    }
    fn value(
        &mut self,
        n: Xml<'_, '_>,
        slots: &mut BTreeMap<String, bool>,
    ) -> Result<Value, Box<Diagnostic>> {
        let forms = usize::from(n.has_attribute("value"))
            + usize::from(n.has_attribute("get"))
            + usize::from(n.children().next().is_some());
        if forms > 1 {
            return Err(self.error(
                n,
                "XS1403",
                "Signature: argument requires exactly one of value, get, or body",
            ));
        }
        if let Some(v) = n.attribute("value") {
            Ok(Value::Literal(v.into()))
        } else if n.has_attribute("get") {
            Ok(Value::Get(self.get(n)?))
        } else {
            Ok(Value::Body(self.body(n, slots)?))
        }
    }
    fn invoke(
        &mut self,
        n: Xml<'_, '_>,
        slots: &mut BTreeMap<String, bool>,
    ) -> Result<(Kind, Vec<RawDecodedValue>), Box<Diagnostic>> {
        self.attrs(n, &["ref"])?;
        let (target, _) = self.qname(n, "ref")?;
        let mut decoded = vec![self.attr_map(n, "ref", "target")];
        let mut args = Vec::new();
        let mut fills = Vec::new();
        let mut names = BTreeSet::new();
        let mut fill_names = BTreeSet::new();
        for c in n.children() {
            if trivia(c) {
                continue;
            }
            if builtin(c, "arg") {
                self.attrs(c, &["name", "value", "get"])?;
                let name = self.local(c, "name")?;
                decoded.push(self.attr_map(c, "name", format!("args.{name}.name")));
                if c.has_attribute("value") {
                    decoded.push(self.attr_map(c, "value", format!("args.{name}.value")));
                } else if c.has_attribute("get") {
                    decoded.push(self.attr_map(c, "get", format!("args.{name}.binding")));
                }
                if !names.insert(name.clone()) {
                    return Err(self.error(
                        c,
                        "XS1404",
                        format!("Signature: duplicate argument {name}"),
                    ));
                }
                args.push(Argument {
                    name,
                    value: self.value(c, slots)?,
                })
            } else if builtin(c, "fill") {
                self.attrs(c, &["name"])?;
                let name = self.local(c, "name")?;
                decoded.push(self.attr_map(c, "name", format!("fills.{name}.name")));
                if !fill_names.insert(name.clone()) {
                    return Err(self.error(
                        c,
                        "XS1405",
                        format!("Signature: duplicate fill {name}"),
                    ));
                }
                fills.push(Fill {
                    name,
                    body: self.body(c, slots)?,
                })
            } else {
                return Err(self.error(
                    c,
                    "XS1107",
                    "Syntax: expand children must be xs:arg or xs:fill",
                ));
            }
        }
        Ok((
            Kind::Invoke {
                target,
                args,
                fills,
            },
            decoded,
        ))
    }
    fn node(
        &mut self,
        n: Xml<'_, '_>,
        slots: &mut BTreeMap<String, bool>,
    ) -> Result<Node, Box<Diagnostic>> {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.node_inner(n, slots))
    }
    fn node_inner(
        &mut self,
        n: Xml<'_, '_>,
        slots: &mut BTreeMap<String, bool>,
    ) -> Result<Node, Box<Diagnostic>> {
        let lexical = n.is_element().then(|| self.lexical_name(n));
        let mut decoded = if n.is_text() {
            vec![RawDecodedValue {
                field: "value".into(),
                segments: decoded_text(
                    self.source,
                    self.bom,
                    n.range().start,
                    n.text().unwrap_or(""),
                ),
            }]
        } else {
            Vec::new()
        };
        let kind = if n.is_text() {
            Kind::Text(n.text().unwrap_or("").into())
        } else if n.is_comment() {
            Kind::Comment(n.text().unwrap_or("").into())
        } else if let Some(pi) = n.pi() {
            Kind::Pi {
                target: pi.target.into(),
                data: pi.value.unwrap_or("").into(),
            }
        } else if n.tag_name().namespace() != Some(DSL_NAMESPACE) {
            let name = expanded(n.tag_name().namespace(), n.tag_name().name());
            let mut attrs = Vec::new();
            for (index, a) in n.attributes().enumerate() {
                decoded.push(self.attribute_map(
                    a.position(),
                    a.value(),
                    format!("attributes.{index}.value"),
                ));
                attrs.push((expanded(a.namespace(), a.name()), a.value().into()));
            }
            Kind::Element {
                name,
                attrs,
                children: self.body(n, slots)?,
            }
        } else {
            match n.tag_name().name() {
                "insert" => {
                    self.attrs(n, &["get"])?;
                    self.empty(n)?;
                    decoded.push(self.attr_map(n, "get", "binding"));
                    Kind::Insert(self.get(n)?)
                }
                "slot" => {
                    self.attrs(n, &["name", "required"])?;
                    self.empty(n)?;
                    let name = self.local(n, "name")?;
                    decoded.push(self.attr_map(n, "name", "slot.name"));
                    let required = match n.attribute("required") {
                        None | Some("false") => false,
                        Some("true") => true,
                        _ => {
                            return Err(self.error(
                                n,
                                "XS1108",
                                "Syntax: required must be true or false",
                            ));
                        }
                    };
                    if slots.insert(name.clone(), required).is_some() {
                        return Err(self.error(
                            n,
                            "XS1406",
                            format!("Signature: duplicate slot {name}"),
                        ));
                    }
                    Kind::Slot { name }
                }
                "expand" => {
                    let (kind, values) = self.invoke(n, slots)?;
                    decoded.extend(values);
                    kind
                }
                "ifr" => {
                    self.attrs(n, &["get", "str", "pattern"])?;
                    if n.has_attribute("get") == n.has_attribute("str") {
                        return Err(self.error(
                            n,
                            "XS1109",
                            "Syntax: ifr requires exactly one of get or str",
                        ));
                    }
                    let input = if n.has_attribute("get") {
                        decoded.push(self.attr_map(n, "get", "input.binding"));
                        Value::Get(self.get(n)?)
                    } else {
                        decoded.push(self.attr_map(n, "str", "input.literal"));
                        Value::Literal(self.attr(n, "str")?)
                    };
                    let pattern = self.attr(n, "pattern")?;
                    decoded.push(self.attr_map(n, "pattern", "pattern"));
                    let regex = Regex::new(&pattern)
                        .map_err(|e| self.error(n, "XS1600", format!("Regex: {e}")))?;
                    let mut captures = Vec::new();
                    for capture in regex.capture_names().skip(1) {
                        let Some(capture) = capture.filter(|v| ncname(v)) else {
                            return Err(self.error(n,"XS1601","Regex: only NCName named captures are permitted; positional captures are forbidden"));
                        };
                        captures.push(capture.into())
                    }
                    Kind::If {
                        input,
                        pattern,
                        captures,
                        body: self.body(n, slots)?,
                    }
                }
                other => {
                    return Err(self.error(
                        n,
                        "XS1110",
                        format!("Syntax: unknown or misplaced xs:{other}"),
                    ));
                }
            }
        };
        Ok(Node {
            loc: self.loc(n),
            lexical,
            kind,
            decoded,
        })
    }

    /// 从精确源片段保留作者的 QName 拼写。 / Preserves the authored QName spelling.
    fn lexical_name(&self, n: Xml<'_, '_>) -> String {
        let loc = self.loc(n);
        let bytes = &self.source.bytes()[self.bom + loc.start..self.bom + loc.end];
        let start = bytes
            .iter()
            .position(|byte| *byte == b'<')
            .map_or(0, |index| index + 1);
        let end = bytes[start..]
            .iter()
            .position(|byte| byte.is_ascii_whitespace() || matches!(*byte, b'/' | b'>'))
            .map_or(bytes.len(), |index| start + index);
        String::from_utf8_lossy(&bytes[start..end]).into_owned()
    }

    fn attr_map(&self, n: Xml<'_, '_>, name: &str, field: impl Into<String>) -> RawDecodedValue {
        let attribute = n
            .attributes()
            .find(|attribute| attribute.namespace().is_none() && attribute.name() == name)
            .expect("validated attribute exists");
        self.attribute_map(attribute.position(), attribute.value(), field)
    }

    fn attribute_map(
        &self,
        position: usize,
        value: &str,
        field: impl Into<String>,
    ) -> RawDecodedValue {
        let bytes = &self.source.bytes()[self.bom..];
        let equals = bytes[position..]
            .iter()
            .position(|byte| *byte == b'=')
            .map(|offset| position + offset)
            .expect("parsed XML attribute contains equals");
        let quote = bytes[equals + 1..]
            .iter()
            .position(|byte| matches!(*byte, b'\'' | b'"'))
            .map(|offset| equals + 1 + offset)
            .expect("parsed XML attribute contains quote");
        RawDecodedValue {
            field: field.into(),
            segments: decoded_text(self.source, self.bom, quote + 1, value),
        }
    }
}

/// 从 XML 字面形式建立解码 UTF-8 值到源字节的分段映射。
/// Builds a decoded UTF-8 value-to-source-byte segment map from XML lexical forms.
fn decoded_text(
    source: &SourceBlob,
    bom: usize,
    start: usize,
    value: &str,
) -> Vec<RawDecodedSegment> {
    let bytes = &source.bytes()[bom..];
    let mut cursor = start;
    let mut value_at = 0;
    let mut result = Vec::new();
    while value_at < value.len() && cursor < bytes.len() {
        if bytes[cursor..].starts_with(b"<![CDATA[") {
            let content = cursor + 9;
            let Some(relative_end) = bytes[content..].windows(3).position(|w| w == b"]]>") else {
                break;
            };
            let end = content + relative_end;
            let mut at = content;
            while at < end && value_at < value.len() {
                let Some(source_len) = utf8_char_len(bytes[at]) else {
                    break;
                };
                let Ok(ch) = std::str::from_utf8(&bytes[at..at + source_len]) else {
                    break;
                };
                let ch = ch.chars().next().expect("one UTF-8 scalar is nonempty");
                let decoded_len = if ch == '\r' { 1 } else { source_len };
                let source_end = if ch == '\r' && bytes.get(at + 1) == Some(&b'\n') {
                    at + 2
                } else {
                    at + source_len
                };
                push_segment(
                    &mut result,
                    value_at,
                    value_at + decoded_len,
                    at,
                    source_end,
                    DecodedSyntax::CData,
                );
                value_at += decoded_len;
                at = source_end;
            }
            cursor = end + 3;
            continue;
        }
        if bytes[cursor] == b'&'
            && let Some(relative_end) = bytes[cursor..].iter().position(|b| *b == b';')
        {
            let end = cursor + relative_end + 1;
            let reference = std::str::from_utf8(&bytes[cursor + 1..end - 1]).unwrap_or("");
            if let Some(decoded) = decode_reference(reference) {
                let length = decoded.len_utf8();
                push_segment(
                    &mut result,
                    value_at,
                    value_at + length,
                    cursor,
                    end,
                    if reference.starts_with('#') {
                        DecodedSyntax::CharacterReference
                    } else {
                        DecodedSyntax::EntityReference
                    },
                );
                value_at += length;
                cursor = end;
                continue;
            }
            let next = bytes.get(end).copied();
            let entity_len = next
                .filter(|byte| !matches!(*byte, b'<' | b'&'))
                .and_then(|_| std::str::from_utf8(&bytes[end..]).ok())
                .and_then(|tail| tail.chars().next())
                .and_then(|anchor| value[value_at..].find(anchor))
                .unwrap_or(value.len() - value_at);
            push_segment(
                &mut result,
                value_at,
                value_at + entity_len,
                cursor,
                end,
                DecodedSyntax::EntityReference,
            );
            value_at += entity_len;
            cursor = end;
            continue;
        }
        if bytes[cursor] == b'<' {
            break;
        }
        let Some(source_len) = utf8_char_len(bytes[cursor]) else {
            break;
        };
        let Ok(ch) = std::str::from_utf8(&bytes[cursor..cursor + source_len]) else {
            break;
        };
        let ch = ch.chars().next().expect("one UTF-8 scalar is nonempty");
        let decoded_len = if ch == '\r' { 1 } else { source_len };
        let source_end = if ch == '\r' && bytes.get(cursor + 1) == Some(&b'\n') {
            cursor + 2
        } else {
            cursor + source_len
        };
        push_segment(
            &mut result,
            value_at,
            value_at + decoded_len,
            cursor,
            source_end,
            DecodedSyntax::LiteralText,
        );
        value_at += decoded_len;
        cursor = source_end;
    }
    result
}

fn decode_reference(reference: &str) -> Option<char> {
    match reference {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        value if value.starts_with("#x") => u32::from_str_radix(&value[2..], 16)
            .ok()
            .and_then(char::from_u32),
        value if value.starts_with('#') => value[1..].parse().ok().and_then(char::from_u32),
        _ => None,
    }
}

fn push_segment(
    result: &mut Vec<RawDecodedSegment>,
    value_start: usize,
    value_end: usize,
    source_start: usize,
    source_end: usize,
    syntax: squish_ir::DecodedSyntax,
) {
    if value_start >= value_end {
        return;
    }
    if let Some(previous) = result.last_mut()
        && matches!(syntax, DecodedSyntax::LiteralText | DecodedSyntax::CData)
        && previous.syntax == syntax
        && previous.value_end == value_start
        && previous.source_end == source_start
        && previous.value_end - previous.value_start == previous.source_end - previous.source_start
        && value_end - value_start == source_end - source_start
    {
        previous.value_end = value_end;
        previous.source_end = source_end;
    } else {
        result.push(RawDecodedSegment {
            value_start,
            value_end,
            source_start,
            source_end,
            syntax,
        });
    }
}

fn utf8_char_len(first: u8) -> Option<usize> {
    match first {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

fn import_spec(raw: &str) -> Result<ImportSpec, &'static str> {
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return Err("src must be a nonempty static URI without control characters");
    }
    if raw.contains(['?', '#']) {
        return Err("source imports do not support URI queries or fragments");
    }
    if let Some(rest) = raw.strip_prefix("pkg:") {
        let (alias, export) = rest
            .split_once('/')
            .ok_or("package import must be pkg:ALIAS/EXPORT")?;
        if !dependency_alias(alias) || export.is_empty() {
            return Err("package import has an invalid alias or empty export");
        }
        return Ok(ImportSpec::PackageExport {
            dependency_alias: alias.into(),
            export: export.into(),
        });
    }
    if raw.starts_with("file:/") {
        Ok(ImportSpec::AbsoluteFileUri(raw.into()))
    } else if raw.starts_with("file:") {
        Err("absolute file URI must begin with file:/")
    } else if raw.contains(':') {
        Err("unsupported URI scheme")
    } else {
        Ok(ImportSpec::RelativeUri(raw.into()))
    }
}

fn dependency_alias(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && !matches!(value, "." | "..")
}
fn expanded(uri: Option<&str>, local: &str) -> ExpandedName {
    ExpandedName {
        namespace_uri: uri.unwrap_or("").into(),
        local_name: local.into(),
    }
}
fn builtin(n: Xml<'_, '_>, name: &str) -> bool {
    n.is_element() && n.tag_name().namespace() == Some(DSL_NAMESPACE) && n.tag_name().name() == name
}
fn trivia(n: Xml<'_, '_>) -> bool {
    n.is_comment() || (n.is_text() && n.text().unwrap_or("").chars().all(xml_space))
}
fn xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}
fn ncname(s: &str) -> bool {
    fn start(c: char) -> bool {
        matches!(c,'A'..='Z'|'_'|'a'..='z'|'\u{C0}'..='\u{D6}'|'\u{D8}'..='\u{F6}'|'\u{F8}'..='\u{2FF}'|'\u{370}'..='\u{37D}'|'\u{37F}'..='\u{1FFF}'|'\u{200C}'..='\u{200D}'|'\u{2070}'..='\u{218F}'|'\u{2C00}'..='\u{2FEF}'|'\u{3001}'..='\u{D7FF}'|'\u{F900}'..='\u{FDCF}'|'\u{FDF0}'..='\u{FFFD}'|'\u{10000}'..='\u{EFFFF}')
    }
    let mut chars = s.chars();
    chars.next().is_some_and(start) && chars.all(|c| {
        start(c)
            || matches!(c,'-'|'.'|'0'..='9'|'\u{B7}'|'\u{300}'..='\u{36F}'|'\u{203F}'..='\u{2040}')
    })
}
fn text_pos_offset(text: &str, row: usize, col: usize) -> usize {
    let line_start = text
        .split_inclusive('\n')
        .take(row.saturating_sub(1))
        .map(str::len)
        .sum::<usize>();
    let line = &text[line_start
        ..text[line_start..]
            .find('\n')
            .map_or(text.len(), |end| line_start + end)];
    let column = line
        .char_indices()
        .nth(col.saturating_sub(1))
        .map_or(line.len(), |(offset, _)| offset);
    line_start + column
}
pub(crate) fn diagnostic(
    source: &SourceBlob,
    code: &str,
    phase: Phase,
    message: impl Into<String>,
    start: usize,
    end: usize,
) -> Box<Diagnostic> {
    Box::new(Diagnostic {
        id: DiagnosticId::new(format!("xml-front-{code}-{start}"))
            .expect("generated id is nonempty"),
        code: code.into(),
        severity: Severity::Error,
        phase,
        message: message.into(),
        primary: Span::new(source.id().to_protocol(), start as u64, end as u64).ok(),
        related: Vec::new(),
        help: None,
    })
}
