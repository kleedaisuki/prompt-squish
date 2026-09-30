//! 精确解析图的机器管理表示。 / Machine-managed exact resolution graph.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{GitReference, LOCK_VERSION, ProjectError, ValidationIssue};

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
    /// Exact skill snapshots, independent of the XML package graph.
    #[serde(default, rename = "skill", skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<LockedSkill>,
}

/// One installed Agent Skill snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LockedSkill {
    /// Manifest key, SKILL.md frontmatter name, and installed directory name.
    pub name: String,
    /// Exact source identity used to obtain this snapshot.
    pub source: LockedSkillSource,
    /// Digest of the normalized complete skill tree, not just SKILL.md.
    pub digest: String,
}

/// Pinned skill source; local paths retain mutable intent while digest pins content.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LockedSkillSource {
    /// Directory relative to the declaring manifest.
    Path { path: PathBuf, mutable: bool },
    /// Repository pinned to a full commit and a repository-contained skill directory.
    Git {
        repository: String,
        revision: String,
        subdir: Option<PathBuf>,
        /// Original mutable selector. Empty means repository HEAD.
        #[serde(default, flatten)]
        reference: GitReference,
    },
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
    /// Portable archive locator pinned by the exact SHA-256 archive digest.
    Sopack { path: PathBuf, checksum: String },
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
        value.skills.sort_by(|a, b| a.name.cmp(&b.name));
        value.validate()?;
        Ok(value)
    }

    /// 序列化为确定性 TOML。 / Serializes deterministic TOML.
    pub fn to_toml(&self) -> Result<String, ProjectError> {
        let mut normalized = self.clone();
        normalized.packages.sort_by(|a, b| a.id.cmp(&b.id));
        normalized.skills.sort_by(|a, b| a.name.cmp(&b.name));
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
                LockedSource::Sopack { path, checksum } => {
                    if path.as_os_str().is_empty() || path.is_absolute() {
                        issues.push(ValidationIssue::new(
                            "package.source.sopack",
                            "sopack locator must be non-empty and relative",
                        ));
                    }
                    if !checksum.strip_prefix("sha256:").is_some_and(|hex| {
                        hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                    }) {
                        issues.push(ValidationIssue::new(
                            "package.source.checksum",
                            "SOPack requires an exact SHA-256 archive checksum",
                        ));
                    }
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
        let mut skill_names = BTreeSet::new();
        for skill in &self.skills {
            if !valid_skill_name(&skill.name) || !skill_names.insert(skill.name.as_str()) {
                issues.push(ValidationIssue::new(
                    "skill.name",
                    format!("invalid or duplicate skill name `{}`", skill.name),
                ));
            }
            validate_digest(
                &format!("skill.{}.digest", skill.name),
                &skill.digest,
                &mut issues,
            );
            match &skill.source {
                LockedSkillSource::Path { path, mutable } => {
                    if path.as_os_str().is_empty() || path.is_absolute() {
                        issues.push(ValidationIssue::new(
                            format!("skill.{}.source.path", skill.name),
                            "path must be non-empty and relative",
                        ));
                    }
                    if !mutable {
                        issues.push(ValidationIssue::new(
                            format!("skill.{}.source.mutable", skill.name),
                            "path sources must declare mutable = true",
                        ));
                    }
                }
                LockedSkillSource::Git {
                    repository,
                    revision,
                    subdir,
                    reference,
                } => {
                    if repository.trim().is_empty() || !is_git_object_id(revision) {
                        issues.push(ValidationIssue::new(
                            format!("skill.{}.source", skill.name),
                            "git repository and full 40- or 64-hex commit ID are required",
                        ));
                    }
                    if subdir.as_ref().is_some_and(|path| {
                        path.as_os_str().is_empty()
                            || path.is_absolute()
                            || path
                                .components()
                                .any(|part| matches!(part, std::path::Component::ParentDir))
                    }) {
                        issues.push(ValidationIssue::new(
                            format!("skill.{}.source.subdir", skill.name),
                            "subdir must stay within the repository",
                        ));
                    }
                    let selectors = [
                        ("branch", &reference.branch),
                        ("tag", &reference.tag),
                        ("rev", &reference.rev),
                    ];
                    if selectors
                        .iter()
                        .filter(|(_, selector)| selector.is_some())
                        .count()
                        > 1
                    {
                        issues.push(ValidationIssue::new(
                            format!("skill.{}.source", skill.name),
                            "git branch, tag, and rev are mutually exclusive",
                        ));
                    }
                    for (key, selector) in selectors {
                        if selector
                            .as_ref()
                            .is_some_and(|value| value.trim().is_empty())
                        {
                            issues.push(ValidationIssue::new(
                                format!("skill.{}.source.{key}", skill.name),
                                "git selector cannot be empty",
                            ));
                        }
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

fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
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
            skills: Vec::new(),
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
            skills: Vec::new(),
        };
        assert!(lock.validate().is_err());
    }

    #[test]
    fn legacy_lock_and_skill_snapshots_round_trip() {
        let legacy = r#"lock-version = 1
resolver-version = "r/1"
manifest-digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
package = []
"#;
        let mut lock = Lockfile::parse(legacy).unwrap();
        assert!(lock.skills.is_empty());
        lock.skills = vec![
            LockedSkill {
                name: "zeta".into(),
                source: LockedSkillSource::Path {
                    path: "skills/zeta".into(),
                    mutable: true,
                },
                digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            LockedSkill {
                name: "alpha".into(),
                source: LockedSkillSource::Git {
                    repository: "https://example.invalid/skills.git".into(),
                    revision: "a".repeat(40),
                    subdir: Some("skills/alpha".into()),
                    reference: GitReference {
                        tag: Some("v1".into()),
                        ..GitReference::default()
                    },
                },
                digest: "sha256:cccccccccccccccccccccccccccccccc".into(),
            },
        ];
        let encoded = lock.to_toml().unwrap();
        assert!(encoded.contains("[[skill]]"));
        let parsed = Lockfile::parse(&encoded).unwrap();
        assert_eq!(parsed.skills[0].name, "alpha");
        assert!(encoded.contains("tag = \"v1\""));
        assert!(
            matches!(&parsed.skills[0].source, LockedSkillSource::Git { reference, .. } if reference.tag.as_deref() == Some("v1"))
        );
        assert_eq!(parsed.skills[1].name, "zeta");
        lock.skills.push(lock.skills[0].clone());
        assert!(lock.validate().is_err());
    }

    #[test]
    fn legacy_git_skill_without_selector_means_head_and_invalid_selectors_fail() {
        let source = format!(
            r#"lock-version = 1
resolver-version = "r/1"
manifest-digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
package = []
[[skill]]
name = "review"
digest = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
[skill.source]
kind = "git"
repository = "https://example.invalid/skills.git"
revision = "{}"
"#,
            "a".repeat(40)
        );
        let mut lock = Lockfile::parse(&source).unwrap();
        let LockedSkillSource::Git { reference, .. } = &mut lock.skills[0].source else {
            panic!("expected git")
        };
        assert_eq!(reference, &GitReference::default());
        reference.branch = Some("main".into());
        reference.tag = Some("v1".into());
        assert!(lock.validate().is_err());
    }
}
