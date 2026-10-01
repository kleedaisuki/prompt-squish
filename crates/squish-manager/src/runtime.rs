//! 构建运行时端口；管理器只编排领域操作。 / Build-runtime ports; the manager only orchestrates domain operations.

use std::{collections::BTreeMap, fmt, path::Path, sync::Arc};

use squish_backend::{
    BackendCacheIdentity, BackendOutput, BackendRequest, BackendRequestRef, SquishOptions,
};
use squish_build::{
    ActionKey, ActionRecord, ArtifactRead, CommittedGeneration, GenerationRef, Publication,
    PublicationPath, PublicationTargetId, VerifiedAction, VerifiedBlob,
};
use squish_ir::SourceKey;
use squish_link::{Budgets, InstantiateOutput, LinkOutput, LinkedProgram, UnitClosure};
use squish_protocol::Digest;
use squish_source::SourceBlob;
use squish_xml_front::{FrontendOutput, FrontendSourceContext};

/// 持久化 generation 的隔离空间。 / Isolated persistence space for generations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationSpace {
    /// 用户目标产物。 / User target artifacts.
    TargetArtifacts,
    /// 项目 build catalog。 / Project build catalog.
    BuildCatalog,
}

/// 封闭计划必须冻结的完整工具链身份。 / Complete toolchain identity frozen into a sealed plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRuntimeDescriptor {
    /// 前端语义 ABI。 / Frontend semantic ABI.
    pub frontend_abi: String,
    /// 静态链接器语义 ABI。 / Static-linker semantic ABI.
    pub linker_abi: String,
    /// 实例化器/求值器语义 ABI。 / Instantiator/evaluator semantic ABI.
    pub evaluator_abi: String,
    /// 传给后端的文档 ABI。 / Document ABI supplied to the backend.
    pub document_abi: String,
}

/// 宿主运行时的稳定结构化错误。 / Stable structured error returned by a host runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildRuntimeErrorKind {
    /// 可用性、I/O 或底层存储故障。 / Availability, I/O, or underlying storage failure.
    Storage,
    /// 已持久状态不满足完整性不变量。 / Persisted state violates an integrity invariant.
    Corrupt,
    /// 工具链或其他非存储运行时失败。 / Toolchain or other non-storage runtime failure.
    Tool,
}

/// 宿主运行时的稳定结构化错误。 / Stable structured error returned by a host runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRuntimeError {
    kind: BuildRuntimeErrorKind,
    code: &'static str,
    message: String,
    diagnostic: Option<Arc<squish_protocol::Diagnostic>>,
}

impl BuildRuntimeError {
    /// 从稳定代码和非本地化详情构造工具链故障。 /
    /// Constructs a toolchain failure from a stable code and non-localized detail.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: BuildRuntimeErrorKind::Tool,
            code,
            message: message.into(),
            diagnostic: None,
        }
    }

    /// 构造存储/可用性故障。 / Constructs a storage or availability failure.
    pub fn storage(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: BuildRuntimeErrorKind::Storage,
            code,
            message: message.into(),
            diagnostic: None,
        }
    }

    /// 构造持久状态完整性故障。 / Constructs a persisted-state integrity failure.
    pub fn corrupt(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: BuildRuntimeErrorKind::Corrupt,
            code,
            message: message.into(),
            diagnostic: None,
        }
    }

    /// Retains compiler-owned source locations and expansion context without flattening them.
    #[must_use]
    pub fn with_diagnostic(mut self, diagnostic: squish_protocol::Diagnostic) -> Self {
        self.diagnostic = Some(Arc::new(diagnostic));
        self
    }

    /// Returns structured diagnostic evidence; absence preserves legacy runtime behavior.
    pub fn diagnostic(&self) -> Option<&squish_protocol::Diagnostic> {
        self.diagnostic.as_deref()
    }

    /// 返回失败类别。 / Returns the failure category.
    pub const fn kind(&self) -> BuildRuntimeErrorKind {
        self.kind
    }

    /// 返回稳定机器代码。 / Returns the stable machine code.
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// 返回非本地化详情。 / Returns the non-localized detail.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BuildRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for BuildRuntimeError {}

/// 一次调用共享的完整构建运行时。 / Complete invocation-scoped build runtime.
///
/// 实现拥有具体存储、发布和工具链选择；不得发射 kernel 事件。 /
/// Implementations own concrete storage, publication, and toolchain selection and must not emit
/// kernel events.
pub trait BuildRuntime: Send + Sync {
    /// 返回本对象的工具链身份。 / Returns this object's toolchain identity.
    fn descriptor(&self) -> BuildRuntimeDescriptor;
    /// 返回给定选项的后端缓存身份。 / Returns the backend cache identity for options.
    fn backend_identity(
        &self,
        options: SquishOptions,
    ) -> Result<BackendCacheIdentity, BuildRuntimeError>;
    /// 读取并由运行时校验 blob。 / Reads and runtime-verifies a blob.
    fn read_blob(&self, digest: &Digest) -> Result<Option<Vec<u8>>, BuildRuntimeError>;
    /// Acquires a digest-verified immutable snapshot, checking legacy runtime responses.
    /// A caller-held handle remains valid independently of a bounded host session cache.
    fn read_verified(&self, digest: &Digest) -> Result<Option<VerifiedBlob>, BuildRuntimeError> {
        let Some(bytes) = self.read_blob(digest)? else {
            return Ok(None);
        };
        let blob = VerifiedBlob::from_owned(bytes);
        if blob.digest() != digest {
            return Err(BuildRuntimeError::corrupt(
                "runtime_blob_digest",
                "runtime returned bytes that do not match the requested digest",
            ));
        }
        Ok(Some(blob))
    }
    /// Shares acquired bytes without copying the complete payload.
    fn read_blob_shared(&self, digest: &Digest) -> Result<Option<Arc<[u8]>>, BuildRuntimeError> {
        Ok(self.read_verified(digest)?.map(|blob| blob.shared_bytes()))
    }
    /// Writes a verified snapshot; legacy adapters must still return its exact identity.
    fn write_verified_blob(&self, blob: &VerifiedBlob) -> Result<Digest, BuildRuntimeError> {
        let digest = self.write_blob(blob.bytes())?;
        if &digest != blob.digest() {
            return Err(BuildRuntimeError::corrupt(
                "runtime_blob_digest",
                "runtime returned an incorrect written blob identity",
            ));
        }
        Ok(digest)
    }
    /// Acquires all action outputs explicitly, preserving declaration order and integrity.
    fn lookup_action_verified(
        &self,
        key: &ActionKey,
    ) -> Result<Option<VerifiedAction>, BuildRuntimeError> {
        let Some(record) = self.lookup_action(key)? else {
            return Ok(None);
        };
        if &record.key != key {
            return Err(BuildRuntimeError::corrupt(
                "runtime_action_key",
                "runtime returned an action record for a different key",
            ));
        }
        let mut blobs = Vec::with_capacity(record.outputs.len());
        for output in &record.outputs {
            let Some(blob) = self.read_verified(&output.digest)? else {
                return Ok(None);
            };
            if blob.bytes().len() as u64 != output.size {
                return Err(BuildRuntimeError::corrupt(
                    "runtime_blob_size",
                    "runtime returned an action output with incorrect size",
                ));
            }
            blobs.push(blob);
        }
        Ok(Some(VerifiedAction { record, blobs }))
    }
    /// Flushes advisory cache recency updates without changing durable action semantics.
    /// Legacy runtimes do not need an invocation-scoped writeback phase.
    fn flush_advisory(&self) -> Result<(), BuildRuntimeError> {
        Ok(())
    }
    /// 幂等写入 blob，并返回已校验摘要。 / Idempotently writes a blob and returns its verified digest.
    fn write_blob(&self, bytes: &[u8]) -> Result<Digest, BuildRuntimeError>;
    /// 查找已验证动作记录。 / Looks up a verified action record.
    fn lookup_action(&self, key: &ActionKey) -> Result<Option<ActionRecord>, BuildRuntimeError>;
    /// 幂等记录成功动作。 / Idempotently records a successful action.
    fn record_action(&self, record: &ActionRecord) -> Result<(), BuildRuntimeError>;
    /// 查询某隔离空间的当前 generation。 / Queries the current generation in one isolated space.
    fn current_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
    ) -> Result<Option<CommittedGeneration>, BuildRuntimeError>;
    /// 读取并验证 generation 成员。 / Reads and verifies a generation member.
    fn read_generation_artifact(
        &self,
        space: GenerationSpace,
        generation: &GenerationRef,
        destination: &PublicationPath,
    ) -> Result<(ArtifactRead, Vec<u8>), BuildRuntimeError>;
    /// 原子发布完整 generation。 / Atomically publishes a complete generation.
    fn publish_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
        publications: &[Publication],
    ) -> Result<CommittedGeneration, BuildRuntimeError>;
    /// Publishes explicit acquired snapshots without relying on host cache retention.
    /// The default preserves custom publishers' legacy contract.
    fn publish_generation_verified(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
        publications: &[Publication],
        blobs: &[VerifiedBlob],
    ) -> Result<CommittedGeneration, BuildRuntimeError> {
        // Validate the complete handoff before creating any derived CAS state.
        // Duplicate content may serve several destinations; persist it only once.
        let supplied = blobs
            .iter()
            .map(|blob| (blob.digest(), blob))
            .collect::<std::collections::HashMap<_, _>>();
        let mut needed = std::collections::HashSet::with_capacity(publications.len());
        for publication in publications {
            needed.insert(&publication.output.digest);
            if let Some(blob) = supplied.get(&publication.output.digest) {
                if blob.bytes().len() as u64 != publication.output.size {
                    return Err(BuildRuntimeError::corrupt(
                        "runtime_publication_size",
                        "publication size does not match its verified snapshot",
                    ));
                }
            }
        }
        if supplied.keys().any(|digest| !needed.contains(digest)) {
            return Err(BuildRuntimeError::corrupt(
                "runtime_publication_blob",
                "publication supplied an unrelated verified snapshot",
            ));
        }
        for blob in supplied.values() {
            self.write_verified_blob(blob)?;
        }
        self.publish_generation(space, target, publications)
    }
    /// 使用选定前端编译冻结源。 / Compiles a frozen source with the selected frontend.
    fn compile(
        &self,
        source: &SourceBlob,
        context: &FrontendSourceContext,
    ) -> Result<FrontendOutput, BuildRuntimeError>;
    /// 使用选定链接器链接封闭单元。 / Links a closed unit set with the selected linker.
    fn link(
        &self,
        entry: &SourceKey,
        closure: UnitClosure,
    ) -> Result<LinkOutput, BuildRuntimeError>;
    /// Links shared immutable unit handles without copying their IR bodies.
    ///
    /// The default preserves existing runtime implementations. Production adapters
    /// override this method to retain shared ownership through the selected linker.
    fn link_shared(
        &self,
        entry: &SourceKey,
        closure: squish_link::SharedUnitClosure,
    ) -> Result<LinkOutput, BuildRuntimeError> {
        self.link(
            entry,
            UnitClosure {
                snapshot: (*closure.snapshot).clone(),
                units: closure
                    .units
                    .into_iter()
                    .map(|(key, unit)| (key, (*unit).clone()))
                    .collect(),
            },
        )
    }
    /// Links units whose validation and context-independent optimization are already sealed.
    /// Legacy runtime implementations retain their shared-unit fallback contract.
    fn link_prepared(
        &self,
        entry: &SourceKey,
        closure: squish_link::PreparedUnitClosure,
    ) -> Result<LinkOutput, BuildRuntimeError> {
        self.link_shared(
            entry,
            squish_link::SharedUnitClosure {
                snapshot: closure.snapshot,
                units: closure
                    .units
                    .into_iter()
                    .map(|(key, unit)| (key, unit.unit().clone()))
                    .collect(),
            },
        )
    }
    /// 使用选定求值器实例化程序。 / Instantiates a program with the selected evaluator.
    fn instantiate(
        &self,
        program: &LinkedProgram,
        arguments: BTreeMap<String, String>,
        budgets: Budgets,
    ) -> Result<InstantiateOutput, BuildRuntimeError>;
    /// 使用选定后端渲染产物。 / Renders output with the selected backend.
    fn render(&self, request: BackendRequest) -> Result<BackendOutput, BuildRuntimeError>;
    /// Renders an immutable document view while preserving the owned request contract.
    /// Production adapters override this method to avoid copying the document.
    fn render_shared(
        &self,
        document: &squish_ir::LinkedDocumentIr,
        trace: &squish_ir::ExpansionTrace,
        options: SquishOptions,
    ) -> Result<BackendOutput, BuildRuntimeError> {
        self.render_ref(BackendRequestRef {
            document,
            trace,
            options,
        })
    }
    /// Renders borrowed immutable inputs; legacy runtimes retain their owned contract.
    fn render_ref(
        &self,
        request: BackendRequestRef<'_>,
    ) -> Result<BackendOutput, BuildRuntimeError> {
        self.render(BackendRequest {
            document: request.document.clone(),
            trace: request.trace.clone(),
            options: request.options,
        })
    }
}

/// 在计划恢复边界打开调用级运行时的端口。 / Port opening an invocation runtime at the planning recovery boundary.
pub trait BuildRuntimeProvider: Send + Sync {
    /// 为已规范化项目根打开一个共享运行时。 / Opens one shared runtime for a normalized project root.
    fn open_build_runtime(
        &self,
        project_root: &Path,
    ) -> Result<Arc<dyn BuildRuntime>, crate::ServiceError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_build::{OutputName, ProducedOutput};
    use squish_protocol::ArtifactKind;

    /// Deliberately legacy-only adapter exercises all additive default bridges.
    struct LegacyRuntime {
        bytes: Option<Vec<u8>>,
        written_digest: Digest,
        record: Option<ActionRecord>,
        writes: std::sync::Mutex<Vec<Vec<u8>>>,
    }

    impl BuildRuntime for LegacyRuntime {
        fn descriptor(&self) -> BuildRuntimeDescriptor {
            unimplemented!()
        }
        fn backend_identity(
            &self,
            _: SquishOptions,
        ) -> Result<BackendCacheIdentity, BuildRuntimeError> {
            unimplemented!()
        }
        fn read_blob(&self, _: &Digest) -> Result<Option<Vec<u8>>, BuildRuntimeError> {
            Ok(self.bytes.clone())
        }
        fn write_blob(&self, bytes: &[u8]) -> Result<Digest, BuildRuntimeError> {
            self.writes.lock().unwrap().push(bytes.to_vec());
            Ok(self.written_digest.clone())
        }
        fn lookup_action(&self, _: &ActionKey) -> Result<Option<ActionRecord>, BuildRuntimeError> {
            Ok(self.record.clone())
        }
        fn record_action(&self, _: &ActionRecord) -> Result<(), BuildRuntimeError> {
            unimplemented!()
        }
        fn current_generation(
            &self,
            _: GenerationSpace,
            _: &PublicationTargetId,
        ) -> Result<Option<CommittedGeneration>, BuildRuntimeError> {
            unimplemented!()
        }
        fn read_generation_artifact(
            &self,
            _: GenerationSpace,
            _: &GenerationRef,
            _: &PublicationPath,
        ) -> Result<(ArtifactRead, Vec<u8>), BuildRuntimeError> {
            unimplemented!()
        }
        fn publish_generation(
            &self,
            _: GenerationSpace,
            target: &PublicationTargetId,
            publications: &[Publication],
        ) -> Result<CommittedGeneration, BuildRuntimeError> {
            let writes = self.writes.lock().unwrap();
            for publication in publications {
                assert!(writes.iter().any(|bytes| {
                    VerifiedBlob::from_shared(Arc::from(bytes.as_slice())).digest()
                        == &publication.output.digest
                }));
            }
            Ok(CommittedGeneration {
                identity: squish_build::GenerationRef {
                    target: target.clone(),
                    generation: squish_build::GenerationId::from_hex(&"00".repeat(32)).unwrap(),
                },
                artifacts: Vec::new(),
            })
        }
        fn compile(
            &self,
            _: &SourceBlob,
            _: &FrontendSourceContext,
        ) -> Result<FrontendOutput, BuildRuntimeError> {
            unimplemented!()
        }
        fn link(&self, _: &SourceKey, _: UnitClosure) -> Result<LinkOutput, BuildRuntimeError> {
            unimplemented!()
        }
        fn instantiate(
            &self,
            _: &LinkedProgram,
            _: BTreeMap<String, String>,
            _: Budgets,
        ) -> Result<InstantiateOutput, BuildRuntimeError> {
            unimplemented!()
        }
        fn render(&self, _: BackendRequest) -> Result<BackendOutput, BuildRuntimeError> {
            unimplemented!()
        }
    }

    fn fixture() -> (LegacyRuntime, VerifiedBlob) {
        let blob = VerifiedBlob::from_owned(b"immutable output".to_vec());
        (
            LegacyRuntime {
                bytes: Some(blob.bytes().to_vec()),
                written_digest: blob.digest().clone(),
                record: None,
                writes: std::sync::Mutex::new(Vec::new()),
            },
            blob,
        )
    }

    fn publication(blob: &VerifiedBlob, name: &str) -> Publication {
        Publication {
            output: ProducedOutput {
                name: OutputName::new(name).unwrap(),
                kind: ArtifactKind::Other("test".into()),
                digest: blob.digest().clone(),
                size: blob.bytes().len() as u64,
            },
            name: squish_build::LogicalArtifactName::new(name).unwrap(),
            destination: PublicationPath::new(name).unwrap(),
        }
    }

    #[test]
    fn legacy_verified_publication_restores_explicit_snapshots_once() {
        let (mut runtime, blob) = fixture();
        runtime.bytes = None;
        let target = PublicationTargetId::new("restore").unwrap();
        let publications = [publication(&blob, "first"), publication(&blob, "second")];
        runtime
            .publish_generation_verified(
                GenerationSpace::BuildCatalog,
                &target,
                &publications,
                &[blob.clone(), blob.clone()],
            )
            .unwrap();
        assert_eq!(*runtime.writes.lock().unwrap(), vec![blob.bytes().to_vec()]);
    }

    #[test]
    fn legacy_verified_publication_rejects_bad_handoff_and_wrong_write_identity() {
        let (mut runtime, blob) = fixture();
        let target = PublicationTargetId::new("restore").unwrap();
        let mut declared = publication(&blob, "first");
        declared.output.size += 1;
        assert_eq!(
            runtime
                .publish_generation_verified(
                    GenerationSpace::BuildCatalog,
                    &target,
                    &[declared],
                    std::slice::from_ref(&blob)
                )
                .unwrap_err()
                .code(),
            "runtime_publication_size"
        );
        assert!(runtime.writes.lock().unwrap().is_empty());
        let unrelated = VerifiedBlob::from_owned(b"unrelated".to_vec());
        assert_eq!(
            runtime
                .publish_generation_verified(
                    GenerationSpace::BuildCatalog,
                    &target,
                    &[publication(&blob, "first")],
                    &[unrelated]
                )
                .unwrap_err()
                .code(),
            "runtime_publication_blob"
        );
        assert!(runtime.writes.lock().unwrap().is_empty());
        runtime.written_digest = VerifiedBlob::from_owned(b"wrong identity".to_vec())
            .digest()
            .clone();
        assert_eq!(
            runtime
                .publish_generation_verified(
                    GenerationSpace::BuildCatalog,
                    &target,
                    &[publication(&blob, "first")],
                    &[blob]
                )
                .unwrap_err()
                .code(),
            "runtime_blob_digest"
        );
    }

    #[test]
    fn structured_diagnostic_does_not_change_legacy_runtime_error_identity() {
        let diagnostic = squish_protocol::Diagnostic {
            id: squish_protocol::DiagnosticId::new("evaluation").unwrap(),
            code: "RUN013".into(),
            severity: squish_protocol::Severity::Error,
            phase: squish_protocol::Phase::Instantiate,
            message: "max-expansions budget exceeded".into(),
            primary: Some(
                squish_protocol::Span::new(
                    squish_protocol::OpaqueSourceId::new("sopack://digest/library/lib.xml")
                        .unwrap(),
                    3,
                    9,
                )
                .unwrap(),
            ),
            related: Vec::new(),
            help: None,
        };
        let error =
            BuildRuntimeError::new("host_instantiate", "RUN013: max-expansions budget exceeded")
                .with_diagnostic(diagnostic.clone());
        assert_eq!(error.code(), "host_instantiate");
        assert_eq!(error.kind(), BuildRuntimeErrorKind::Tool);
        assert_eq!(error.diagnostic(), Some(&diagnostic));
        assert!(
            BuildRuntimeError::new("legacy", "legacy error")
                .diagnostic()
                .is_none()
        );
    }

    #[test]
    fn legacy_read_and_write_bridge_verify_identity() {
        let (runtime, blob) = fixture();
        let acquired = runtime.read_verified(blob.digest()).unwrap().unwrap();
        assert_eq!(acquired.digest(), blob.digest());
        assert_eq!(acquired.bytes(), blob.bytes());
        assert_eq!(
            runtime
                .read_blob_shared(blob.digest())
                .unwrap()
                .unwrap()
                .as_ref(),
            blob.bytes()
        );
        assert_eq!(runtime.write_verified_blob(&blob).unwrap(), *blob.digest());
        runtime.flush_advisory().unwrap();
    }

    #[test]
    fn legacy_wrong_read_or_write_digest_is_corruption() {
        let (mut runtime, blob) = fixture();
        runtime.bytes = Some(b"substituted".to_vec());
        assert_eq!(
            runtime.read_verified(blob.digest()).unwrap_err().kind(),
            BuildRuntimeErrorKind::Corrupt
        );
        runtime.written_digest = VerifiedBlob::from_owned(b"other".to_vec()).digest().clone();
        assert_eq!(
            runtime.write_verified_blob(&blob).unwrap_err().kind(),
            BuildRuntimeErrorKind::Corrupt
        );
    }

    #[test]
    fn legacy_verified_action_bridge_rejects_wrong_manifest_key() {
        let (mut runtime, _) = fixture();
        let requested = ActionKey::new("requested").unwrap();
        runtime.record = Some(ActionRecord {
            key: ActionKey::new("other").unwrap(),
            outputs: Vec::new(),
        });
        assert_eq!(
            runtime
                .lookup_action_verified(&requested)
                .unwrap_err()
                .code(),
            "runtime_action_key"
        );
    }

    #[test]
    fn verified_action_bridge_preserves_outputs_and_rejects_size_mismatch() {
        let (mut runtime, blob) = fixture();
        let key = ActionKey::new("action").unwrap();
        runtime.record = Some(ActionRecord {
            key: key.clone(),
            outputs: vec![ProducedOutput {
                name: OutputName::new("result").unwrap(),
                kind: ArtifactKind::Other("test".into()),
                digest: blob.digest().clone(),
                size: blob.bytes().len() as u64,
            }],
        });
        let action = runtime.lookup_action_verified(&key).unwrap().unwrap();
        assert_eq!(action.record, runtime.record.clone().unwrap());
        assert_eq!(action.blobs.len(), 1);
        assert_eq!(action.blobs[0].bytes(), blob.bytes());
        runtime.record.as_mut().unwrap().outputs[0].size += 1;
        assert_eq!(
            runtime.lookup_action_verified(&key).unwrap_err().kind(),
            BuildRuntimeErrorKind::Corrupt
        );
        runtime.bytes = None;
        assert!(runtime.lookup_action_verified(&key).unwrap().is_none());
    }
}
