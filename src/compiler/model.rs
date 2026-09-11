//! Frozen language definitions and logical source identity. / 冻结的语言定义与逻辑源码身份。
use super::CompileError;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};
/// Expanded XML name. / XML 扩展名。
pub(super) type Name = (String, String);
/// Original source span. / 原始源码区间。
#[derive(Debug)]
pub(super) struct Loc {
    /// Normalized logical loader path. / 规范化逻辑加载路径。
    pub path: PathBuf,
    /// One-based source line. / 从 1 开始的源码行号。
    pub line: usize,
    /// Inclusive UTF-8 byte offset. / UTF-8 字节起点（含）。
    pub start: usize,
    /// Exclusive UTF-8 byte offset. / UTF-8 字节终点（不含）。
    pub end: usize,
}
impl Loc {
    /// Attach a diagnostic to this span. / 将诊断关联到本区间。
    pub fn error(&self, message: impl Into<String>) -> CompileError {
        CompileError {
            path: self.path.clone(),
            line: self.line,
            message: format!(
                "{} [{}:{} bytes {}..{}]",
                message.into(),
                file_uri(&self.path).unwrap_or_else(|_| self.path.display().to_string()),
                self.line,
                self.start,
                self.end
            ),
        }
    }
}
/// Immutable executable syntax. / 不可变可执行语法。
#[derive(Debug)]
pub(super) struct Node {
    /// Immutable definition-site span. / 不可变定义位置区间。
    pub loc: Loc,
    /// Data or computation at this node. / 本节点的数据或计算操作。
    pub kind: Kind,
}
impl Drop for Node {
    /// Flatten child ownership before destruction, even for deeply authored XML.
    /// 销毁前展平子节点所有权，即使深层手写 XML 也不会递归耗尽本机栈。
    fn drop(&mut self) {
        let mut pending = Vec::new();
        detach_children(&mut self.kind, &mut pending);
        while let Some(mut node) = pending.pop() {
            detach_children(&mut node.kind, &mut pending);
        }
    }
}
/// Move nested nodes onto an explicit destruction stack. / 将嵌套节点移入显式销毁栈。
fn detach_children(kind: &mut Kind, pending: &mut Vec<Node>) {
    match kind {
        Kind::Element { children, .. } => pending.append(children),
        Kind::If { input, body, .. } => {
            pending.append(body);
            if let Value::Body(nodes) = input {
                pending.append(nodes);
            }
        }
        Kind::Invoke { args, fills, .. } => {
            for arg in args {
                if let Value::Body(nodes) = &mut arg.value {
                    pending.append(nodes);
                }
            }
            for fill in fills {
                pending.append(&mut fill.body);
            }
        }
        _ => {}
    }
}
/// XML data and compile-time operations. / XML 数据与编译期操作。
#[derive(Debug)]
pub(super) enum Kind {
    Text(String),
    Comment(String),
    Pi(String),
    Element {
        /// Serialized XML QName or local slot name. / 序列化 XML QName 或局部 slot 名。
        name: String,
        /// Decoded attributes including namespace fixup. / 解码属性及命名空间修复声明。
        attrs: Vec<(String, String)>,
        /// Ordered XML children. / 有序 XML 子节点。
        children: Vec<Node>,
    },
    Insert(String),
    If {
        /// Literal or lexical binding input. / 字面量或词法绑定输入。
        input: Value,
        /// Prevalidated non-backtracking regex. / 预校验的非回溯正则。
        regex: regex::Regex,
        /// Branch lexical body. / 分支词法主体。
        body: Vec<Node>,
    },
    Slot {
        /// Serialized XML QName or local slot name. / 序列化 XML QName 或局部 slot 名。
        name: String,
        /// Required-fill contract. / 必需填充契约。
        required: bool,
    },
    Invoke {
        /// Statically bound macro target. / 静态绑定宏目标。
        target: Target,
        /// Arguments evaluated before all fills. / 先于全部 fill 求值的实参。
        args: Vec<Argument>,
        /// Caller-evaluated XML sequences. / 调用者求值的 XML 序列。
        fills: Vec<Fill>,
    },
}
/// Scalar construction forms. / 标量构造形式。
#[derive(Debug)]
pub(super) enum Value {
    Literal(String),
    Get(String),
    Body(Vec<Node>),
}
/// Statically resolved invocation target. / 静态调用目标。
#[derive(Debug)]
pub(super) enum Target {
    Named(Name),
    /// Frozen definition index after whole-program linking. / 全程序链接后的冻结定义索引。
    Linked(usize),
}
/// Caller-evaluated scalar argument. / 在调用者求值的标量实参。
#[derive(Debug)]
pub(super) struct Argument {
    /// Local NCName identifier. / 局部 NCName 标识符。
    pub name: String,
    /// Caller-side scalar expression. / 调用者侧标量表达式。
    pub value: Value,
    /// Immutable definition-site span. / 不可变定义位置区间。
    pub loc: Loc,
}
/// Caller-evaluated XML sequence. / 在调用者求值的 XML 序列。
#[derive(Debug)]
pub(super) struct Fill {
    /// Local NCName identifier. / 局部 NCName 标识符。
    pub name: String,
    /// Ordered lexical body, never cloned during expansion. / 有序词法主体，展开时不复制。
    pub body: Vec<Node>,
    /// Immutable definition-site span. / 不可变定义位置区间。
    pub loc: Loc,
}
/// Interned macro definition and signature. / 驻留宏定义与签名。
#[derive(Debug)]
pub(super) struct MacroDef {
    /// Stable invocation-local definition index. / 单次编译内稳定定义索引。
    pub id: usize,
    /// Immutable definition-site span. / 不可变定义位置区间。
    pub loc: Loc,
    /// Explicit expanded macro symbol. / 显式宏扩展符号。
    pub name: Name,
    /// Required scalar parameter names. / 必需标量参数名。
    pub params: Vec<String>,
    /// Slot signature: true means required. / Slot 签名：true 表示必需。
    pub slots: BTreeMap<String, bool>,
    /// Ordered lexical body, never cloned during expansion. / 有序词法主体，展开时不复制。
    pub body: Vec<Node>,
}
/// One parsed logical resource. / 一个已解析逻辑资源。
#[derive(Debug)]
pub(super) struct Unit {
    /// Normalized logical loader path. / 规范化逻辑加载路径。
    pub path: PathBuf,
    /// Optional explicit entry symbol and declaration origin. / 可选显式入口符号及声明来源。
    pub entry: Option<(Name, Loc)>,
    /// Explicit named definitions in ID order. / 按 ID 排序的显式命名定义。
    pub macros: Vec<MacroDef>,
    /// Static source edges with diagnostic origins. / 带诊断来源的静态源码边。
    pub references: Vec<(PathBuf, Loc)>,
}
/// Fully discovered immutable program. / 完整发现的不可变程序。
#[derive(Debug)]
pub(super) struct Program {
    /// Frozen definitions indexed by ID. / 按 ID 索引的冻结定义。
    pub defs: Vec<MacroDef>,
    /// Global expanded-name symbol table. / 全局扩展名符号表。
    pub symbols: BTreeMap<Name, usize>,
    /// Explicit root entry definition ID. / 显式根入口定义 ID。
    pub root: usize,
    /// Root module entry declaration, distinct from the macro definition. / 根模块入口声明，与宏定义位置分离。
    pub entry_loc: Loc,
}
/// Normalize lexically without filesystem canonicalization or symlink folding.
/// 仅词法规范化，不访问文件系统或折叠符号链接。
pub(super) fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if result.file_name().is_some_and(|n| n != "..") {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            _ => result.push(part.as_os_str()),
        }
    }
    result
}
/// Resolve supported file-loader references at their definition site.
/// 按定义位置解析文件加载器支持的引用。
pub(super) fn resolve_path(base: &Path, src: &str) -> Result<PathBuf, String> {
    if src.is_empty() || src.chars().any(char::is_control) {
        return Err("Source: src must be a nonempty static URI without control characters".into());
    }
    let base = url::Url::from_file_path(normalize(base))
        .map_err(|()| "Source: definition base is not an absolute file path".to_owned())?;
    let resolved = base
        .join(src)
        .map_err(|error| format!("Source: invalid URI: {error}"))?;
    if resolved.scheme() != "file" {
        return Err(format!(
            "Source: unsupported URI scheme '{}' for file loader",
            resolved.scheme()
        ));
    }
    if resolved.query().is_some() || resolved.fragment().is_some() {
        return Err("Source: file loader does not support URI queries or fragments".into());
    }
    let path = resolved
        .to_file_path()
        .map_err(|()| "Source: file URI cannot be represented as a native path".to_owned())?;
    Ok(normalize(&path))
}

/// Serialize logical file identity without resolving physical aliases.
/// 序列化逻辑文件身份，不解析物理文件别名；调用方保证使用绝对路径。
pub(super) fn file_uri(path: &Path) -> Result<String, String> {
    url::Url::from_file_path(normalize(path))
        .map(String::from)
        .map_err(|()| "Source: logical source path cannot be represented as a file URI".into())
}

/// Definition base URI with a trailing slash for relative source resolution.
/// 定义位置的基 URI，末尾斜杠保证相对源码解析语义。
pub(super) fn directory_uri(path: &Path) -> Result<String, String> {
    let uri = file_uri(path)?;
    url::Url::parse(&uri)
        .and_then(|uri| uri.join("."))
        .map(String::from)
        .map_err(|error| format!("Source: invalid definition base URI: {error}"))
}
