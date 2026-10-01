use std::{fmt, sync::Arc};

use squish_build::WorkerFailure;
use squish_protocol::{Diagnostic, DiagnosticId, Phase, Severity};

/// 外部服务端口报告的可诊断失败。 / Diagnosable failure reported by an external service port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceError {
    code: String,
    message: String,
}

impl ServiceError {
    /// 创建稳定代码和用户消息。 / Creates an error from a stable code and user message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// 返回稳定机器代码。 / Returns the stable machine code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// 返回用户消息。 / Returns the user-facing message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ServiceError {}

/// 管理器的结构化领域失败。 / Structured manager-domain failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagerError {
    code: String,
    phase: Phase,
    message: String,
    diagnostic: Option<Arc<Diagnostic>>,
}

impl ManagerError {
    /// 创建可安全映射为 worker 失败和协议诊断的错误。 / Creates an error safely mappable to a worker failure and protocol diagnostic.
    pub fn new(code: impl Into<String>, phase: Phase, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            phase,
            message: message.into(),
            diagnostic: None,
        }
    }

    /// Retains source evidence while preserving the manager error code and phase.
    pub fn with_diagnostic(mut self, diagnostic: Diagnostic) -> Self {
        self.diagnostic = Some(Arc::new(diagnostic));
        self
    }

    /// Returns upstream source evidence, if the failing compiler stage provided it.
    pub fn source_diagnostic(&self) -> Option<&Diagnostic> {
        self.diagnostic.as_deref()
    }

    /// 返回稳定机器代码。 / Returns the stable machine code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// 返回失败阶段。 / Returns the failure phase.
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// 返回用户消息。 / Returns the user-facing message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// 转换为 worker 失败。 / Converts to a worker failure.
    pub fn worker_failure(&self) -> WorkerFailure {
        WorkerFailure::new(self.code.clone(), self.message.clone())
    }

    /// 转换为指定实例 ID 的协议诊断。 / Converts to a protocol diagnostic with the given instance ID.
    pub fn diagnostic(&self, id: DiagnosticId) -> Diagnostic {
        Diagnostic {
            id,
            code: self.code.clone(),
            severity: Severity::Error,
            phase: self.phase,
            message: self.message.clone(),
            primary: self.diagnostic.as_ref().and_then(|d| d.primary.clone()),
            related: self
                .diagnostic
                .as_ref()
                .map(|d| d.related.clone())
                .unwrap_or_default(),
            help: self.diagnostic.as_ref().and_then(|d| d.help.clone()),
        }
    }
}

impl fmt::Display for ManagerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ManagerError {}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_protocol::{OpaqueSourceId, RelatedSpan, Span};

    #[test]
    fn compiler_source_evidence_preserves_manager_failure_contract() {
        let span = Span::new(
            OpaqueSourceId::new("sopack://digest/lib.xml").unwrap(),
            1,
            9,
        )
        .unwrap();
        let upstream = Diagnostic {
            id: DiagnosticId::new("compiler-diagnostic").unwrap(),
            code: "RUN013".into(),
            severity: Severity::Error,
            phase: Phase::Instantiate,
            message: "compiler budget".into(),
            primary: Some(span.clone()),
            related: vec![RelatedSpan {
                span,
                label: "call".into(),
            }],
            help: Some("choose a finite expansion".into()),
        };
        let error = ManagerError::new("MGB073", Phase::Instantiate, "manager budget")
            .with_diagnostic(upstream.clone());
        let emitted = error.diagnostic(DiagnosticId::new("action-failure").unwrap());
        assert_eq!(emitted.code, "MGB073");
        assert_eq!(emitted.message, "manager budget");
        assert_eq!(emitted.primary, upstream.primary);
        assert_eq!(emitted.related, upstream.related);
        assert_eq!(emitted.help, upstream.help);
        assert_eq!(error.worker_failure().code, "MGB073");
    }
}
