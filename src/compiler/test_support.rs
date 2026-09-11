//! Test-only compact fixtures materialize a real entry and an imported macro module.
//! 紧凑测试样例生成真实入口与被导入的宏模块，不放宽生产语法。
use super::{CompileError, CompileOptions, CompileResult, Compiler};
use std::path::Path;

/// Wrap compact fixture source; this is not accepted by the production parser.
/// 包装紧凑样例；该测试容器不被生产解析器接受。
pub(super) fn fixture(body: &str) -> String {
    format!(
        r#"<fixture xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test">{body}</fixture>"#
    )
}

/// Adapt fixture setup only, preserving the real compiler and loader contracts.
/// 仅适配样例准备过程，保留真实编译器及加载器契约。
#[derive(Default)]
pub(super) struct TestCompiler {
    /// Compiler under test. / 待测编译器。
    compiler: Compiler,
}
impl TestCompiler {
    /// Configure real execution budgets and root arguments. / 配置真实执行限制与入口参数。
    pub(super) fn with_options(options: CompileOptions) -> Self {
        Self {
            compiler: Compiler::with_options(options),
        }
    }

    /// Materialize an isolated macro library, then invoke the production compiler.
    /// 生成隔离的宏库，然后调用生产编译器。
    pub(super) fn compile<F>(
        &self,
        path: &Path,
        source: &str,
        mut loader: F,
    ) -> Result<CompileResult, CompileError>
    where
        F: FnMut(&Path) -> Result<String, String>,
    {
        if !source.starts_with("<fixture ") {
            return self.compiler.compile(path, source, loader);
        }
        let doc = roxmltree::Document::parse(source).expect("well-formed test fixture");
        let mut macros = String::new();
        let mut content = String::new();
        for node in doc.root_element().children() {
            let target = if node.has_tag_name(("https://xmlsquish.moesegfault.dev/ns", "macro")) {
                &mut macros
            } else {
                &mut content
            };
            target.push_str(&source[node.range()]);
        }
        let ns = r#"xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test""#;
        let import = if macros.is_empty() {
            ""
        } else {
            r#"<xs:import src="__fixture_macros.xml"/>"#
        };
        let entry = format!("<xs:entry {ns}>{import}{content}</xs:entry>");
        let library = format!("<xs:module {ns}>{macros}</xs:module>");
        self.compiler.compile(path, &entry, |p| {
            if !macros.is_empty()
                && p.file_name()
                    .is_some_and(|name| name == "__fixture_macros.xml")
            {
                Ok(library.clone())
            } else {
                loader(p)
            }
        })
    }
}
