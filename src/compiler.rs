//! Compile-time XML macros; ordinary markup remains byte-for-byte intact.
//! 编译期 XML 宏；普通标记保持原始字节，每次文件引入使用独立变量环境。
//!
//! `version="adaptive"` selects the current language; `warnings="strict"` is
//! the only diagnostic policy (errors are never recovered). Both are also file
//! metadata in the `meta` namespace. Physical name/path/dir stay in `file`.
//! `openat="parent"` overlays parent effective metadata on that child. Every include
//! edge defaults to `self`; inheritance requires an explicit parent edge each time.
//! 元数据属于 meta，物理文件信息属于 file；每条引入边独立指定 parent，省略默认为 self。
//! mount's optional `rename` expands in the caller and changes only the root name.
//! mount 可选 rename 在调用方展开，仅改根名，保留属性、子树及物理文件身份。
//! Regex `pattern` and insert `get` are typed literal parameters, not interpolated.
//! `ifr` searches (use anchors for full matches); insert escapes XML text and accepts
//! an unprefixed variable name such as `meta:author`, never a dollar expression.
//! pattern 为字面正则，get 为不带美元符号的变量名；insert 输出转义后的 XML 文本。
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
use regex::RegexBuilder;
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// A source-located compile-time log. / 带源码位置的编译期日志。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileLog {
    /// Originating document path. / 来源文档路径。
    pub path: PathBuf,
    /// One-based source line. / 从 1 开始的源码行号。
    pub line: usize,
    /// Human-readable message. / 可读消息。
    pub message: String,
}
/// Compiled XML and ordered logs. / 编译后的 XML 与按执行顺序排列的日志。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileResult {
    /// XML with compile-time syntax resolved. / 已消除编译期语法的 XML。
    pub output: String,
    /// Logs in evaluation order. / 按求值顺序排列的日志。
    pub logs: Vec<CompileLog>,
}
/// A fatal error at its originating file and line. / 错误来源文件及行号对应的致命错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    /// Originating document path. / 来源文档路径。
    pub path: PathBuf,
    /// One-based source line. / 从 1 开始的源码行号。
    pub line: usize,
    /// Human-readable message. / 可读消息。
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
    /// Captures the current environment and system values for reuse.
    /// 快照当前环境与系统值，供后续编译复用。
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
    /// Compiles one document with fresh locals and caller-controlled includes.
    /// 以独立局部变量编译一个文档；引入文件由调用方加载。
    ///
    /// The loader receives paths resolved relative to the including document.
    /// Its failures become source-located compilation errors; no output is returned
    /// on failure. See the crate-level example for an in-memory invocation.
    /// 加载器接收相对引入文档解析后的路径；失败转换为带源码位置的错误，
    /// 不返回部分产物。内存调用示例见 crate 顶层文档。
    pub fn compile(
        &self,
        path: &Path,
        source: &str,
        mut loader: impl FnMut(&Path) -> Result<String, String>,
    ) -> Result<CompileResult, CompileError> {
        let mut state = RunState::default();
        let output = self.file(
            path,
            source,
            &mut loader,
            &mut state,
            IncludeContext::default(),
        )?;
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
        context: IncludeContext,
    ) -> Result<String, CompileError> {
        // A BOM is a file envelope, not part of a mounted subtree or tag name.
        // BOM 属于文件编码外壳，不能成为挂载子树或根名称区间的一部分。
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
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
        if let Some(rename) = &context.rename {
            rename_root(&mut nodes, rename);
        }
        let mut frame = Frame {
            path,
            locals: HashMap::new(),
            physical: HashMap::from([
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
            metadata: HashMap::new(),
            inherited: context.metadata,
        };
        let result = self.render(&nodes, &mut frame, loader, state, context.children_only);
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
            "mount" => &["path", "openat", "rename"],
            "import" => &["path", "openat"],
            "ifr" => &["str", "pattern"],
            "insert" => &["get"],
            _ => return Err(fail(&format!("unknown macro '{name}'"))),
        };
        if !matches!(macro_name, "let" | "set")
            && (allowed
                .iter()
                .filter(|key| !matches!(**key, "openat" | "rename"))
                .any(|key| !attrs.iter().any(|(name, _)| name == key))
                || attrs.iter().any(|(k, _)| !allowed.contains(&k.as_str())))
        {
            return Err(fail(&format!(
                "{name} requires attributes {}",
                allowed
                    .iter()
                    .copied()
                    .filter(|key| !matches!(*key, "openat" | "rename"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if !matches!(macro_name, "if" | "ifn" | "ifr")
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
            .map(|(k, v)| {
                let value = if (macro_name == "ifr" && k == "pattern") || macro_name == "insert" {
                    v.clone()
                } else {
                    self.expand(v, frame, node.line)?
                };
                Ok((k.clone(), value))
            })
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
            "ifr" => {
                let regex = RegexBuilder::new(&values["pattern"])
                    .size_limit(10 * 1024 * 1024)
                    .dfa_size_limit(2 * 1024 * 1024)
                    .build()
                    .map_err(|e| fail(&format!("invalid regex: {e}")))?;
                if regex.is_match(&values["str"]) {
                    out.push_str(&self.render(children, frame, loader, state, false)?);
                }
            }
            "insert" => {
                let value = self.lookup(&values["get"], frame, node.line)?;
                if !value.chars().all(is_xml_char) {
                    return Err(fail(
                        "insert value contains a character forbidden by XML 1.0",
                    ));
                }
                out.push_str(
                    &value
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;"),
                );
            }
            "mount" | "import" => {
                let rename = values.get("rename").cloned();
                if let Some(name) = &rename
                    && !valid_root_name(name)
                {
                    return Err(fail(&format!(
                        "invalid mount rename '{name}': expected an ordinary XML QName"
                    )));
                }
                let inherit = match values.get("openat").map(String::as_str) {
                    None | Some("self") => false,
                    Some("parent") => true,
                    Some(other) => {
                        return Err(fail(&format!(
                            "invalid openat '{other}': expected self or parent"
                        )));
                    }
                };
                let metadata = if inherit {
                    frame.effective_metadata()
                } else {
                    HashMap::new()
                };
                let path = normalize(
                    &frame
                        .path
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(&values["path"]),
                );
                let source = loader(&path)
                    .map_err(|e| fail(&format!("cannot load {}: {e}", path.display())))?;
                out.push_str(&self.file(
                    &path,
                    &source,
                    loader,
                    state,
                    IncludeContext {
                        children_only: macro_name == "import",
                        metadata,
                        rename,
                    },
                )?);
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
            let effective = frame.inherited.get(name).unwrap_or(&value);
            if metadata
                && ((name == "version" && effective != "adaptive")
                    || (name == "warnings" && effective != "strict"))
            {
                return Err(error(
                    frame.path,
                    line,
                    &format!("unsupported {name} mode '{effective}'"),
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
    fn lookup<'a>(
        &'a self,
        name: &str,
        frame: &'a Frame<'_>,
        line: usize,
    ) -> Result<&'a str, CompileError> {
        if !valid_name(name) {
            return Err(error(frame.path, line, "invalid variable reference"));
        }
        match name.split_once(':') {
            None => frame.locals.get(name),
            Some(("file", key)) => frame.physical.get(key),
            Some(("meta", key)) => frame.inherited.get(key).or_else(|| frame.metadata.get(key)),
            Some(("sys", key)) => self.sys.get(key),
            Some(("env", key)) => self.env.get(&env_key(key)),
            _ => None,
        }
        .map(String::as_str)
        .ok_or_else(|| error(frame.path, line, &format!("undefined variable '${name}'")))
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
            let value = self.lookup(name, frame, line)?;
            out.push_str(value);
            rest = &rest[end..];
        }
        out.push_str(rest);
        Ok(out)
    }
}
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
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
    physical: HashMap<String, String>,
    metadata: HashMap<String, String>,
    inherited: HashMap<String, String>,
}
impl Frame<'_> {
    fn effective_metadata(&self) -> HashMap<String, String> {
        let mut metadata = self.metadata.clone();
        metadata.extend(self.inherited.clone());
        metadata
    }
}
#[derive(Default)]
struct IncludeContext {
    children_only: bool,
    metadata: HashMap<String, String>,
    rename: Option<String>,
}

/// Replace only name spans, never serialize attributes or rewrite descendants.
/// 仅替换已解析根的名称区间，不重新序列化属性，也不改写后代。
fn rename_root(nodes: &mut [Node], replacement: &str) {
    for node in nodes {
        if let Kind::Element {
            name, open, close, ..
        } = &mut node.kind
        {
            open.replace_range(1..1 + name.len(), replacement);
            if !close.is_empty() {
                close.replace_range(2..2 + name.len(), replacement);
            }
            *name = replacement.into();
            return;
        }
    }
}

fn valid_root_name(name: &str) -> bool {
    let mut parts = name.split(':');
    let first = parts.next().unwrap_or_default();
    if !valid_ncname(first) {
        return false;
    }
    match parts.next() {
        None => true,
        Some(local) => {
            !matches!(first, "xmlsquish" | "xmlns") && valid_ncname(local) && parts.next().is_none()
        }
    }
}

fn valid_ncname(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_name_start)
        && chars.all(|c| {
            is_name_start(c)
                || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
        })
}

// XML 1.0 Fifth Edition NameStartChar, excluding colon for NCName.
// XML 1.0 第五版名称首字符；NCName 不允许冒号。
fn is_name_start(c: char) -> bool {
    matches!(c, 'A'..='Z' | '_' | 'a'..='z' | '\u{c0}'..='\u{d6}'
        | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}' | '\u{370}'..='\u{37d}'
        | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}'
        | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}'
        | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
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
#[path = "compiler.test.rs"]
mod tests;
