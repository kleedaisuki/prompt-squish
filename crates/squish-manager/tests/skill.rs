//! Focused manager-level tests for the public skill dependency workflow.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use squish_kernel::{CancellationToken, Capability, EventSink, InvocationContext, SinkError};
use squish_manager::{
    InvocationSettings, ManagerCapability, ProjectBuildLayout, ResolveRequest,
    ResolvedDependencies, ResolvedSkillSource, ServiceError, Services,
};
use squish_project::{LOCK_VERSION, Lockfile, ResolutionMode};
use squish_protocol::{
    AddSkillRequest, Event, InstallSkillRequest, InvocationId, LockMode, OperationRequest,
    OperationResult, ProjectPath, RemoveSkillRequest, SkillName, SkillSource, SyncSkillsRequest,
};
use tempfile::{Builder, TempDir};

/// A protocol event sink that preserves manager lifecycle checks without output.
struct Sink;

impl EventSink for Sink {
    fn emit(&self, _: Event) -> Result<(), SinkError> {
        Ok(())
    }
}

/// A deterministic local-source and XML-package resolver for skill manager tests.
struct LocalServices;

impl Services for LocalServices {
    fn storage_layout(&self, project: &Path) -> Result<ProjectBuildLayout, ServiceError> {
        Ok(ProjectBuildLayout::project_local_for_tests(project))
    }

    fn materialize_locked(
        &self,
        _: &Path,
        _: &Lockfile,
        _: ResolutionMode,
    ) -> Result<Vec<squish_repository::PackageLocation>, ServiceError> {
        Ok(Vec::new())
    }

    fn resolve(&self, request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        let mut lock = request.prior_lock.cloned().unwrap_or(Lockfile {
            lock_version: LOCK_VERSION,
            resolver_version: "skill-test/1".into(),
            manifest_digest: request.manifest_digest.into(),
            packages: Vec::new(),
            skills: Vec::new(),
        });
        lock.manifest_digest = request.manifest_digest.into();
        Ok(ResolvedDependencies {
            lockfile: lock,
            packages: Vec::new(),
        })
    }

    fn resolve_skill_source(
        &self,
        root: &Path,
        source: &SkillSource,
        _: Option<&str>,
        _: ResolutionMode,
    ) -> Result<ResolvedSkillSource, ServiceError> {
        let SkillSource::Path { path } = source else {
            return Err(ServiceError::new(
                "test_source",
                "only local skill fixtures supported",
            ));
        };
        let directory = Path::new(path.as_str());
        let directory = if directory.is_absolute() {
            directory.to_path_buf()
        } else {
            root.join(directory)
        };
        let mut files = BTreeMap::new();
        for entry in
            fs::read_dir(&directory).map_err(|e| ServiceError::new("test_source", e.to_string()))?
        {
            let entry = entry.map_err(|e| ServiceError::new("test_source", e.to_string()))?;
            if !entry
                .file_type()
                .map_err(|e| ServiceError::new("test_source", e.to_string()))?
                .is_file()
            {
                return Err(ServiceError::new("test_source", "non-regular fixture"));
            }
            files.insert(
                PathBuf::from(entry.file_name()),
                fs::read(entry.path())
                    .map_err(|e| ServiceError::new("test_source", e.to_string()))?,
            );
        }
        Ok(ResolvedSkillSource {
            revision: None,
            files,
        })
    }
}

fn project() -> TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".temp");
    fs::create_dir_all(&root).unwrap();
    let temp = Builder::new()
        .prefix("manager-skill-")
        .tempdir_in(root)
        .unwrap();
    fs::write(
        temp.path().join("xmlsquish.toml"),
        "manifest-version = 1\n[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    temp
}

fn context() -> InvocationContext {
    InvocationContext::new(
        InvocationId::new("skill-test").unwrap(),
        CancellationToken::default(),
        Arc::new(Sink),
    )
}

fn project_path(root: &Path) -> ProjectPath {
    ProjectPath::new(root.to_string_lossy().into_owned()).unwrap()
}

fn add_request(root: &Path, dry_run: bool) -> AddSkillRequest {
    AddSkillRequest {
        project: project_path(root),
        skill: SkillName::new("review").unwrap(),
        source: SkillSource::Path {
            path: ProjectPath::new("skill-src").unwrap(),
        },
        lock: LockMode::Update,
        dry_run,
    }
}

fn skill_source(root: &Path) {
    let source = root.join("skill-src");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: review\ndescription: Review an XML project.\n---\n# Review\n",
    )
    .unwrap();
    fs::write(source.join("notes.txt"), b"support file").unwrap();
}

#[test]
fn add_sync_and_remove_manage_complete_tree() {
    let temp = project();
    skill_source(temp.path());
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let add = manager.execute(
        &OperationRequest::AddSkill(add_request(temp.path(), false)),
        &context(),
    );
    assert!(
        matches!(add.result, OperationResult::AddSkill(_)),
        "{add:?}"
    );
    let destination = temp.path().join(".agents/skills/review");
    assert_eq!(
        fs::read(destination.join("notes.txt")).unwrap(),
        b"support file"
    );
    let lock =
        Lockfile::parse(&fs::read_to_string(temp.path().join("xmlsquish.lock")).unwrap()).unwrap();
    assert_eq!(lock.skills.len(), 1);
    assert!(
        fs::read_to_string(temp.path().join("xmlsquish.toml"))
            .unwrap()
            .contains("review = { path = \"skill-src\" }")
    );

    fs::remove_dir_all(&destination).unwrap();
    let sync = manager.execute(
        &OperationRequest::SyncSkills(SyncSkillsRequest {
            project: project_path(temp.path()),
            lock: LockMode::Frozen,
            dry_run: false,
        }),
        &context(),
    );
    assert!(
        matches!(sync.result, OperationResult::SyncSkills(ref v) if v.installed == 1 && v.unchanged == 0),
        "{sync:?}"
    );
    assert_eq!(
        fs::read(destination.join("notes.txt")).unwrap(),
        b"support file"
    );

    let remove = manager.execute(
        &OperationRequest::RemoveSkill(RemoveSkillRequest {
            project: project_path(temp.path()),
            skill: SkillName::new("review").unwrap(),
            lock: LockMode::Update,
            dry_run: false,
        }),
        &context(),
    );
    assert!(
        matches!(remove.result, OperationResult::RemoveSkill(_)),
        "{remove:?}"
    );
    assert!(!destination.exists());
}

#[test]
fn dry_run_and_user_edit_do_not_destroy_projection() {
    let temp = project();
    skill_source(temp.path());
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let dry = manager.execute(
        &OperationRequest::AddSkill(add_request(temp.path(), true)),
        &context(),
    );
    assert!(
        matches!(dry.result, OperationResult::AddSkill(ref v) if v.dry_run),
        "{dry:?}"
    );
    assert!(!temp.path().join("xmlsquish.lock").exists());
    assert!(!temp.path().join(".agents/skills/review").exists());

    manager.execute(
        &OperationRequest::AddSkill(add_request(temp.path(), false)),
        &context(),
    );
    let edited = temp.path().join(".agents/skills/review/notes.txt");
    fs::write(&edited, b"user changed this").unwrap();
    let remove = manager.execute(
        &OperationRequest::RemoveSkill(RemoveSkillRequest {
            project: project_path(temp.path()),
            skill: SkillName::new("review").unwrap(),
            lock: LockMode::Update,
            dry_run: false,
        }),
        &context(),
    );
    assert!(
        matches!(remove.result, OperationResult::Unavailable { .. }),
        "{remove:?}"
    );
    assert_eq!(fs::read(edited).unwrap(), b"user changed this");
    assert_eq!(
        Lockfile::parse(&fs::read_to_string(temp.path().join("xmlsquish.lock")).unwrap())
            .unwrap()
            .skills
            .len(),
        1
    );
}

#[test]
fn bundled_install_is_independent_of_project_lock() {
    let temp = project();
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let outcome = manager.execute(
        &OperationRequest::InstallSkill(InstallSkillRequest {
            project: Some(project_path(temp.path())),
            force: false,
            dry_run: false,
        }),
        &context(),
    );
    assert!(
        matches!(outcome.result, OperationResult::InstallSkill(ref v) if v.changed),
        "{outcome:?}"
    );
    assert!(
        temp.path()
            .join(".agents/skills/prompt-squish/SKILL.md")
            .exists()
    );
    assert!(!temp.path().join("xmlsquish.lock").exists());
    let second = manager.execute(
        &OperationRequest::InstallSkill(InstallSkillRequest {
            project: Some(project_path(temp.path())),
            force: false,
            dry_run: false,
        }),
        &context(),
    );
    assert!(
        matches!(second.result, OperationResult::InstallSkill(ref v) if !v.changed),
        "{second:?}"
    );
}

#[test]
fn absolute_local_path_is_recorded_relative_to_manifest() {
    let temp = project();
    skill_source(temp.path());
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let mut request = add_request(temp.path(), false);
    request.source = SkillSource::Path {
        path: ProjectPath::new(temp.path().join("skill-src").to_string_lossy().into_owned())
            .unwrap(),
    };
    let outcome = manager.execute(&OperationRequest::AddSkill(request), &context());
    assert!(
        matches!(outcome.result, OperationResult::AddSkill(_)),
        "{outcome:?}"
    );
    assert!(
        fs::read_to_string(temp.path().join("xmlsquish.toml"))
            .unwrap()
            .contains("review = { path = \"skill-src\" }")
    );
}

#[test]
fn sync_prunes_only_stale_dependency_projection() {
    let temp = project();
    skill_source(temp.path());
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let add = manager.execute(
        &OperationRequest::AddSkill(add_request(temp.path(), false)),
        &context(),
    );
    assert!(
        matches!(add.result, OperationResult::AddSkill(_)),
        "{add:?}"
    );
    let destination = temp.path().join(".agents/skills/review");
    let saved: Vec<_> = fs::read_dir(&destination)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).unwrap())
        })
        .collect();
    let bundle = manager.execute(
        &OperationRequest::InstallSkill(InstallSkillRequest {
            project: Some(project_path(temp.path())),
            force: false,
            dry_run: false,
        }),
        &context(),
    );
    assert!(
        matches!(bundle.result, OperationResult::InstallSkill(_)),
        "{bundle:?}"
    );
    let remove = manager.execute(
        &OperationRequest::RemoveSkill(RemoveSkillRequest {
            project: project_path(temp.path()),
            skill: SkillName::new("review").unwrap(),
            lock: LockMode::Update,
            dry_run: false,
        }),
        &context(),
    );
    assert!(
        matches!(remove.result, OperationResult::RemoveSkill(_)),
        "{remove:?}"
    );
    fs::create_dir(&destination).unwrap();
    for (name, bytes) in saved {
        fs::write(destination.join(name), bytes).unwrap();
    }
    let sync = manager.execute(
        &OperationRequest::SyncSkills(SyncSkillsRequest {
            project: project_path(temp.path()),
            lock: LockMode::Frozen,
            dry_run: false,
        }),
        &context(),
    );
    assert!(
        matches!(sync.result, OperationResult::SyncSkills(ref v) if v.removed == 1),
        "{sync:?}"
    );
    assert!(!destination.exists());
    assert!(
        temp.path()
            .join(".agents/skills/prompt-squish/SKILL.md")
            .exists()
    );
}
