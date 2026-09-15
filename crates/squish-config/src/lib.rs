//! xmlsquish 管理器的确定性配置领域。 / Deterministic configuration domain for the xmlsquish manager.
//!
//! The core performs only caller-directed filesystem reads. It never discovers a
//! platform home, reads environment variables, contacts a registry, or obtains a
//! credential. Callers select the home and workspace and inject CLI overrides.

#![forbid(unsafe_code)]

mod error;
mod model;

pub use error::{ConfigError, ConfigLayer, SourceLocation};
pub use model::{
    AuthScope, BuildConfig, ColorPolicy, Config, ConfigHome, ConfigLoader, EffectiveConfig,
    ExplainEntry, ManagerConfig, MessageFormat, ProgressPolicy, Provenance, Registry, RegistryId,
    RegistryIndex, SourceConfig, TermConfig, Verbosity,
};
