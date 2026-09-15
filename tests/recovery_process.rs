//! 真实子进程死亡后的自动恢复验收。 / Automatic-recovery acceptance after real child-process death.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const SELECTOR: &str = "XMLSQUISH_TEST_PROCESS_EXIT_AT";
const CRASH_STATUS: i32 = 86;

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xmlsquish"));
    command.env_remove(SELECTOR);
    command
}

struct Fixture {
    root: tempfile::TempDir,
    home: tempfile::TempDir,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp");
        fs::create_dir_all(&scratch).unwrap();
        let root = tempfile::Builder::new()
            .prefix(&format!("recovery-{name}-project-"))
            .tempdir_in(&scratch)
            .unwrap();
        let home = tempfile::Builder::new()
            .prefix(&format!("recovery-{name}-home-"))
            .tempdir_in(&scratch)
            .unwrap();
        fs::create_dir_all(root.path().join("src")).unwrap();
        fs::write(
            root.path().join("xmlsquish.toml"),
            "manifest-version = 1\n[package]\nname = \"fixture\"\nversion = \"1.0.0\"\n\n[target.chat]\nentry = \"src/main.xml\"\n",
        )
        .unwrap();
        fs::write(root.path().join("src/main.xml"), source("A")).unwrap();
        Self { root, home }
    }

    fn command(&self) -> Command {
        let mut command = binary();
        command
            .current_dir(self.root.path())
            .env("XMLSQUISH_HOME", self.home.path());
        command
    }

    fn build(&self) -> Output {
        self.command().args(["build", "--plain"]).output().unwrap()
    }

    fn crash(&self, selector: &str, args: &[&str]) -> Output {
        self.command()
            .env(SELECTOR, selector)
            .args(args)
            .output()
            .unwrap()
    }

    fn inspect_prompt(&self) -> serde_json::Value {
        let output = self
            .command()
            .args([
                "inspect",
                "artifact",
                "target/xmlsquish/chat.prompt",
                "--format=json",
            ])
            .output()
            .unwrap();
        assert_success(&output);
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn prompt_digest(&self) -> String {
        let document = self.inspect_prompt();
        let hex = document["value"]["artifact"]["digest"]["hex"]
            .as_str()
            .expect("artifact digest hex is public JSON");
        format!("blake3:{hex}")
    }

    fn write_source(&self, value: &str) {
        fs::write(self.root.path().join("src/main.xml"), source(value)).unwrap();
    }

    fn write_invalid_source(&self) {
        fs::write(self.root.path().join("src/main.xml"), b"<not-xml").unwrap();
    }

    fn repository_transactions(&self) -> PathBuf {
        self.root.path().join(".xmlsquish/transactions")
    }

    fn artifact_journal(&self) -> PathBuf {
        self.root
            .path()
            .join("target/xmlsquish/.squish-publish/generation-journal.json")
    }

    fn catalog_journal(&self) -> PathBuf {
        let catalog = self.home.path().join("state/catalog/projects");
        let namespace = fs::read_dir(catalog)
            .unwrap()
            .next()
            .expect("project namespace exists")
            .unwrap()
            .path();
        namespace.join(".squish-publish/generation-journal.json")
    }
}

fn source(value: &str) -> String {
    format!(
        "<xs:entry xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><message>{value}</message></xs:entry>"
    )
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_crashed(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(CRASH_STATUS),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_domain_failure(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_transactions_empty(path: &Path) {
    if !path.exists() {
        return;
    }
    assert!(
        fs::read_dir(path).unwrap().next().is_none(),
        "repository recovery left transaction state in {}",
        path.display()
    );
}

/// Workspace fixture for real process-death tests of `new` publication.
/// 用于 `new` 发布真实进程死亡测试的工作区夹具。
struct NewRecoveryFixture {
    root: tempfile::TempDir,
    home: tempfile::TempDir,
    destination: PathBuf,
}

impl NewRecoveryFixture {
    fn new(name: &str) -> Self {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp");
        fs::create_dir_all(&scratch).unwrap();
        let root = tempfile::Builder::new()
            .prefix(&format!("recovery-new-{name}-workspace-"))
            .tempdir_in(&scratch)
            .unwrap();
        let home = tempfile::Builder::new()
            .prefix(&format!("recovery-new-{name}-home-"))
            .tempdir_in(&scratch)
            .unwrap();
        fs::write(
            root.path().join("xmlsquish.toml"),
            "manifest-version = 1\n[workspace]\nmembers = []\n",
        )
        .unwrap();
        let destination = root.path().join("packages/new-member");
        Self {
            root,
            home,
            destination,
        }
    }

    fn command(&self) -> Command {
        let mut command = binary();
        command
            .current_dir(self.root.path())
            .env("XMLSQUISH_HOME", self.home.path());
        command
    }

    fn crash_new(&self, selector: &str) -> Output {
        self.command()
            .env(SELECTOR, selector)
            .arg("new")
            .arg(&self.destination)
            .args(["--vcs=none", "--quiet"])
            .output()
            .unwrap()
    }

    fn retry_new(&self) -> Output {
        self.command()
            .arg("new")
            .arg(&self.destination)
            .args(["--vcs=none", "--quiet"])
            .output()
            .unwrap()
    }

    fn recover_through_fmt(&self) -> Output {
        self.command()
            .args(["fmt", "--manifest-path"])
            .arg(self.root.path().join("xmlsquish.toml"))
            .args(["--workspace", "--check", "--plain"])
            .output()
            .unwrap()
    }

    fn assert_complete(&self) {
        assert!(self.destination.join("xmlsquish.toml").is_file());
        assert!(self.destination.join("src/prompt.xml").is_file());
        let workspace = fs::read_to_string(self.root.path().join("xmlsquish.toml")).unwrap();
        assert_eq!(workspace.matches("packages/new-member").count(), 1);
        let stale = walk_named(self.root.path(), |name| {
            name.starts_with(".xmlsquish-new-stage-") || name.starts_with(".xmlsquish-published-")
        });
        assert!(stale.is_empty(), "stale creation state: {stale:?}");
    }
}

fn walk_named(root: &Path, predicate: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    fn visit(root: &Path, predicate: &dyn Fn(&str) -> bool, found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if predicate(&entry.file_name().to_string_lossy()) {
                found.push(path.clone());
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                visit(&path, predicate, found);
            }
        }
    }
    let mut found = Vec::new();
    visit(root, &predicate, &mut found);
    found
}

#[test]
fn new_creation_prepared_death_rolls_back_then_retry_creates_once() {
    let fixture = NewRecoveryFixture::new("prepared");
    assert_crashed(&fixture.crash_new("repository.creation-prepared"));
    assert!(!fixture.destination.exists());
    assert_success(&fixture.retry_new());
    fixture.assert_complete();
}

#[test]
fn new_postpublication_deaths_roll_forward_via_ordinary_project_discovery() {
    for selector in [
        "repository.creation-published",
        "repository.creation-workspace-replaced",
        "repository.creation-completed",
    ] {
        let fixture = NewRecoveryFixture::new(selector.rsplit('-').next().unwrap());
        assert_crashed(&fixture.crash_new(selector));
        assert!(
            fixture.destination.join("xmlsquish.toml").is_file(),
            "{selector} must be post-publication"
        );
        assert_success(&fixture.recover_through_fmt());
        fixture.assert_complete();
    }
}

#[test]
fn unknown_selector_is_a_deterministic_configuration_failure() {
    let fixture = Fixture::new("invalid-selector");
    let output = fixture.crash("unknown.scope", &["build", "--message-format=json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("TEST_FAULT001"));
    assert!(stdout.contains("fault-injection"));
    assert!(!fixture.root.path().join("target/xmlsquish").exists());
}

#[test]
fn artifact_generation_before_commit_keeps_previous_good_generation() {
    let fixture = Fixture::new("artifact-predecision");
    assert_success(&fixture.build());
    let before = fixture.prompt_digest();
    fixture.write_source("B");

    let crashed = fixture.crash(
        "artifact-generation.generation-staged",
        &["build", "--plain"],
    );
    assert_crashed(&crashed);
    assert!(!fixture.artifact_journal().exists());
    assert_eq!(fixture.prompt_digest(), before);
}

#[test]
fn artifact_generation_commit_decision_rolls_forward_on_plain_invocation() {
    let fixture = Fixture::new("artifact-postdecision");
    assert_success(&fixture.build());
    let before = fixture.prompt_digest();
    fixture.write_source("B");

    let crashed = fixture.crash("artifact-generation.commit-decision", &["build", "--plain"]);
    assert_crashed(&crashed);
    assert!(fixture.artifact_journal().exists());

    // A different, invalid source prevents the recovery command from recreating generation B.
    // 不同的非法源码使恢复命令无法重算 generation B，避免掩盖恢复缺陷。
    fixture.write_invalid_source();
    assert_domain_failure(&fixture.build());
    assert!(!fixture.artifact_journal().exists());
    assert_ne!(fixture.prompt_digest(), before);
}

#[test]
fn build_catalog_commit_decision_rolls_forward_on_plain_invocation() {
    let fixture = Fixture::new("catalog-postdecision");
    assert_success(&fixture.build());
    let before = fixture.prompt_digest();
    fixture.write_source("B");

    let crashed = fixture.crash("build-catalog.commit-decision", &["build", "--plain"]);
    assert_crashed(&crashed);
    assert!(fixture.catalog_journal().exists());

    fixture.write_invalid_source();
    assert_domain_failure(&fixture.build());
    assert!(!fixture.catalog_journal().exists());
    assert_ne!(fixture.prompt_digest(), before);
}

#[test]
fn repository_candidates_without_commit_decision_are_discarded() {
    let fixture = Fixture::new("repository-predecision");
    assert_success(&fixture.build());
    let dependency = fixture.root.path().join("dep");
    fs::create_dir_all(&dependency).unwrap();
    fs::write(
        dependency.join("xmlsquish.toml"),
        "manifest-version = 1\n[package]\nname = \"dep\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let manifest = fs::read(fixture.root.path().join("xmlsquish.toml")).unwrap();
    let lock_path = fixture.root.path().join("xmlsquish.lock");
    let lock = fs::read(&lock_path).expect("baseline build writes an exact lock");

    let crashed = fixture.crash(
        "repository.candidates-staged",
        &["add", "dep", "--path", "dep", "--plain"],
    );
    assert_crashed(&crashed);
    assert_success(&fixture.build());
    assert_eq!(
        fs::read(fixture.root.path().join("xmlsquish.toml")).unwrap(),
        manifest
    );
    assert_eq!(fs::read(lock_path).unwrap(), lock);
    assert_transactions_empty(&fixture.repository_transactions());
}

#[test]
fn repository_commit_decision_rolls_manifest_and_lock_forward_together() {
    let fixture = Fixture::new("repository-postdecision");
    let dependency = fixture.root.path().join("dep");
    fs::create_dir_all(&dependency).unwrap();
    fs::write(
        dependency.join("xmlsquish.toml"),
        "manifest-version = 1\n[package]\nname = \"dep\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();

    let crashed = fixture.crash(
        "repository.commit-decided",
        &["add", "dep", "--path", "dep", "--plain"],
    );
    assert_crashed(&crashed);
    fixture.write_invalid_source();
    assert_domain_failure(&fixture.build());
    let manifest = fs::read_to_string(fixture.root.path().join("xmlsquish.toml")).unwrap();
    let lock = fs::read_to_string(fixture.root.path().join("xmlsquish.lock")).unwrap();
    assert!(manifest.contains("[dependencies]"));
    assert!(manifest.contains("dep"));
    assert!(lock.contains("dep"));
    assert_transactions_empty(&fixture.repository_transactions());
}
