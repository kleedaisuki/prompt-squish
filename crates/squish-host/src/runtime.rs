//! 生产构建运行时组合。 / Production build-runtime composition.

use std::{
    collections::BTreeMap,
    fs::File,
    sync::{Arc, Mutex},
};

use squish_backend::{
    Backend, BackendCacheIdentity, BackendOutput, BackendRequest, SquishBackend, SquishOptions,
};
use squish_build::{
    ActionIndex, ActionKey, ActionRecord, ArtifactRead, BlobStore, CommittedGeneration,
    GenerationRef, Publication, PublicationPath, PublicationTargetId,
};
use squish_ir::SourceKey;
use squish_link::{
    Budgets, InstantiateOutput, Instantiator, LinkOutput, LinkedProgram, StaticLinker, UnitClosure,
};
use squish_manager::{
    BuildRuntime, BuildRuntimeDescriptor, BuildRuntimeError, GenerationSpace, ProjectBuildLayout,
};
use squish_protocol::{Digest, DigestAlgorithm};
use squish_publish::{
    ExternalPublisherLayout, FileArtifactPublisher, PublishError, PublishObserver,
};
use squish_source::SourceBlob;
use squish_store::{ActionKey as StoreActionKey, BlobDigest, Cas, CasError, VerifiedActionIndex};
use squish_xml_front::{FrontendOutput, FrontendSourceContext};

/// 在一个调用中共享同一 CAS、动作索引与两个隔离 publisher 的生产运行时。 /
/// Production runtime sharing one CAS, one action index, and two isolated publishers for an
/// invocation.
pub struct ProductionBuildRuntime {
    layout: ProjectBuildLayout,
    cancellation: squish_kernel::CancellationToken,
    target_observer: Arc<dyn PublishObserver>,
    catalog_observer: Arc<dyn PublishObserver>,
    cas: Mutex<Option<Arc<Cas>>>,
    index: Mutex<Option<Arc<VerifiedActionIndex>>>,
    maintenance: Mutex<Option<Arc<File>>>,
    targets: Mutex<Option<Arc<FileArtifactPublisher<SharedCas>>>>,
    catalog: Mutex<Option<Arc<FileArtifactPublisher<SharedCas>>>>,
}

impl ProductionBuildRuntime {
    /// 记录权威布局与观察者，但不打开或创建任何持久存储。 /
    /// Records the authoritative layout and observers without opening or creating persistence.
    #[must_use]
    pub fn new(
        layout: ProjectBuildLayout,
        cancellation: squish_kernel::CancellationToken,
        target_observer: Arc<dyn PublishObserver>,
        catalog_observer: Arc<dyn PublishObserver>,
    ) -> Self {
        Self {
            layout,
            cancellation,
            target_observer,
            catalog_observer,
            cas: Mutex::new(None),
            index: Mutex::new(None),
            maintenance: Mutex::new(None),
            targets: Mutex::new(None),
            catalog: Mutex::new(None),
        }
    }

    fn cas(&self) -> Result<Arc<Cas>, BuildRuntimeError> {
        self.maintenance()?;
        initialize(&self.cas, "CAS", || {
            Cas::open(self.layout.cas_root()).map_err(|error| storage_error("host_cas_open", error))
        })
    }

    fn index(&self) -> Result<Arc<VerifiedActionIndex>, BuildRuntimeError> {
        initialize(&self.index, "action index", || {
            VerifiedActionIndex::open(self.layout.action_index(), self.cas()?)
                .map_err(|error| storage_error("host_action_index_open", error))
        })
    }

    fn publisher(
        &self,
        space: GenerationSpace,
    ) -> Result<Arc<FileArtifactPublisher<SharedCas>>, BuildRuntimeError> {
        self.maintenance()?;
        let publication_lock = self.layout.metadata_root().join("publication.lock");
        let (slot, root, state, prefix, observer, code, name) = match space {
            GenerationSpace::TargetArtifacts => (
                &self.targets,
                self.layout.artifacts_root().to_path_buf(),
                self.layout.publications_root().to_path_buf(),
                self.layout.publication_prefix().to_path_buf(),
                self.target_observer.clone(),
                "host_target_publisher_open",
                "target publisher",
            ),
            GenerationSpace::BuildCatalog => (
                &self.catalog,
                self.layout.catalog_root().join("artifacts"),
                self.layout.catalog_root().join("state"),
                std::path::PathBuf::new(),
                self.catalog_observer.clone(),
                "host_catalog_publisher_open",
                "catalog publisher",
            ),
        };
        initialize(slot, name, || {
            let layout = ExternalPublisherLayout::new(root, state, prefix, publication_lock);
            FileArtifactPublisher::with_external_layout(layout, SharedCas(self.cas()?), observer)
                .map_err(|error| publication_error(code, error))
        })
    }

    fn maintenance(&self) -> Result<Arc<File>, BuildRuntimeError> {
        initialize(&self.maintenance, "maintenance lease", || {
            super::acquire_build_lease(&self.layout, &self.cancellation).map_err(|error| {
                let code = if error.kind() == std::io::ErrorKind::Interrupted {
                    "host_maintenance_cancelled"
                } else {
                    "host_maintenance_lock"
                };
                storage_error(code, error)
            })
        })
    }

    /// 确保项目派生存储的整个操作生命期持有共享维护租约。 /
    /// Ensures a shared maintenance lease covers the lifetime of project-derived storage use.
    pub(crate) fn acquire_maintenance(&self) -> Result<(), BuildRuntimeError> {
        self.maintenance().map(|_| ())
    }

    pub(crate) fn has_maintenance_lease(&self) -> Result<bool, BuildRuntimeError> {
        let lease = self.maintenance.lock().map_err(|_| {
            BuildRuntimeError::storage(
                "host_runtime_coordination",
                "maintenance initialization mutex is poisoned",
            )
        })?;
        Ok(lease.is_some())
    }

    /// 枚举动作索引中仍通过共享 CAS 完整性校验的记录。 /
    /// Enumerates action records that still pass integrity validation through the shared CAS.
    pub fn cache_records(&self) -> Result<Vec<squish_protocol::CachedAction>, BuildRuntimeError> {
        let index = self.index()?;
        let mut after: Option<StoreActionKey> = None;
        let mut records = Vec::new();
        loop {
            let page = index
                .manifest_page(after, squish_store::MAX_MANIFEST_PAGE_SIZE)
                .map_err(|error| storage_error("host_cache_catalog", error))?;
            for manifest in page.manifests {
                let action_key =
                    squish_protocol::ActionKeyId::new(manifest.record.key.as_str().to_owned())
                        .map_err(|error| corrupt_error("host_cache_catalog", error))?;
                let outputs = manifest
                    .record
                    .outputs
                    .into_iter()
                    .map(|output| {
                        Ok(squish_protocol::Artifact {
                            id: squish_protocol::ArtifactId::new(output.name.as_str().to_owned())
                                .map_err(|error| corrupt_error("host_cache_catalog", error))?,
                            kind: output.kind,
                            uri: format!("cas:blake3:{}", output.digest.hex()),
                            size: output.size,
                            digest: output.digest,
                        })
                    })
                    .collect::<Result<Vec<_>, BuildRuntimeError>>()?;
                records.push(squish_protocol::CachedAction {
                    action_key,
                    result_digest: protocol_digest(manifest.result_digest),
                    outputs,
                });
            }
            after = page.next_after;
            if after.is_none() {
                break;
            }
        }
        Ok(records)
    }
}

impl BuildRuntime for ProductionBuildRuntime {
    fn descriptor(&self) -> BuildRuntimeDescriptor {
        BuildRuntimeDescriptor {
            frontend_abi: "xmlsquish.xml/1".into(),
            linker_abi: "xmlsquish.link/1".into(),
            evaluator_abi: "xmlsquish.instantiate/1".into(),
            document_abi: squish_backend::DOCUMENT_ABI.into(),
        }
    }

    fn backend_identity(
        &self,
        options: SquishOptions,
    ) -> Result<BackendCacheIdentity, BuildRuntimeError> {
        Ok(SquishBackend.cache_identity(options))
    }

    fn read_blob(&self, digest: &Digest) -> Result<Option<Vec<u8>>, BuildRuntimeError> {
        self.cas()?
            .get(blob_digest(digest)?)
            .map_err(|error| storage_error("host_blob_read", error))
    }

    fn write_blob(&self, bytes: &[u8]) -> Result<Digest, BuildRuntimeError> {
        let digest = self
            .cas()?
            .put(bytes)
            .map_err(|error| storage_error("host_blob_write", error))?;
        Digest::new(DigestAlgorithm::Blake3, digest.as_bytes().to_vec())
            .map_err(|error| corrupt_error("host_blob_digest", error))
    }

    fn lookup_action(&self, key: &ActionKey) -> Result<Option<ActionRecord>, BuildRuntimeError> {
        self.index()?
            .lookup(key)
            .map_err(|error| storage_error("host_action_lookup", error))
    }

    fn record_action(&self, record: &ActionRecord) -> Result<(), BuildRuntimeError> {
        self.index()?
            .record(record)
            .map_err(|error| storage_error("host_action_record", error))
    }

    fn current_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
    ) -> Result<Option<CommittedGeneration>, BuildRuntimeError> {
        self.publisher(space)?
            .current_generation(target)
            .map_err(|error| publication_error("host_generation_read", error))
    }

    fn read_generation_artifact(
        &self,
        space: GenerationSpace,
        generation: &GenerationRef,
        destination: &PublicationPath,
    ) -> Result<(ArtifactRead, Vec<u8>), BuildRuntimeError> {
        let mut bytes = Vec::new();
        let read = self
            .publisher(space)?
            .read_generation_artifact(generation, destination, &mut bytes)
            .map_err(|error| publication_error("host_generation_artifact_read", error))?;
        Ok((read, bytes))
    }

    fn publish_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
        publications: &[Publication],
    ) -> Result<CommittedGeneration, BuildRuntimeError> {
        self.publisher(space)?
            .publish_generation(target, publications)
            .map_err(|error| publication_error("host_generation_publish", error))
    }

    fn compile(
        &self,
        source: &SourceBlob,
        context: &FrontendSourceContext,
    ) -> Result<FrontendOutput, BuildRuntimeError> {
        squish_xml_front::compile(source, context)
            .map_err(|error| BuildRuntimeError::new("host_frontend_compile", error.message))
    }

    fn link(
        &self,
        entry: &SourceKey,
        closure: UnitClosure,
    ) -> Result<LinkOutput, BuildRuntimeError> {
        StaticLinker
            .link(entry, closure)
            .map_err(|error| tool_error("host_static_link", error))
    }

    fn instantiate(
        &self,
        program: &LinkedProgram,
        arguments: BTreeMap<String, String>,
        budgets: Budgets,
    ) -> Result<InstantiateOutput, BuildRuntimeError> {
        Instantiator
            .instantiate(program, arguments, budgets)
            .map_err(|error| tool_error("host_instantiate", error))
    }

    fn render(&self, request: BackendRequest) -> Result<BackendOutput, BuildRuntimeError> {
        SquishBackend
            .emit(request)
            .map_err(|error| tool_error("host_backend_render", error))
    }
}

#[derive(Clone)]
struct SharedCas(Arc<Cas>);

impl BlobStore for SharedCas {
    type Error = CasError;

    fn copy_to(
        &self,
        digest: &squish_build::ContentDigest,
        sink: &mut dyn std::io::Write,
    ) -> Result<bool, Self::Error> {
        self.0.copy_to(digest, sink)
    }

    fn write_from(
        &self,
        source: &mut dyn std::io::Read,
    ) -> Result<squish_build::ContentDigest, Self::Error> {
        self.0.write_from(source)
    }
}

fn initialize<T>(
    slot: &Mutex<Option<Arc<T>>>,
    capability: &str,
    open: impl FnOnce() -> Result<T, BuildRuntimeError>,
) -> Result<Arc<T>, BuildRuntimeError> {
    let mut slot = slot.lock().map_err(|_| {
        BuildRuntimeError::storage(
            "host_runtime_coordination",
            format!("{capability} initialization mutex is poisoned"),
        )
    })?;
    if let Some(value) = slot.as_ref() {
        return Ok(value.clone());
    }
    let value = Arc::new(open()?);
    *slot = Some(value.clone());
    Ok(value)
}

fn blob_digest(digest: &Digest) -> Result<BlobDigest, BuildRuntimeError> {
    if digest.algorithm() != &DigestAlgorithm::Blake3 {
        return Err(BuildRuntimeError::corrupt(
            "host_blob_digest",
            "build runtime accepts only BLAKE3 blob identities",
        ));
    }
    let bytes: [u8; 32] = digest.bytes().try_into().map_err(|_| {
        BuildRuntimeError::corrupt(
            "host_blob_digest",
            "BLAKE3 blob identity has invalid length",
        )
    })?;
    Ok(BlobDigest::from_bytes(bytes))
}

fn protocol_digest(digest: BlobDigest) -> Digest {
    Digest::new(DigestAlgorithm::Blake3, digest.as_bytes().to_vec())
        .expect("BLAKE3 digest has the protocol's canonical length")
}

fn tool_error(code: &'static str, error: impl std::fmt::Display) -> BuildRuntimeError {
    BuildRuntimeError::new(code, error.to_string())
}

fn storage_error(code: &'static str, error: impl std::fmt::Display) -> BuildRuntimeError {
    BuildRuntimeError::storage(code, error.to_string())
}

fn corrupt_error(code: &'static str, error: impl std::fmt::Display) -> BuildRuntimeError {
    BuildRuntimeError::corrupt(code, error.to_string())
}

fn publication_error(code: &'static str, error: PublishError<CasError>) -> BuildRuntimeError {
    let message = error.to_string();
    match error {
        PublishError::Store(_)
        | PublishError::Io(_)
        | PublishError::MaintenancePending(_)
        | PublishError::Superseded(_) => BuildRuntimeError::storage(code, message),
        PublishError::InvalidDestination(_)
        | PublishError::AliasConflict(_)
        | PublishError::Symlink(_)
        | PublishError::MissingBlob
        | PublishError::IntegrityMismatch
        | PublishError::UnsupportedDigest(_)
        | PublishError::Journal(_) => BuildRuntimeError::corrupt(code, message),
    }
}

#[cfg(test)]
mod tests {
    use std::{io, sync::Barrier, thread};

    use squish_ir::PackageInstanceId;
    use squish_manager::BuildRuntimeErrorKind;
    use squish_source::{
        LogicalPath, PackageId, SnapshotBuilder, SourceId, SourceLocator, SourceProvider,
    };

    use super::*;

    #[test]
    fn publisher_integrity_and_io_failures_keep_distinct_categories() {
        let corrupt = publication_error("fixture", PublishError::<CasError>::IntegrityMismatch);
        assert_eq!(corrupt.kind(), BuildRuntimeErrorKind::Corrupt);

        let storage = publication_error(
            "fixture",
            PublishError::<CasError>::Io(io::Error::other("fixture unavailable")),
        );
        assert_eq!(storage.kind(), BuildRuntimeErrorKind::Storage);
    }

    #[derive(Clone)]
    struct MemorySource(Vec<u8>);

    impl SourceProvider for MemorySource {
        fn read(&self, _: &SourceLocator) -> io::Result<Vec<u8>> {
            Ok(self.0.clone())
        }
    }

    fn layout(root: &std::path::Path) -> ProjectBuildLayout {
        ProjectBuildLayout::new(std::fs::canonicalize(root).unwrap(), "build").unwrap()
    }

    fn runtime(layout: ProjectBuildLayout) -> ProductionBuildRuntime {
        ProductionBuildRuntime::new(
            layout,
            squish_kernel::CancellationToken::default(),
            Arc::new(squish_publish::NoopObserver),
            Arc::new(squish_publish::NoopObserver),
        )
    }

    #[test]
    fn compile_does_not_initialize_any_persistence_capability() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = layout(temporary.path());
        let paths = [
            layout.cas_root().to_owned(),
            layout.action_index().to_owned(),
            layout.artifacts_root().to_owned(),
            layout.catalog_root().to_owned(),
        ];
        let runtime = runtime(layout);
        let xml = format!(
            r#"<xs:entry xmlns:xs="{}"><R/></xs:entry>"#,
            squish_xml_front::DSL_NAMESPACE
        );
        let mut builder = SnapshotBuilder::new(MemorySource(xml.into_bytes()));
        let source = builder
            .load(
                SourceId::new(
                    PackageId::new("fixture").unwrap(),
                    LogicalPath::new("src/main.xml").unwrap(),
                ),
                SourceLocator::file("unused"),
            )
            .unwrap();
        let context = FrontendSourceContext::new(PackageInstanceId {
            source_kind: 1,
            canonical_source: "workspace:fixture".into(),
            package_name: "fixture".into(),
            exact_revision: "manifest:fixture@1".into(),
        });

        runtime.compile(&source, &context).unwrap();
        assert!(paths.iter().all(|path| !path.exists()));
    }

    #[test]
    fn concurrent_first_cas_acquisition_reuses_one_instance() {
        let temporary = tempfile::tempdir().unwrap();
        let runtime = Arc::new(runtime(layout(temporary.path())));
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|_| {
                let runtime = runtime.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    Arc::as_ptr(&runtime.cas().unwrap()) as usize
                })
            })
            .collect::<Vec<_>>();
        let identities = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert!(identities.iter().all(|identity| *identity == identities[0]));
    }

    #[test]
    fn failed_index_initialization_retries_without_blocking_blob_storage() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = layout(temporary.path());
        let runtime = runtime(layout.clone());
        runtime.acquire_maintenance().unwrap();
        std::fs::create_dir_all(layout.action_index()).unwrap();

        assert!(runtime.index().is_err());
        runtime.write_blob(b"independent CAS").unwrap();
        std::fs::remove_dir(layout.action_index()).unwrap();
        assert!(runtime.index().is_ok());
    }

    #[test]
    fn target_and_catalog_publishers_share_the_short_publication_lock() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = layout(temporary.path());
        let expected = layout.metadata_root().join("publication.lock");
        let runtime = runtime(layout);

        let targets = runtime.publisher(GenerationSpace::TargetArtifacts).unwrap();
        let catalog = runtime.publisher(GenerationSpace::BuildCatalog).unwrap();
        assert_eq!(targets.lock_path(), catalog.lock_path());
        assert_eq!(targets.lock_path(), expected);
    }
}
