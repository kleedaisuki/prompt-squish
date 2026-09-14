//! 窄锁、乐观、可恢复的清单/锁变更协议。 / Narrow-lock optimistic recoverable manifest/lock mutation protocol.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    LOCK_FILE_NAME, Lockfile, MANIFEST_FILE_NAME, Manifest, ProjectError, ValidationIssue,
};

/// 由 manager 适配器生成的不透明事务 ID。 / Opaque transaction ID supplied by the manager adapter.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransactionId(pub String);

/// 高层变更意图。 / High-level mutation intent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MutationKind {
    /// Add/replace one dependency. / 添加/替换一个依赖。
    AddDependency { alias: String },
    /// Remove one dependency. / 删除一个依赖。
    RemoveDependency { alias: String },
}

/// 事务中一个文件的 compare-and-replace 计划。 / Compare-and-replace plan for one transaction file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct MutationFile {
    /// Workspace-relative authoritative path. / 工作区相对权威路径。
    pub path: PathBuf,
    /// Digest observed before slow candidate resolution. / 慢候选解析前观察到的摘要。
    pub expected_digest: String,
    /// Candidate bytes staged after compare succeeds. / 比较成功后暂存的候选字节。
    pub candidate: Vec<u8>,
    /// Digest of candidate bytes, computed by the repository adapter. / 由仓库适配器计算的候选字节摘要。
    pub candidate_digest: String,
}

/// 慢 resolve 完成后、获取 writer lock 之前的不可变计划。 / Immutable plan after slow resolution and before acquiring the writer lock.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct MutationPlan {
    /// Stable transaction identity. / 稳定事务身份。
    pub id: TransactionId,
    /// User-visible operation. / 用户可见操作。
    pub kind: MutationKind,
    /// Candidate manifest-set digest that must equal the lock's digest. / 必须等于锁摘要的候选清单集摘要。
    pub manifest_digest: String,
    /// Complete authoritative read-set observed before slow resolution. / 慢解析前观察到的完整权威读集。
    pub observations: BTreeMap<PathBuf, String>,
    /// Every authoritative file changed as one generation. / 作为同一 generation 变更的所有权威文件。
    pub files: Vec<MutationFile>,
    /// Maximum optimistic recomputations after the initial plan. / 初始计划后的最大乐观重算次数。
    pub retry_limit: u8,
}

/// 持久化 journal 的记录；适配器顺序落盘。 / Records for a durable journal, flushed in order by an adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "kebab-case")]
pub enum JournalRecord {
    /// Candidate bytes have been staged. / 候选字节已暂存。
    Staged {
        id: TransactionId,
        generation: u64,
        files: Vec<(PathBuf, String)>,
    },
    /// Durable commit decision; recovery must roll forward after this point. / 持久提交决策；此后恢复必须向前完成。
    CommitDecided { id: TransactionId, generation: u64 },
    /// One target was atomically replaced. / 一个目标已原子替换。
    Replaced {
        id: TransactionId,
        generation: u64,
        path: PathBuf,
    },
    /// All replacements are durable and staging may be collected. / 所有替换已持久，可清理暂存。
    Completed { id: TransactionId, generation: u64 },
}

/// 乐观比较结果。 / Optimistic comparison outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitPreparation {
    /// Digests match; do only staging/journaling/replaces while holding the narrow lock. / 摘要匹配；持窄锁时只做暂存/journal/替换。
    Ready {
        generation: u64,
        records: Vec<JournalRecord>,
    },
    /// State changed; release the lock and recompute/resolve before a bounded retry. / 状态已变；释放锁并在有界重试前重算/解析。
    Retry { changed: Vec<PathBuf> },
    /// Contention outlived the bounded automatic retry budget. / 竞争超过了有界自动重试预算。
    Contended { changed: Vec<PathBuf>, attempts: u8 },
}

/// 只计算协议记录、不获锁不写文件的纯领域服务。 / Pure domain service computing protocol records without locking or writing.
pub struct MutationPlanner;

impl MutationPlanner {
    /// 从已完整 resolve 的锁和文件候选构造 coherent plan。 / Builds a coherent plan from a fully resolved lock and file candidates.
    pub fn plan(
        id: TransactionId,
        kind: MutationKind,
        manifest_digest: String,
        lock: &Lockfile,
        observations: BTreeMap<PathBuf, String>,
        files: Vec<MutationFile>,
    ) -> Result<MutationPlan, ProjectError> {
        lock.validate()?;
        let mut issues = Vec::new();
        if lock.manifest_digest != manifest_digest {
            issues.push(ValidationIssue::new(
                "manifest-digest",
                "candidate lock does not describe the candidate manifest set",
            ));
        }
        if files.len() < 2 {
            issues.push(ValidationIssue::new(
                "files",
                "coherent project mutation must include at least a manifest and lockfile",
            ));
        }
        let mut paths = std::collections::BTreeSet::new();
        for file in &files {
            if file.path.is_absolute() || file.path.as_os_str().is_empty() {
                issues.push(ValidationIssue::new(
                    "files.path",
                    "mutation paths must be non-empty and workspace-relative",
                ));
            }
            if !paths.insert(&file.path) {
                issues.push(ValidationIssue::new(
                    "files.path",
                    format!("duplicate mutation path `{}`", file.path.display()),
                ));
            }
            if file.expected_digest.is_empty() || file.candidate_digest.is_empty() {
                issues.push(ValidationIssue::new(
                    "files.digest",
                    "expected and candidate digests are required",
                ));
            }
            if observations.get(&file.path) != Some(&file.expected_digest) {
                issues.push(ValidationIssue::new(
                    "observations",
                    format!(
                        "write `{}` must agree with the complete read-set",
                        file.path.display()
                    ),
                ));
            }
        }
        if observations.is_empty() {
            issues.push(ValidationIssue::new(
                "observations",
                "transaction read-set cannot be empty",
            ));
        }
        let manifest_files: Vec<_> = files
            .iter()
            .filter(|file| {
                file.path
                    .file_name()
                    .is_some_and(|name| name == MANIFEST_FILE_NAME)
            })
            .collect();
        if manifest_files.is_empty() {
            issues.push(ValidationIssue::new(
                "files",
                "project mutation must include an xmlsquish.toml candidate",
            ));
        }
        for file in manifest_files {
            if std::str::from_utf8(&file.candidate)
                .ok()
                .and_then(|source| Manifest::parse(source).ok())
                .is_none()
            {
                issues.push(ValidationIssue::new(
                    format!("files.{}", file.path.display()),
                    "manifest candidate is not valid UTF-8 xmlsquish TOML",
                ));
            }
        }
        let lock_files: Vec<_> = files
            .iter()
            .filter(|file| {
                file.path
                    .file_name()
                    .is_some_and(|name| name == LOCK_FILE_NAME)
            })
            .collect();
        if lock_files.len() != 1 {
            issues.push(ValidationIssue::new(
                "files",
                "project mutation must include exactly one xmlsquish.lock candidate",
            ));
        } else if std::str::from_utf8(&lock_files[0].candidate)
            .ok()
            .and_then(|source| Lockfile::parse(source).ok())
            .as_ref()
            != Some(lock)
        {
            issues.push(ValidationIssue::new(
                "files.xmlsquish.lock",
                "lock candidate bytes must encode the resolved candidate lock",
            ));
        }
        if issues.is_empty() {
            Ok(MutationPlan {
                id,
                kind,
                manifest_digest,
                observations,
                files,
                retry_limit: 4,
            })
        } else {
            Err(ProjectError::Validation(issues))
        }
    }

    /// 在获取工作区窄 writer lock 并重读后比较 digest。 / Compares digests after taking the narrow workspace writer lock and re-reading state.
    pub fn prepare_commit(
        plan: &MutationPlan,
        observed: &BTreeMap<PathBuf, String>,
        generation: u64,
        attempt: u8,
    ) -> CommitPreparation {
        let changed: Vec<_> = plan
            .observations
            .iter()
            .filter(|(path, digest)| observed.get(*path) != Some(*digest))
            .map(|(path, _)| path.clone())
            .collect();
        if !changed.is_empty() {
            return if attempt < plan.retry_limit {
                CommitPreparation::Retry { changed }
            } else {
                CommitPreparation::Contended {
                    changed,
                    attempts: attempt,
                }
            };
        }
        let files = plan
            .files
            .iter()
            .map(|file| (file.path.clone(), file.candidate_digest.clone()))
            .collect();
        let mut records = vec![
            JournalRecord::Staged {
                id: plan.id.clone(),
                generation,
                files,
            },
            JournalRecord::CommitDecided {
                id: plan.id.clone(),
                generation,
            },
        ];
        records.extend(plan.files.iter().map(|file| JournalRecord::Replaced {
            id: plan.id.clone(),
            generation,
            path: file.path.clone(),
        }));
        records.push(JournalRecord::Completed {
            id: plan.id.clone(),
            generation,
        });
        CommitPreparation::Ready {
            generation,
            records,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LockedPackage, LockedSource};
    use semver::Version;
    use std::collections::BTreeMap;

    fn lock() -> Lockfile {
        Lockfile {
            lock_version: 1,
            resolver_version: "r/1".into(),
            manifest_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            packages: vec![LockedPackage {
                id: "root".into(),
                name: "root".into(),
                version: Version::new(1, 0, 0),
                source: LockedSource::Path {
                    path: ".".into(),
                    mutable: true,
                },
                manifest_digest: "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                dependencies: BTreeMap::new(),
            }],
        }
    }
    fn file(path: &str, expected: &str) -> MutationFile {
        let candidate = if path == "xmlsquish.lock" {
            lock().to_toml().unwrap().into_bytes()
        } else {
            b"manifest-version = 1\n[package]\nname = \"root\"\nversion = \"1.0.0\"\n".to_vec()
        };
        MutationFile {
            path: path.into(),
            expected_digest: expected.into(),
            candidate,
            candidate_digest: format!("new-{expected}"),
        }
    }
    fn read_set() -> BTreeMap<PathBuf, String> {
        BTreeMap::from([
            ("xmlsquish.toml".into(), "m1".into()),
            ("xmlsquish.lock".into(), "l1".into()),
            ("packages/peer/xmlsquish.toml".into(), "p1".into()),
        ])
    }

    #[test]
    fn stale_read_requests_retry_without_commit_records() {
        let plan = MutationPlanner::plan(
            TransactionId("t1".into()),
            MutationKind::AddDependency { alias: "x".into() },
            lock().manifest_digest.clone(),
            &lock(),
            read_set(),
            vec![file("xmlsquish.toml", "m1"), file("xmlsquish.lock", "l1")],
        )
        .unwrap();
        let observed = BTreeMap::from([
            ("xmlsquish.toml".into(), "m2".into()),
            ("xmlsquish.lock".into(), "l1".into()),
            ("packages/peer/xmlsquish.toml".into(), "p1".into()),
        ]);
        assert_eq!(
            MutationPlanner::prepare_commit(&plan, &observed, 4, 0),
            CommitPreparation::Retry {
                changed: vec!["xmlsquish.toml".into()]
            }
        );
    }

    #[test]
    fn matching_read_yields_roll_forward_journal() {
        let plan = MutationPlanner::plan(
            TransactionId("t1".into()),
            MutationKind::RemoveDependency { alias: "x".into() },
            lock().manifest_digest.clone(),
            &lock(),
            read_set(),
            vec![file("xmlsquish.toml", "m1"), file("xmlsquish.lock", "l1")],
        )
        .unwrap();
        let observed = BTreeMap::from([
            ("xmlsquish.toml".into(), "m1".into()),
            ("xmlsquish.lock".into(), "l1".into()),
            ("packages/peer/xmlsquish.toml".into(), "p1".into()),
        ]);
        let CommitPreparation::Ready { records, .. } =
            MutationPlanner::prepare_commit(&plan, &observed, 5, 0)
        else {
            panic!("should be ready")
        };
        assert!(matches!(records[1], JournalRecord::CommitDecided { .. }));
        assert!(matches!(
            records.last(),
            Some(JournalRecord::Completed { .. })
        ));
    }

    #[test]
    fn changed_read_only_member_requests_retry() {
        let plan = MutationPlanner::plan(
            TransactionId("t2".into()),
            MutationKind::AddDependency { alias: "x".into() },
            lock().manifest_digest.clone(),
            &lock(),
            read_set(),
            vec![file("xmlsquish.toml", "m1"), file("xmlsquish.lock", "l1")],
        )
        .unwrap();
        let mut observed = read_set();
        observed.insert("packages/peer/xmlsquish.toml".into(), "p2".into());
        assert_eq!(
            MutationPlanner::prepare_commit(&plan, &observed, 5, 0),
            CommitPreparation::Retry {
                changed: vec!["packages/peer/xmlsquish.toml".into()]
            }
        );
    }
}
