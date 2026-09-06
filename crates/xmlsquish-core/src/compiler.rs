//! Compile-time XML macros; ordinary markup remains byte-for-byte intact.
//! 编译期 XML 宏；普通标记保持原始字节，每次文件引入使用独立变量环境。
//!
//! `version="adaptive"` selects the current language; `warnings="strict"` is
//! the only diagnostic policy (errors are never recovered). Both are also file
//! metadata. Other values are rejected rather than silently approximated.
//! 当前只支持 adaptive 语言与 strict 诊断策略，二者同时可作为文件元数据读取。
//!
//! Definitions and `set` assignments execute left-to-right. Assignment requires
//! an existing current-file local; builtin namespaces are immutable.
//! let 定义与 set 赋值从左到右执行；赋值仅允许已定义的当前文件局部变量。
//! Only macro and xmlsquish PI attributes expand `$name` / `$namespace:name`;
//! `$$` denotes a literal dollar. Expanded values are not recursively expanded.
//! 定义从左到右执行；仅编译语法展开变量，$$ 表示美元符号，展开值不会再次展开。
//!
//! `sys:time` is Unix seconds captured at construction; environment values are
//! captured at the same time. Includes resolve relative to their containing file.
//! `sys:time` 为创建编译器时的 Unix 秒数；环境变量同时快照，引入路径相对当前文件。
//!
//! Comments and processing instructions are removed. Only the exact xmlsquish
//! PI target executes. Ordinary DOCTYPE bytes are retained in standalone output,
//! but no external entities or DTD resources are ever loaded. Includes attach
//! only the root (mount) or its contents (import), not the document prolog.
//! 删除注释及处理指令，只有精确的 xmlsquish 目标被执行；不读取任何外部实体。
//! 主文件保留 DOCTYPE 字节，引入文件仅挂载根或其内容，不挂载文档序言。
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileLog {
    pub path: PathBuf,
    pub line: usize,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileResult {
    pub output: String,
    pub logs: Vec<CompileLog>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub path: PathBuf,
    pub line: usize,
    pub message: String,
}
impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.path.display(), self.line, self.message)
    }
}
impl Error for CompileError {}

/// Immutable system/environment snapshot shared by one compilation batch.
/// 一批编译共享的只读系统和环境快照。
pub struct Compiler {
    sys: HashMap<String, String>,
    env: HashMap<String, String>,
}
impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}
impl Compiler {
    pub fn new() -> Self {
        let platform = match std::env::consts::OS {
            "windows" => "win32",
            "macos" => "darwin",
            other => other,
        };
        Self {
            sys: HashMap::from([
                ("platform".into(), platform.into()),
                ("os".into(), std::env::consts::OS.into()),
                ("arch".into(), std::env::consts::ARCH.into()),
                (
                    "time".into(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .to_string(),
                ),
            ]),
            env: std::env::vars_os()
                .filter_map(|(k, v)| Some((env_key(&k.into_string().ok()?), v.into_string().ok()?)))
                .collect(),
        }
    }
    pub fn compile(
        &self,
        path: &Path,
        source: &str,
        mut loader: impl FnMut(&Path) -> Result<String, String>,
    ) -> Result<CompileResult, CompileError> {
        let mut state = RunState::default();
        let output = self.file(path, source, &mut loader, &mut state, false)?;
        Ok(CompileResult {
            output,
            logs: state.logs,
        })
    }
    fn file(
        &self,
        path: &Path,
        source: &str,
        loader: &mut impl FnMut(&Path) -> Result<String, String>,
        state: &mut RunState,
        children_only: bool,
    ) -> Result<String, CompileError> {
        // Resolve real-file aliases to detect symlink cycles; virtual loaders fall back
        // to lexical normalization. 真实路径用于检测符号链接循环，虚拟加载器退回词法路径。
        let identity = path.canonicalize().unwrap_or_else(|_| normalize(path));
        if state.paths.contains(&identity) {
            return Err(error(path, 1, "include cycle detected"));
        }
        if state.paths.len() >= 128 {
            return Err(error(path, 1, "include nesting exceeds 128 files"));
        }
        state.paths.push(identity);
        let mut nodes = parse(path, source, state.paths.len() > 1)?;
        if state.paths.len() > 1 {
            nodes.retain(|n| !matches!(n.kind, Kind::Raw(_)));
        }
        let mut frame = Frame {
            path,
            locals: HashMap::new(),
            metadata: HashMap::from([
                (
                    "name".into(),
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                ),
                ("path".into(), path.to_string_lossy().into_owned()),
                (
                    "dir".into(),
                    path.parent()
                        .unwrap_or(Path::new("."))
                        .to_string_lossy()
                        .into_owned(),
                ),
            ]),
        };
        let result = self.render(&nodes, &mut frame, loader, state, children_only);
        state.paths.pop();
        result
    }
    fn render(
        &self,
        nodes: &[Node],
        frame: &mut Frame<'_>,
        loader: &mut impl FnMut(&Path) -> Result<String, String>,
        state: &mut RunState,
        unwrap_root: bool,
    ) -> Result<String, CompileError> {
        if state.depth >= 128 {
            return Err(error(
                frame.path,
                nodes.first().map_or(1, |n| n.line),
                "combined render nesting exceeds 128 levels",
            ));
        }
        state.depth += 1;
        let result = self.render_nodes(nodes, frame, loader, state, unwrap_root);
        state.depth -= 1;
        result
    }
    fn render_nodes(
        &self,
        nodes: &[Node],
        frame: &mut Frame<'_>,
        loader: &mut impl FnMut(&Path) -> Result<String, String>,
        state: &mut RunState,
        unwrap_root: bool,
    ) -> Result<String, CompileError> {
        let mut out = String::new();
        for node in nodes {
            match &node.kind {
                Kind::Raw(raw) => out.push_str(raw),
                Kind::Metadata(attrs) => self.define(attrs, true, frame, node.line)?,
                Kind::Element { name, .. } if name.starts_with("xmlsquish:") => {
                    out.push_str(&self.eval_macro(node, frame, loader, state)?);
                }
                Kind::Element {
                    open,
                    close,
                    children,
                    ..
                } => {
                    if !unwrap_root {
                        out.push_str(open);
                    }
                    out.push_str(&self.render(children, frame, loader, state, false)?);
                    if !unwrap_root {
                        out.push_str(close);
                    }
                }
            }
        }
        Ok(out)
    }
    fn eval_macro(
        &self,
        node: &Node,
        frame: &mut Frame<'_>,
        loader: &mut impl FnMut(&Path) -> Result<String, String>,
        state: &mut RunState,
    ) -> Result<String, CompileError> {
        let Kind::Element {
            name,
            attrs,
            children,
            ..
        } = &node.kind
        else {
            unreachable!()
        };
        let mut out = String::new();
        let fail = |message: &str| error(frame.path, node.line, message);
        let macro_name = &name[10..];
        let allowed: &[&str] = match macro_name {
            "let" | "set" => &[],
            "log" => &["msg"],
            "if" | "ifn" => &["lhs", "rhs"],
            "mount" | "import" => &["path"],
            _ => return Err(fail(&format!("unknown macro '{name}'"))),
        };
        if !matches!(macro_name, "let" | "set")
            && (attrs.len() != allowed.len()
                || attrs.iter().any(|(k, _)| !allowed.contains(&k.as_str())))
        {
            return Err(fail(&format!(
                "{name} requires attributes {}",
                allowed.join(", ")
            )));
        }
        if !matches!(macro_name, "if" | "ifn")
            && children
                .iter()
                .any(|n| !matches!(&n.kind, Kind::Raw(s) if s.trim().is_empty()))
        {
            return Err(fail("macro does not accept children"));
        }
        if matches!(macro_name, "let" | "set") {
            if macro_name == "let" {
                self.define(attrs, false, frame, node.line)?;
            } else {
                self.assign(attrs, frame, node.line)?;
            }
            return Ok(out);
        }
        let values = attrs
            .iter()
            .map(|(k, v)| Ok((k.clone(), self.expand(v, frame, node.line)?)))
            .collect::<Result<HashMap<_, _>, CompileError>>()?;
        match macro_name {
            "log" => state.logs.push(CompileLog {
                path: frame.path.into(),
                line: node.line,
                message: values["msg"].clone(),
            }),
            "if" | "ifn" => {
                if (values["lhs"] == values["rhs"]) == (macro_name == "if") {
                    out.push_str(&self.render(children, frame, loader, state, false)?);
                }
            }
            "mount" | "import" => {
                let path = normalize(
                    &frame
                        .path
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(&values["path"]),
                );
                let source = loader(&path)
                    .map_err(|e| fail(&format!("cannot load {}: {e}", path.display())))?;
                out.push_str(&self.file(&path, &source, loader, state, macro_name == "import")?);
            }
            _ => unreachable!(),
        }
        Ok(out)
    }
    fn assign(
        &self,
        attrs: &[(String, String)],
        frame: &mut Frame<'_>,
        line: usize,
    ) -> Result<(), CompileError> {
        for (name, value) in attrs {
            if !valid_name(name) || name.contains(':') {
                return Err(error(
                    frame.path,
                    line,
                    "assignment requires an unqualified local variable name",
                ));
            }
            if !frame.locals.contains_key(name) {
                return Err(error(
                    frame.path,
                    line,
                    &format!("undefined variable '${name}'"),
                ));
            }
            let value = self.expand(value, frame, line)?;
            frame.locals.insert(name.clone(), value);
        }
        Ok(())
    }
    fn define(
        &self,
        attrs: &[(String, String)],
        metadata: bool,
        frame: &mut Frame<'_>,
        line: usize,
    ) -> Result<(), CompileError> {
        for (name, value) in attrs {
            if !valid_name(name) || name.contains(':') {
                return Err(error(
                    frame.path,
                    line,
                    "definition requires an unqualified variable name",
                ));
            }
            let value = self.expand(value, frame, line)?;
            if metadata
                && ((name == "version" && value != "adaptive")
                    || (name == "warnings" && value != "strict"))
            {
                return Err(error(
                    frame.path,
                    line,
                    &format!("unsupported {name} mode '{value}'"),
                ));
            }
            let map = if metadata {
                &mut frame.metadata
            } else {
                &mut frame.locals
            };
            if map.contains_key(name) {
                return Err(error(
                    frame.path,
                    line,
                    &format!("duplicate definition '{name}'"),
                ));
            }
            map.insert(name.clone(), value);
        }
        Ok(())
    }
    fn expand(&self, value: &str, frame: &Frame<'_>, line: usize) -> Result<String, CompileError> {
        let mut out = String::new();
        let mut rest = value;
        while let Some(pos) = rest.find('$') {
            out.push_str(&rest[..pos]);
            rest = &rest[pos + 1..];
            if let Some(tail) = rest.strip_prefix('$') {
                out.push('$');
                rest = tail;
                continue;
            }
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-' | '.')))
                .unwrap_or(rest.len());
            let name = &rest[..end];
            if !valid_name(name) {
                return Err(error(frame.path, line, "invalid variable reference"));
            }
            let value = match name.split_once(':') {
                None => frame.locals.get(name),
                Some(("file", key)) => frame.metadata.get(key),
                Some(("sys", key)) => self.sys.get(key),
                Some(("env", key)) => self.env.get(&env_key(key)),
                _ => None,
            }
            .ok_or_else(|| error(frame.path, line, &format!("undefined variable '${name}'")))?;
            out.push_str(value);
            rest = &rest[end..];
        }
        out.push_str(rest);
        Ok(out)
    }
}
fn env_key(name: &str) -> String {
    if cfg!(windows) {
        name.to_ascii_uppercase()
    } else {
        name.to_owned()
    }
}
fn valid_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if out.file_name().is_some_and(|s| s != "..") => {
                out.pop();
            }
            _ => out.push(c.as_os_str()),
        }
    }
    out
}
fn error(path: &Path, line: usize, message: &str) -> CompileError {
    CompileError {
        path: path.into(),
        line,
        message: message.into(),
    }
}
#[derive(Default)]
struct RunState {
    paths: Vec<PathBuf>,
    logs: Vec<CompileLog>,
    depth: usize,
}
struct Frame<'a> {
    path: &'a Path,
    locals: HashMap<String, String>,
    metadata: HashMap<String, String>,
}
struct Node {
    line: usize,
    kind: Kind,
}
enum Kind {
    Raw(String),
    Metadata(Vec<(String, String)>),
    Element {
        name: String,
        attrs: Vec<(String, String)>,
        open: String,
        close: String,
        children: Vec<Node>,
    },
}
fn attributes(
    event: &BytesStart<'_>,
    path: &Path,
    line: usize,
) -> Result<Vec<(String, String)>, CompileError> {
    event
        .attributes()
        .map(|a| {
            let a = a.map_err(|e| error(path, line, &e.to_string()))?;
            let key = std::str::from_utf8(a.key.as_ref())
                .map_err(|e| error(path, line, &e.to_string()))?
                .to_owned();
            let value = a
                .unescape_value()
                .map_err(|e| error(path, line, &e.to_string()))?
                .into_owned();
            Ok((key, value))
        })
        .collect()
}
fn parse(path: &Path, source: &str, require_root: bool) -> Result<Vec<Node>, CompileError> {
    let mut reader = Reader::from_str(source);
    let mut levels: Vec<Vec<Node>> = vec![Vec::new()];
    let mut roots = 0;
    let mut next_line = 1;
    loop {
        let start = reader.buffer_position() as usize;
        let line = next_line;
        let event = reader
            .read_event()
            .map_err(|e| error(path, line, &e.to_string()))?;
        let end = reader.buffer_position() as usize;
        let raw = &source[start..end];
        next_line += raw
            .bytes()
            .enumerate()
            .filter(|(i, b)| {
                *b == b'\r'
                    || (*b == b'\n'
                        && (start + i == 0 || source.as_bytes()[start + i - 1] != b'\r'))
            })
            .count();
        let kind = match event {
            Event::Eof => break,
            Event::Comment(_) | Event::Decl(_) => continue,
            Event::PI(pi) if pi.target() == b"xmlsquish" => {
                let content = std::str::from_utf8(pi.as_ref())
                    .map_err(|e| error(path, line, &e.to_string()))?;
                Kind::Metadata(attributes(
                    &BytesStart::from_content(content, 9),
                    path,
                    line,
                )?)
            }
            Event::PI(_) => continue,
            Event::Start(ref tag) | Event::Empty(ref tag) => {
                if levels.len() == 1 {
                    roots += 1;
                }
                if levels.len() > 256 {
                    return Err(error(path, line, "XML nesting exceeds 256 elements"));
                }
                let name = std::str::from_utf8(tag.name().as_ref())
                    .map_err(|e| error(path, line, &e.to_string()))?
                    .to_owned();
                if require_root && levels.len() == 1 && name.starts_with("xmlsquish:") {
                    return Err(error(
                        path,
                        line,
                        "included root must be an ordinary XML element",
                    ));
                }
                // Decode only compiler syntax: payload entities remain lexical.
                // 只解码编译器语法，正文实体保留原样。
                let attrs = if name.starts_with("xmlsquish:") {
                    attributes(tag, path, line)?
                } else {
                    Vec::new()
                };
                let node = Node {
                    line,
                    kind: Kind::Element {
                        name,
                        attrs,
                        open: raw.into(),
                        close: String::new(),
                        children: Vec::new(),
                    },
                };
                levels.last_mut().unwrap().push(node);
                if matches!(event, Event::Start(_)) {
                    levels.push(Vec::new());
                }
                continue;
            }
            Event::End(_) => {
                if levels.len() == 1 {
                    return Err(error(path, line, "unexpected closing tag"));
                }
                let children = levels.pop().unwrap();
                if let Kind::Element {
                    close,
                    children: target,
                    ..
                } = &mut levels.last_mut().unwrap().last_mut().unwrap().kind
                {
                    *close = raw.into();
                    *target = children;
                }
                continue;
            }
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_)
                if require_root && levels.len() == 1 && !raw.trim().is_empty() =>
            {
                return Err(error(path, line, "text outside XML root"));
            }
            _ => Kind::Raw(raw.into()),
        };
        levels.last_mut().unwrap().push(Node { line, kind });
    }
    if levels.len() != 1 {
        return Err(error(path, 1, "unclosed XML element"));
    }
    if require_root && roots != 1 {
        return Err(error(path, 1, "expected exactly one XML root element"));
    }
    Ok(levels.pop().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn compile(source: &str) -> Result<CompileResult, CompileError> {
        Compiler::new().compile(Path::new("prompt.xml"), source, |_| {
            Err("unexpected load".into())
        })
    }
    #[test]
    fn metadata_and_macros_are_removed_without_touching_payload() {
        let r = compile("<?xml version=\"1.0\"?><?xmlsquish author='klee'?><?xmlsquish?><r a='$missing'><!--gone--><xmlsquish:let msg='Hello &amp; 世界'/><xmlsquish:log msg='$msg'/><x>$msg</x><![CDATA[$missing]]></r>").unwrap();
        assert_eq!(
            r.output,
            "<r a='$missing'><x>$msg</x><![CDATA[$missing]]></r>"
        );
        assert_eq!(r.logs[0].message, "Hello & 世界");
    }
    #[test]
    fn execution_is_ordered_and_false_branches_are_lazy() {
        let r = compile("<r><xmlsquish:let a='yes' b='$a'/><xmlsquish:if lhs='$a' rhs='$b'><xmlsquish:let result='OK'/><ok/></xmlsquish:if><xmlsquish:ifn lhs='$a' rhs='yes'><xmlsquish:log msg='$missing'/></xmlsquish:ifn><xmlsquish:log msg='$result'/></r>").unwrap();
        assert_eq!(r.output, "<r><ok/></r>");
        assert_eq!(r.logs[0].message, "OK");
    }
    #[test]
    fn diagnostics_report_lines_and_definition_failures() {
        let e = compile("<r>\n<xmlsquish:log msg='$missing'/></r>").unwrap_err();
        assert_eq!(e.line, 2);
        assert!(e.message.contains("undefined"));
        assert!(
            compile("<r><xmlsquish:let a='1'/><xmlsquish:let a='2'/></r>")
                .unwrap_err()
                .message
                .contains("duplicate")
        );
        assert!(
            compile("<?xmlsquish name='other'?><r/>")
                .unwrap_err()
                .message
                .contains("duplicate")
        );
        assert!(compile("<r><xmlsquish:nope/></r>").is_err());
        assert!(compile("<r><xmlsquish:log wrong='x'/></r>").is_err());
    }
    #[test]
    fn includes_have_independent_frames_and_relative_paths() {
        let source = "<r><xmlsquish:let x='parent'/><xmlsquish:mount path='sub/a.xml'/><xmlsquish:import path='sub/a.xml'/><xmlsquish:log msg='$x'/></r>";
        let r = Compiler::new().compile(Path::new("base/p.xml"), source, |p| {
            assert_eq!(p, Path::new("base/sub/a.xml"));
            Ok("<?xmlsquish who='child'?><child><xmlsquish:let x='child'/><xmlsquish:log msg='$file:name'/><a/><b/></child>".into())
        }).unwrap();
        assert_eq!(r.output, "<r><child><a/><b/></child><a/><b/></r>");
        assert_eq!(r.logs.len(), 3);
        assert_eq!(r.logs[0].message, "a.xml");
        assert_eq!(r.logs[2].message, "parent");
    }
    #[test]
    fn child_cannot_see_parent_locals() {
        let e = Compiler::new()
            .compile(
                Path::new("p.xml"),
                "<r><xmlsquish:let x='parent'/><xmlsquish:mount path='a.xml'/></r>",
                |_| Ok("<r><xmlsquish:log msg='$x'/></r>".into()),
            )
            .unwrap_err();
        assert_eq!(e.path, Path::new("a.xml"));
        assert!(e.message.contains("undefined"));
    }
    #[test]
    fn detects_cycles_and_invalid_included_documents() {
        let s = "<r><xmlsquish:mount path='./p.xml'/></r>";
        assert!(
            Compiler::new()
                .compile(Path::new("p.xml"), s, |_| Ok(s.into()))
                .unwrap_err()
                .message
                .contains("cycle")
        );
        assert!(
            Compiler::new()
                .compile(
                    Path::new("p.xml"),
                    "<r><xmlsquish:mount path='a.xml'/></r>",
                    |_| Ok("<a/><b/>".into())
                )
                .is_err()
        );
    }
    #[test]
    fn fragments_and_lexical_markup_remain_compatible() {
        assert_eq!(
            compile("<a  x = 'y'/> <b/>").unwrap().output,
            "<a  x = 'y'/> <b/>"
        );
        assert!(compile("<a><b></a>").is_err());
        assert!(compile("<a>").is_err());
    }
    #[test]
    fn snapshot_and_metadata_expansion() {
        let c = Compiler::new();
        let s = "<?xmlsquish date='$sys:time'?><r><xmlsquish:log msg='$file:date'/><xmlsquish:log msg='$$literal'/></r>";
        let a = c
            .compile(Path::new("p.xml"), s, |_| unreachable!())
            .unwrap();
        let b = c
            .compile(Path::new("p.xml"), s, |_| unreachable!())
            .unwrap();
        assert_eq!(a.logs, b.logs);
        assert_eq!(a.logs[1].message, "$literal");
    }
    #[test]
    fn xml_line_endings_count_cr_and_crlf_once() {
        for newline in ["\r", "\n", "\r\n"] {
            let source = format!(
                "<r>{newline}<!-- a{newline}b -->{newline}<xmlsquish:log msg='$missing'/></r>"
            );
            assert_eq!(compile(&source).unwrap_err().line, 4);
        }
    }
    #[test]
    fn environment_snapshot_is_read_only_and_missing_is_an_error() {
        let c = Compiler {
            sys: HashMap::new(),
            env: HashMap::from([(env_key("http_proxy"), "example".into())]),
        };
        let run = |s: &str| c.compile(Path::new("p.xml"), s, |_| unreachable!());
        assert_eq!(
            run("<r><xmlsquish:log msg='$env:http_proxy'/></r>")
                .unwrap()
                .logs[0]
                .message,
            "example"
        );
        assert!(
            run("<r><xmlsquish:log msg='$env:missing'/></r>")
                .unwrap_err()
                .message
                .contains("undefined")
        );
        for name in ["env:http_proxy", "sys:time", "file:name"] {
            assert!(run(&format!("<r><xmlsquish:let {name}='bad'/></r>")).is_err());
        }
        assert_eq!(c.env[&env_key("http_proxy")], "example");
    }
    #[test]
    fn pragma_modes_are_explicit_and_fake_targets_do_not_execute() {
        let r = compile("<?xmlsquish version='adaptive' warnings='strict'?><?xmlsquish-other invalid='$missing'?><r><xmlsquish:log msg='$file:version/$file:warnings'/></r>").unwrap();
        assert_eq!(r.logs[0].message, "adaptive/strict");
        for attr in ["version='future'", "warnings='ignore'"] {
            assert!(
                compile(&format!("<?xmlsquish {attr}?><r/>"))
                    .unwrap_err()
                    .message
                    .contains("unsupported")
            );
        }
    }
    #[test]
    fn skipped_branches_never_load_files_and_dollar_values_are_not_recursive() {
        let r = compile("<r><xmlsquish:if lhs='a' rhs='b'><xmlsquish:mount path='$missing'/></xmlsquish:if><xmlsquish:let a='$$missing'/><xmlsquish:log msg='$a $$$$'/></r>").unwrap();
        assert_eq!(r.output, "<r></r>");
        assert_eq!(r.logs[0].message, "$missing $$");
        assert!(compile("<r><xmlsquish:log msg='$'/></r>").is_err());
    }
    #[test]
    fn malformed_macro_arguments_are_rejected() {
        for body in [
            "<xmlsquish:if lhs='a'/>",
            "<xmlsquish:log msg='a' extra='b'/>",
            "<xmlsquish:mount/>",
            "<xmlsquish:let a='1' a='2'/>",
            "<xmlsquish:log msg='a'>text</xmlsquish:log>",
            "<xmlsquish:let a='1'><child/></xmlsquish:let>",
        ] {
            assert!(compile(&format!("<r>{body}</r>")).is_err(), "{body}");
        }
    }
    #[test]
    fn includes_reject_outside_entities_and_duplicate_metadata() {
        for child in [
            "&amp;<r/>",
            "<r/>&amp;",
            "<?xmlsquish x='a'?><?xmlsquish x='b'?><r/>",
            "<xmlsquish:if lhs='a' rhs='a'><r/></xmlsquish:if>",
        ] {
            assert!(
                Compiler::new()
                    .compile(
                        Path::new("p.xml"),
                        "<r><xmlsquish:import path='child.xml'/></r>",
                        |_| Ok(child.into())
                    )
                    .is_err(),
                "{child}"
            );
        }
    }
    #[test]
    fn nesting_limits_fail_without_stack_overflow() {
        let deep = format!("{}{}", "<r>".repeat(257), "</r>".repeat(257));
        assert!(compile(&deep).unwrap_err().message.contains("nesting"));
        let mut n = 0;
        let error = Compiler::new()
            .compile(
                Path::new("p.xml"),
                "<r><xmlsquish:mount path='1.xml'/></r>",
                |_| {
                    n += 1;
                    Ok(format!("<r><xmlsquish:mount path='{}.xml'/></r>", n + 1))
                },
            )
            .unwrap_err();
        assert!(error.message.contains("nesting"));
    }
    #[test]
    fn assignment_updates_only_existing_locals_left_to_right() {
        let r=compile("<r><xmlsquish:let a='old' b='old'/><xmlsquish:set a='new' b='$a'/><xmlsquish:log msg='$a/$b'/></r>").unwrap();
        assert_eq!(r.logs[0].message, "new/new");
        assert_eq!(r.output, "<r></r>");
        for body in [
            "<xmlsquish:set a='new'/>",
            "<xmlsquish:set sys:time='x'/>",
            "<xmlsquish:set env:PATH='x'/>",
            "<xmlsquish:set file:name='x'/>",
        ] {
            assert!(compile(&format!("<r>{body}</r>")).is_err());
        }
        let e = Compiler::new()
            .compile(
                Path::new("p.xml"),
                "<r><xmlsquish:let a='parent'/><xmlsquish:mount path='child.xml'/></r>",
                |_| Ok("<r><xmlsquish:set a='child'/></r>".into()),
            )
            .unwrap_err();
        assert!(e.message.contains("undefined"));
    }
    #[test]
    fn combined_include_and_element_depth_is_bounded() {
        let source = format!(
            "{}<xmlsquish:mount path='child.xml'/>{}",
            "<r>".repeat(80),
            "</r>".repeat(80)
        );
        let child = format!("{}{}", "<r>".repeat(80), "</r>".repeat(80));
        let e = Compiler::new()
            .compile(Path::new("p.xml"), &source, |_| Ok(child.clone()))
            .unwrap_err();
        assert!(e.message.contains("combined render nesting"));
    }
    #[test]
    fn environment_key_comparison_matches_platform() {
        let c = Compiler {
            sys: HashMap::new(),
            env: HashMap::from([(env_key("HTTP_PROXY"), "proxy".into())]),
        };
        let result = c.compile(
            Path::new("p.xml"),
            "<r><xmlsquish:log msg='$env:http_proxy'/></r>",
            |_| unreachable!(),
        );
        if cfg!(windows) {
            assert_eq!(result.unwrap().logs[0].message, "proxy");
        } else {
            assert!(result.is_err());
        }
    }
}
