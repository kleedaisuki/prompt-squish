//! 严格解析模块与静态指令。 / Strict module and directive parsing.
use super::{CompileError, model::*};
use roxmltree::{Document, Node as Xml};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// 内建命名空间。 / Builtin namespace identity.
const NS: &str = "https://xmlsquish.moesegfault.dev/ns";

/// 解析一次冻结的源码。 / Parse one immutable source unit.
pub(super) fn parse(path: &Path, text: &str, next_id: &mut usize) -> Result<Unit, CompileError> {
    let doc = Document::parse(text).map_err(|e| CompileError {
        path: path.into(),
        line: e.pos().row as usize,
        message: format!("XML: {e}"),
    })?;
    let root = doc.root_element();
    let mut parser = Parser {
        path,
        newlines: text
            .bytes()
            .enumerate()
            .filter_map(|(i, b)| (b == b'\n').then_some(i))
            .collect(),
        references: Vec::new(),
    };
    if !builtin(root, "module") {
        return Err(parser
            .loc(root)
            .error("Syntax: source root must be xs:module"));
    }
    parser.attrs(root, &[])?;
    let main_id = *next_id;
    *next_id += 1;
    let mut main = MacroDef {
        id: main_id,
        loc: parser.loc(root),
        name: None,
        params: Vec::new(),
        slots: BTreeMap::new(),
        body: Vec::new(),
    };
    let mut macros = Vec::new();
    let mut content = false;
    for child in root
        .prev_siblings()
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        main.body.push(parser.node(child, &mut main.slots)?);
    }
    for child in root.children() {
        if builtin(child, "import") || builtin(child, "param") || builtin(child, "macro") {
            if content {
                return Err(parser
                    .loc(child)
                    .error("Syntax: declarations must precede module content"));
            }
            if builtin(child, "import") {
                parser.attrs(child, &["src"])?;
                parser.empty(child)?;
                parser.source(child)?;
            } else if builtin(child, "param") {
                parser.param(child, &mut main.params)?;
            } else {
                macros.push(parser.macro_def(child, next_id)?);
            }
        } else {
            if !trivia(child) {
                content = true;
            }
            main.body.push(parser.node(child, &mut main.slots)?);
        }
    }
    for child in root.next_siblings().skip(1) {
        main.body.push(parser.node(child, &mut main.slots)?);
    }
    macros.insert(0, main);
    Ok(Unit {
        path: path.into(),
        main: main_id,
        macros,
        references: parser.references,
    })
}

/// 词法解析上下文。 / Lexical parser context.
struct Parser<'a> {
    /// 定义源码路径。 / Definition source path.
    path: &'a Path,
    /// 预索引行界限，避免重复扫描源码。 / Preindexed line boundaries avoid repeated source scans.
    newlines: Vec<usize>,
    /// 完整静态源码闭包边。 / Static source discovery edges.
    references: Vec<(std::path::PathBuf, Loc)>,
}
impl Parser<'_> {
    /// 节点源码范围。 / Node source span.
    fn loc(&self, n: Xml<'_, '_>) -> Loc {
        let r = n.range();
        Loc {
            path: self.path.into(),
            line: self.newlines.partition_point(|offset| *offset < r.start) + 1,
            start: r.start,
            end: r.end,
        }
    }
    /// 拒绝拼写错误和额外属性。 / Reject unknown or namespaced directive attributes.
    fn attrs(&self, n: Xml<'_, '_>, allowed: &[&str]) -> Result<(), CompileError> {
        for a in n.attributes() {
            if a.namespace().is_some() || !allowed.contains(&a.name()) {
                return Err(self
                    .loc(n)
                    .error(format!("Syntax: unexpected attribute {}", a.name())));
            }
        }
        Ok(())
    }
    /// 必需属性，不默认为空。 / Require an explicit attribute.
    fn attr(&self, n: Xml<'_, '_>, key: &str) -> Result<String, CompileError> {
        n.attribute(key).map(str::to_owned).ok_or_else(|| {
            self.loc(n)
                .error(format!("Syntax: missing {key} attribute"))
        })
    }
    /// 空指令只允许排版空白。 / Empty directives allow only formatting whitespace.
    fn empty(&self, n: Xml<'_, '_>) -> Result<(), CompileError> {
        if n.children().any(|c| {
            !c.is_text()
                || !c
                    .text()
                    .unwrap_or("")
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\r' | '\n'))
        }) {
            Err(self.loc(n).error("Syntax: directive must be empty"))
        } else {
            Ok(())
        }
    }
    /// 验证局部名称。 / Validate a namespace-free local name.
    fn local(&self, n: Xml<'_, '_>, key: &str) -> Result<String, CompileError> {
        let s = self.attr(n, key)?;
        if !ncname(&s) {
            return Err(self
                .loc(n)
                .error(format!("Namespace: {key} must be an NCName")));
        }
        Ok(s)
    }
    /// 在定义位置解析 QName。 / Resolve QName at its lexical definition site.
    fn qname(&self, n: Xml<'_, '_>, key: &str) -> Result<Name, CompileError> {
        let s = self.attr(n, key)?;
        let (prefix, local) = s
            .split_once(':')
            .filter(|(p, l)| ncname(p) && ncname(l))
            .ok_or_else(|| {
                self.loc(n)
                    .error("Namespace: macro name/reference must be a prefixed QName")
            })?;
        let uri = n.lookup_namespace_uri(Some(prefix)).ok_or_else(|| {
            self.loc(n)
                .error(format!("Namespace: unbound prefix {prefix}"))
        })?;
        if uri == NS {
            return Err(self
                .loc(n)
                .error("Namespace: builtin namespace cannot identify user macros"));
        }
        Ok((uri.into(), local.into()))
    }
    /// 记录静态引用，包括不可达代码。 / Record static references including unreachable code.
    fn source(&mut self, n: Xml<'_, '_>) -> Result<std::path::PathBuf, CompileError> {
        let src = self.attr(n, "src")?;
        let path =
            resolve_path(self.path, &src).map_err(|e| self.loc(n).error(format!("Source: {e}")))?;
        self.references.push((path.clone(), self.loc(n)));
        Ok(path)
    }
    /// 声明唯一参数。 / Declare a unique required parameter.
    fn param(&self, n: Xml<'_, '_>, params: &mut Vec<String>) -> Result<(), CompileError> {
        self.attrs(n, &["name"])?;
        self.empty(n)?;
        let name = self.local(n, "name")?;
        if params.contains(&name) {
            return Err(self
                .loc(n)
                .error(format!("Signature: duplicate parameter {name}")));
        }
        params.push(name);
        Ok(())
    }
    /// 解析命名宏及声明区。 / Parse a named macro and its declaration prefix.
    fn macro_def(&mut self, n: Xml<'_, '_>, ids: &mut usize) -> Result<MacroDef, CompileError> {
        self.attrs(n, &["name"])?;
        let name = self.qname(n, "name")?;
        let id = *ids;
        *ids += 1;
        let mut def = MacroDef {
            id,
            loc: self.loc(n),
            name: Some(name),
            params: Vec::new(),
            slots: BTreeMap::new(),
            body: Vec::new(),
        };
        let mut content = false;
        for c in n.children() {
            if builtin(c, "param") {
                if content {
                    return Err(self
                        .loc(c)
                        .error("Syntax: parameters must precede macro content"));
                }
                self.param(c, &mut def.params)?;
            } else {
                if !trivia(c) {
                    content = true;
                }
                def.body.push(self.node(c, &mut def.slots)?);
            }
        }
        Ok(def)
    }
    /// 保留字符数据并解析嵌套指令。 / Preserve text and parse nested directives.
    fn body(
        &mut self,
        n: Xml<'_, '_>,
        slots: &mut BTreeMap<String, bool>,
    ) -> Result<Vec<Node>, CompileError> {
        n.children().map(|c| self.node(c, slots)).collect()
    }
    /// 验证 binding 路径语法。 / Validate scalar binding syntax.
    fn get(&self, n: Xml<'_, '_>) -> Result<String, CompileError> {
        let s = self.attr(n, "get")?;
        if !s
            .split_once('.')
            .is_some_and(|(scope, name)| matches!(scope, "arg" | "file" | "match") && ncname(name))
        {
            return Err(self
                .loc(n)
                .error("Scalar: get must name file.*, arg.*, or match.* binding"));
        }
        Ok(s)
    }
    /// 解析调用者作用域中的参数值。 / Parse an argument value in caller scope.
    fn value(
        &mut self,
        n: Xml<'_, '_>,
        slots: &mut BTreeMap<String, bool>,
    ) -> Result<Value, CompileError> {
        let forms = usize::from(n.has_attribute("value"))
            + usize::from(n.has_attribute("get"))
            + usize::from(n.children().next().is_some());
        if forms > 1 {
            return Err(self
                .loc(n)
                .error("Signature: argument requires exactly one of value, get, or body"));
        }
        if let Some(v) = n.attribute("value") {
            Ok(Value::Literal(v.into()))
        } else if n.has_attribute("get") {
            Ok(Value::Get(self.get(n)?))
        } else {
            Ok(Value::Body(self.body(n, slots)?))
        }
    }
    /// 解析静态调用与显式数据。 / Parse static invocation and explicit invocation data.
    fn invoke(
        &mut self,
        n: Xml<'_, '_>,
        slots: &mut BTreeMap<String, bool>,
    ) -> Result<Kind, CompileError> {
        let target = if builtin(n, "mount") {
            self.attrs(n, &["src"])?;
            Target::Source(self.source(n)?)
        } else {
            self.attrs(n, &["ref"])?;
            Target::Named(self.qname(n, "ref")?)
        };
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
                if !names.insert(name.clone()) {
                    return Err(self
                        .loc(c)
                        .error(format!("Signature: duplicate argument {name}")));
                }
                args.push(Argument {
                    name,
                    value: self.value(c, slots)?,
                    loc: self.loc(c),
                });
            } else if builtin(c, "fill") {
                self.attrs(c, &["name"])?;
                let name = self.local(c, "name")?;
                if !fill_names.insert(name.clone()) {
                    return Err(self
                        .loc(c)
                        .error(format!("Signature: duplicate fill {name}")));
                }
                fills.push(Fill {
                    name,
                    body: self.body(c, slots)?,
                    loc: self.loc(c),
                });
            } else {
                return Err(self
                    .loc(c)
                    .error("Syntax: call/mount children must be xs:arg or xs:fill"));
            }
        }
        Ok(Kind::Invoke {
            target,
            args,
            fills,
        })
    }
    /// 将 XML 节点转换为不可变编译节点。 / Convert XML nodes to immutable compiled nodes.
    fn node(
        &mut self,
        n: Xml<'_, '_>,
        slots: &mut BTreeMap<String, bool>,
    ) -> Result<Node, CompileError> {
        // XML 结构深度不是语言限制；按需增长解析栈而非拒绝深层文档。
        // XML structural depth is not a language limit; grow the parser stack on demand.
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.node_inner(n, slots))
    }
    /// 在受保护的栈段解析一个节点。 / Parse one node inside a protected stack segment.
    fn node_inner(
        &mut self,
        n: Xml<'_, '_>,
        slots: &mut BTreeMap<String, bool>,
    ) -> Result<Node, CompileError> {
        let kind = if n.is_text() {
            Kind::Text(n.text().unwrap_or("").into())
        } else if n.is_comment() {
            Kind::Comment(n.text().unwrap_or("").into())
        } else if let Some(pi) = n.pi() {
            Kind::Pi(match pi.value {
                Some(v) => format!("{} {}", pi.target, v),
                None => pi.target.into(),
            })
        } else if n.tag_name().namespace() != Some(NS) {
            let name = qualified(n, n.tag_name().namespace(), n.tag_name().name(), false);
            let mut attrs: Vec<_> = n
                .namespaces()
                .filter(|ns| ns.uri() != NS && ns.name() != Some("xml"))
                .map(|ns| {
                    (
                        ns.name()
                            .map_or_else(|| "xmlns".into(), |p| format!("xmlns:{p}")),
                        ns.uri().into(),
                    )
                })
                .collect();
            // 防止挂载后意外继承调用者默认命名空间。 / Prevent accidental default namespace inheritance after mounting.
            if !attrs.iter().any(|(name, _)| name == "xmlns") {
                attrs.push(("xmlns".into(), String::new()));
            }
            for a in n.attributes() {
                if a.namespace() == Some(NS) {
                    return Err(self
                        .loc(n)
                        .error("Namespace: internal attributes are reserved"));
                }
                attrs.push((
                    qualified(n, a.namespace(), a.name(), true),
                    a.value().into(),
                ));
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
                    Kind::Insert(self.get(n)?)
                }
                "slot" => {
                    self.attrs(n, &["name", "required"])?;
                    self.empty(n)?;
                    let name = self.local(n, "name")?;
                    let required = match n.attribute("required") {
                        None | Some("false") => false,
                        Some("true") => true,
                        _ => {
                            return Err(self
                                .loc(n)
                                .error("Syntax: required must be true or false"));
                        }
                    };
                    if slots.insert(name.clone(), required).is_some() {
                        return Err(self
                            .loc(n)
                            .error(format!("Signature: duplicate slot {name}")));
                    }
                    Kind::Slot { name, required }
                }
                "call" | "mount" => self.invoke(n, slots)?,
                "ifr" => {
                    self.attrs(n, &["get", "str", "pattern"])?;
                    if n.has_attribute("get") == n.has_attribute("str") {
                        return Err(self
                            .loc(n)
                            .error("Syntax: ifr requires exactly one of get or str"));
                    }
                    let input = if n.has_attribute("get") {
                        Value::Get(self.get(n)?)
                    } else {
                        Value::Literal(self.attr(n, "str")?)
                    };
                    let regex = regex::Regex::new(&self.attr(n, "pattern")?)
                        .map_err(|e| self.loc(n).error(format!("Regex: {e}")))?;
                    for capture in regex.capture_names().skip(1) {
                        if !capture.is_some_and(ncname) {
                            return Err(self.loc(n).error("Regex: only NCName named captures are permitted; positional captures are forbidden"));
                        }
                    }
                    Kind::If {
                        input,
                        regex,
                        body: self.body(n, slots)?,
                    }
                }
                other => {
                    return Err(self
                        .loc(n)
                        .error(format!("Syntax: unknown or misplaced xs:{other}")));
                }
            }
        };
        Ok(Node {
            loc: self.loc(n),
            kind,
        })
    }
}
/// 检查内建展开名。 / Check a builtin expanded name.
fn builtin(n: Xml<'_, '_>, name: &str) -> bool {
    n.is_element() && n.tag_name().namespace() == Some(NS) && n.tag_name().name() == name
}
/// 声明区和调用容器允许格式空白及注释。 / Ignore formatting trivia in declaration/call containers.
fn trivia(n: Xml<'_, '_>) -> bool {
    n.is_comment()
        || (n.is_text()
            && n.text()
                .unwrap_or("")
                .chars()
                .all(|c| matches!(c, ' ' | '\t' | '\r' | '\n')))
}
/// XML 1.0 NCName 字符范围。 / XML 1.0 NCName character ranges.
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
/// 重建有效前缀；每个输出元素携带其有效命名空间。 / Reconstruct prefixes with in-scope namespaces on every output element.
fn qualified(n: Xml<'_, '_>, uri: Option<&str>, local: &str, attribute: bool) -> String {
    match uri {
        None => local.into(),
        Some("http://www.w3.org/XML/1998/namespace") => format!("xml:{local}"),
        Some(uri) => {
            let prefix = n
                .namespaces()
                .find(|ns| ns.uri() == uri && (!attribute || ns.name().is_some()))
                .and_then(|ns| ns.name());
            prefix.map_or_else(|| local.into(), |p| format!("{p}:{local}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 包装测试模块。 / Wrap a test module with namespaces.
    fn unit(body: &str) -> Result<Unit, CompileError> {
        parse(
            &std::env::current_dir().unwrap().join("test/main.xml"),
            &format!(r#"<xs:module xmlns:xs="{NS}" xmlns:m="urn:macros">{body}</xs:module>"#),
            &mut 0,
        )
    }
    #[test]
    fn expanded_names_and_ids_are_stable() {
        let parsed=unit(r#"<xs:macro name="m:f"><xs:param name="p"/><xs:insert get="arg.p"/></xs:macro><Root/>"#).unwrap();
        assert_eq!(parsed.main, 0);
        assert_eq!(parsed.macros[1].id, 1);
        assert_eq!(
            parsed.macros[1].name,
            Some(("urn:macros".into(), "f".into()))
        );
    }
    #[test]
    fn invalid_static_syntax_is_rejected_even_in_dead_macro() {
        for bad in [
            r#"<xs:macro name="f"/>"#,
            r#"<Root/><xs:param name="x"/>"#,
            r#"<xs:macro name="m:f"><xs:ifr str="" pattern="(a)"/></xs:macro>"#,
            r#"<xs:call ref="m:f"><xs:arg name="x" value="a">b</xs:arg></xs:call>"#,
            r#"<xs:slot name="x"/><xs:slot name="x"/>"#,
            r#"<xs:insert get="slot.x"/>"#,
            r#"<xs:macro name="xs:f"/>"#,
        ] {
            assert!(unit(bad).is_err(), "accepted {bad}");
        }
    }
    #[test]
    fn dead_mounts_are_discovered_and_slots_are_lexical() {
        let parsed=unit(r#"<xs:macro name="m:f"><xs:ifr str="" pattern="a"><xs:mount src="lib/../part.xml"><xs:fill name="content"><xs:slot name="inner" required="true"/></xs:fill></xs:mount></xs:ifr></xs:macro><Root/>"#).unwrap();
        assert_eq!(parsed.references.len(), 1);
        assert_eq!(parsed.macros[1].slots.get("inner"), Some(&true));
    }
    #[test]
    fn scalar_body_retains_whitespace_and_entity_text() {
        let parsed =
            unit(r#"<xs:call ref="m:f"><xs:arg name="x"> &amp; </xs:arg></xs:call>"#).unwrap();
        let Kind::Invoke { args, .. } = &parsed.macros[0].body[0].kind else {
            panic!()
        };
        let Value::Body(body) = &args[0].value else {
            panic!()
        };
        assert!(matches!(&body[0].kind,Kind::Text(s) if s==" & "));
    }
    #[test]
    fn namespace_reset_and_document_processing_instructions_survive() {
        let text =
            format!(r#"<?before yes?><xs:module xmlns:xs="{NS}"><Root/></xs:module><?after?>"#);
        let parsed = parse(
            &std::env::current_dir().unwrap().join("test/main.xml"),
            &text,
            &mut 0,
        )
        .unwrap();
        assert!(matches!(&parsed.macros[0].body[0].kind,Kind::Pi(s) if s=="before yes"));
        let Kind::Element { attrs, .. } = &parsed.macros[0].body[1].kind else {
            panic!()
        };
        assert!(attrs.contains(&("xmlns".into(), "".into())));
        assert!(matches!(&parsed.macros[0].body[2].kind,Kind::Pi(s) if s=="after"));
    }
    #[test]
    fn xml_ncname_ranges_are_enforced() {
        assert!(ncname("参数"));
        assert!(ncname("a.b-1"));
        assert!(!ncname("1x"));
        assert!(!ncname("a:b"));
    }

    #[test]
    fn deeply_nested_xml_does_not_overflow_native_stack() {
        let depth = 6000;
        let xml = format!("{}leaf{}", "<a>".repeat(depth), "</a>".repeat(depth));
        let parsed = unit(&xml).unwrap();
        let mut node = &parsed.macros[0].body[0];
        for _ in 0..depth {
            let Kind::Element { children, .. } = &node.kind else {
                panic!("expected nested element");
            };
            node = &children[0];
        }
        assert!(matches!(&node.kind, Kind::Text(text) if text == "leaf"));
        // 同时覆盖深层 AST 的销毁路径。 / Also exercise destruction of the deep AST.
        drop(parsed);
    }

    #[test]
    fn deep_full_compilation_and_error_unwinding_are_stack_safe() {
        let depth = 6000;
        let body = format!("{}leaf{}", "<a>".repeat(depth), "</a>".repeat(depth));
        let source = format!(r#"<xs:module xmlns:xs="{NS}">{body}</xs:module>"#);
        let result = super::super::Compiler::default()
            .compile(Path::new("deep.xml"), &source, |_| unreachable!())
            .unwrap();
        assert_eq!(result.output.matches("</a>").count(), depth);
        let invalid = format!(
            "{body}{}<xs:unknown/>{}",
            "<a>".repeat(depth),
            "</a>".repeat(depth)
        );
        let error = unit(&invalid).unwrap_err();
        assert!(error.message.contains("unknown or misplaced"));
    }

    #[test]
    fn empty_argument_body_is_an_empty_scalar() {
        let parsed = unit(r#"<xs:call ref="m:f"><xs:arg name="x"/></xs:call>"#).unwrap();
        let Kind::Invoke { args, .. } = &parsed.macros[0].body[0].kind else {
            panic!()
        };
        assert!(matches!(&args[0].value, Value::Body(body) if body.is_empty()));
    }
}

#[cfg(test)]
#[path = "parser.contract.test.rs"]
mod contract_tests;
