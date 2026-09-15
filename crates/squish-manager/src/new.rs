//! 新项目的候选生成、单动作计划与可恢复发布。 / New-project candidate generation, single-action planning, and recoverable publication.

use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use squish_build::{
    Action, ActionResult, BuildPlan, Dispatch, InputRef, KeyRecipe, ResourceClass, Resources,
};
use squish_kernel::{CancellationToken, InvocationContext, OperationOutcome};
use squish_project::{NewProjectSpec, ProjectScaffold, ScaffoldVcs};
use squish_protocol::{
    ActionId, ActionKind, ActionTotals, CreatedProjectFile, Digest, DigestAlgorithm, JobId,
    NewPackageName, NewRequest, NewResult, OperationKind, OperationResult, Phase, PlanMode,
    PlanningAttemptId, PlanningStepId, PlanningStepKind, ProjectDestination, ProjectFilePath,
    TargetName, VcsChoice, VcsDisposition, VcsResult, WorkspacePlacement,
};
use squish_repository::{CreateProjectRequest, ProjectFile, ProjectVcs};

use crate::{
    DurabilityPorts, Effect, InvocationSettings, ManagerError, PlannedWork, PreparedPlan,
    ProjectCreationLocation, ProjectCreationStatus, Services,
    orchestrator::{
        self, CachedResult, PlanningRecorder, ResolvedInputs, WorkDisposition, WorkExecutor,
    },
};

/// 新项目计划唯一的写动作。 / Sole write action in a new-project plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NewWork {
    /// 原子发布完整候选项目。 / Atomically publishes the complete candidate project.
    CreateProject,
}

impl PlannedWork for NewWork {
    fn kind(&self) -> ActionKind {
        ActionKind::CreateProject
    }

    fn effect(&self) -> Effect {
        Effect::WriteEffect
    }
}

/// 已冻结且可由唯一写动作提交的新项目候选。 / Frozen new-project candidate committed by the sole write action.
#[derive(Clone, Debug)]
pub struct NewCandidate {
    request: CreateProjectRequest,
    result: NewResult,
    snapshot: Digest,
}

impl NewCandidate {
    /// 从只读定位结果和确定脚手架形成精确候选。 / Forms an exact candidate from a read-only location and deterministic scaffold.
    pub fn prepare(
        location: ProjectCreationLocation,
        package: NewPackageName,
    ) -> Result<Self, ManagerError> {
        let scaffold_vcs = match location.vcs {
            ProjectVcs::None => ScaffoldVcs::None,
            ProjectVcs::InheritedGit | ProjectVcs::InitializeGit => ScaffoldVcs::Git,
        };
        let scaffold =
            ProjectScaffold::generate(&NewProjectSpec::new(package.clone(), scaffold_vcs))
                .map_err(|error| {
                    manager_error(
                        "XS3502",
                        Phase::Manage,
                        "could not generate project scaffold",
                        error,
                    )
                })?;
        let files = scaffold
            .files()
            .iter()
            .map(|file| ProjectFile {
                path: file.path().split('/').collect::<PathBuf>(),
                bytes: file.contents().to_vec(),
            })
            .collect::<Vec<_>>();
        let created = scaffold
            .files()
            .iter()
            .map(|file| CreatedProjectFile {
                path: ProjectFilePath::new(file.path()).expect("scaffold paths are canonical"),
                digest: digest(file.contents()),
                size: u64::try_from(file.contents().len()).unwrap_or(u64::MAX),
            })
            .collect::<Vec<_>>();
        let workspace = location
            .workspace
            .as_ref()
            .map(|membership| {
                Ok(WorkspacePlacement {
                    manifest: ProjectDestination::new(membership.root.join("xmlsquish.toml")),
                    member: ProjectFilePath::new(membership.member.clone()).map_err(|error| {
                        manager_error(
                            "XS3500",
                            Phase::Discover,
                            "creation locator returned an invalid workspace member",
                            error,
                        )
                    })?,
                })
            })
            .transpose()?;
        let vcs = VcsResult {
            kind: if location.vcs == ProjectVcs::None {
                VcsChoice::None
            } else {
                VcsChoice::Git
            },
            disposition: match location.vcs {
                ProjectVcs::None => VcsDisposition::Disabled,
                ProjectVcs::InheritedGit => VcsDisposition::Reused,
                ProjectVcs::InitializeGit => VcsDisposition::Created,
            },
        };
        let result = NewResult {
            package: package.clone(),
            path: ProjectDestination::new(location.destination.clone()),
            manifest: ProjectDestination::new(location.destination.join("xmlsquish.toml")),
            target: TargetName::new("prompt").expect("static target name is non-empty"),
            created,
            workspace,
            vcs,
        };
        let request = CreateProjectRequest {
            destination: location.destination,
            files,
            package_name: package.as_str().to_owned(),
            vcs: location.vcs,
            workspace: location.workspace,
        };
        let snapshot = candidate_digest(&request, &result)?;
        Ok(Self {
            request,
            result,
            snapshot,
        })
    }

    /// 构造 `CreateProject` 单动作写计划。 / Builds the single-action `CreateProject` write plan.
    pub fn plan(&self) -> Result<PreparedPlan<NewWork>, ManagerError> {
        let id = ActionId::new("new.10.create-project").expect("static action ID is non-empty");
        let inputs = self
            .result
            .created
            .iter()
            .map(|file| InputRef::Blob(file.digest.clone()))
            .collect();
        let key = KeyRecipe::new(
            "xmlsquish-new-v1",
            inputs,
            BTreeMap::from([
                ("candidate".into(), self.snapshot.hex()),
                ("package".into(), self.result.package.as_str().to_owned()),
            ]),
        )
        .map_err(|error| {
            manager_error(
                "XS3503",
                Phase::Manage,
                "invalid project creation key",
                error,
            )
        })?;
        let graph = BuildPlan::new([Action {
            id: id.clone(),
            key,
            kind: ActionKind::CreateProject,
            class: ResourceClass::Io,
            resources: Resources::new(0, 1, 0),
            dependencies: Vec::new(),
            outputs: Vec::new(),
        }])
        .map_err(|error| {
            manager_error(
                "XS3503",
                Phase::Manage,
                "invalid project creation plan",
                error,
            )
        })?;
        PreparedPlan::new(graph, BTreeMap::from([(id, NewWork::CreateProject)])).map_err(|error| {
            manager_error(
                "XS3503",
                Phase::Manage,
                "invalid project creation work mapping",
                error,
            )
        })
    }
}

/// 通过管理器的统一规划与调度路径执行 `new`。 / Executes `new` through the manager's unified planning and scheduling path.
pub fn execute(
    request: &NewRequest,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
) -> OperationOutcome {
    let job = JobId::new(format!("new-{}", context.id())).expect("invocation IDs are non-empty");
    let attempt = PlanningAttemptId::new("attempt-1").expect("static planning attempt ID");
    let mut planning = match PlanningRecorder::start(job.clone(), attempt, context) {
        Ok(planning) => planning,
        Err(_) => return unavailable(job, context, true),
    };
    let effective_vcs = request.vcs.unwrap_or(settings.new_vcs);
    let location = match planning.step(step("locate"), PlanningStepKind::Locate, || {
        let location = services
            .locate_project_creation(request.destination.as_path(), effective_vcs)
            .map_err(|error| ManagerError::new(error.code(), Phase::Discover, error.message()))?;
        validate_location(&location, effective_vcs)?;
        Ok(location)
    }) {
        Ok(location) => location,
        Err(failure) => return planning.unavailable(OperationKind::New, &failure),
    };
    let package = match planning.step(
        step("prepare-candidate"),
        PlanningStepKind::PrepareCandidate,
        || {
            let package = infer_package(request, &location)?;
            NewCandidate::prepare(location, package)
        },
    ) {
        Ok(candidate) => candidate,
        Err(failure) => return planning.unavailable(OperationKind::New, &failure),
    };
    let plan = match planning.step(
        step("validate-plan"),
        PlanningStepKind::ValidatePlan,
        || package.plan(),
    ) {
        Ok(plan) => plan,
        Err(failure) => return planning.unavailable(OperationKind::New, &failure),
    };
    let sealed = match planning.seal(plan, &package.snapshot, PlanMode::Execute) {
        Ok(sealed) => sealed,
        Err(failure) => return orchestrator::planning_outcome(job, OperationKind::New, &failure),
    };
    let committed = Arc::new(Mutex::new(false));
    let executor = NewExecutor {
        services,
        candidate: &package,
        committed: committed.clone(),
        faults: durability.repository(),
    };
    match orchestrator::run(sealed, &executor, context, settings) {
        Ok(report) => {
            let created = *committed.lock().expect("creation state poisoned");
            OperationOutcome {
                job,
                result: if created {
                    OperationResult::New(package.result)
                } else {
                    OperationResult::Unavailable {
                        kind: OperationKind::New,
                    }
                },
                totals: report.totals,
                root_failures: report.root_failures,
                cancelled: report.cancelled,
            }
        }
        Err(_) => unavailable(job, context, true),
    }
}

fn infer_package(
    request: &NewRequest,
    location: &ProjectCreationLocation,
) -> Result<NewPackageName, ManagerError> {
    if let Some(package) = &request.name {
        return Ok(package.clone());
    }
    let leaf = location
        .destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ManagerError::new(
                "XS3501",
                Phase::Manage,
                "cannot infer a package name from the destination; pass --name",
            )
        })?;
    NewPackageName::new(leaf).map_err(|_| {
        ManagerError::new(
            "XS3501",
            Phase::Manage,
            format!("destination leaf `{leaf}` is not a valid package name; pass --name"),
        )
    })
}

fn validate_location(
    location: &ProjectCreationLocation,
    requested: VcsChoice,
) -> Result<(), ManagerError> {
    if !normalized_absolute(&location.destination) {
        return Err(ManagerError::new(
            "XS3500",
            Phase::Discover,
            "project creation locator returned a non-absolute or non-normalized destination",
        ));
    }
    let vcs_matches = matches!(
        (requested, location.vcs),
        (VcsChoice::None, ProjectVcs::None)
            | (
                VcsChoice::Git,
                ProjectVcs::InheritedGit | ProjectVcs::InitializeGit
            )
    );
    if !vcs_matches {
        return Err(ManagerError::new(
            "XS3500",
            Phase::Discover,
            "project creation locator returned a VCS disposition that contradicts the request",
        ));
    }
    if location.workspace.as_ref().is_some_and(|workspace| {
        !normalized_absolute(&workspace.root)
            || ProjectFilePath::new(workspace.member.clone()).is_err()
    }) {
        return Err(ManagerError::new(
            "XS3500",
            Phase::Discover,
            "project creation locator returned invalid workspace placement",
        ));
    }
    Ok(())
}

fn normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
}

struct NewExecutor<'a> {
    services: &'a dyn Services,
    candidate: &'a NewCandidate,
    committed: Arc<Mutex<bool>>,
    faults: Arc<dyn squish_repository::FaultInjector>,
}

impl WorkExecutor<NewWork> for NewExecutor<'_> {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &NewWork,
        _: &PreparedPlan<NewWork>,
        _: &ResolvedInputs,
    ) -> Result<Option<CachedResult>, ManagerError> {
        Ok(None)
    }

    fn execute(
        &self,
        _: &Dispatch,
        _: &NewWork,
        _: &PreparedPlan<NewWork>,
        _: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition {
        if cancellation.is_cancelled() {
            return WorkDisposition::Cancelled { events: Vec::new() };
        }
        let completion_token = cancellation.clone();
        match self.services.create_project(
            &self.candidate.request,
            cancellation,
            self.faults.clone(),
        ) {
            Ok(ProjectCreationStatus::Created(receipt)) => {
                if receipt.destination != self.candidate.request.destination {
                    return ActionResult::failure(
                        "XS3505",
                        "project publisher returned a different destination",
                    )
                    .into();
                }
                *self.committed.lock().expect("creation state poisoned") = true;
                if completion_token.is_cancelled() {
                    WorkDisposition::CommittedAfterCancellation(ActionResult::success())
                } else {
                    ActionResult::success().into()
                }
            }
            Ok(ProjectCreationStatus::CommittedFailure(error)) => {
                let result = ActionResult::failure(error.code(), error.message());
                if completion_token.is_cancelled() {
                    WorkDisposition::CommittedAfterCancellation(result)
                } else {
                    result.into()
                }
            }
            Ok(ProjectCreationStatus::Cancelled) => {
                WorkDisposition::Cancelled { events: Vec::new() }
            }
            Err(error) => ActionResult::failure(error.code(), error.message()).into(),
        }
    }

    fn record(&self, _: &Dispatch, _: &NewWork, _: &ActionResult) -> Result<(), ManagerError> {
        Ok(())
    }
}

fn candidate_digest(
    request: &CreateProjectRequest,
    result: &NewResult,
) -> Result<Digest, ManagerError> {
    let mut hash = blake3::Hasher::new();
    hash.update(b"xmlsquish-new-candidate\0v1");
    let destination = serde_json::to_vec(&result.path).map_err(|error| {
        manager_error(
            "XS3503",
            Phase::Manage,
            "could not encode destination",
            error,
        )
    })?;
    hash_field(&mut hash, &destination);
    hash_field(&mut hash, request.package_name.as_bytes());
    hash_field(
        &mut hash,
        match request.vcs {
            ProjectVcs::None => b"none",
            ProjectVcs::InheritedGit => b"inherited-git",
            ProjectVcs::InitializeGit => b"initialize-git",
        },
    );
    for file in &result.created {
        hash_field(&mut hash, file.path.as_str().as_bytes());
        hash_field(&mut hash, file.digest.bytes());
        hash_field(&mut hash, &file.size.to_le_bytes());
    }
    if let Some(workspace) = &request.workspace {
        let root = serde_json::to_vec(&ProjectDestination::new(workspace.root.clone())).map_err(
            |error| {
                manager_error(
                    "XS3503",
                    Phase::Manage,
                    "could not encode workspace root",
                    error,
                )
            },
        )?;
        hash_field(&mut hash, &root);
        hash_field(&mut hash, workspace.member.as_bytes());
    }
    Ok(
        Digest::new(DigestAlgorithm::Blake3, hash.finalize().as_bytes().to_vec())
            .expect("BLAKE3 digest is canonical"),
    )
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(
        DigestAlgorithm::Blake3,
        blake3::hash(bytes).as_bytes().to_vec(),
    )
    .expect("BLAKE3 digest is canonical")
}

fn hash_field(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn step(value: &str) -> PlanningStepId {
    PlanningStepId::new(value).expect("static planning step IDs are non-empty")
}

fn unavailable(job: JobId, context: &InvocationContext, failed: bool) -> OperationOutcome {
    OperationOutcome {
        job,
        result: OperationResult::Unavailable {
            kind: OperationKind::New,
        },
        totals: ActionTotals::default(),
        root_failures: u64::from(failed && !context.is_cancelled()),
        cancelled: context.is_cancelled(),
    }
}

fn manager_error(
    code: &str,
    phase: Phase,
    context: &str,
    error: impl std::fmt::Display,
) -> ManagerError {
    ManagerError::new(code, phase, format!("{context}: {error}"))
}
