//! 精确解析图的机器管理表示。 / Machine-managed exact resolution graph.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{LOCK_VERSION, ProjectError, ValidationIssue};

/// `xmlsquish.lock` 的精确、可重现解析状态。 / Exact reproducible resolution state in `xmlsquish.lock`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Lockfile {
    /// Lock schema version. / 锁架构版本。
    pub lock_version: u32,
    /// Resolver algorithm/protocol version. / Resolver 算法/协议版本。
    pub resolver_version: String,
    /// Digest of the complete normalized manifest set. / 完整归一化清单集的摘要。
    pub manifest_digest: String,
    /// Exact packages sorted by stable ID when serialized. / 序列化时按稳定 ID 排序的精确包。
    #[serde(rename = "package")]
    pub packages: Vec<LockedPackage>,
}

/// 解析图中的一个精确包节点。 / One exact package node in the resolved graph.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LockedPackage {
    /// Stable opaque node ID referenced by edges. / 被边引用的稳定不透明节点 ID。
    pub id: String,
    /// Published package name. / 发布包名。
    pub name: String,
    /// Exact selected semantic version. / 精确选中的语义版本。
    pub version: Version,
    /// Exact source locator and immutability semantics. / 精确源定位器与不可变语义。
    pub source: LockedSource,
    /// Digest of the package's own manifest identity. / 包自身清单身份摘要。
    pub manifest_digest: String,
    /// Direct edge alias to package ID. / 直接边别名到包 ID。
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

/// 锁定的来源身份；path 显式标记为可变。 / Locked source identity; paths are explicitly mutable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LockedSource {
    /// Registry archive pinned by checksum. / 由 checksum 锁定的 registry 归档。
    Registry { registry: String, checksum: String },
    /// Git repository pinned to an immutable commit and tree checksum. / 锁定到不可变 commit 及 tree checksum 的 Git 仓库。
    Git {
        repository: String,
        revision: String,
        checksum: String,
    },
    /// Mutable local locator; source bytes belong to a build snapshot, not this lock. / 可变本地定位器；源字节属于构建快照而非本锁。
    Path { path: PathBuf, mutable: bool },
    /// Workspace member bound by its manifest identity. / 由清单身份绑定的工作区成员。
    Workspace { member: PathBuf, mutable: bool },
}

impl Lockfile {
    /// 解析并验证精确图。 / Parses and validates the exact graph.
    pub fn parse(source: &str) -> Result<Self, ProjectError> {
        let mut value: Self = toml::from_str(source)?;
        value.packages.sort_by(|a, b| a.id.cmp(&b.id));
        value.validate()?;
        Ok(value)
    }

    /// 序列化为确定性 TOML。 / Serializes deterministic TOML.
    pub fn to_toml(&self) -> Result<String, ProjectError> {
        let mut normalized = self.clone();
        normalized.packages.sort_by(|a, b| a.id.cmp(&b.id));
        normalized.validate()?;
        toml_edit::ser::to_string_pretty(&normalized).map_err(ProjectError::from)
    }

    /// 验证唯一 ID、边完整性和源不变式。 / Validates unique IDs, edge integrity, and source invariants.
    pub fn validate(&self) -> Result<(), ProjectError> {
        let mut issues = Vec::new();
        if self.lock_version != LOCK_VERSION {
            issues.push(ValidationIssue::new(
                "lock-version",
                format!(
                    "unsupported version {}; expected {LOCK_VERSION}",
                    self.lock_version
                ),
            ));
        }
        if self.resolver_version.trim().is_empty() {
            issues.push(ValidationIssue::new(
                "resolver-version",
                "resolver version cannot be empty",
            ));
        }
        validate_digest("manifest-digest", &self.manifest_digest, &mut issues);
        let mut ids = BTreeSet::new();
        for package in &self.packages {
            if package.id.trim().is_empty() || !ids.insert(package.id.as_str()) {
                issues.push(ValidationIssue::new(
                    "package.id",
                    format!("empty or duplicate package ID `{}`", package.id),
                ));
            }
            validate_digest(
                &format!("package.{}.manifest-digest", package.id),
                &package.manifest_digest,
                &mut issues,
            );
            match &package.source {
                LockedSource::Registry { registry, checksum } => {
                    if registry.trim().is_empty() {
                        issues.push(ValidationIssue::new(
                            "package.source.registry",
                            "registry cannot be empty",
                        ));
                    }
                    validate_digest("package.source.checksum", checksum, &mut issues);
                }
                LockedSource::Git {
                    repository,
                    revision,
                    checksum,
                } => {
                    if repository.trim().is_empty() || !is_git_object_id(revision) {
                        issues.push(ValidationIssue::new(
                            "package.source",
                            "git repository and a full 40- or 64-hex object ID are required",
                        ));
                    }
                    validate_digest("package.source.checksum", checksum, &mut issues);
                }
                LockedSource::Path { path, mutable } => {
                    if path.as_os_str().is_empty() || path.is_absolute() {
                        issues.push(ValidationIssue::new(
                            "package.source.path",
                            "path locator must be non-empty and relative",
                        ));
                    }
                    if !mutable {
                        issues.push(ValidationIssue::new(
                            "package.source.mutable",
                            "path sources must declare mutable = true",
                        ));
                    }
                }
                LockedSource::Workspace { member, mutable } => {
                    if member.as_os_str().is_empty() || member.is_absolute() {
                        issues.push(ValidationIssue::new(
                            "package.source.member",
                            "workspace locator must be non-empty and relative",
                        ));
                    }
                    if !mutable {
                        issues.push(ValidationIssue::new(
                            "package.source.mutable",
                            "workspace sources must declare mutable = true",
                        ));
                    }
                }
            }
        }
        for package in &self.packages {
            for (alias, dependency) in &package.dependencies {
                if !ids.contains(dependency.as_str()) {
                    issues.push(ValidationIssue::new(
                        format!("package.{}.dependencies.{alias}", package.id),
                        format!("unknown package ID `{dependency}`"),
                    ));
                }
            }
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ProjectError::Validation(issues))
        }
    }
}

fn validate_digest(path: &str, digest: &str, issues: &mut Vec<ValidationIssue>) {
    let Some((algorithm, bytes)) = digest.split_once(':') else {
        issues.push(ValidationIssue::new(path, "digest must be `algorithm:hex`"));
        return;
    };
    if algorithm.is_empty() || bytes.len() < 32 || !bytes.bytes().all(|b| b.is_ascii_hexdigit()) {
        issues.push(ValidationIssue::new(
            path,
            "digest must have an algorithm and at least 128 bits of hexadecimal data",
        ));
    }
}

fn is_git_object_id(revision: &str) -> bool {
    matches!(revision.len(), 40 | 64) && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_graph_round_trips_deterministically() {
        let lock = Lockfile {
            lock_version: 1,
            resolver_version: "pubgrub/1".into(),
            manifest_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            packages: vec![LockedPackage {
                id: "demo@1.0.0#root".into(),
                name: "demo".into(),
                version: Version::new(1, 0, 0),
                source: LockedSource::Path {
                    path: ".".into(),
                    mutable: true,
                },
                manifest_digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                dependencies: BTreeMap::new(),
            }],
        };
        let encoded = lock.to_toml().unwrap();
        assert_eq!(Lockfile::parse(&encoded).unwrap(), lock);
        assert!(!encoded.contains("source bytes"));
    }

    #[test]
    fn rejects_mutable_git_revision_name() {
        let lock = Lockfile {
            lock_version: 1,
            resolver_version: "r/1".into(),
            manifest_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            packages: vec![LockedPackage {
                id: "git".into(),
                name: "git".into(),
                version: Version::new(1, 0, 0),
                source: LockedSource::Git {
                    repository: "https://example.invalid/repo".into(),
                    revision: "main".into(),
                    checksum: "sha256:cccccccccccccccccccccccccccccccc".into(),
                },
                manifest_digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                dependencies: BTreeMap::new(),
            }],
        };
        assert!(lock.validate().is_err());
    }
}
