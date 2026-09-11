//! Frozen-source XML macro compiler. / 冻结源码的 XML 宏编译器。
mod model;
mod parser;
mod runtime;
use model::*;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    fmt,
    path::{Path, PathBuf},
};
/// Source-located diagnostic log. / 带源码位置的诊断日志。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileLog {
    /// Logical source path. / 逻辑源码路径。
    pub path: PathBuf,
    /// One-based line. / 从 1 开始的行号。
    pub line: usize,
    /// Diagnostic message. / 诊断消息。
    pub message: String,
}
/// Semantic output and separately serialized provenance. / 语义输出与独立序列化的来源信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileResult {
    /// Clean, well-formed XML document. / 清理后的良构 XML 文档。
    pub output: String,
    /// XML provenance view, never substituted for output. / XML 来源视图，不替代输出。
    pub intermediate: String,
    /// Ordered diagnostic logs. / 有序诊断日志。
    pub logs: Vec<CompileLog>,
    /// Actual root frame context for downstream output-stage failures.
    /// 下游输出阶段失败使用的真实根帧上下文。
    root_context: CompileError,
}
impl CompileResult {
    /// Attach a post-expansion output error to the recorded root invocation.
    /// 将展开后的输出错误关联到已记录的根调用，无需重新解析源码或来源 IR。
    pub fn output_error(&self, message: impl AsRef<str>) -> CompileError {
        let mut error = self.root_context.clone();
        error.message = format!("{}{}", message.as_ref(), error.message);
        error
    }
}
/// Fatal source-located diagnostic. / 带源码位置的致命诊断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    /// Logical source path. / 逻辑源码路径。
    pub path: PathBuf,
    /// One-based source line. / 从 1 开始的源码行号。
    pub line: usize,
    /// Error category, reason and expansion chain. / 错误类别、原因与展开链。
    pub message: String,
}
impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {}",
            file_uri(&self.path).unwrap_or_else(|_| self.path.display().to_string()),
            self.line,
            self.message
        )
    }
}
impl Error for CompileError {}
/// Invocation budgets and explicit root arguments. / 单次调用预算与显式根参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOptions {
    /// Maximum simultaneously active macro frames. / 最大同时活动宏帧数。
    pub max_depth: usize,
    /// Maximum total macro frame creations. / 最大宏帧创建总数。
    pub max_expansions: usize,
    /// Maximum serialized bytes of the final output and each temporary argument/fill sequence.
    /// 最终输出及每个临时 argument/fill 序列的最大序列化字节数。
    /// This is not a cumulative allocation or provenance-IR memory limit.
    /// 这不是累计分配量或来源 IR 的内存上限。
    pub max_output_bytes: usize,
    /// Explicit entry macro scalar parameters. / 显式入口宏的标量参数。
    pub args: BTreeMap<String, String>,
}
impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            max_depth: 1024,
            max_expansions: 100_000,
            max_output_bytes: 64 * 1024 * 1024,
            args: BTreeMap::new(),
        }
    }
}
/// Reusable configuration; source snapshots are per compile call.
/// 可复用配置；源码快照仅属于单次编译。
#[derive(Default)]
pub struct Compiler {
    options: CompileOptions,
}
impl Compiler {
    /// Construct with explicit budgets and root parameters. / 使用显式预算与根参数构造。
    pub fn with_options(options: CompileOptions) -> Self {
        Self { options }
    }
    /// Discover and freeze the complete source closure before expansion.
    /// 展开前发现并冻结完整源码闭包；加载器按逻辑路径每次最多调用一次。
    ///
    /// The loader receives normalized logical paths, never symlink-resolved paths.
    /// 加载器接收规范化逻辑路径，不解析符号链接。
    pub fn compile<F>(
        &self,
        path: &Path,
        source: &str,
        mut loader: F,
    ) -> Result<CompileResult, CompileError>
    where
        F: FnMut(&Path) -> Result<String, String>,
    {
        let path = normalize(&if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| CompileError {
                    path: path.to_path_buf(),
                    line: 1,
                    message: format!("Source: {e}"),
                })?
                .join(path)
        });
        let program = discover(&path, source, &mut loader)?;
        runtime::expand(&program, &self.options)
    }
}
/// Classify a syntactically valid library without loading imports or executing macros.
/// 不加载导入或展开宏，仅将语法有效且未声明入口的模块识别为库。
pub(crate) fn is_library(path: &Path, source: &str) -> Result<bool, CompileError> {
    Ok(parser::parse(path, source, &mut 0)?.entry.is_none())
}

/// Intern definitions before traversing edges, so import cycles terminate.
/// 遍历引用前驻留定义，因此 import 环能正常终止。
fn discover<F>(path: &Path, source: &str, loader: &mut F) -> Result<Program, CompileError>
where
    F: FnMut(&Path) -> Result<String, String>,
{
    let mut defs: Vec<MacroDef> = Vec::new();
    let mut symbols: BTreeMap<Name, usize> = BTreeMap::new();
    let mut entries = Vec::new();
    let mut root_entry = None;
    let mut next_id = 0;
    let mut seen = BTreeSet::from([path.to_path_buf()]);
    let mut pending = VecDeque::from([(path.to_path_buf(), source.to_owned())]);
    while let Some((source_path, text)) = pending.pop_front() {
        let unit = parser::parse(&source_path, &text, &mut next_id)?;
        if unit.path == path {
            root_entry = unit.entry;
            if root_entry.is_none() {
                return Err(CompileError {
                    path: path.into(),
                    line: 1,
                    message: "Signature: root module requires an explicit entry QName".into(),
                });
            }
        } else {
            entries.extend(unit.entry);
        }
        for def in unit.macros {
            let name = &def.name;
            if let Some(previous) = symbols.get(name) {
                let first = &defs[*previous];
                return Err(def.loc.error(format!("Namespace: macro redefinition {{{}}}{}; first definition {}:{}; second definition {}:{}",name.0,name.1,first.loc.path.display(),first.loc.line,def.loc.path.display(),def.loc.line)));
            }
            symbols.insert(name.clone(), def.id);
            defs.push(def);
        }
        for (target, loc) in unit.references {
            if seen.insert(target.clone()) {
                let bytes = loader(&target).map_err(|e| {
                    loc.error(format!("Source: cannot load {}: {e}", target.display()))
                })?;
                pending.push_back((target, bytes));
            }
        }
    }
    defs.sort_by_key(|def| def.id);
    let (root_name, entry_loc) = root_entry.expect("root entry checked during discovery");
    for (name, loc) in entries
        .iter()
        .map(|(name, loc)| (name, loc))
        .chain(std::iter::once((&root_name, &entry_loc)))
    {
        if !symbols.contains_key(name) {
            return Err(loc.error(format!(
                "Namespace: unresolved entry {{{}}}{}",
                name.0, name.1
            )));
        }
    }
    let mut program = Program {
        root: symbols[&root_name],
        entry_loc,
        defs,
        symbols,
    };
    validate_expansions(&program)?;
    link_expansions(&mut program);
    Ok(program)
}
/// Validate every expansion before linking, including unreachable definitions.
/// 链接前校验全部展开，包括不可达定义。
fn validate_expansions(program: &Program) -> Result<(), CompileError> {
    let mut pending: Vec<&Node> = program.defs.iter().flat_map(|d| d.body.iter()).collect();
    while let Some(node) = pending.pop() {
        match &node.kind {
            Kind::Element { children, .. } => pending.extend(children),
            Kind::If { input, body, .. } => {
                pending.extend(body);
                if let Value::Body(nodes) = input {
                    pending.extend(nodes);
                }
            }
            Kind::Invoke {
                target,
                args,
                fills,
            } => {
                let id = match target {
                    Target::Named(name) => program.symbols.get(name),
                    Target::Linked(id) => Some(id),
                }
                .ok_or_else(|| {
                    node.loc
                        .error(format!("Namespace: unresolved static target {target:?}"))
                })?;
                let def = &program.defs[*id];
                let supplied: BTreeSet<_> = args.iter().map(|a| a.name.as_str()).collect();
                let expected: BTreeSet<_> = def.params.iter().map(String::as_str).collect();
                let supplied_slots: BTreeSet<_> = fills.iter().map(|f| f.name.as_str()).collect();
                let bad_slots = supplied_slots.iter().any(|n| !def.slots.contains_key(*n))
                    || def
                        .slots
                        .iter()
                        .any(|(n, required)| *required && !supplied_slots.contains(n.as_str()));
                if supplied != expected
                    || supplied.len() != args.len()
                    || bad_slots
                    || supplied_slots.len() != fills.len()
                {
                    return Err(node.loc.error(format!("Signature: missing, unknown or duplicate argument/fill; supplied {supplied:?}, expected {expected:?}; call site {}:{}; definition site {}:{}",node.loc.path.display(),node.loc.line,def.loc.path.display(),def.loc.line)));
                }
                for arg in args {
                    if let Value::Body(body) = &arg.value {
                        pending.extend(body);
                    }
                }
                for fill in fills {
                    pending.extend(&fill.body);
                }
            }
            _ => {}
        }
    }
    Ok(())
}
/// Freeze all symbolic expansion edges into direct definition indices.
/// 将全部符号展开边冻结为直接定义索引；运行时无需名称查询。
fn link_expansions(program: &mut Program) {
    let mut pending: Vec<&mut Node> = program
        .defs
        .iter_mut()
        .flat_map(|def| def.body.iter_mut())
        .collect();
    while let Some(node) = pending.pop() {
        match &mut node.kind {
            Kind::Element { children, .. } => pending.extend(children),
            Kind::If { input, body, .. } => {
                pending.extend(body);
                if let Value::Body(nodes) = input {
                    pending.extend(nodes);
                }
            }
            Kind::Invoke {
                target,
                args,
                fills,
            } => {
                if let Target::Named(name) = target {
                    *target = Target::Linked(program.symbols[name]);
                }
                for arg in args {
                    if let Value::Body(body) = &mut arg.value {
                        pending.extend(body);
                    }
                }
                for fill in fills {
                    pending.extend(&mut fill.body);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;

#[cfg(test)]
#[path = "source.test.rs"]
mod source_tests;
