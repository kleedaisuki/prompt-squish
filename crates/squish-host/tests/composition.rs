//! 生产组合根的跨 crate 契约测试。 / Cross-crate contract tests for the production composition root.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use squish_build::{
    LogicalArtifactName, OutputName, ProducedOutput, Publication, PublicationPath,
    PublicationTargetId,
};
use squish_fetch::{
    FetchError, GitInvocation, GitRunOutput, GitRunner, HttpRequest, HttpResponse, HttpTransport,
    Limits, NoCredentials, NoopObserver,
};
use squish_host::{GitExecution, HostConfig, ProductionHost};
use squish_manager::{GenerationSpace, ProjectBuildLayout, ResolveRequest, Services};
use squish_project::{Manifest, ResolutionMode};
use squish_protocol::ArtifactKind;

struct NoHttp;
impl HttpTransport for NoHttp {
    fn execute(&self, _request: HttpRequest) -> Result<HttpResponse, FetchError> {
        panic!("path-only integration fixture attempted HTTP")
    }
}

struct NoGit;
impl GitRunner for NoGit {
    fn execute(&self, _invocation: GitInvocation) -> std::io::Result<GitRunOutput> {
        panic!("path-only integration fixture attempted Git")
    }
}

fn fixture() -> (tempfile::TempDir, ProductionHost) {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
    std::fs::create_dir_all(&scratch).unwrap();
    let scratch = std::fs::canonicalize(scratch).unwrap();
    let temporary = tempfile::Builder::new()
        .prefix("squish-host-integration-")
        .tempdir_in(scratch)
        .unwrap();
    let root = temporary.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let filesystem = Arc::new(squish_fetch::FilesystemHost::new(&root).unwrap());
    let storage = ProjectBuildLayout::new(&root, "target/xmlsquish").unwrap();
    let host = ProductionHost::open(HostConfig {
        project_root: root,
        storage,
        cancellation: squish_kernel::CancellationToken::default(),
        registries: Vec::new(),
        credentials: Arc::new(NoCredentials),
        http: Arc::new(NoHttp),
        git: GitExecution::Runner(Arc::new(NoGit)),
        limits: Limits::default(),
        observer: Arc::new(NoopObserver),
        filesystem,
    })
    .unwrap();
    (temporary, host)
}

#[test]
fn public_services_preserve_exact_locked_and_frozen_resolution() {
    let (_temporary, host) = fixture();
    let manifest =
        Manifest::parse("manifest-version = 1\n[package]\nname = \"demo\"\nversion = \"1.0.0\"\n")
            .unwrap();
    let manifests = BTreeMap::from([("xmlsquish.toml".into(), manifest)]);
    let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let initial = Services::resolve(
        &host,
        ResolveRequest {
            manifests: &manifests,
            manifest_digest: digest,
            prior_lock: None,
            mode: ResolutionMode::Online,
        },
    )
    .unwrap();

    for mode in [ResolutionMode::Locked, ResolutionMode::Frozen] {
        let reused = Services::resolve(
            &host,
            ResolveRequest {
                manifests: &manifests,
                manifest_digest: digest,
                prior_lock: Some(&initial.lockfile),
                mode,
            },
        )
        .unwrap();
        assert_eq!(reused.lockfile, initial.lockfile);
        assert!(reused.packages.is_empty());
    }
}

#[test]
fn production_runtime_uses_one_cas_and_isolated_generation_spaces() {
    let (temporary, host) = fixture();
    let runtime = Services::open_build_runtime(&host, host.project_root()).unwrap();
    let reopened = Services::open_build_runtime(&host, host.project_root()).unwrap();
    assert!(Arc::ptr_eq(&runtime, &reopened));
    let descriptor = runtime.descriptor();
    assert_eq!(descriptor.frontend_abi, "xmlsquish.xml/1");
    assert_eq!(descriptor.linker_abi, "xmlsquish.link/1");
    assert_eq!(descriptor.evaluator_abi, "xmlsquish.instantiate/1");
    assert_eq!(descriptor.document_abi, squish_backend::DOCUMENT_ABI);

    let digest = runtime.write_blob(b"shared runtime blob").unwrap();
    assert_eq!(
        Services::read_blob(&host, host.project_root(), &digest).unwrap(),
        Some(b"shared runtime blob".to_vec())
    );

    let target = PublicationTargetId::new("runtime-fixture").unwrap();
    let target_generation = runtime
        .publish_generation(GenerationSpace::TargetArtifacts, &target, &[])
        .unwrap();
    let catalog_generation = runtime
        .publish_generation(GenerationSpace::BuildCatalog, &target, &[])
        .unwrap();
    assert_eq!(
        runtime
            .current_generation(GenerationSpace::TargetArtifacts, &target)
            .unwrap(),
        Some(target_generation)
    );
    assert_eq!(
        runtime
            .current_generation(GenerationSpace::BuildCatalog, &target)
            .unwrap(),
        Some(catalog_generation)
    );

    let prompt = runtime.write_blob(b"stable prompt").unwrap();
    runtime
        .publish_generation(
            GenerationSpace::TargetArtifacts,
            &target,
            &[Publication {
                output: ProducedOutput {
                    name: OutputName::new("prompt.prompt").unwrap(),
                    kind: ArtifactKind::Prompt,
                    digest: prompt,
                    size: 13,
                },
                name: LogicalArtifactName::new("prompt.prompt").unwrap(),
                destination: PublicationPath::new("target/xmlsquish/artifacts/prompt.prompt")
                    .unwrap(),
            }],
        )
        .unwrap();
    assert_eq!(
        std::fs::read(
            temporary
                .path()
                .join("project/target/xmlsquish/artifacts/prompt.prompt")
        )
        .unwrap(),
        b"stable prompt"
    );
    assert!(
        !temporary
            .path()
            .join("project/target/xmlsquish/artifacts/target")
            .exists()
    );
    assert!(
        temporary
            .path()
            .join("project/target/xmlsquish/metadata/publications/generations")
            .exists()
    );
}
