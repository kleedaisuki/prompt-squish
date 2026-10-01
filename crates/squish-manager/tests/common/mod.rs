//! 管理器集成测试使用的确定性文件运行时。 / Deterministic filesystem runtime for manager integration tests.
#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use squish_backend::{
    Backend, BackendCacheIdentity, BackendOutput, BackendRequest, SquishBackend, SquishOptions,
};
use squish_build::{
    ActionIndex, ActionKey, ActionRecord, ArtifactDescriptor, ArtifactRead, CommittedGeneration,
    GenerationArtifact, GenerationId, GenerationRef, Publication, PublicationPath,
    PublicationTargetId, VerifiedAction,
};
use squish_ir::SourceKey;
use squish_link::{
    Budgets, InstantiateOutput, Instantiator, LinkOutput, LinkedProgram, StaticLinker, UnitClosure,
};
use squish_manager::{
    BuildRuntime, BuildRuntimeDescriptor, BuildRuntimeError, GenerationSpace, ProjectBuildLayout,
};
use squish_protocol::{ArtifactId, Digest, DigestAlgorithm};
use squish_publish::{FileArtifactPublisher, NoopObserver, PublishError, PublishObserver};
use squish_source::SourceBlob;
use squish_store::{BlobDigest, Cas, CasError, VerifiedActionIndex};
use squish_xml_front::{FrontendOutput, FrontendSourceContext};

/// 共享真实领域实现、但把全部状态固定在测试布局中的运行时。 /
/// Runtime sharing the real domain implementations while pinning all state to a test layout.
pub struct TestBuildRuntime {
    cas: Arc<Cas>,
    index: VerifiedActionIndex,
    targets: FileArtifactPublisher<Cas>,
    catalog: FileArtifactPublisher<Cas>,
}

impl TestBuildRuntime {
    /// 用无操作发布观察者打开运行时。 / Opens a runtime with no-op publication observers.
    pub fn open(layout: &ProjectBuildLayout) -> Result<Self, BuildRuntimeError> {
        Self::with_observers(layout, Arc::new(NoopObserver), Arc::new(NoopObserver))
    }

    /// 用显式 target/catalog 观察者打开运行时。 / Opens a runtime with explicit target/catalog observers.
    pub fn with_observers(
        layout: &ProjectBuildLayout,
        target_observer: Arc<dyn PublishObserver>,
        catalog_observer: Arc<dyn PublishObserver>,
    ) -> Result<Self, BuildRuntimeError> {
        let cas = Arc::new(
            Cas::open(layout.cas_root()).map_err(|error| runtime_error("test_cas_open", error))?,
        );
        let index = VerifiedActionIndex::open(layout.action_index(), cas.clone())
            .map_err(|error| runtime_error("test_action_index_open", error))?;
        let targets = FileArtifactPublisher::with_observer(
            layout.artifacts_root(),
            Cas::open(layout.cas_root()).map_err(|error| runtime_error("test_cas_open", error))?,
            target_observer,
        )
        .map_err(|error| publication_error("test_target_publisher_open", error))?;
        let catalog = FileArtifactPublisher::with_observer(
            layout.catalog_root(),
            Cas::open(layout.cas_root()).map_err(|error| runtime_error("test_cas_open", error))?,
            catalog_observer,
        )
        .map_err(|error| publication_error("test_catalog_publisher_open", error))?;
        Ok(Self {
            cas,
            index,
            targets,
            catalog,
        })
    }

    fn publisher(&self, space: GenerationSpace) -> &FileArtifactPublisher<Cas> {
        match space {
            GenerationSpace::TargetArtifacts => &self.targets,
            GenerationSpace::BuildCatalog => &self.catalog,
        }
    }
}

impl BuildRuntime for TestBuildRuntime {
    fn descriptor(&self) -> BuildRuntimeDescriptor {
        BuildRuntimeDescriptor {
            frontend_abi: squish_xml_front::FRONTEND_ABI.into(),
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
        self.cas
            .get(blob_digest(digest)?)
            .map_err(|error| runtime_error("test_blob_read", error))
    }

    fn write_blob(&self, bytes: &[u8]) -> Result<Digest, BuildRuntimeError> {
        let digest = self
            .cas
            .put(bytes)
            .map_err(|error| runtime_error("test_blob_write", error))?;
        Digest::new(DigestAlgorithm::Blake3, digest.as_bytes().to_vec())
            .map_err(|error| runtime_error("test_blob_digest", error))
    }

    fn lookup_action(&self, key: &ActionKey) -> Result<Option<ActionRecord>, BuildRuntimeError> {
        self.index
            .lookup(key)
            .map_err(|error| runtime_error("test_action_lookup", error))
    }

    fn record_action(&self, record: &ActionRecord) -> Result<(), BuildRuntimeError> {
        self.index
            .record(record)
            .map_err(|error| runtime_error("test_action_record", error))
    }

    fn current_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
    ) -> Result<Option<CommittedGeneration>, BuildRuntimeError> {
        self.publisher(space)
            .current_generation(target)
            .map_err(|error| publication_error("test_generation_read", error))
    }

    fn read_generation_artifact(
        &self,
        space: GenerationSpace,
        generation: &GenerationRef,
        destination: &PublicationPath,
    ) -> Result<(ArtifactRead, Vec<u8>), BuildRuntimeError> {
        let mut bytes = Vec::new();
        let read = self
            .publisher(space)
            .read_generation_artifact(generation, destination, &mut bytes)
            .map_err(|error| publication_error("test_generation_artifact_read", error))?;
        Ok((read, bytes))
    }

    fn publish_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
        publications: &[Publication],
    ) -> Result<CommittedGeneration, BuildRuntimeError> {
        self.publisher(space)
            .publish_generation(target, publications)
            .map_err(|error| publication_error("test_generation_publish", error))
    }

    fn compile(
        &self,
        source: &SourceBlob,
        context: &FrontendSourceContext,
    ) -> Result<FrontendOutput, BuildRuntimeError> {
        squish_xml_front::compile(source, context)
            .map_err(|error| BuildRuntimeError::new("test_frontend_compile", error.message))
    }

    fn link(
        &self,
        entry: &SourceKey,
        closure: UnitClosure,
    ) -> Result<LinkOutput, BuildRuntimeError> {
        StaticLinker
            .link(entry, closure)
            .map_err(|error| runtime_error("test_static_link", error))
    }

    fn instantiate(
        &self,
        program: &LinkedProgram,
        arguments: BTreeMap<String, String>,
        budgets: Budgets,
    ) -> Result<InstantiateOutput, BuildRuntimeError> {
        Instantiator
            .instantiate(program, arguments, budgets)
            .map_err(|error| runtime_error("test_instantiate", error))
    }

    fn render(&self, request: BackendRequest) -> Result<BackendOutput, BuildRuntimeError> {
        SquishBackend
            .emit(request)
            .map_err(|error| runtime_error("test_backend_render", error))
    }
}

fn blob_digest(digest: &Digest) -> Result<BlobDigest, BuildRuntimeError> {
    if digest.algorithm() != &DigestAlgorithm::Blake3 {
        return Err(BuildRuntimeError::new(
            "test_blob_digest",
            "test runtime accepts only BLAKE3 blob identities",
        ));
    }
    let bytes: [u8; 32] = digest.bytes().try_into().map_err(|_| {
        BuildRuntimeError::new(
            "test_blob_digest",
            "BLAKE3 blob identity has invalid length",
        )
    })?;
    Ok(BlobDigest::from_bytes(bytes))
}

fn runtime_error(code: &'static str, error: impl std::fmt::Display) -> BuildRuntimeError {
    BuildRuntimeError::new(code, error.to_string())
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

/// 可独立注入失败的运行时能力。 / Runtime capabilities that can fail independently.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RuntimeFault {
    /// A legacy adapter returns a valid manifest for a different requested action key.
    LookupWrongKey,
    /// A verified adapter omits a companion output handle from an otherwise valid manifest.
    LookupWrongOutputCount,
    /// An adapter claims the correct published descriptor while returning changed bytes.
    ReadPublishedWrongBytes,
    /// Blob 写入。 / Blob writes.
    WriteBlob,
    /// 前端编译。 / Frontend compilation.
    Compile,
    /// Target generation 发布。 / Target-generation publication.
    PublishTarget,
    /// Catalog generation 发布。 / Catalog-generation publication.
    PublishCatalog,
    /// Catalog 读取返回完整性故障。 / Catalog reads return an integrity failure.
    ReadCatalogCorrupt,
    /// Catalog 读取返回存储故障。 / Catalog reads return a storage failure.
    ReadCatalogStorage,
}

type PublishedMemberKey = (u8, String, [u8; 32], String);

#[derive(Default)]
struct MemoryState {
    blobs: BTreeMap<Vec<u8>, Vec<u8>>,
    actions: BTreeMap<String, ActionRecord>,
    current: BTreeMap<(u8, String), CommittedGeneration>,
    generations: BTreeMap<(u8, String, [u8; 32]), CommittedGeneration>,
    /// Counts deliberately invalid cache results, proving tests actually reach a warm lookup.
    injected_cache_results: usize,
    /// Independent committed file snapshots, separate from disposable CAS.
    published: BTreeMap<PublishedMemberKey, Vec<u8>>,
    cas_reads: usize,
    cas_writes: usize,
    published_reads: usize,
}

/// 不接触文件系统且按能力注入失败的构建运行时。 /
/// Filesystem-free build runtime with capability-specific fault injection.
#[derive(Clone)]
pub struct MemoryBuildRuntime {
    descriptor: BuildRuntimeDescriptor,
    faults: Arc<BTreeSet<RuntimeFault>>,
    state: Arc<Mutex<MemoryState>>,
}

impl MemoryBuildRuntime {
    /// Number of valid stored cache results deliberately damaged by the test adapter.
    pub fn injected_cache_results(&self) -> usize {
        self.state.lock().unwrap().injected_cache_results
    }
    /// Drops disposable CAS while retaining independently committed generation files.
    pub fn clear_cas(&self) {
        self.state.lock().unwrap().blobs.clear();
    }

    /// Removes target current pointers while preserving immutable committed member snapshots.
    pub fn clear_target_currents(&self) {
        self.state
            .lock()
            .unwrap()
            .current
            .retain(|(space, _), _| *space != space_key(GenerationSpace::TargetArtifacts));
    }

    /// Counts CAS boundary calls, allowing inspection to prove it is read-only.
    pub fn cas_io_counts(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap();
        (state.cas_reads, state.cas_writes)
    }

    /// Counts actual generation-member reads, excluding descriptor-only lookup.
    pub fn published_read_count(&self) -> usize {
        self.state.lock().unwrap().published_reads
    }

    /// Number of disposable content snapshots available after repair.
    pub fn cas_blob_count(&self) -> usize {
        self.state.lock().unwrap().blobs.len()
    }

    /// 创建无故障运行时。 / Creates a fault-free runtime.
    pub fn new() -> Self {
        Self {
            descriptor: default_descriptor(),
            faults: Arc::new(BTreeSet::new()),
            state: Arc::new(Mutex::new(MemoryState::default())),
        }
    }

    /// 替换冻结工具链身份。 / Replaces the frozen toolchain identity.
    pub fn with_descriptor(mut self, descriptor: BuildRuntimeDescriptor) -> Self {
        self.descriptor = descriptor;
        self
    }

    /// 添加一个能力级确定性故障。 / Adds one deterministic capability-level fault.
    pub fn with_fault(mut self, fault: RuntimeFault) -> Self {
        Arc::make_mut(&mut self.faults).insert(fault);
        self
    }

    fn fail(&self, fault: RuntimeFault, code: &'static str) -> Result<(), BuildRuntimeError> {
        if self.faults.contains(&fault) {
            Err(BuildRuntimeError::new(
                code,
                format!("injected {fault:?} fault"),
            ))
        } else {
            Ok(())
        }
    }
}

impl Default for MemoryBuildRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildRuntime for MemoryBuildRuntime {
    fn descriptor(&self) -> BuildRuntimeDescriptor {
        self.descriptor.clone()
    }

    fn backend_identity(
        &self,
        options: SquishOptions,
    ) -> Result<BackendCacheIdentity, BuildRuntimeError> {
        Ok(SquishBackend.cache_identity(options))
    }

    fn read_blob(&self, digest: &Digest) -> Result<Option<Vec<u8>>, BuildRuntimeError> {
        let mut state = self.state.lock().unwrap();
        state.cas_reads += 1;
        Ok(state.blobs.get(digest.bytes()).cloned())
    }

    fn write_blob(&self, bytes: &[u8]) -> Result<Digest, BuildRuntimeError> {
        self.fail(RuntimeFault::WriteBlob, "memory_blob_write")?;
        let digest = blake3::hash(bytes);
        let mut state = self.state.lock().unwrap();
        state.cas_writes += 1;
        state
            .blobs
            .insert(digest.as_bytes().to_vec(), bytes.to_vec());
        drop(state);
        Digest::new(DigestAlgorithm::Blake3, digest.as_bytes().to_vec())
            .map_err(|error| runtime_error("memory_blob_digest", error))
    }

    fn lookup_action(&self, key: &ActionKey) -> Result<Option<ActionRecord>, BuildRuntimeError> {
        let mut record = self
            .state
            .lock()
            .unwrap()
            .actions
            .get(key.as_str())
            .cloned();
        if self.faults.contains(&RuntimeFault::LookupWrongKey) {
            if let Some(record) = &mut record {
                record.key = ActionKey::new("different-action-key").unwrap();
                self.state.lock().unwrap().injected_cache_results += 1;
            }
        }
        Ok(record)
    }

    fn lookup_action_verified(
        &self,
        key: &ActionKey,
    ) -> Result<Option<VerifiedAction>, BuildRuntimeError> {
        let Some(record) = self.lookup_action(key)? else {
            return Ok(None);
        };
        let mut blobs = Vec::new();
        for output in &record.outputs {
            let Some(blob) = self.read_verified(&output.digest)? else {
                return Ok(None);
            };
            blobs.push(blob);
        }
        if self.faults.contains(&RuntimeFault::LookupWrongOutputCount) {
            blobs.pop();
            self.state.lock().unwrap().injected_cache_results += 1;
        }
        Ok(Some(VerifiedAction { record, blobs }))
    }

    fn record_action(&self, record: &ActionRecord) -> Result<(), BuildRuntimeError> {
        self.state
            .lock()
            .unwrap()
            .actions
            .insert(record.key.as_str().into(), record.clone());
        Ok(())
    }

    fn current_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
    ) -> Result<Option<CommittedGeneration>, BuildRuntimeError> {
        if space == GenerationSpace::BuildCatalog {
            if self.faults.contains(&RuntimeFault::ReadCatalogCorrupt) {
                return Err(BuildRuntimeError::corrupt(
                    "memory_catalog_corrupt",
                    "injected corrupt catalog",
                ));
            }
            if self.faults.contains(&RuntimeFault::ReadCatalogStorage) {
                return Err(BuildRuntimeError::storage(
                    "memory_catalog_storage",
                    "injected catalog storage failure",
                ));
            }
        }
        Ok(self
            .state
            .lock()
            .unwrap()
            .current
            .get(&(space_key(space), target.as_str().into()))
            .cloned())
    }

    fn read_generation_artifact(
        &self,
        space: GenerationSpace,
        generation: &GenerationRef,
        destination: &PublicationPath,
    ) -> Result<(ArtifactRead, Vec<u8>), BuildRuntimeError> {
        let mut state = self.state.lock().unwrap();
        state.published_reads += 1;
        let key = (
            space_key(space),
            generation.target.as_str().into(),
            *generation.generation.as_bytes(),
        );
        let Some(committed) = state.generations.get(&key) else {
            return Ok((ArtifactRead::NotFound, Vec::new()));
        };
        let Some(member) = committed
            .artifacts
            .iter()
            .find(|member| member.path == *destination)
        else {
            return Ok((ArtifactRead::NotFound, Vec::new()));
        };
        let mut bytes = state
            .published
            .get(&(
                space_key(space),
                generation.target.as_str().into(),
                *generation.generation.as_bytes(),
                destination.as_str().into(),
            ))
            .cloned()
            .ok_or_else(|| {
                BuildRuntimeError::new("memory_generation_read", "published member is absent")
            })?;
        if self.faults.contains(&RuntimeFault::ReadPublishedWrongBytes) {
            if let Some(byte) = bytes.first_mut() {
                *byte ^= 1;
            } else {
                bytes.push(0);
            }
        }
        Ok((ArtifactRead::Verified(member.descriptor.clone()), bytes))
    }

    fn publish_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
        publications: &[Publication],
    ) -> Result<CommittedGeneration, BuildRuntimeError> {
        self.fail(
            match space {
                GenerationSpace::TargetArtifacts => RuntimeFault::PublishTarget,
                GenerationSpace::BuildCatalog => RuntimeFault::PublishCatalog,
            },
            "memory_generation_publish",
        )?;
        let mut publications = publications.to_vec();
        publications.sort_by(|left, right| {
            left.destination
                .cmp(&right.destination)
                .then_with(|| left.output.name.cmp(&right.output.name))
        });
        let mut hash = blake3::Hasher::new();
        hash.update(&[space_key(space)]);
        hash.update(target.as_str().as_bytes());
        let state = self.state.lock().unwrap();
        let mut artifacts = Vec::with_capacity(publications.len());
        let mut published = Vec::with_capacity(publications.len());
        for publication in &publications {
            let bytes = state
                .blobs
                .get(publication.output.digest.bytes())
                .ok_or_else(|| {
                    BuildRuntimeError::new(
                        "memory_generation_publish",
                        "publication blob is absent",
                    )
                })?;
            if bytes.len() as u64 != publication.output.size {
                return Err(BuildRuntimeError::new(
                    "memory_generation_publish",
                    "publication size differs from blob",
                ));
            }
            published.push((publication.destination.as_str().to_owned(), bytes.clone()));
            hash.update(publication.destination.as_str().as_bytes());
            hash.update(publication.output.digest.bytes());
            artifacts.push(GenerationArtifact {
                descriptor: ArtifactDescriptor {
                    id: ArtifactId::new(publication.output.name.as_str())
                        .expect("output names are non-empty"),
                    name: publication.name.clone(),
                    kind: publication.output.kind.clone(),
                    digest: publication.output.digest.clone(),
                    size: publication.output.size,
                },
                path: publication.destination.clone(),
            });
        }
        drop(state);
        let generation = GenerationId::from_bytes(*hash.finalize().as_bytes());
        let committed = CommittedGeneration {
            identity: GenerationRef {
                target: target.clone(),
                generation,
            },
            artifacts,
        };
        let mut state = self.state.lock().unwrap();
        for (path, bytes) in published {
            state.published.insert(
                (
                    space_key(space),
                    target.as_str().into(),
                    *generation.as_bytes(),
                    path,
                ),
                bytes,
            );
        }
        state.generations.insert(
            (
                space_key(space),
                target.as_str().into(),
                *generation.as_bytes(),
            ),
            committed.clone(),
        );
        state.current.insert(
            (space_key(space), target.as_str().into()),
            committed.clone(),
        );
        Ok(committed)
    }

    fn compile(
        &self,
        source: &SourceBlob,
        context: &FrontendSourceContext,
    ) -> Result<FrontendOutput, BuildRuntimeError> {
        self.fail(RuntimeFault::Compile, "memory_frontend_compile")?;
        squish_xml_front::compile(source, context)
            .map_err(|error| BuildRuntimeError::new("memory_frontend_compile", error.message))
    }

    fn link(
        &self,
        entry: &SourceKey,
        closure: UnitClosure,
    ) -> Result<LinkOutput, BuildRuntimeError> {
        StaticLinker
            .link(entry, closure)
            .map_err(|error| runtime_error("memory_static_link", error))
    }

    fn instantiate(
        &self,
        program: &LinkedProgram,
        arguments: BTreeMap<String, String>,
        budgets: Budgets,
    ) -> Result<InstantiateOutput, BuildRuntimeError> {
        Instantiator
            .instantiate(program, arguments, budgets)
            .map_err(|error| runtime_error("memory_instantiate", error))
    }

    fn render(&self, request: BackendRequest) -> Result<BackendOutput, BuildRuntimeError> {
        SquishBackend
            .emit(request)
            .map_err(|error| runtime_error("memory_backend_render", error))
    }
}

fn default_descriptor() -> BuildRuntimeDescriptor {
    BuildRuntimeDescriptor {
        frontend_abi: squish_xml_front::FRONTEND_ABI.into(),
        linker_abi: "xmlsquish.link/1".into(),
        evaluator_abi: "xmlsquish.instantiate/1".into(),
        document_abi: squish_backend::DOCUMENT_ABI.into(),
    }
}

const fn space_key(space: GenerationSpace) -> u8 {
    match space {
        GenerationSpace::TargetArtifacts => 0,
        GenerationSpace::BuildCatalog => 1,
    }
}
