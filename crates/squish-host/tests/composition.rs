//! 生产组合根的跨 crate 契约测试。 / Cross-crate contract tests for the production composition root.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use squish_fetch::{
    FetchError, GitInvocation, GitRunOutput, GitRunner, HttpRequest, HttpResponse, HttpTransport,
    Limits, NoCredentials, NoopObserver,
};
use squish_host::{GitExecution, HostConfig, ProductionHost};
use squish_manager::{ResolveRequest, Services, StorageLayout};
use squish_project::{Manifest, ResolutionMode};

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
    let storage = StorageLayout::new(
        temporary.path().join("cas"),
        temporary.path().join("actions.sqlite"),
        temporary.path().join("publish"),
        temporary.path().join("catalog"),
    )
    .unwrap();
    let host = ProductionHost::open(HostConfig {
        project_root: root,
        source_cache_root: temporary.path().join("sources"),
        storage,
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
