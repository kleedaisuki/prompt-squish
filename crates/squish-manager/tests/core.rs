use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use squish_build::{
    Action, ActionEvent, ActionKind, ActionResult, BuildPlan, Dispatch, InputRef, KeyRecipe,
    Output, OutputName, OutputRef, ProducedOutput, ResourceClass, Resources, ResultSource,
};
use squish_kernel::{
    CancellationToken, Capability, CapabilityDescriptor, EventSink, InvocationContext, Kernel,
    OperationOutcome, SinkError,
};
use squish_manager::{
    ArtifactLocator, Effect, InvocationSettings, ManagerCapability, ManagerError, PlannedWork,
    PreparedPlan, PreparedPlanError, ProjectBuildLayout, ResolveRequest, ResolvedDependencies,
    ServiceError, Services,
    orchestrator::{
        ExecutionState, PlanningRecorder, ResolvedInputs, SealedPlan, WorkDisposition,
        WorkExecutor, finalize, run,
    },
};
use squish_project::{Lockfile, ResolutionMode};
use squish_protocol::{
    ActionId, ActionTotals, ArtifactKind, Digest, DigestAlgorithm, Event, EventPayload,
    FinalizationId, FinalizationKind, FormatRequest, FormatResult, FormatSelection, InvocationId,
    JobId, OperationKind, OperationRequest, OperationResult, Phase, PlanMode, PlanningAttemptId,
    PlanningStepId, PlanningStepKind, ProjectPath, StyleEdition, SupersedeReason, WorkspaceScope,
};
use squish_repository::PackageLocation;

#[derive(Clone, Copy, Debug)]
struct TestWork(ActionKind, Effect);

impl PlannedWork for TestWork {
    fn kind(&self) -> ActionKind {
        self.0
    }
    fn effect(&self) -> Effect {
        self.1
    }
}

fn id(value: &str) -> ActionId {
    ActionId::new(value).unwrap()
}

fn action(value: &str, kind: ActionKind) -> Action {
    Action {
        id: id(value),
        key: KeyRecipe::new("test/1", Vec::new(), BTreeMap::new()).unwrap(),
        kind,
        class: ResourceClass::Cpu,
        resources: Resources::new(1, 0, 0),
        dependencies: Vec::new(),
        outputs: Vec::new(),
    }
}

fn seal<W: PlannedWork>(
    job: JobId,
    prepared: PreparedPlan<W>,
    context: &InvocationContext,
) -> SealedPlan<W> {
    let mut planning =
        PlanningRecorder::start(job, PlanningAttemptId::new("attempt-1").unwrap(), context)
            .unwrap();
    planning
        .step(
            PlanningStepId::new("validate").unwrap(),
            PlanningStepKind::ValidatePlan,
            || Ok(()),
        )
        .unwrap();
    planning
        .seal(
            prepared,
            &Digest::new(DigestAlgorithm::Blake3, vec![3; 32]).unwrap(),
            PlanMode::Execute,
        )
        .unwrap()
}

#[test]
fn effect_cacheability_is_closed_to_transforms() {
    assert!(Effect::Transform.cacheable());
    assert!(!Effect::ReadEffect.cacheable());
    assert!(!Effect::WriteEffect.cacheable());
    assert!(!Effect::Coordination.cacheable());
}

#[test]
fn artifact_locator_rejects_an_empty_path() {
    assert!(ArtifactLocator::new("").is_err());
    assert_eq!(
        ArtifactLocator::new("target/result.prompt")
            .unwrap()
            .as_path(),
        Path::new("target/result.prompt")
    );
}

#[test]
fn project_build_layout_derives_every_owned_path_from_one_root() {
    let temporary = tempfile::tempdir().unwrap();
    let project = std::fs::canonicalize(temporary.path()).unwrap();
    assert!(ProjectBuildLayout::new("relative/project", "target/xmlsquish").is_err());
    assert!(ProjectBuildLayout::new(&project, "../outside").is_err());
    assert!(ProjectBuildLayout::new(&project, "./target/xmlsquish").is_err());
    assert!(ProjectBuildLayout::new(&project, "").is_err());

    let layout = ProjectBuildLayout::new(&project, "dist/prompts").unwrap();
    let root = project.join("dist/prompts");
    assert_eq!(layout.ownership_root(), root);
    assert_eq!(layout.artifacts_root(), root.join("artifacts"));
    assert_eq!(layout.source_cache_root(), root.join("cache/sources"));
    assert_eq!(layout.cas_root(), root.join("cache/cas"));
    assert_eq!(layout.action_index(), root.join("cache/actions.sqlite3"));
    assert_eq!(layout.metadata_root(), root.join("metadata"));
    assert_eq!(layout.layout_marker(), root.join("metadata/layout.json"));
    assert_eq!(
        layout.publications_root(),
        root.join("metadata/publications")
    );
    assert_eq!(layout.catalog_root(), root.join("metadata/catalog"));
    assert_eq!(layout.work_root(), root.join("work"));
    assert_eq!(
        layout.publication_prefix(),
        Path::new("dist/prompts/artifacts")
    );
    assert_eq!(
        layout.coordination_lock(),
        project.join("dist/.prompts.xmlsquish.lock")
    );
    assert_eq!(
        layout.clean_journal(),
        project.join("dist/.prompts.xmlsquish.clean.json")
    );
    assert_eq!(
        layout.trash_prefix(),
        project.join("dist/.prompts.xmlsquish-trash-")
    );
}

#[test]
fn project_build_layout_rejects_target_dir_symlink_escape() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let link = project.path().join("linked-target");
    create_directory_symlink(outside.path(), &link);
    let project = std::fs::canonicalize(project.path()).unwrap();
    assert!(ProjectBuildLayout::new(project, "linked-target/xmlsquish").is_err());
}

#[cfg(unix)]
#[test]
fn project_build_layout_rejects_target_dir_symlink_within_project() {
    let project = tempfile::tempdir().unwrap();
    let destination = project.path().join("real-target");
    std::fs::create_dir(&destination).unwrap();
    let link = project.path().join("linked-target");
    create_directory_symlink(&destination, &link);
    let project = std::fs::canonicalize(project.path()).unwrap();

    assert_eq!(
        ProjectBuildLayout::new(project, "linked-target/xmlsquish").unwrap_err(),
        squish_manager::ProjectBuildLayoutError::TargetDirAlias
    );
}

#[cfg(windows)]
#[test]
fn project_build_layout_rejects_target_dir_junction_within_project() {
    let project = tempfile::tempdir().unwrap();
    let destination = project.path().join("real-target");
    std::fs::create_dir(&destination).unwrap();
    let junction = project.path().join("junction-target");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&destination)
        .status()
        .unwrap();
    assert!(status.success());
    let project = std::fs::canonicalize(project.path()).unwrap();

    assert_eq!(
        ProjectBuildLayout::new(project, "junction-target/xmlsquish").unwrap_err(),
        squish_manager::ProjectBuildLayoutError::TargetDirAlias
    );
}

#[cfg(windows)]
#[test]
fn project_build_layout_accepts_real_target_directory_with_mixed_case_spelling() {
    let project = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(project.path()).unwrap();
    std::fs::create_dir_all(canonical.join("TARGET/xmlsquish")).unwrap();

    let layout = ProjectBuildLayout::new(&canonical, "target/xmlsquish").unwrap();

    assert_eq!(layout.ownership_root(), canonical.join("target/xmlsquish"));
    layout.validate_existing_aliases().unwrap();
}

#[cfg(unix)]
#[test]
fn project_build_layout_revalidation_rejects_new_target_alias() {
    let project = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(project.path()).unwrap();
    let layout = ProjectBuildLayout::new(&canonical, "target/xmlsquish").unwrap();
    let destination = canonical.join("real-target");
    std::fs::create_dir(&destination).unwrap();
    create_directory_symlink(&destination, &canonical.join("target"));

    assert_eq!(
        layout.validate_existing_aliases().unwrap_err(),
        squish_manager::ProjectBuildLayoutError::TargetDirAlias
    );
}

#[cfg(windows)]
#[test]
fn project_build_layout_revalidation_rejects_new_target_junction() {
    let project = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(project.path()).unwrap();
    let layout = ProjectBuildLayout::new(&canonical, "target/xmlsquish").unwrap();
    let destination = canonical.join("real-target");
    std::fs::create_dir(&destination).unwrap();
    let junction = canonical.join("target");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&destination)
        .status()
        .unwrap();
    assert!(status.success());

    assert_eq!(
        layout.validate_existing_aliases().unwrap_err(),
        squish_manager::ProjectBuildLayoutError::TargetDirAlias
    );
}

#[cfg(unix)]
#[test]
fn project_build_layout_revalidation_rejects_replaced_project_root() {
    let container = tempfile::tempdir().unwrap();
    let project = container.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let project = std::fs::canonicalize(project).unwrap();
    let layout = ProjectBuildLayout::new(&project, "target/xmlsquish").unwrap();
    let moved = container.path().join("moved-project");
    std::fs::rename(&project, &moved).unwrap();
    create_directory_symlink(&moved, &project);

    assert_eq!(
        layout.validate_existing_aliases().unwrap_err(),
        squish_manager::ProjectBuildLayoutError::ProjectRootChanged
    );
}

#[cfg(windows)]
#[test]
fn project_build_layout_revalidation_rejects_replaced_project_root() {
    let container = tempfile::tempdir().unwrap();
    let project = container.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let project = std::fs::canonicalize(project).unwrap();
    let layout = ProjectBuildLayout::new(&project, "target/xmlsquish").unwrap();
    let moved = container.path().join("moved-project");
    std::fs::rename(&project, &moved).unwrap();
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&project)
        .arg(&moved)
        .status()
        .unwrap();
    assert!(status.success());

    assert_eq!(
        layout.validate_existing_aliases().unwrap_err(),
        squish_manager::ProjectBuildLayoutError::ProjectRootChanged
    );
}

#[test]
fn project_build_layout_rejects_dangling_target_dir_symlink() {
    let project = tempfile::tempdir().unwrap();
    let missing = project.path().join("missing-destination");
    let link = project.path().join("dangling-target");
    create_directory_symlink(&missing, &link);
    let project = std::fs::canonicalize(project.path()).unwrap();
    assert!(ProjectBuildLayout::new(project, "dangling-target/xmlsquish").is_err());
}

#[cfg(unix)]
fn create_directory_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn create_directory_symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_dir(target, link).unwrap();
}

#[test]
fn prepared_plan_requires_exact_ids_and_kinds() {
    let graph = BuildPlan::new([action("a", ActionKind::Scan)]).unwrap();
    let missing = PreparedPlan::<TestWork>::new(graph.clone(), BTreeMap::new()).unwrap_err();
    assert_eq!(missing, PreparedPlanError::MissingWork(id("a")));

    let wrong = BTreeMap::from([(id("a"), TestWork(ActionKind::Link, Effect::Transform))]);
    assert!(matches!(
        PreparedPlan::new(graph.clone(), wrong),
        Err(PreparedPlanError::KindMismatch { .. })
    ));

    let extra = BTreeMap::from([
        (id("a"), TestWork(ActionKind::Scan, Effect::Transform)),
        (id("b"), TestWork(ActionKind::Scan, Effect::Transform)),
    ]);
    assert_eq!(
        PreparedPlan::new(graph, extra).unwrap_err(),
        PreparedPlanError::ExtraWork(id("b"))
    );
}

struct FakeServices;

impl Services for FakeServices {
    fn storage_layout(&self, project_root: &Path) -> Result<ProjectBuildLayout, ServiceError> {
        Ok(ProjectBuildLayout::project_local_for_tests(project_root))
    }

    fn materialize_locked(
        &self,
        _project: &Path,
        _lock: &Lockfile,
        _mode: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError> {
        unreachable!()
    }

    fn resolve(&self, _request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        unreachable!()
    }
}

#[test]
fn manager_is_one_capability_for_all_seven_operations() {
    let manager = ManagerCapability::new(FakeServices, InvocationSettings::default());
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id, "project-manager");
    assert_eq!(
        descriptor.operations,
        &[
            OperationKind::New,
            OperationKind::Build,
            OperationKind::Format,
            OperationKind::Add,
            OperationKind::Remove,
            OperationKind::Inspect,
            OperationKind::Clean,
        ]
    );
}

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

impl EventSink for Events {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

struct Executor;

impl WorkExecutor<TestWork> for Executor {
    fn lookup(
        &self,
        _dispatch: &Dispatch,
        _work: &TestWork,
        _plan: &PreparedPlan<TestWork>,
        _inputs: &ResolvedInputs,
    ) -> Result<Option<squish_manager::orchestrator::CachedResult>, ManagerError> {
        Ok(None)
    }

    fn execute(
        &self,
        _dispatch: &Dispatch,
        _work: &TestWork,
        _plan: &PreparedPlan<TestWork>,
        _inputs: &ResolvedInputs,
        _cancellation: CancellationToken,
    ) -> WorkDisposition {
        ActionResult::success().into()
    }

    fn record(
        &self,
        _dispatch: &Dispatch,
        _work: &TestWork,
        _result: &ActionResult,
    ) -> Result<(), ManagerError> {
        Ok(())
    }
}

struct Orchestrated;

static TEST_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "orchestrated-test",
    operations: &[OperationKind::Format],
    summary: "test",
};

impl Capability for Orchestrated {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &TEST_DESCRIPTOR
    }

    fn execute(
        &self,
        operation: &OperationRequest,
        context: &InvocationContext,
    ) -> OperationOutcome {
        let OperationRequest::Format(request) = operation else {
            unreachable!()
        };
        let parent = action("z-parent", ActionKind::Compile);
        let mut child = action("a-child", ActionKind::Format);
        child.dependencies.push(parent.id.clone());
        let graph = BuildPlan::new([child, parent]).unwrap();
        let work = BTreeMap::from([
            (
                id("z-parent"),
                TestWork(ActionKind::Compile, Effect::Transform),
            ),
            (
                id("a-child"),
                TestWork(ActionKind::Format, Effect::Transform),
            ),
        ]);
        let job = JobId::new("job").unwrap();
        let report = run(
            seal(
                job.clone(),
                PreparedPlan::new(graph, work).unwrap(),
                context,
            ),
            &Executor,
            context,
            &InvocationSettings::default(),
        )
        .unwrap();
        OperationOutcome {
            job: JobId::new("job").unwrap(),
            result: OperationResult::Format(FormatResult {
                selected: Vec::new(),
                changed: Vec::new(),
                check: request.check,
                diffs: Vec::new(),
            }),
            totals: report.totals,
            root_failures: report.root_failures,
            cancelled: report.cancelled,
        }
    }
}

#[test]
fn orchestrator_emits_a_kernel_valid_lifecycle() {
    let sink = Arc::new(Events::default());
    let context = InvocationContext::new(
        InvocationId::new("invocation").unwrap(),
        CancellationToken::default(),
        sink.clone(),
    );
    let capability = Orchestrated;
    let capabilities: [&dyn Capability; 1] = [&capability];
    let kernel = Kernel::new(&capabilities).unwrap();
    let request = OperationRequest::Format(FormatRequest {
        project: ProjectPath::new(".").unwrap(),
        scope: WorkspaceScope::Current,
        selection: FormatSelection::All,
        style: Some(StyleEdition::new("1").unwrap()),
        check: true,
        diff: false,
    });
    let outcome = kernel.dispatch(&request, &context).unwrap();
    assert_eq!(
        outcome.summary.totals,
        ActionTotals {
            succeeded: 2,
            ..ActionTotals::default()
        }
    );
    assert_eq!(sink.0.lock().unwrap().len(), 13);
}

struct PreflightFailure;

impl Capability for PreflightFailure {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &TEST_DESCRIPTOR
    }

    fn execute(
        &self,
        _operation: &OperationRequest,
        context: &InvocationContext,
    ) -> OperationOutcome {
        squish_manager::orchestrator::fail(
            JobId::new("failed-job").unwrap(),
            OperationKind::Format,
            ActionKind::Snapshot,
            ManagerError::new("test_preflight", Phase::Discover, "project was not found"),
            context,
            &InvocationSettings::default(),
        )
    }
}

#[test]
fn preflight_failure_is_a_kernel_valid_failed_job() {
    let context = InvocationContext::new(
        InvocationId::new("failed-invocation").unwrap(),
        CancellationToken::default(),
        Arc::new(Events::default()),
    );
    let capability = PreflightFailure;
    let capabilities: [&dyn Capability; 1] = [&capability];
    let kernel = Kernel::new(&capabilities).unwrap();
    let request = OperationRequest::Format(FormatRequest {
        project: ProjectPath::new("missing").unwrap(),
        scope: WorkspaceScope::Current,
        selection: FormatSelection::All,
        style: Some(StyleEdition::new("1").unwrap()),
        check: true,
        diff: false,
    });
    let outcome = kernel.dispatch(&request, &context).unwrap();
    assert_eq!(outcome.summary.totals.failed, 0);
    assert_eq!(outcome.summary.root_failures, 1);
    assert!(matches!(
        outcome.result,
        OperationResult::Unavailable {
            kind: OperationKind::Format
        }
    ));
}

struct ConcurrentExecutor {
    active: AtomicUsize,
    maximum: AtomicUsize,
}

impl WorkExecutor<TestWork> for ConcurrentExecutor {
    fn lookup(
        &self,
        _dispatch: &Dispatch,
        _work: &TestWork,
        _plan: &PreparedPlan<TestWork>,
        _inputs: &ResolvedInputs,
    ) -> Result<Option<squish_manager::orchestrator::CachedResult>, ManagerError> {
        Ok(None)
    }

    fn execute(
        &self,
        _dispatch: &Dispatch,
        _work: &TestWork,
        _plan: &PreparedPlan<TestWork>,
        _inputs: &ResolvedInputs,
        _cancellation: CancellationToken,
    ) -> WorkDisposition {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(40));
        self.active.fetch_sub(1, Ordering::SeqCst);
        ActionResult::success().into()
    }

    fn record(
        &self,
        _dispatch: &Dispatch,
        _work: &TestWork,
        _result: &ActionResult,
    ) -> Result<(), ManagerError> {
        Ok(())
    }
}

#[test]
fn jobs_bounds_real_parallelism_and_completion_events_are_stable() {
    let actions = ["b", "a", "c"].map(|name| {
        let mut action = action(name, ActionKind::Compile);
        action.key = KeyRecipe::new(format!("test/{name}"), Vec::new(), BTreeMap::new()).unwrap();
        action
    });
    let graph = BuildPlan::new(actions).unwrap();
    let work = ["a", "b", "c"]
        .into_iter()
        .map(|name| (id(name), TestWork(ActionKind::Compile, Effect::Transform)))
        .collect();
    let sink = Arc::new(Events::default());
    let context = InvocationContext::new(
        InvocationId::new("parallel").unwrap(),
        CancellationToken::default(),
        sink.clone(),
    );
    let executor = ConcurrentExecutor {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
    };
    let settings = InvocationSettings {
        jobs: 2,
        ..InvocationSettings::default()
    };
    let report = run(
        seal(
            JobId::new("parallel-job").unwrap(),
            PreparedPlan::new(graph, work).unwrap(),
            &context,
        ),
        &executor,
        &context,
        &settings,
    )
    .unwrap();
    assert_eq!(report.totals.succeeded, 3);
    assert_eq!(executor.maximum.load(Ordering::SeqCst), 2);
    let succeeded: Vec<_> = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ActionSucceeded { action, .. } => Some(action.as_str().to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(succeeded, ["a", "b", "c"]);
}

struct InputExecutor {
    seen: Mutex<BTreeMap<ActionId, ResolvedInputs>>,
}

impl WorkExecutor<TestWork> for InputExecutor {
    fn lookup(
        &self,
        _dispatch: &Dispatch,
        _work: &TestWork,
        _plan: &PreparedPlan<TestWork>,
        _inputs: &ResolvedInputs,
    ) -> Result<Option<squish_manager::orchestrator::CachedResult>, ManagerError> {
        Ok(None)
    }

    fn execute(
        &self,
        dispatch: &Dispatch,
        _work: &TestWork,
        _plan: &PreparedPlan<TestWork>,
        inputs: &ResolvedInputs,
        _cancellation: CancellationToken,
    ) -> WorkDisposition {
        self.seen
            .lock()
            .unwrap()
            .insert(dispatch.action.id.clone(), inputs.clone());
        if dispatch.action.outputs.is_empty() {
            ActionResult::success().into()
        } else {
            ActionResult {
                outcome: Ok(()),
                outputs: vec![ProducedOutput {
                    name: OutputName::new("value").unwrap(),
                    kind: ArtifactKind::Metadata,
                    digest: Digest::new(DigestAlgorithm::Sha256, vec![7; 32]).unwrap(),
                    size: 1,
                }],
                events: Vec::new(),
            }
            .into()
        }
    }

    fn record(
        &self,
        _dispatch: &Dispatch,
        _work: &TestWork,
        _result: &ActionResult,
    ) -> Result<(), ManagerError> {
        Ok(())
    }
}

#[test]
fn single_flight_follower_outputs_reach_its_dependent() {
    let output = Output {
        name: OutputName::new("value").unwrap(),
        kind: ArtifactKind::Metadata,
    };
    let mut leader = action("b-leader", ActionKind::Compile);
    leader.outputs.push(output.clone());
    let mut follower = action("c-follower", ActionKind::Compile);
    follower.outputs.push(output);
    let dependent = |name: &str, dependency: &str| {
        let dependency = id(dependency);
        let mut action = action(name, ActionKind::Link);
        action.dependencies.push(dependency.clone());
        action.key = KeyRecipe::new(
            "dependent/1",
            vec![InputRef::Output(OutputRef {
                action: dependency,
                output: OutputName::new("value").unwrap(),
            })],
            BTreeMap::from([("owner".to_owned(), name.to_owned())]),
        )
        .unwrap();
        action
    };
    let graph = BuildPlan::new([
        leader,
        follower,
        dependent("d-from-leader", "b-leader"),
        dependent("e-from-follower", "c-follower"),
    ])
    .unwrap();
    let work = graph
        .actions()
        .map(|action| (action.id.clone(), TestWork(action.kind, Effect::Transform)))
        .collect();
    let context = InvocationContext::new(
        InvocationId::new("single-flight").unwrap(),
        CancellationToken::default(),
        Arc::new(Events::default()),
    );
    let executor = InputExecutor {
        seen: Mutex::new(BTreeMap::new()),
    };
    let report = run(
        seal(
            JobId::new("single-flight-job").unwrap(),
            PreparedPlan::new(graph, work).unwrap(),
            &context,
        ),
        &executor,
        &context,
        &InvocationSettings {
            jobs: 2,
            ..InvocationSettings::default()
        },
    )
    .unwrap();
    let seen = executor.seen.lock().unwrap();
    assert!(!seen.contains_key(&id("c-follower")));
    assert!(
        seen[&id("d-from-leader")]
            .outputs(&id("b-leader"))
            .is_some()
    );
    assert!(
        seen[&id("e-from-follower")]
            .outputs(&id("c-follower"))
            .is_some()
    );
    let follower = report
        .actions
        .iter()
        .find(|fact| fact.action == id("c-follower"))
        .unwrap();
    assert!(follower.key.is_some());
    assert_eq!(follower.source, Some(ResultSource::SingleFlight));
    assert_eq!(follower.outputs.len(), 1);
}

fn one_plan(name: &str, kind: ActionKind, effect: Effect) -> PreparedPlan<TestWork> {
    PreparedPlan::new(
        BuildPlan::new([action(name, kind)]).unwrap(),
        BTreeMap::from([(id(name), TestWork(kind, effect))]),
    )
    .unwrap()
}

#[test]
fn plan_digest_is_retry_stable_and_snapshot_sensitive() {
    fn identity(
        attempt: &str,
        seed: u8,
        kind: ActionKind,
        mode: PlanMode,
    ) -> squish_manager::orchestrator::PlanIdentity {
        let context = InvocationContext::new(
            InvocationId::new(format!("invocation-{attempt}-{seed}")).unwrap(),
            CancellationToken::default(),
            Arc::new(Events::default()),
        );
        PlanningRecorder::start(
            JobId::new("same-job").unwrap(),
            PlanningAttemptId::new(attempt).unwrap(),
            &context,
        )
        .unwrap()
        .seal(
            one_plan("work", kind, Effect::Transform),
            &Digest::new(DigestAlgorithm::Blake3, vec![seed; 32]).unwrap(),
            mode,
        )
        .unwrap()
        .identity()
        .clone()
    }
    let first = identity("attempt-1", 1, ActionKind::Compile, PlanMode::Execute);
    let retry = identity("attempt-2", 1, ActionKind::Compile, PlanMode::Execute);
    let changed = identity("attempt-3", 2, ActionKind::Compile, PlanMode::Execute);
    let changed_graph = identity("attempt-4", 1, ActionKind::Backend, PlanMode::Execute);
    let report_only = identity("attempt-5", 1, ActionKind::Compile, PlanMode::ReportOnly);
    assert_eq!(first.digest, retry.digest);
    assert_ne!(first.plan, retry.plan);
    assert_ne!(first.digest, changed.digest);
    assert_ne!(first.digest, changed_graph.digest);
    assert_ne!(first.digest, report_only.digest);
}

struct DeferredEventExecutor;

impl WorkExecutor<TestWork> for DeferredEventExecutor {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &TestWork,
        _: &PreparedPlan<TestWork>,
        _: &ResolvedInputs,
    ) -> Result<Option<squish_manager::orchestrator::CachedResult>, ManagerError> {
        Ok(None)
    }

    fn execute(
        &self,
        _: &Dispatch,
        _: &TestWork,
        _: &PreparedPlan<TestWork>,
        _: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition {
        cancellation.cancel();
        let mut result = ActionResult::success();
        result.events.push(ActionEvent::Message {
            code: "before-deferred".into(),
            message: "worker fact before terminal deferral".into(),
        });
        WorkDisposition::CommittedAfterCancellation(result)
    }

    fn record(&self, _: &Dispatch, _: &TestWork, _: &ActionResult) -> Result<(), ManagerError> {
        Ok(())
    }
}

#[test]
fn committed_cancellation_flushes_worker_events_before_exact_next_terminal() {
    let events = Arc::new(Events::default());
    let context = InvocationContext::new(
        InvocationId::new("deferred-adjacency").unwrap(),
        CancellationToken::default(),
        events.clone(),
    );
    let sealed = PlanningRecorder::start(
        JobId::new("deferred-job").unwrap(),
        PlanningAttemptId::new("attempt-1").unwrap(),
        &context,
    )
    .unwrap()
    .seal(
        one_plan("create", ActionKind::CreateProject, Effect::WriteEffect),
        &Digest::new(DigestAlgorithm::Blake3, vec![7; 32]).unwrap(),
        PlanMode::Execute,
    )
    .unwrap();
    let report = run(
        sealed,
        &DeferredEventExecutor,
        &context,
        &InvocationSettings::default(),
    )
    .unwrap();
    assert!(report.cancelled);
    let events = events.0.lock().unwrap();
    let diagnostic = events
        .iter()
        .position(|event| matches!(event.payload, EventPayload::Diagnostic(_)))
        .unwrap();
    let deferred = events
        .iter()
        .position(|event| matches!(event.payload, EventPayload::CancellationDeferred { .. }))
        .unwrap();
    let succeeded = events
        .iter()
        .position(|event| matches!(event.payload, EventPayload::ActionSucceeded { .. }))
        .unwrap();
    assert!(diagnostic < deferred);
    assert_eq!(deferred + 1, succeeded);
}

#[test]
fn v2_seal_rejects_planning_placeholder_actions() {
    let context = InvocationContext::new(
        InvocationId::new("placeholder").unwrap(),
        CancellationToken::default(),
        Arc::new(Events::default()),
    );
    let planning = PlanningRecorder::start(
        JobId::new("placeholder-job").unwrap(),
        PlanningAttemptId::new("attempt-1").unwrap(),
        &context,
    )
    .unwrap();
    assert!(matches!(
        planning.seal(
            one_plan("fake-snapshot", ActionKind::Snapshot, Effect::ReadEffect),
            &Digest::new(DigestAlgorithm::Blake3, vec![1; 32]).unwrap(),
            PlanMode::Execute,
        ),
        Err(squish_manager::orchestrator::PlanningFailure::Failed(_))
    ));
}

struct CancelledPlanning;

impl Capability for CancelledPlanning {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &TEST_DESCRIPTOR
    }

    fn execute(
        &self,
        _operation: &OperationRequest,
        context: &InvocationContext,
    ) -> OperationOutcome {
        let mut planning = PlanningRecorder::start(
            JobId::new("cancelled-job").unwrap(),
            PlanningAttemptId::new("attempt-1").unwrap(),
            context,
        )
        .unwrap();
        let failure = planning
            .step(
                PlanningStepId::new("locate").unwrap(),
                PlanningStepKind::Locate,
                || Ok(()),
            )
            .unwrap_err();
        planning.unavailable(OperationKind::Format, &failure)
    }
}

#[test]
fn planning_cancellation_is_not_reported_as_failure() {
    let token = CancellationToken::default();
    token.cancel();
    let context = InvocationContext::new(
        InvocationId::new("cancelled").unwrap(),
        token,
        Arc::new(Events::default()),
    );
    let capability = CancelledPlanning;
    let capabilities: [&dyn Capability; 1] = [&capability];
    let kernel = Kernel::new(&capabilities).unwrap();
    let outcome = kernel.dispatch(&format_request(), &context).unwrap();
    assert_eq!(outcome.summary.root_failures, 0);
    assert_eq!(outcome.summary.totals.total(), 0);
    assert_eq!(outcome.summary.status.code(), 130);
}

fn format_request() -> OperationRequest {
    OperationRequest::Format(FormatRequest {
        project: ProjectPath::new(".").unwrap(),
        scope: WorkspaceScope::Current,
        selection: FormatSelection::All,
        style: None,
        check: true,
        diff: false,
    })
}

struct SupersedingExecutor {
    calls: AtomicUsize,
}

impl WorkExecutor<TestWork> for SupersedingExecutor {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &TestWork,
        _: &PreparedPlan<TestWork>,
        _: &ResolvedInputs,
    ) -> Result<Option<squish_manager::orchestrator::CachedResult>, ManagerError> {
        Ok(None)
    }

    fn execute(
        &self,
        _: &Dispatch,
        _: &TestWork,
        _: &PreparedPlan<TestWork>,
        _: &ResolvedInputs,
        _: CancellationToken,
    ) -> WorkDisposition {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            WorkDisposition::Superseded {
                reason: SupersedeReason::AuthoritativeRevisionChanged,
                events: Vec::new(),
            }
        } else {
            ActionResult::success().into()
        }
    }

    fn record(&self, _: &Dispatch, _: &TestWork, _: &ActionResult) -> Result<(), ManagerError> {
        Ok(())
    }
}

struct ReplanningCapability {
    executor: SupersedingExecutor,
}

impl Capability for ReplanningCapability {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &TEST_DESCRIPTOR
    }

    fn execute(
        &self,
        operation: &OperationRequest,
        context: &InvocationContext,
    ) -> OperationOutcome {
        let OperationRequest::Format(request) = operation else {
            unreachable!()
        };
        let job = JobId::new("replanned-job").unwrap();
        let seed = Digest::new(DigestAlgorithm::Blake3, vec![9; 32]).unwrap();
        let mut final_report = None;
        for ordinal in 1..=2 {
            let planning = PlanningRecorder::start(
                job.clone(),
                PlanningAttemptId::new(format!("attempt-{ordinal}")).unwrap(),
                context,
            )
            .unwrap();
            let sealed = planning
                .seal(
                    one_plan(
                        "commit",
                        ActionKind::CommitTransaction,
                        Effect::Coordination,
                    ),
                    &seed,
                    PlanMode::Execute,
                )
                .unwrap();
            let report = run(
                sealed,
                &self.executor,
                context,
                &InvocationSettings::default(),
            )
            .unwrap();
            if report.superseded.is_none() {
                final_report = Some(report);
                break;
            }
        }
        let report = final_report.unwrap();
        OperationOutcome {
            job,
            result: OperationResult::Format(FormatResult {
                selected: Vec::new(),
                changed: Vec::new(),
                check: request.check,
                diffs: Vec::new(),
            }),
            totals: report.totals,
            root_failures: report.root_failures,
            cancelled: report.cancelled,
        }
    }
}

#[test]
fn superseded_plan_can_replan_and_finish_in_one_kernel_job() {
    let context = InvocationContext::new(
        InvocationId::new("replanning").unwrap(),
        CancellationToken::default(),
        Arc::new(Events::default()),
    );
    let capability = ReplanningCapability {
        executor: SupersedingExecutor {
            calls: AtomicUsize::new(0),
        },
    };
    let capabilities: [&dyn Capability; 1] = [&capability];
    let outcome = Kernel::new(&capabilities)
        .unwrap()
        .dispatch(&format_request(), &context)
        .unwrap();
    assert_eq!(outcome.summary.totals.succeeded, 1);
    assert_eq!(capability.executor.calls.load(Ordering::SeqCst), 2);
}

struct FailingExecutor;

impl WorkExecutor<TestWork> for FailingExecutor {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &TestWork,
        _: &PreparedPlan<TestWork>,
        _: &ResolvedInputs,
    ) -> Result<Option<squish_manager::orchestrator::CachedResult>, ManagerError> {
        Ok(None)
    }

    fn execute(
        &self,
        _: &Dispatch,
        _: &TestWork,
        _: &PreparedPlan<TestWork>,
        _: &ResolvedInputs,
        _: CancellationToken,
    ) -> WorkDisposition {
        ActionResult::failure("test_failure", "expected failure").into()
    }

    fn record(&self, _: &Dispatch, _: &TestWork, _: &ActionResult) -> Result<(), ManagerError> {
        Ok(())
    }
}

#[test]
fn execution_facts_preserve_failure_blocking_and_optional_keys() {
    let failed = action("failed", ActionKind::Compile);
    let mut blocked = action("blocked", ActionKind::Link);
    blocked.dependencies.push(failed.id.clone());
    blocked.key = KeyRecipe::new(
        "blocked/1",
        Vec::new(),
        BTreeMap::from([("identity".to_owned(), "blocked".to_owned())]),
    )
    .unwrap();
    let graph = BuildPlan::new([failed, blocked]).unwrap();
    let work = graph
        .actions()
        .map(|action| (action.id.clone(), TestWork(action.kind, Effect::Transform)))
        .collect();
    let context = InvocationContext::new(
        InvocationId::new("failure-facts").unwrap(),
        CancellationToken::default(),
        Arc::new(Events::default()),
    );
    let report = run(
        seal(
            JobId::new("failure-facts-job").unwrap(),
            PreparedPlan::new(graph, work).unwrap(),
            &context,
        ),
        &FailingExecutor,
        &context,
        &InvocationSettings::default(),
    )
    .unwrap();
    let failed = report
        .actions
        .iter()
        .find(|fact| fact.action == id("failed"))
        .unwrap();
    let blocked = report
        .actions
        .iter()
        .find(|fact| fact.action == id("blocked"))
        .unwrap();
    assert!(matches!(failed.state, ExecutionState::Failed { .. }));
    assert!(failed.key.is_some());
    assert!(matches!(blocked.state, ExecutionState::Blocked { .. }));
    assert!(blocked.key.is_none());
}

struct FailedFinalization;

impl Capability for FailedFinalization {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &TEST_DESCRIPTOR
    }

    fn execute(
        &self,
        _operation: &OperationRequest,
        context: &InvocationContext,
    ) -> OperationOutcome {
        let job = JobId::new("finalization-job").unwrap();
        let report = run(
            seal(
                job.clone(),
                one_plan("compile", ActionKind::Compile, Effect::Transform),
                context,
            ),
            &Executor,
            context,
            &InvocationSettings::default(),
        )
        .unwrap();
        let failure = finalize(
            &job,
            FinalizationId::new("persist-build-catalog").unwrap(),
            FinalizationKind::PersistBuildCatalog,
            context,
            || -> Result<(), ManagerError> {
                Err(ManagerError::new(
                    "catalog_write_failed",
                    Phase::Cache,
                    "catalog could not be persisted",
                ))
            },
        )
        .unwrap_err();
        OperationOutcome {
            job,
            result: OperationResult::Unavailable {
                kind: OperationKind::Format,
            },
            totals: report.totals,
            root_failures: report.root_failures + failure.root_failures(),
            cancelled: report.cancelled,
        }
    }
}

#[test]
fn failed_finalization_is_a_kernel_visible_root_failure() {
    let context = InvocationContext::new(
        InvocationId::new("finalization").unwrap(),
        CancellationToken::default(),
        Arc::new(Events::default()),
    );
    let capability = FailedFinalization;
    let capabilities: [&dyn Capability; 1] = [&capability];
    let outcome = Kernel::new(&capabilities)
        .unwrap()
        .dispatch(&format_request(), &context)
        .unwrap();
    assert_eq!(outcome.summary.totals.succeeded, 1);
    assert_eq!(outcome.summary.root_failures, 1);
    assert_eq!(outcome.summary.status.code(), 1);
}
