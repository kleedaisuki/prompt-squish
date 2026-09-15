use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    Access, ContentDigest, FetchError, GitObjectFormat, GitOid, HostContext, LogicalFile,
    LogicalTree, ManifestDigest, Materializer, Sha256Digest, SourceEvent,
};

/// 明确的、无 revision-expression 的 Git 选择器。 / Explicit Git selector without revision-expression semantics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum GitSelector {
    Head,
    Branch(String),
    Tag(String),
    Rev(String),
}

/// 带 commit/tree/content 身份的候选。 / Candidate with commit, tree, and content identities.
#[derive(Clone, Debug)]
pub struct ExactGitCandidate {
    pub commit: GitOid,
    pub root_tree: GitOid,
    pub package_tree: GitOid,
    pub subdir: String,
    pub content_digest: ContentDigest,
    pub manifest_digest: ManifestDigest,
    pub manifest: squish_project::Manifest,
}

#[derive(Serialize, Deserialize)]
struct Observation {
    commit: String,
    format: GitObjectFormat,
}

static FETCH_NONCE: AtomicU64 = AtomicU64::new(1);

/// 以 protocol v2 和共享 bare object DB 实现的 Git port。 / Git port backed by protocol v2 and a shared bare object database.
#[derive(Clone)]
pub struct GitHost {
    context: HostContext,
    materializer: Materializer,
}

impl GitHost {
    /// 创建 Git source host。 / Creates a Git source host.
    pub fn new(context: HostContext) -> Self {
        Self {
            materializer: Materializer::new(context.clone()),
            context,
        }
    }

    /// 解析 selector，并直接读取对象数据库中的 blob；不创建 working tree。 / Resolves a selector and reads blobs directly from the object database, without a working tree.
    pub fn resolve_exact(
        &self,
        repository: &str,
        selector: &GitSelector,
        subdir: &str,
        access: Access,
    ) -> Result<ExactGitCandidate, FetchError> {
        validate_repository(repository)?;
        validate_subdir(subdir)?;
        validate_selector(selector)?;
        let observation = self.observation_path(repository, selector);
        let (commit, format) = match access {
            Access::LocalOnly => {
                let observed = fs::read(&observation)
                    .ok()
                    .and_then(|v| serde_json::from_slice::<Observation>(&v).ok())
                    .ok_or_else(|| {
                        FetchError::OfflineMiss(format!(
                            "Git selector observation for {}",
                            stable_repo(repository)
                        ))
                    })?;
                (observed.commit, observed.format)
            }
            Access::Online => {
                let format = self.remote_object_format(repository)?;
                let db = self.db_path(repository, format);
                self.ensure_db(&db, format)?;
                let value = self.fetch_selector(&db, repository, selector)?;
                if let Some(parent) = observation.parent() {
                    fs::create_dir_all(parent)?;
                }
                write_sidecar(
                    &observation,
                    &serde_json::to_vec(&Observation {
                        commit: value.clone(),
                        format,
                    })?,
                )?;
                (value, format)
            }
        };
        let db = self.db_path(repository, format);
        if !db.join("HEAD").exists() {
            return Err(FetchError::OfflineMiss(format!(
                "Git object database for {}",
                stable_repo(repository)
            )));
        }
        self.require_type(&db, &commit, "commit")?;
        if self.object_format(&db)? != format {
            return Err(FetchError::Integrity(
                "Git database object format mismatch".into(),
            ));
        }
        let commit_oid = GitOid::new(format, commit.clone())?;
        let root = self
            .git_text(&db, &["rev-parse", &format!("{commit}^{{tree}}")])?
            .trim()
            .to_owned();
        let package = if subdir == "." {
            root.clone()
        } else {
            self.git_text(&db, &["rev-parse", &format!("{root}:{subdir}")])?
                .trim()
                .to_owned()
        };
        self.require_type(&db, &package, "tree")?;
        let tree = self.read_tree(&db, &package)?;
        let manifest_file = tree
            .files
            .iter()
            .find(|f| f.path == "xmlsquish.toml")
            .ok_or_else(|| FetchError::Git(format!("manifest missing in subdir {subdir}")))?;
        let manifest_digest = ManifestDigest(Sha256Digest(hex::encode(Sha256::digest(
            &manifest_file.bytes,
        ))));
        let manifest = squish_project::Manifest::parse(
            std::str::from_utf8(&manifest_file.bytes)
                .map_err(|_| FetchError::Integrity("Git manifest is not UTF-8".into()))?,
        )?;
        self.context.observer.emit(SourceEvent::IntegrityVerified {
            identity: stable_repo(repository),
            content_digest: tree.content_digest.0.0.clone(),
            files: tree.files.len(),
            bytes: tree.files.iter().map(|f| f.bytes.len() as u64).sum(),
        });
        let _ = self
            .materializer
            .materialize(&stable_repo(repository), &tree)?;
        Ok(ExactGitCandidate {
            commit: commit_oid,
            root_tree: GitOid::new(format, root)?,
            package_tree: GitOid::new(format, package)?,
            subdir: subdir.into(),
            content_digest: tree.content_digest,
            manifest_digest,
            manifest,
        })
    }

    /// 仅按 lock 中的完整 commit/tree 身份物化；绝不重新解析 ref。 / Materializes solely from full locked commit/tree identities and never re-resolves a ref.
    pub fn materialize_locked(
        &self,
        repository: &str,
        commit: &GitOid,
        package_tree: &GitOid,
        subdir: &str,
        expected_content: &ContentDigest,
        access: Access,
    ) -> Result<crate::MaterializedPackage, FetchError> {
        validate_repository(repository)?;
        validate_subdir(subdir)?;
        let db = self.db_path(repository, commit.format);
        self.ensure_db(&db, commit.format)?;
        if self.require_type(&db, &commit.hex, "commit").is_err() {
            if access == Access::LocalOnly {
                return Err(FetchError::OfflineMiss(format!(
                    "Git commit {}",
                    commit.hex
                )));
            }
            self.fetch_selector(&db, repository, &GitSelector::Rev(commit.hex.clone()))?;
            self.require_type(&db, &commit.hex, "commit")?;
        }
        if self.object_format(&db)? != commit.format || package_tree.format != commit.format {
            return Err(FetchError::Integrity(
                "locked Git object format mismatch".into(),
            ));
        }
        let root = self.git_text(&db, &["rev-parse", &format!("{}^{{tree}}", commit.hex)])?;
        let selected = if subdir == "." {
            root.trim().to_owned()
        } else {
            self.git_text(&db, &["rev-parse", &format!("{}:{subdir}", root.trim())])?
                .trim()
                .to_owned()
        };
        if selected != package_tree.hex {
            return Err(FetchError::Integrity(
                "locked Git package-tree mismatch".into(),
            ));
        }
        let tree = self.read_tree(&db, &selected)?;
        if tree.content_digest != *expected_content {
            return Err(FetchError::Integrity(
                "locked Git content digest mismatch".into(),
            ));
        }
        self.materializer.materialize(
            &format!("git:{}#{}@{}", stable_repo(repository), subdir, commit.hex),
            &tree,
        )
    }

    fn read_tree(&self, db: &Path, tree: &str) -> Result<LogicalTree, FetchError> {
        let output = self.run(db, &["ls-tree", "-rz", "--full-tree", "-r", tree])?;
        let mut files = Vec::new();
        for entry in output.stdout.split(|b| *b == 0).filter(|v| !v.is_empty()) {
            let tab = entry
                .iter()
                .position(|b| *b == b'\t')
                .ok_or_else(|| FetchError::Git("malformed ls-tree output".into()))?;
            let header = std::str::from_utf8(&entry[..tab])
                .map_err(|_| FetchError::Git("malformed ls-tree header".into()))?;
            let mut fields = header.split_ascii_whitespace();
            let mode = fields.next().unwrap_or("");
            let kind = fields.next().unwrap_or("");
            let oid = fields.next().unwrap_or("");
            let path = std::str::from_utf8(&entry[tab + 1..])
                .map_err(|_| FetchError::Path("non-UTF-8 Git path".into()))?;
            if !matches!(mode, "100644" | "100755") || kind != "blob" {
                return Err(FetchError::Unsupported(format!(
                    "Git mode {mode} at {path}"
                )));
            }
            let body = self.run(db, &["cat-file", "blob", oid])?.stdout;
            files.push(LogicalFile {
                path: path.into(),
                bytes: body,
            });
        }
        LogicalTree::build(files, &self.context.limits)
    }

    fn fetch_selector(
        &self,
        db: &Path,
        repo: &str,
        selector: &GitSelector,
    ) -> Result<String, FetchError> {
        let remote = match selector {
            GitSelector::Head => "HEAD".into(),
            GitSelector::Branch(v) => format!("refs/heads/{v}"),
            GitSelector::Tag(v) => format!("refs/tags/{v}"),
            GitSelector::Rev(v) => v.clone(),
        };
        let anchor = format!(
            "refs/xmlsquish/fetch-{}-{}",
            std::process::id(),
            FETCH_NONCE.fetch_add(1, Ordering::Relaxed)
        );
        let request_id = FETCH_NONCE.fetch_add(1, Ordering::Relaxed);
        self.context
            .observer
            .emit(SourceEvent::NetworkRequestStarted {
                request_id,
                origin: stable_repo(repo),
                purpose: "git-fetch".into(),
            });
        let status = self.run(
            db,
            &[
                "-c",
                "protocol.version=2",
                "fetch",
                "--atomic",
                "--no-tags",
                repo,
                &format!("{remote}:{anchor}"),
            ],
        );
        if status.is_err() && matches!(selector, GitSelector::Rev(_)) {
            return Err(FetchError::Git("revision not fetchable".into()));
        }
        status?;
        let peeled = self
            .git_text(
                db,
                &["rev-parse", "--verify", &format!("{anchor}^{{commit}}")],
            )?
            .trim()
            .to_owned();
        if let GitSelector::Rev(expected) = selector
            && &peeled != expected
        {
            return Err(FetchError::Integrity(
                "fetched Git revision does not match requested OID".into(),
            ));
        }
        self.context
            .observer
            .emit(SourceEvent::NetworkRequestFinished {
                request_id,
                status: "success".into(),
                received_bytes: 0,
                validator_used: false,
            });
        Ok(peeled)
    }
    fn ensure_db(&self, db: &Path, format: GitObjectFormat) -> Result<(), FetchError> {
        fs::create_dir_all(db.parent().unwrap())?;
        let init_lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(db.with_extension("init.lock"))?;
        init_lock.lock_exclusive()?;
        if !db.join("HEAD").exists() {
            let out = Command::new("git")
                .args([
                    "init",
                    "--bare",
                    &format!("--object-format={}", object_format_name(format)),
                ])
                .arg(db)
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()?;
            if !out.status.success() {
                return Err(FetchError::Git(
                    "cannot initialize bare object database".into(),
                ));
            }
        }
        Ok(())
    }
    fn object_format(&self, db: &Path) -> Result<GitObjectFormat, FetchError> {
        match self
            .git_text(db, &["rev-parse", "--show-object-format"])?
            .trim()
        {
            "sha1" => Ok(GitObjectFormat::Sha1),
            "sha256" => Ok(GitObjectFormat::Sha256),
            v => Err(FetchError::Git(format!("unsupported object format {v}"))),
        }
    }
    fn remote_object_format(&self, repository: &str) -> Result<GitObjectFormat, FetchError> {
        let url = Url::parse(repository).map_err(|e| FetchError::Config(e.to_string()))?;
        if url.scheme() == "file" {
            let path = url
                .to_file_path()
                .map_err(|_| FetchError::Config("invalid file Git URL".into()))?;
            let output = Command::new("git")
                .arg("-C")
                .arg(path)
                .args(["rev-parse", "--show-object-format"])
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()?;
            if output.status.success() {
                return match String::from_utf8_lossy(&output.stdout).trim() {
                    "sha1" => Ok(GitObjectFormat::Sha1),
                    "sha256" => Ok(GitObjectFormat::Sha256),
                    _ => Err(FetchError::Git(
                        "unsupported remote Git object format".into(),
                    )),
                };
            }
        }
        let request_id = FETCH_NONCE.fetch_add(1, Ordering::Relaxed);
        self.context
            .observer
            .emit(SourceEvent::NetworkRequestStarted {
                request_id,
                origin: stable_repo(repository),
                purpose: "git-capabilities".into(),
            });
        let output = Command::new("git")
            .args([
                "-c",
                "protocol.version=2",
                "ls-remote",
                "--symref",
                repository,
                "HEAD",
            ])
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "Never")
            .env("GIT_TRACE_PACKET", "1")
            .output()?;
        if !output.status.success() {
            return Err(FetchError::Git(
                "cannot observe remote Git capabilities".into(),
            ));
        }
        self.context
            .observer
            .emit(SourceEvent::NetworkRequestFinished {
                request_id,
                status: "success".into(),
                received_bytes: 0,
                validator_used: false,
            });
        let trace = String::from_utf8_lossy(&output.stderr);
        if trace.contains("object-format=sha256") {
            Ok(GitObjectFormat::Sha256)
        } else if trace.contains("object-format=sha1") {
            Ok(GitObjectFormat::Sha1)
        } else {
            Err(FetchError::Git(
                "remote did not advertise a supported Git object format".into(),
            ))
        }
    }
    fn require_type(&self, db: &Path, oid: &str, want: &str) -> Result<(), FetchError> {
        let got = self.git_text(db, &["cat-file", "-t", oid])?;
        if got.trim() != want {
            return Err(FetchError::Git(format!(
                "object is {}, expected {want}",
                got.trim()
            )));
        }
        Ok(())
    }
    fn git_text(&self, db: &Path, args: &[&str]) -> Result<String, FetchError> {
        String::from_utf8(self.run(db, args)?.stdout)
            .map_err(|_| FetchError::Git("Git produced non-UTF-8 plumbing output".into()))
    }
    fn run(&self, db: &Path, args: &[&str]) -> Result<Output, FetchError> {
        let out = Command::new("git")
            .arg(format!("--git-dir={}", db.display()))
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "Never")
            .output()?;
        if !out.status.success() {
            return Err(FetchError::Git(format!(
                "Git command failed with status {}",
                out.status.code().unwrap_or(-1)
            )));
        }
        Ok(out)
    }
    fn db_path(&self, repo: &str, format: GitObjectFormat) -> PathBuf {
        self.context
            .cache
            .join("v1/git/db")
            .join(hex::encode(Sha256::digest(repo)))
            .join(object_format_name(format))
    }
    fn observation_path(&self, repo: &str, selector: &GitSelector) -> PathBuf {
        let value = serde_json::to_vec(selector).unwrap_or_default();
        self.context
            .cache
            .join("v1/git/observations")
            .join(hex::encode(Sha256::digest(repo)))
            .join(format!("{}.json", hex::encode(Sha256::digest(value))))
    }
}

impl squish_resolver::GitPort for GitHost {
    fn resolve(
        &self,
        repository: &str,
        reference: &squish_project::GitReference,
        access: squish_resolver::Access,
    ) -> Result<squish_resolver::GitCandidate, squish_resolver::SourceUnavailable> {
        let selector = if let Some(v) = &reference.branch {
            GitSelector::Branch(v.clone())
        } else if let Some(v) = &reference.tag {
            GitSelector::Tag(v.clone())
        } else if let Some(v) = &reference.rev {
            GitSelector::Rev(v.clone())
        } else {
            GitSelector::Head
        };
        let access = if access == squish_resolver::Access::Online {
            Access::Online
        } else {
            Access::LocalOnly
        };
        self.resolve_exact(repository, &selector, ".", access)
            .map(|c| squish_resolver::GitCandidate {
                revision: c.commit.hex,
                checksum: format!("sha256:{}", c.content_digest.0.0),
                manifest: c.manifest,
            })
            .map_err(|e| squish_resolver::SourceUnavailable {
                identity: stable_repo(repository),
                detail: e.to_string(),
            })
    }
    fn contains(&self, repository: &str, revision: &str, checksum: &str) -> bool {
        let Some(d) = checksum.strip_prefix("sha256:") else {
            return false;
        };
        let Ok(digest) = Sha256Digest::parse(d.to_owned()) else {
            return false;
        };
        let format = match revision.len() {
            40 => GitObjectFormat::Sha1,
            64 => GitObjectFormat::Sha256,
            _ => return false,
        };
        let db = self.db_path(repository, format);
        self.require_type(&db, revision, "commit").is_ok()
            && self.materializer.contains_complete(&ContentDigest(digest))
    }
}

fn validate_repository(repo: &str) -> Result<(), FetchError> {
    let u = Url::parse(repo).map_err(|e| FetchError::Config(e.to_string()))?;
    if !matches!(u.scheme(), "https" | "ssh" | "file")
        || u.username() != ""
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(FetchError::Config(
            "Git URL must be credential-free absolute https/ssh/file URI".into(),
        ));
    }
    Ok(())
}
fn validate_selector(s: &GitSelector) -> Result<(), FetchError> {
    match s {
        GitSelector::Rev(v) => {
            if !matches!(v.len(), 40 | 64) || !v.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(FetchError::Config("rev must be a full Git OID".into()));
            }
        }
        GitSelector::Branch(v) | GitSelector::Tag(v) => {
            if v.is_empty()
                || v.starts_with('-')
                || v.starts_with('/')
                || v.ends_with('/')
                || v.contains("..")
                || v.contains("@{")
                || v.contains('\\')
                || v.split('/')
                    .any(|p| p.is_empty() || p == "." || p.ends_with('.') || p.ends_with(".lock"))
                || v.chars().any(char::is_control)
            {
                return Err(FetchError::Config("invalid Git ref name".into()));
            }
        }
        GitSelector::Head => {}
    }
    Ok(())
}
fn validate_subdir(v: &str) -> Result<(), FetchError> {
    if v == "." {
        return Ok(());
    }
    if v.is_empty()
        || v.starts_with('/')
        || v.contains('\\')
        || v.split('/').any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err(FetchError::Path(v.into()));
    }
    Ok(())
}
fn stable_repo(repo: &str) -> String {
    if let Ok(mut u) = Url::parse(repo) {
        let _ = u.set_username("");
        let _ = u.set_password(None);
        u.set_query(None);
        u.set_fragment(None);
        u.to_string()
    } else {
        "invalid-git-url".into()
    }
}

fn object_format_name(format: GitObjectFormat) -> &'static str {
    match format {
        GitObjectFormat::Sha1 => "sha1",
        GitObjectFormat::Sha256 => "sha256",
    }
}

fn write_sidecar(path: &Path, bytes: &[u8]) -> Result<(), FetchError> {
    let parent = path
        .parent()
        .ok_or_else(|| FetchError::Git("observation path has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("update.lock"))?;
    lock.lock_exclusive()?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut temporary, bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|e| FetchError::Io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn resolves_local_branch_without_a_checkout_filter() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!(
                "fetch-git-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
        let repo = root.join("repo");
        let cache = root.join("cache");
        fs::create_dir_all(&repo).unwrap();
        command(&repo, &["init"]);
        command(&repo, &["config", "user.email", "test@example.invalid"]);
        command(&repo, &["config", "user.name", "test"]);
        command(&repo, &["config", "core.autocrlf", "false"]);
        command(&repo, &["config", "filter.reject.clean", "cat"]);
        command(&repo, &["config", "filter.reject.smudge", "false"]);
        command(&repo, &["config", "filter.reject.required", "true"]);
        fs::write(
            repo.join("xmlsquish.toml"),
            "manifest-version = 1\n[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        fs::write(repo.join("input.xml"), b"a\r\nb\n").unwrap();
        fs::write(repo.join(".gitattributes"), b"*.xml filter=reject\n").unwrap();
        command(&repo, &["add", "."]);
        command(&repo, &["commit", "-m", "fixture"]);
        let branch = String::from_utf8(
            Command::new("git")
                .current_dir(&repo)
                .args(["branch", "--show-current"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();
        let context = HostContext::new(cache).unwrap();
        let host = GitHost::new(context);
        let url = Url::from_file_path(fs::canonicalize(&repo).unwrap())
            .unwrap()
            .to_string();
        let candidate = host
            .resolve_exact(&url, &GitSelector::Branch(branch), ".", Access::Online)
            .unwrap();
        assert_eq!(candidate.manifest.package.unwrap().name, "demo");
        assert_eq!(candidate.commit.hex.len(), 40);
        let expected = LogicalTree::build(
            vec![
                LogicalFile {
                    path: ".gitattributes".into(),
                    bytes: b"*.xml filter=reject\n".to_vec(),
                },
                LogicalFile {
                    path: "input.xml".into(),
                    bytes: b"a\r\nb\n".to_vec(),
                },
                LogicalFile {
                    path: "xmlsquish.toml".into(),
                    bytes: fs::read(repo.join("xmlsquish.toml")).unwrap(),
                },
            ],
            &crate::Limits::default(),
        )
        .unwrap();
        assert_eq!(candidate.content_digest, expected.content_digest);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sha256_repository_uses_a_sha256_bare_database_when_supported() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!(
                "fetch-git-sha256-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
        let repo = root.join("repo");
        fs::create_dir_all(&root).unwrap();
        let supported = Command::new("git")
            .current_dir(&root)
            .args(["init", "--object-format=sha256", "repo"])
            .status()
            .unwrap()
            .success();
        if !supported {
            let _ = fs::remove_dir_all(root);
            return;
        }
        command(&repo, &["config", "user.email", "test@example.invalid"]);
        command(&repo, &["config", "user.name", "test"]);
        fs::write(
            repo.join("xmlsquish.toml"),
            "manifest-version = 1\n[package]\nname = \"sha-demo\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        command(&repo, &["add", "."]);
        command(&repo, &["commit", "-m", "fixture"]);
        let branch = String::from_utf8(
            Command::new("git")
                .current_dir(&repo)
                .args(["branch", "--show-current"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();
        let context = HostContext::new(root.join("cache")).unwrap();
        let host = GitHost::new(context);
        let url = Url::from_file_path(fs::canonicalize(&repo).unwrap())
            .unwrap()
            .to_string();
        let candidate = host
            .resolve_exact(&url, &GitSelector::Branch(branch), ".", Access::Online)
            .unwrap();
        assert_eq!(candidate.commit.format, GitObjectFormat::Sha256);
        assert_eq!(candidate.commit.hex.len(), 64);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_branch_fetches_cannot_rebind_each_others_anchor() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!(
                "fetch-git-race-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        command(&repo, &["init"]);
        command(&repo, &["config", "user.email", "test@example.invalid"]);
        command(&repo, &["config", "user.name", "test"]);
        fs::write(
            repo.join("xmlsquish.toml"),
            "manifest-version = 1\n[package]\nname = \"race\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        command(&repo, &["add", "."]);
        command(&repo, &["commit", "-m", "a"]);
        command(&repo, &["branch", "branch-a"]);
        fs::write(repo.join("extra"), b"b").unwrap();
        command(&repo, &["add", "."]);
        command(&repo, &["commit", "-m", "b"]);
        command(&repo, &["branch", "branch-b"]);
        let oid_a = git_stdout(&repo, &["rev-parse", "branch-a"]);
        let oid_b = git_stdout(&repo, &["rev-parse", "branch-b"]);
        let host = GitHost::new(HostContext::new(root.join("cache")).unwrap());
        let url = Url::from_file_path(fs::canonicalize(&repo).unwrap())
            .unwrap()
            .to_string();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = [("branch-a", oid_a), ("branch-b", oid_b)]
            .into_iter()
            .map(|(branch, expected)| {
                let host = host.clone();
                let url = url.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let got = host
                        .resolve_exact(
                            &url,
                            &GitSelector::Branch(branch.into()),
                            ".",
                            Access::Online,
                        )
                        .unwrap()
                        .commit
                        .hex;
                    assert_eq!(got, expected);
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }

    fn command(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    fn git_stdout(repo: &Path, args: &[&str]) -> String {
        String::from_utf8(
            Command::new("git")
                .current_dir(repo)
                .args(args)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned()
    }
}
