//! 阶段化缓存键的语义投影。 / Semantic projections for phase-local cache keys.

use crate::{Budgets, LinkedProgram};
use squish_ir::{
    Digest, LinkedImage, ObjectDigest, ResolutionSnapshot, SourceKey, encode_linked_image,
};
use std::collections::BTreeMap;

/// 链接阶段投影；故意排除运行参数、预算和后端选项。
/// Link-stage projection deliberately excluding runtime arguments, budgets, and backend options.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkKeyProjection {
    /// 能改变重定位语义的 linker ABI。 / Linker ABI capable of changing relocation semantics.
    pub linker_abi: String,
    /// 显式 entry 链接根。 / Explicit entry link root.
    pub entry: SourceKey,
    /// 已排序的冻结解析图。 / Sorted frozen resolution graph.
    pub resolution: ResolutionSnapshot,
}

impl LinkKeyProjection {
    /// 以稳定字段编码计算领域分隔键。 / Computes a domain-separated key over a stable field encoding.
    #[must_use]
    pub fn digest(&self) -> Digest {
        let mut bytes = Vec::new();
        field(&mut bytes, self.linker_abi.as_bytes());
        source(&mut bytes, &self.entry);
        for (key, revision) in &self.resolution.units {
            source(&mut bytes, key);
            bytes.extend_from_slice(&revision.semantic.0.bytes);
            bytes.push(match revision.kind {
                squish_ir::UnitKind::Entry => 1,
                squish_ir::UnitKind::Module => 2,
            });
        }
        for edge in &self.resolution.imports {
            source(&mut bytes, &edge.importer);
            bytes.extend_from_slice(&edge.import.0.to_le_bytes());
            source(&mut bytes, &edge.target);
        }
        Digest::sha256("link-key-v1", &bytes)
    }
}

/// 实例化阶段投影；参数、求值器身份和所有预算都是语义输入。
/// Instantiation-stage projection; arguments, evaluator identity, and every budget are semantic inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstantiateKeyProjection {
    /// 求值器语义 ABI。 / Evaluator semantic ABI.
    evaluator_abi: String,
    /// 不包含后端选项的链接镜像。 / Linked image without backend options.
    linked_image: LinkedImage,
    /// 规范名称排序的 entry 参数。 / Entry arguments sorted by canonical name.
    arguments: BTreeMap<String, String>,
    /// 即使未耗尽也参与键的预算。 / Budgets included even when not exhausted.
    budgets: Budgets,
    /// 完整对象摘要把调试/source attachment 身份纳入 trace 复用边界。
    /// Full object digests include debug/source-attachment identity in trace reuse.
    objects: Vec<(SourceKey, ObjectDigest)>,
}

impl InstantiateKeyProjection {
    /// 从已验证会话视图建立完整投影。 / Builds a complete projection from a validated session view.
    #[must_use]
    pub fn new(
        evaluator_abi: impl Into<String>,
        program: &LinkedProgram,
        arguments: BTreeMap<String, String>,
        budgets: Budgets,
    ) -> Self {
        let objects = program
            .image()
            .units
            .iter()
            .enumerate()
            .map(|(slot, unit)| {
                (
                    unit.source.clone(),
                    program
                        .object(slot as u32)
                        .expect("a reconstructed program has one object per unit"),
                )
            })
            .collect();
        Self {
            evaluator_abi: evaluator_abi.into(),
            linked_image: program.image().clone(),
            arguments,
            budgets,
            objects,
        }
    }

    /// 仅计算文档语义复用键；不允许用它复用 provenance。 / Computes the document-semantic reuse key only; it must not reuse provenance.
    #[must_use]
    pub fn document_digest(&self) -> Digest {
        let mut bytes = Vec::new();
        field(&mut bytes, self.evaluator_abi.as_bytes());
        field(&mut bytes, &encode_linked_image(&self.linked_image));
        for (name, value) in &self.arguments {
            field(&mut bytes, name.as_bytes());
            field(&mut bytes, value.as_bytes());
        }
        bytes.extend_from_slice(&self.budgets.max_depth.to_le_bytes());
        bytes.extend_from_slice(&self.budgets.max_expansions.to_le_bytes());
        bytes.extend_from_slice(&self.budgets.max_output_bytes.to_le_bytes());
        Digest::sha256("instantiate-document-key-v1", &bytes)
    }

    /// 计算 document+trace 联合结果键，包含完整对象/调试身份。 / Computes the joint document-and-trace key including full object/debug identity.
    #[must_use]
    pub fn digest(&self) -> Digest {
        let mut bytes = self.document_digest().bytes.to_vec();
        for (source_key, object) in &self.objects {
            source(&mut bytes, source_key);
            bytes.extend_from_slice(&object.0.bytes);
        }
        Digest::sha256("instantiate-bundle-key-v1", &bytes)
    }

    /// 返回本次预算。 / Returns the projected budgets.
    #[must_use]
    pub fn budgets(&self) -> Budgets {
        self.budgets
    }

    /// 返回 entry 参数。 / Returns the projected entry arguments.
    #[must_use]
    pub fn arguments(&self) -> &BTreeMap<String, String> {
        &self.arguments
    }

    /// 返回求值器 ABI。 / Returns the evaluator ABI.
    #[must_use]
    pub fn evaluator_abi(&self) -> &str {
        &self.evaluator_abi
    }

    /// 返回 linked image。 / Returns the linked image.
    #[must_use]
    pub fn linked_image(&self) -> &LinkedImage {
        &self.linked_image
    }
}

fn field(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_le_bytes());
    out.extend_from_slice(value);
}
fn source(out: &mut Vec<u8>, key: &SourceKey) {
    match key {
        SourceKey::AdHoc { uri } => {
            out.push(1);
            field(out, uri.as_bytes());
        }
        SourceKey::Project { package, path } => {
            out.push(2);
            out.extend_from_slice(&package.source_kind.to_le_bytes());
            field(out, package.canonical_source.as_bytes());
            field(out, package.package_name.as_bytes());
            field(out, package.exact_revision.as_bytes());
            out.extend_from_slice(&(path.len() as u64).to_le_bytes());
            for part in path {
                field(out, part.as_bytes());
            }
        }
    }
}
