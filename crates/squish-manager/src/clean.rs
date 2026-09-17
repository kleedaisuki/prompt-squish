//! 项目私有构建状态与失效依赖缓存的安全清理。 / Safe cleaning of project-private build state and invalid dependency caches.

use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};

use squish_build::{
    Action, ActionResult, BuildPlan, Dispatch, KeyRecipe, ResourceClass, Resources,
};
use squish_kernel::{CancellationToken, InvocationContext, OperationOutcome};
use squish_protocol::{
    ActionId, ActionKind, ActionTotals, CleanRequest, CleanResult, Digest, DigestAlgorithm, JobId,
    OperationKind, OperationResult, Phase, PlanMode, PlanningAttemptId, PlanningStepId,
    PlanningStepKind,
};
use squish_repository::{Discovery, ProjectRepository};

use crate::{
    Effect, InvocationSettings, ManagerError, PlannedWork, PreparedPlan, ProjectCleanStatus,
    Services,
    orchestrator::{
        self, CachedResult, PlanningRecorder, ResolvedInputs, WorkDisposition, WorkExecutor,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CleanWork {
    ProjectState,
}

impl PlannedWork for CleanWork {
    fn kind(&self) -> ActionKind {
        ActionKind::Clean
    }

    fn effect(&self) -> Effect {
        Effect::WriteEffect
    }
}

fn prepare(project: &str) -> Result<PreparedPlan<CleanWork>, ManagerError> {
    let id = ActionId::new("clean.10.project-state").expect("static action ID is non-empty");
    let key = KeyRecipe::new(
        "xmlsquish-clean-v1",
        Vec::new(),
        BTreeMap::from([("project".into(), project.to_owned())]),
    )
    .map_err(|error| manager_error("XS3602", "invalid clean action key", error))?;
    let graph = BuildPlan::new([Action {
        id: id.clone(),
        key,
        kind: ActionKind::Clean,
        class: ResourceClass::Io,
        resources: Resources::new(0, 1, 0),
        dependencies: Vec::new(),
        outputs: Vec::new(),
    }])
    .map_err(|error| manager_error("XS3602", "invalid clean plan", error))?;
    PreparedPlan::new(graph, BTreeMap::from([(id, CleanWork::ProjectState)]))
        .map_err(|error| manager_error("XS3602", "invalid clean work mapping", error))
}

/// 通过统一规划和动作生命周期执行一次项目清理。 / Executes one project clean through the unified planning and action lifecycle.
pub fn execute<S: Services>(
    request: &CleanRequest,
    services: &S,
    settings: &InvocationSettings,
    context: &InvocationContext,
) -> OperationOutcome {
    let job = JobId::new(format!("clean:{}", context.id())).expect("invocation ID is non-empty");
    let attempt = PlanningAttemptId::new("clean-attempt-1").expect("static attempt is non-empty");
    let mut planning = match PlanningRecorder::start(job.clone(), attempt, context) {
        Ok(planning) => planning,
        Err(_) => return unavailable(job, context),
    };
    let repository = match planning.step(step("clean.locate"), PlanningStepKind::Locate, || {
        ProjectRepository::discover(Discovery::Explicit(PathBuf::from(request.project.as_str())))
            .map_err(|error| {
                ManagerError::new(
                    "XS3601",
                    Phase::Discover,
                    format!("could not discover project for cleaning: {error}"),
                )
            })
    }) {
        Ok(repository) => repository,
        Err(failure) => return planning.unavailable(OperationKind::Clean, &failure),
    };
    let plan = match planning.step(
        step("clean.validate-plan"),
        PlanningStepKind::ValidatePlan,
        || prepare(request.project.as_str()),
    ) {
        Ok(plan) => plan,
        Err(failure) => return planning.unavailable(OperationKind::Clean, &failure),
    };
    let seed = request_digest(request.project.as_str());
    let sealed = match planning.seal(plan, &seed, PlanMode::Execute) {
        Ok(sealed) => sealed,
        Err(failure) => return orchestrator::planning_outcome(job, OperationKind::Clean, &failure),
    };
    let result = Mutex::new(None);
    let executor = CleanExecutor {
        services,
        project_root: repository.root(),
        result: &result,
    };
    match orchestrator::run(sealed, &executor, context, settings) {
        Ok(report) => OperationOutcome {
            job,
            result: result
                .into_inner()
                .expect("clean result state poisoned")
                .map_or(
                    OperationResult::Unavailable {
                        kind: OperationKind::Clean,
                    },
                    OperationResult::Clean,
                ),
            totals: report.totals,
            root_failures: report.root_failures,
            cancelled: report.cancelled,
        },
        Err(_) => unavailable(job, context),
    }
}

struct CleanExecutor<'a, S> {
    services: &'a S,
    project_root: &'a std::path::Path,
    result: &'a Mutex<Option<CleanResult>>,
}

impl<S: Services> WorkExecutor<CleanWork> for CleanExecutor<'_, S> {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &CleanWork,
        _: &PreparedPlan<CleanWork>,
        _: &ResolvedInputs,
    ) -> Result<Option<CachedResult>, ManagerError> {
        Ok(None)
    }

    fn execute(
        &self,
        _: &Dispatch,
        _: &CleanWork,
        _: &PreparedPlan<CleanWork>,
        _: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition {
        if cancellation.is_cancelled() {
            return WorkDisposition::Cancelled { events: Vec::new() };
        }
        let completion_token = cancellation.clone();
        match self.services.clean_project(self.project_root, cancellation) {
            Ok(ProjectCleanStatus::Cleaned(result)) => {
                *self.result.lock().expect("clean result state poisoned") = Some(result);
                let completed = ActionResult::success();
                if completion_token.is_cancelled() {
                    WorkDisposition::CommittedAfterCancellation(completed)
                } else {
                    completed.into()
                }
            }
            Ok(ProjectCleanStatus::CommittedFailure { result, error }) => {
                *self.result.lock().expect("clean result state poisoned") = Some(result);
                let failed = ActionResult::failure(error.code(), error.message());
                if completion_token.is_cancelled() {
                    WorkDisposition::CommittedAfterCancellation(failed)
                } else {
                    failed.into()
                }
            }
            Ok(ProjectCleanStatus::Cancelled) => WorkDisposition::Cancelled { events: Vec::new() },
            Err(error) => ActionResult::failure(error.code(), error.message()).into(),
        }
    }

    fn record(&self, _: &Dispatch, _: &CleanWork, _: &ActionResult) -> Result<(), ManagerError> {
        Ok(())
    }
}

fn request_digest(project: &str) -> Digest {
    let digest = blake3::hash(project.as_bytes());
    Digest::new(DigestAlgorithm::Blake3, digest.as_bytes().to_vec())
        .expect("BLAKE3 always produces a valid digest")
}

fn step(value: &str) -> PlanningStepId {
    PlanningStepId::new(value).expect("static planning step is non-empty")
}

fn manager_error(
    code: &'static str,
    message: &'static str,
    error: impl std::fmt::Display,
) -> ManagerError {
    ManagerError::new(code, Phase::Manage, format!("{message}: {error}"))
}

fn unavailable(job: JobId, context: &InvocationContext) -> OperationOutcome {
    OperationOutcome {
        job,
        result: OperationResult::Unavailable {
            kind: OperationKind::Clean,
        },
        totals: ActionTotals::default(),
        root_failures: u64::from(!context.is_cancelled()),
        cancelled: context.is_cancelled(),
    }
}
