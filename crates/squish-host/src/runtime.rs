//! 生产构建运行时组合。 / Production build-runtime composition.

use std::{collections::BTreeMap, sync::Arc};

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
    BuildRuntime, BuildRuntimeDescriptor, BuildRuntimeError, GenerationSpace, StorageLayout,
};
use squish_protocol::{Digest, DigestAlgorithm};
use squish_publish::{FileArtifactPublisher, PublishError, PublishObserver};
use squish_source::SourceBlob;
use squish_store::{ActionKey as StoreActionKey, BlobDigest, Cas, CasError, VerifiedActionIndex};
use squish_xml_front::{FrontendOutput, FrontendSourceContext};

/// 在一个调用中共享同一 CAS、动作索引与两个隔离 publisher 的生产运行时。 /
/// Production runtime sharing one CAS, one action index, and two isolated publishers for an
/// invocation.
pub struct ProductionBuildRuntime {
    cas: Arc<Cas>,
    index: VerifiedActionIndex,
    targets: FileArtifactPublisher<SharedCas>,
    catalog: FileArtifactPublisher<SharedCas>,
}

impl ProductionBuildRuntime {
    /// 从宿主唯一权威布局打开完整运行时，并恢复两个发布空间。 /
    /// Opens the complete runtime from the host's sole authoritative layout and recovers both
    /// publication spaces.
    pub fn open(
        layout: &StorageLayout,
        target_observer: Arc<dyn PublishObserver>,
        catalog_observer: Arc<dyn PublishObserver>,
    ) -> Result<Self, BuildRuntimeError> {
        let cas = Arc::new(
            Cas::open(layout.cas_root()).map_err(|error| storage_error("host_cas_open", error))?,
        );
        let index = VerifiedActionIndex::open(layout.action_index(), cas.clone())
            .map_err(|error| storage_error("host_action_index_open", error))?;
        let targets = FileArtifactPublisher::with_observer(
            layout.publication_root(),
            SharedCas(cas.clone()),
            target_observer,
        )
        .map_err(|error| publication_error("host_target_publisher_open", error))?;
        let catalog = FileArtifactPublisher::with_observer(
            layout.catalog_root(),
            SharedCas(cas.clone()),
            catalog_observer,
        )
        .map_err(|error| publication_error("host_catalog_publisher_open", error))?;
        Ok(Self {
            cas,
            index,
            targets,
            catalog,
        })
    }

    /// 枚举动作索引中仍通过共享 CAS 完整性校验的记录。 /
    /// Enumerates action records that still pass integrity validation through the shared CAS.
    pub fn cache_records(&self) -> Result<Vec<squish_protocol::CachedAction>, BuildRuntimeError> {
        let mut after: Option<StoreActionKey> = None;
        let mut records = Vec::new();
        loop {
            let page = self
                .index
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

    fn publisher(&self, space: GenerationSpace) -> &FileArtifactPublisher<SharedCas> {
        match space {
            GenerationSpace::TargetArtifacts => &self.targets,
            GenerationSpace::BuildCatalog => &self.catalog,
        }
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
        self.cas
            .get(blob_digest(digest)?)
            .map_err(|error| storage_error("host_blob_read", error))
    }

    fn write_blob(&self, bytes: &[u8]) -> Result<Digest, BuildRuntimeError> {
        let digest = self
            .cas
            .put(bytes)
            .map_err(|error| storage_error("host_blob_write", error))?;
        Digest::new(DigestAlgorithm::Blake3, digest.as_bytes().to_vec())
            .map_err(|error| corrupt_error("host_blob_digest", error))
    }

    fn lookup_action(&self, key: &ActionKey) -> Result<Option<ActionRecord>, BuildRuntimeError> {
        self.index
            .lookup(key)
            .map_err(|error| storage_error("host_action_lookup", error))
    }

    fn record_action(&self, record: &ActionRecord) -> Result<(), BuildRuntimeError> {
        self.index
            .record(record)
            .map_err(|error| storage_error("host_action_record", error))
    }

    fn current_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
    ) -> Result<Option<CommittedGeneration>, BuildRuntimeError> {
        self.publisher(space)
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
            .publisher(space)
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
        self.publisher(space)
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
        PublishError::Store(_) | PublishError::Io(_) => BuildRuntimeError::storage(code, message),
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
    use std::io;

    use squish_manager::BuildRuntimeErrorKind;

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
}
