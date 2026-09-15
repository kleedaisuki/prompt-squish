use std::fmt;

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
}

impl ManagerError {
    /// 创建可安全映射为 worker 失败和协议诊断的错误。 / Creates an error safely mappable to a worker failure and protocol diagnostic.
    pub fn new(code: impl Into<String>, phase: Phase, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            phase,
            message: message.into(),
        }
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
            primary: None,
            related: Vec::new(),
            help: None,
        }
    }
}

impl fmt::Display for ManagerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ManagerError {}
