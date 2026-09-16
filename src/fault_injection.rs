//! 仅测试构建启用的进程死亡注入适配器。 /
//! Process-death injection adapters enabled only in test builds.

use std::{ffi::OsString, fmt, io, sync::Arc};

use squish_manager::DurabilityPorts;
use squish_publish::{DurablePoint, NoopObserver, PublishEvent, PublishObserver};
use squish_repository::{FaultInjector, FaultPoint, NoFault};

use crate::FaultPorts;

const SELECTOR_VARIABLE: &str = "XMLSQUISH_TEST_PROCESS_EXIT_AT";
const PROCESS_DEATH_EXIT_CODE: i32 = 86;

/// 故障选择器配置错误。 / Fault-selector configuration error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FaultConfigurationError(String);

impl fmt::Display for FaultConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for FaultConfigurationError {}

#[derive(Clone, Copy)]
enum Selection {
    ArtifactGeneration(DurablePoint),
    BuildCatalog(DurablePoint),
    Repository(FaultPoint),
}

/// 从已捕获的环境快照解析一次封闭选择器。 /
/// Parses the closed selector once from an already captured environment snapshot.
pub fn from_environment(
    environment: &[(OsString, OsString)],
) -> Result<FaultPorts, FaultConfigurationError> {
    let value = environment
        .iter()
        .find(|(name, _)| name == SELECTOR_VARIABLE)
        .map(|(_, value)| value);
    let Some(value) = value else {
        return Ok(FaultPorts::default());
    };
    let value = value.to_str().ok_or_else(|| {
        FaultConfigurationError(format!("{SELECTOR_VARIABLE} must contain valid Unicode"))
    })?;
    let selection = parse(value)?;
    Ok(match selection {
        Selection::ArtifactGeneration(point) => FaultPorts {
            durability: DurabilityPorts::new(Arc::new(NoFault)),
            target_observer: Arc::new(ExitPublisher(point)),
            catalog_observer: Arc::new(NoopObserver),
        },
        Selection::BuildCatalog(point) => FaultPorts {
            durability: DurabilityPorts::new(Arc::new(NoFault)),
            target_observer: Arc::new(NoopObserver),
            catalog_observer: Arc::new(ExitPublisher(point)),
        },
        Selection::Repository(point) => FaultPorts {
            durability: DurabilityPorts::new(Arc::new(ExitRepository(point))),
            target_observer: Arc::new(NoopObserver),
            catalog_observer: Arc::new(NoopObserver),
        },
    })
}

fn parse(value: &str) -> Result<Selection, FaultConfigurationError> {
    match value {
        "artifact-generation.generation-staged" => Ok(Selection::ArtifactGeneration(
            DurablePoint::GenerationStaged,
        )),
        "artifact-generation.commit-decision" => {
            Ok(Selection::ArtifactGeneration(DurablePoint::CommitDecision))
        }
        "build-catalog.commit-decision" => {
            Ok(Selection::BuildCatalog(DurablePoint::CommitDecision))
        }
        "repository.candidates-staged" => Ok(Selection::Repository(FaultPoint::CandidatesStaged)),
        "repository.commit-decided" => Ok(Selection::Repository(FaultPoint::CommitDecided)),
        "repository.creation-prepared" => Ok(Selection::Repository(FaultPoint::CreationPrepared)),
        "repository.creation-published" => Ok(Selection::Repository(FaultPoint::CreationPublished)),
        "repository.creation-workspace-replaced" => {
            Ok(Selection::Repository(FaultPoint::CreationWorkspaceReplaced))
        }
        "repository.creation-completed" => Ok(Selection::Repository(FaultPoint::CreationCompleted)),
        _ => Err(FaultConfigurationError(format!(
            "unsupported {SELECTOR_VARIABLE} value `{value}`"
        ))),
    }
}

struct ExitPublisher(DurablePoint);

impl PublishObserver for ExitPublisher {
    fn observe(&self, event: &PublishEvent) {
        if event == &PublishEvent::DurablePoint(self.0) {
            std::process::exit(PROCESS_DEATH_EXIT_CODE);
        }
    }
}

struct ExitRepository(FaultPoint);

impl FaultInjector for ExitRepository {
    fn check(&self, point: FaultPoint) -> io::Result<()> {
        if point == self.0 {
            std::process::exit(PROCESS_DEATH_EXIT_CODE);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_language_is_closed() {
        let environment = vec![(
            OsString::from(SELECTOR_VARIABLE),
            OsString::from("unknown.scope"),
        )];
        let error = from_environment(&environment).err().unwrap();
        assert!(error.to_string().contains("unsupported"));
    }

    #[test]
    fn selector_language_covers_project_creation_boundaries() {
        for selector in [
            "repository.creation-prepared",
            "repository.creation-published",
            "repository.creation-workspace-replaced",
            "repository.creation-completed",
        ] {
            assert!(matches!(parse(selector), Ok(Selection::Repository(_))));
        }
    }
}
