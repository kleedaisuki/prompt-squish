//! prompt-squish 的 XML 前端领域。 / XML frontend domain for prompt-squish.
//!
//! 此 crate 是一个纯前端：它仅观察已冻结的 [`squish_source::SourceBlob`]，
//! 绝不读取文件、解析依赖或执行宏。导入作为带范围的可重定位声明保留，
//! 供后续 linker 绑定。
//! This crate is a pure frontend: it only observes a frozen
//! [`squish_source::SourceBlob`] and never reads files, resolves dependencies, or executes
//! macros. Imports remain spanned relocatable declarations for a later linker to bind.

#![forbid(unsafe_code)]

mod ast;
mod lower;
mod parser;
#[cfg(test)]
mod tests;

use squish_ir::{
    PackageInstanceId, RelocatableUnitIr, Validate, decode_relocatable_unit,
    encode_relocatable_unit,
};
use squish_protocol::{Diagnostic, DiagnosticId, Phase, Severity, Span};
use squish_source::SourceBlob;

/// 当前 XML DSL 命名空间。 / Current XML DSL namespace.
pub const DSL_NAMESPACE: &str = "https://xmlsquish.moesegfault.dev/ns";

/// 前端成功产物。 / Successful frontend product.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrontendOutput {
    /// 可持久化、可重定位的语义 IR。 / Persistent relocatable semantic IR.
    pub unit: RelocatableUnitIr,
}

/// Resolver 为前端冻结的源上下文。 / Resolver-frozen source context for the frontend.
///
/// path/workspace 来源应使用 manifest identity，registry/Git 来源应使用
/// lock 中的 exact revision。前端绝不用包名伪造 revision。
/// Path/workspace sources use manifest identity, while registry/Git sources use the lock's
/// exact revision. The frontend never fabricates a revision from a package name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrontendSourceContext {
    package_instance: PackageInstanceId,
}

impl FrontendSourceContext {
    /// 包装 resolver 已选定的精确包实例。 / Wraps the exact resolver-selected package instance.
    #[must_use]
    pub const fn new(package_instance: PackageInstanceId) -> Self {
        Self { package_instance }
    }

    /// 返回精确包实例。 / Returns the exact package instance.
    #[must_use]
    pub const fn package_instance(&self) -> &PackageInstanceId {
        &self.package_instance
    }
}

/// 编译一个已冻结 XML 单元。 / Compiles one frozen XML unit.
///
/// 成功返回前，IR 会经过结构 verifier 和二进制 codec round-trip；因此调用者
/// 不会观察到只能在缓存重载时才暴露的前端错误。
/// Before success, the IR passes structural verification and a binary-codec round trip, so a
/// caller cannot observe frontend output that fails only after a cache reload.
///
/// # Errors
///
/// 返回稳定 code 和字节 span 的结构化诊断。 / Returns a structured diagnostic with a stable
/// code and byte span.
pub fn compile(
    source: &SourceBlob,
    context: &FrontendSourceContext,
) -> Result<FrontendOutput, Box<Diagnostic>> {
    if context.package_instance.package_name != source.id().package().as_str() {
        return Err(Box::new(Diagnostic {
            id: DiagnosticId::new("xml-front-source-context")
                .expect("static diagnostic id is valid"),
            code: "XS1700".into(),
            severity: Severity::Error,
            phase: Phase::Analyze,
            message: format!(
                "resolved package {:?} does not own source package {:?}",
                context.package_instance.package_name,
                source.id().package().as_str()
            ),
            primary: None,
            related: Vec::new(),
            help: Some("pass the resolver-selected PackageInstanceId for this source".into()),
        }));
    }
    let parsed = parser::parse(source)?;
    let unit = lower::lower(source, context, parsed)
        .map_err(|message| Box::new(internal(source, message)))?;
    unit.validate()
        .map_err(|error| Box::new(internal(source, format!("IR verification failed: {error}"))))?;
    let bytes = encode_relocatable_unit(&unit);
    let decoded = decode_relocatable_unit(&bytes).map_err(|error| {
        Box::new(internal(
            source,
            format!("IR codec round-trip failed: {error}"),
        ))
    })?;
    if decoded != unit {
        return Err(Box::new(internal(
            source,
            "IR codec round-trip changed the unit",
        )));
    }
    Ok(FrontendOutput { unit })
}

fn internal(source: &SourceBlob, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        id: DiagnosticId::new("xml-front-internal").expect("static diagnostic id is valid"),
        code: "XS0000".into(),
        severity: Severity::Error,
        phase: Phase::Analyze,
        message: message.into(),
        primary: Span::new(source.id().to_protocol(), 0, source.bytes().len() as u64).ok(),
        related: Vec::new(),
        help: Some("please report this frontend invariant failure".into()),
    }
}
