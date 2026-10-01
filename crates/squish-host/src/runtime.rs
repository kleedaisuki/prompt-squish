//! 生产构建运行时组合。 / Production build-runtime composition.

use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    sync::{Arc, Mutex},
};

use squish_backend::{
    Backend, BackendCacheIdentity, BackendOutput, BackendRequest, BackendRequestRef, SquishBackend,
    SquishOptions,
};
use squish_build::{
    ActionIndex, ActionKey, ActionRecord, ArtifactRead, BlobStore, CommittedGeneration,
    GenerationRef, Publication, PublicationPath, PublicationTargetId, VerifiedAction, VerifiedBlob,
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
use squish_store::{
    ActionKey as StoreActionKey, BlobDigest, Cas, CasError, CasSession, VerifiedActionIndex,
};
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
    session: Mutex<Option<Arc<CasSession>>>,
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
            session: Mutex::new(None),
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

    fn session(&self) -> Result<Arc<CasSession>, BuildRuntimeError> {
        initialize(&self.session, "CAS session", || {
            Ok(CasSession::new(self.cas()?))
        })
    }

    fn index(&self) -> Result<Arc<VerifiedActionIndex>, BuildRuntimeError> {
        initialize(&self.index, "action index", || {
            VerifiedActionIndex::open_in_session(self.layout.action_index(), self.session()?)
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
            FileArtifactPublisher::with_external_layout(
                layout,
                SharedCas {
                    session: self.session()?,
                    snapshots: HashMap::new(),
                },
                observer,
            )
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

impl Drop for ProductionBuildRuntime {
    fn drop(&mut self) {
        // Recency is advisory: explicit finish reports errors; drop is best-effort fallback.
        if let Ok(index) = self.index.get_mut() {
            if let Some(index) = index.as_ref() {
                let _ = index.flush_touches();
            }
        }
    }
}

impl BuildRuntime for ProductionBuildRuntime {
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
        self.cas()?
            .get(blob_digest(digest)?)
            .map_err(|error| storage_error("host_blob_read", error))
    }

    fn read_verified(&self, digest: &Digest) -> Result<Option<VerifiedBlob>, BuildRuntimeError> {
        self.session()?
            .acquire(blob_digest(digest)?)
            .map_err(|error| storage_error("host_blob_read", error))
    }

    fn read_blob_shared(&self, digest: &Digest) -> Result<Option<Arc<[u8]>>, BuildRuntimeError> {
        Ok(self.read_verified(digest)?.map(|blob| blob.shared_bytes()))
    }

    fn write_verified_blob(&self, blob: &VerifiedBlob) -> Result<Digest, BuildRuntimeError> {
        let digest = self
            .session()?
            .put_verified(blob)
            .map_err(|error| storage_error("host_blob_write", error))?;
        Ok(protocol_digest(digest))
    }

    fn lookup_action_verified(
        &self,
        key: &ActionKey,
    ) -> Result<Option<VerifiedAction>, BuildRuntimeError> {
        self.index()?
            .lookup_verified(key)
            .map_err(|error| storage_error("host_action_lookup", error))
    }

    fn flush_advisory(&self) -> Result<(), BuildRuntimeError> {
        let index = self.index.lock().map_err(|_| {
            BuildRuntimeError::storage(
                "host_runtime_coordination",
                "action index initialization mutex is poisoned",
            )
        })?;
        if let Some(index) = index.as_ref() {
            index
                .flush_touches()
                .map_err(|error| storage_error("host_action_touch", error))?;
        }
        Ok(())
    }

    fn write_blob(&self, bytes: &[u8]) -> Result<Digest, BuildRuntimeError> {
        let digest = self
            .cas()?
            .put(bytes)
            .map_err(|error| storage_error("host_blob_write", error))?;
        Ok(protocol_digest(digest))
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

    fn publish_generation_verified(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
        publications: &[Publication],
        blobs: &[VerifiedBlob],
    ) -> Result<CommittedGeneration, BuildRuntimeError> {
        self.maintenance()?;
        let (root, state, prefix, observer) = match space {
            GenerationSpace::TargetArtifacts => (
                self.layout.artifacts_root().to_path_buf(),
                self.layout.publications_root().to_path_buf(),
                self.layout.publication_prefix().to_path_buf(),
                self.target_observer.clone(),
            ),
            GenerationSpace::BuildCatalog => (
                self.layout.catalog_root().join("artifacts"),
                self.layout.catalog_root().join("state"),
                std::path::PathBuf::new(),
                self.catalog_observer.clone(),
            ),
        };
        let layout = ExternalPublisherLayout::new(
            root,
            state,
            prefix,
            self.layout.metadata_root().join("publication.lock"),
        );
        // These are caller-supplied handles, not an unbounded retention cache. The
        // temporary publication view dies immediately after the atomic operation.
        let cas = SharedCas {
            session: self.session()?,
            snapshots: blobs
                .iter()
                .map(|blob| (blob.digest().clone(), blob.clone()))
                .collect(),
        };
        FileArtifactPublisher::with_external_layout(layout, cas, observer)
            .map_err(|error| publication_error("host_generation_publish", error))?
            .publish_generation(target, publications)
            .map_err(|error| publication_error("host_generation_publish", error))
    }

    fn compile(
        &self,
        source: &SourceBlob,
        context: &FrontendSourceContext,
    ) -> Result<FrontendOutput, BuildRuntimeError> {
        squish_xml_front::compile(source, context).map_err(|error| {
            BuildRuntimeError::new("host_frontend_compile", error.message.clone())
                .with_diagnostic(*error)
        })
    }

    fn link(
        &self,
        entry: &SourceKey,
        closure: UnitClosure,
    ) -> Result<LinkOutput, BuildRuntimeError> {
        self.link_shared(entry, closure.into())
    }

    fn link_shared(
        &self,
        entry: &SourceKey,
        closure: squish_link::SharedUnitClosure,
    ) -> Result<LinkOutput, BuildRuntimeError> {
        let evidence = closure.units.values().cloned().collect::<Vec<_>>();
        StaticLinker
            .link_shared(entry, closure)
            .map_err(|error| link_error(error, &evidence))
    }

    fn link_prepared(
        &self,
        entry: &SourceKey,
        closure: squish_link::PreparedUnitClosure,
    ) -> Result<LinkOutput, BuildRuntimeError> {
        let evidence = closure
            .units
            .values()
            .map(|unit| unit.unit().clone())
            .collect::<Vec<_>>();
        StaticLinker
            .link_prepared(entry, closure)
            .map_err(|error| link_error(error, &evidence))
    }

    fn instantiate(
        &self,
        program: &LinkedProgram,
        arguments: BTreeMap<String, String>,
        budgets: Budgets,
    ) -> Result<InstantiateOutput, BuildRuntimeError> {
        Instantiator
            .instantiate_with_diagnostics(program, arguments, budgets)
            .map_err(|failure| instantiation_error(program, failure))
    }

    fn render(&self, request: BackendRequest) -> Result<BackendOutput, BuildRuntimeError> {
        SquishBackend
            .emit(request)
            .map_err(|error| tool_error("host_backend_render", error))
    }
    fn render_ref(
        &self,
        request: BackendRequestRef<'_>,
    ) -> Result<BackendOutput, BuildRuntimeError> {
        SquishBackend
            .emit_ref(request)
            .map_err(|error| tool_error("host_backend_render", error))
    }
}

#[derive(Clone)]
struct SharedCas {
    session: Arc<CasSession>,
    snapshots: HashMap<Digest, VerifiedBlob>,
}

impl BlobStore for SharedCas {
    type Error = CasError;

    fn copy_to(
        &self,
        digest: &squish_build::ContentDigest,
        sink: &mut dyn std::io::Write,
    ) -> Result<bool, Self::Error> {
        if let Some(blob) = self.snapshots.get(digest) {
            sink.write_all(blob.bytes())?;
            return Ok(true);
        }
        self.session.copy_to(digest, sink)
    }

    fn write_from(
        &self,
        source: &mut dyn std::io::Read,
    ) -> Result<squish_build::ContentDigest, Self::Error> {
        self.session.write_from(source)
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

/// Projects exact linker evidence only on failure; successful linking retains Arc handles only.
fn link_error(
    error: squish_link::LinkError,
    units: &[Arc<squish_ir::RelocatableUnitIr>],
) -> BuildRuntimeError {
    use squish_ir::{EntityKind, UnitEncodingOverrides, VerifiedUnitEncoding};
    use squish_protocol::{Diagnostic, DiagnosticId, Phase, Severity};
    let origin_span = error.origin.as_ref().and_then(|qualified| {
        let mut found = None;
        let mut matched = false;
        for unit in units {
            let proof = VerifiedUnitEncoding::new(unit).ok()?;
            if proof.object_digest(UnitEncodingOverrides::default()).ok()? != qualified.object {
                continue;
            }
            // Ambiguous asserted object identities must never select a guessed provider.
            if matched {
                return None;
            }
            matched = true;
            let origin = &unit
                .origins()
                .entries
                .get(qualified.local.0 as usize)?
                .origin;
            found = source_origin_span(unit, origin);
        }
        found
    });
    let definition_span = || {
        // Only duplicate-definition errors identify the declaration as the failure
        // site. Argument/fill errors concern a call, not the target declaration.
        if error.code != "LNK023" {
            return None;
        }
        let source = error.source.as_deref()?;
        let symbol = error.symbol.as_deref()?;
        let unit = units.iter().find(|unit| &unit.header().source == source)?;
        VerifiedUnitEncoding::new(unit).ok()?;
        let definition = unit
            .definitions()
            .iter()
            .find(|definition| &definition.symbol == symbol)?;
        let origin = &unit
            .origins()
            .entries
            .iter()
            .find(|entry| {
                entry.entity_kind == EntityKind::Definition && entry.local_id == definition.id.0
            })?
            .origin;
        source_origin_span(unit, origin)
    };
    let precise = origin_span.or_else(definition_span);
    let primary = precise.clone().or_else(|| {
        error.source.as_deref().and_then(|key| {
            squish_protocol::Span::new(protocol_source_key(key.clone())?, 0, 0).ok()
        })
    });
    let symbol = error
        .symbol
        .as_deref()
        .map(|symbol| format!(" {{{}}}{}", symbol.namespace_uri, symbol.local_name))
        .unwrap_or_default();
    let help = if precise.is_some() {
        format!(
            "Linker {}{}; location resolved from the exact source attachment.",
            error.code, symbol
        )
    } else {
        format!(
            "Linker {}{}; a source-only position denotes byte 0, not a precise declaration span.",
            error.code, symbol
        )
    };
    let diagnostic = Diagnostic {
        id: DiagnosticId::new(format!("linker-{}", error.code))
            .expect("nonempty diagnostic identity"),
        code: error.code.to_owned(),
        severity: Severity::Error,
        phase: Phase::Link,
        message: error.message.clone(),
        primary,
        related: Vec::new(),
        help: Some(help),
    };
    BuildRuntimeError::new("host_static_link", error.to_string()).with_diagnostic(diagnostic)
}

/// Resolves an origin's attached source and checks its declared byte bounds.
fn source_origin_span(
    unit: &squish_ir::RelocatableUnitIr,
    origin: &squish_ir::Origin,
) -> Option<squish_protocol::Span> {
    let source = unit.sources().records.get(origin.source.0 as usize)?;
    let length = source
        .exact_bytes
        .byte_len
        .checked_sub(u64::from(source.bom_len))?;
    if origin.span.start > origin.span.end || origin.span.end > length {
        return None;
    }
    squish_protocol::Span::new(
        protocol_source_key(source.key.clone())?,
        origin.span.start,
        origin.span.end,
    )
    .ok()
}

/// Converts compiler-owned object-qualified provenance at the host protocol boundary.
fn instantiation_error(
    program: &LinkedProgram,
    failure: squish_link::InstantiationFailure,
) -> BuildRuntimeError {
    use squish_protocol::{Diagnostic, DiagnosticId, Phase, RelatedSpan, Severity};
    let primary = failure
        .error
        .origin
        .as_ref()
        .and_then(|origin| diagnostic_span(program, origin))
        .or_else(|| {
            failure.frames.last().and_then(|frame| {
                frame
                    .call_origin
                    .as_ref()
                    .and_then(|origin| diagnostic_span(program, origin))
                    .or_else(|| diagnostic_span(program, &frame.definition_origin))
            })
        });
    let mut related = Vec::with_capacity(failure.frames.len() * 2);
    for (depth, frame) in failure.frames.iter().enumerate() {
        let kind = match &frame.identity {
            squish_ir::FrameIdentity::Entry => "entry",
            squish_ir::FrameIdentity::Macro(_) => "macro",
        };
        if let Some(span) = frame
            .call_origin
            .as_ref()
            .and_then(|origin| diagnostic_span(program, origin))
        {
            related.push(RelatedSpan {
                span,
                label: format!("expansion frame {}: {kind} call", depth),
            });
        }
        if let Some(span) = diagnostic_span(program, &frame.definition_origin) {
            related.push(RelatedSpan {
                span,
                label: format!("expansion frame {}: {kind} definition", depth),
            });
        }
    }
    let diagnostic = Diagnostic {
        id: DiagnosticId::new(format!("evaluator-{}", failure.error.code))
            .expect("nonempty diagnostic identity"),
        code: failure.error.code.to_owned(),
        severity: Severity::Error,
        phase: Phase::Instantiate,
        message: failure.error.message.clone(),
        primary,
        related,
        help: Some(format!(
            "Evaluator {} failed; expansion context lists {} frames from entry to failure.",
            failure.error.code,
            failure.error.frame_chain.len()
        )),
    };
    BuildRuntimeError::new("host_instantiate", failure.error.to_string())
        .with_diagnostic(diagnostic)
}

/// Resolves exactly the referenced source attachment, never a same-name provider guess.
fn diagnostic_span(
    program: &LinkedProgram,
    origin: &squish_ir::QualifiedOriginRef,
) -> Option<squish_protocol::Span> {
    let (key, span) = program.resolve_origin(origin)?;
    squish_protocol::Span::new(protocol_source_key(key)?, span.start, span.end).ok()
}

/// Maps logical identity, not a physical checkout locator, to an opaque diagnostic URI.
fn protocol_source_key(key: SourceKey) -> Option<squish_protocol::OpaqueSourceId> {
    use squish_source::{LogicalPath, PackageId, SourceId};
    match key {
        SourceKey::AdHoc { uri } => squish_protocol::OpaqueSourceId::new(uri).ok(),
        SourceKey::Project { package, path } => {
            let logical = LogicalPath::new(path.join("/")).ok()?;
            let source = SourceId::new(PackageId::new(package.package_name).ok()?, logical);
            if package.source_kind == 5 {
                // Both provider namespace and UTF-8 path are canonical logical identity.
                let suffix = source.uri().strip_prefix("xmlsquish://")?;
                squish_protocol::OpaqueSourceId::new(format!(
                    "sopack://{}/{suffix}",
                    package.exact_revision
                ))
                .ok()
            } else {
                Some(source.to_protocol())
            }
        }
    }
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
    fn frontend_and_linker_errors_preserve_authoritative_source_spans() {
        std::fs::create_dir_all(".temp").unwrap();
        let temporary = tempfile::tempdir_in(".temp").unwrap();
        let runtime = runtime(layout(temporary.path()));
        let context = FrontendSourceContext::new(PackageInstanceId {
            source_kind: 5,
            canonical_source: "sopack:archive".into(),
            package_name: "library".into(),
            exact_revision: "archive".into(),
        });
        let source = |xml: String| {
            SnapshotBuilder::new(MemorySource(xml.into_bytes()))
                .load(
                    SourceId::new(
                        PackageId::new("library").unwrap(),
                        LogicalPath::new("providers/p/lib.xml").unwrap(),
                    ),
                    SourceLocator::file("unused"),
                )
                .unwrap()
        };
        let invalid = source(format!(
            r#"<xs:module xmlns:xs="{}"><xs:macro/></xs:module>"#,
            squish_xml_front::DSL_NAMESPACE
        ));
        let expected = squish_xml_front::compile(&invalid, &context).unwrap_err();
        let error = runtime.compile(&invalid, &context).unwrap_err();
        assert_eq!(error.diagnostic(), Some(expected.as_ref()));

        let valid = source(format!(
            r#"<xs:module xmlns:xs="{}" xmlns:m="urn:macro"><xs:macro name="m:conflict"><Text/></xs:macro></xs:module>"#,
            squish_xml_front::DSL_NAMESPACE
        ));
        let unit = Arc::new(runtime.compile(&valid, &context).unwrap().unit);
        let definition = &unit.definitions()[0];
        let error = squish_link::LinkError {
            code: "LNK023",
            message: "duplicate macro symbol in linked closure".into(),
            source: Some(Box::new(unit.header().source.clone())),
            symbol: Some(Box::new(definition.symbol.clone())),
            origin: None,
        };
        let mapped = link_error(error, std::slice::from_ref(&unit));
        let diagnostic = mapped.diagnostic().unwrap();
        let primary = diagnostic.primary.as_ref().unwrap();
        assert_eq!(
            primary.source().as_str(),
            "sopack://archive/library/providers/p/lib.xml"
        );
        assert!(primary.bytes().end > primary.bytes().start);
        assert!(
            diagnostic
                .help
                .as_ref()
                .unwrap()
                .contains("{urn:macro}conflict")
        );
        let call_error = link_error(
            squish_link::LinkError {
                code: "LNK027",
                message: "invalid argument or fill".into(),
                source: Some(Box::new(unit.header().source.clone())),
                symbol: Some(Box::new(definition.symbol.clone())),
                origin: None,
            },
            std::slice::from_ref(&unit),
        );
        assert_eq!(
            call_error
                .diagnostic()
                .unwrap()
                .primary
                .as_ref()
                .unwrap()
                .bytes(),
            0..0
        );
        let qualified = squish_ir::QualifiedOriginRef {
            object: squish_ir::VerifiedUnitEncoding::new(&unit)
                .unwrap()
                .object_digest(squish_ir::UnitEncodingOverrides::default())
                .unwrap(),
            local: squish_ir::OriginId(
                unit.origins()
                    .entries
                    .iter()
                    .position(|entry| {
                        entry.entity_kind == squish_ir::EntityKind::Definition
                            && entry.local_id == definition.id.0
                    })
                    .unwrap() as u32,
            ),
        };
        let exact = link_error(
            squish_link::LinkError {
                code: "LNK030",
                message: "regexpool".into(),
                source: None,
                symbol: None,
                origin: Some(qualified),
            },
            &[unit],
        );
        assert_eq!(exact.diagnostic().unwrap().primary, diagnostic.primary);
    }

    #[test]
    fn source_only_link_failure_uses_honest_zero_width_location() {
        let key = SourceKey::AdHoc {
            uri: "sopack://archive/library/lib.xml".into(),
        };
        let mapped = link_error(
            squish_link::LinkError {
                code: "LNK030",
                message: "regexpool".into(),
                source: Some(Box::new(key)),
                symbol: None,
                origin: None,
            },
            &[],
        );
        let diagnostic = mapped.diagnostic().unwrap();
        assert_eq!(diagnostic.primary.as_ref().unwrap().bytes(), 0..0);
        assert!(
            diagnostic
                .help
                .as_ref()
                .unwrap()
                .contains("not a precise declaration span")
        );
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

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use squish_store::BlobSessionLimits;

    #[test]
    fn publication_snapshot_does_not_reread_after_zero_retention_eviction() {
        std::fs::create_dir_all(".temp").unwrap();
        let directory = tempfile::tempdir_in(".temp").unwrap();
        let cas = Arc::new(Cas::open(directory.path().join("cas")).unwrap());
        let session = Arc::new(CasSession::with_limits(
            cas.clone(),
            BlobSessionLimits {
                max_bytes: 0,
                max_entries: 0,
            },
        ));
        let blob = VerifiedBlob::from_owned(vec![42; 4096]);
        let digest = session.put_verified(&blob).unwrap();
        assert_eq!(session.stats().retained_entries, 0);
        // A snapshot is authority for its immutable bytes, not the mutable CAS path.
        // A fresh legacy read must still detect corruption independently.
        std::fs::write(cas.path_for(digest), b"corrupt").unwrap();
        assert!(cas.get(digest).unwrap().is_none());
        let store = SharedCas {
            session: session.clone(),
            snapshots: HashMap::from([(blob.digest().clone(), blob.clone())]),
        };
        let mut bytes = Vec::new();
        assert!(store.copy_to(blob.digest(), &mut bytes).unwrap());
        assert_eq!(bytes, blob.bytes());
        assert_eq!(session.stats().acquisition_reads, 0);
    }
}

#[cfg(test)]
mod diagnostic_identity_tests {
    use super::*;

    #[test]
    fn archived_diagnostics_keep_namespace_and_encode_logical_paths() {
        let key = |revision: &str| SourceKey::Project {
            package: squish_ir::PackageInstanceId {
                source_kind: 5,
                canonical_source: format!("sopack:{revision}"),
                package_name: "library".into(),
                exact_revision: revision.into(),
            },
            path: vec!["providers".into(), "provider".into(), "lib file.xml".into()],
        };
        let first = protocol_source_key(key("archive-one")).unwrap();
        let second = protocol_source_key(key("archive-two")).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            first.as_str(),
            "sopack://archive-one/library/providers/provider/lib%20file.xml"
        );
    }
}
