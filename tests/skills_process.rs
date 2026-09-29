//! Process-boundary contracts for workspace-managed Agent Skills.
//!
//! All fixtures live under the repository's `.temp` directory. Assertions are
//! intentionally based on the public manifest, lock, and projected files, not
//! on manager-internal cache or journal layout.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Starts the release binary built by Cargo's integration-test harness.
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xmlsquish"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("start xmlsquish")
}

/// Keeps project fixtures on the workspace volume and within the repository.
fn project(label: &str) -> tempfile::TempDir {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp/skills-tests");
    fs::create_dir_all(&scratch).unwrap();
    let root = tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(scratch)
        .unwrap();
    fs::create_dir_all(root.path().join("src")).unwrap();
    fs::write(root.path().join("xmlsquish.toml"),
        "manifest-version = 1\n[package]\nname = \"fixture\"\nversion = \"1.0.0\"\n\n[target.chat]\nentry = \"src/main.xml\"\n").unwrap();
    fs::write(root.path().join("src/main.xml"),
        "<xs:entry xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><message>Hello</message></xs:entry>").unwrap();
    root
}

/// Writes a complete skill, including an asset that must survive projection.
fn skill(root: &Path, name: &str, body: &str) -> PathBuf {
    let dir = root.join("source").join(name);
    fs::create_dir_all(dir.join("references")).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Test skill\n---\n{body}\n"),
    )
    .unwrap();
    fs::write(dir.join("references/example.txt"), b"asset bytes\n").unwrap();
    dir
}

/// Gives the CLI result and its diagnostics when an expected success fails.
fn success(output: Output) {
    assert!(
        output.status.success(),
        "exit={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Checks a domain or usage failure without prescribing an unstable error code.
fn failure(output: Output) {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// The locked digest is external evidence of what was selected for a skill.
fn skill_digest(lock: &str) -> String {
    let section = lock.split("[[skill]]").nth(1).expect("locked skill record");
    section
        .lines()
        .find_map(|line| line.trim().strip_prefix("digest = "))
        .expect("skill digest")
        .trim_matches('"')
        .to_owned()
}

#[test]
fn local_add_dry_run_install_sync_and_remove_preserve_assets_and_do_not_run_scripts() {
    let root = project("lifecycle-");
    let source = skill(root.path(), "review-checks", "Instructions");
    fs::create_dir_all(source.join("scripts")).unwrap();
    let sentinel = root.path().join("script-ran");
    fs::write(
        source.join("scripts/install.sh"),
        format!("#!/bin/sh\ntouch '{}'\n", sentinel.display()),
    )
    .unwrap();
    let manifest = root.path().join("xmlsquish.toml");
    let before = fs::read(&manifest).unwrap();

    success(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--path",
            "source/review-checks",
            "--dry-run",
            "--plain",
        ],
    ));
    assert_eq!(
        fs::read(&manifest).unwrap(),
        before,
        "dry run edited manifest"
    );
    assert!(
        !root.path().join("xmlsquish.lock").exists(),
        "dry run wrote lock"
    );
    assert!(
        !root.path().join(".agents/skills/review-checks").exists(),
        "dry run projected skill"
    );

    success(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--path",
            "source/review-checks",
            "--plain",
        ],
    ));
    let installed = root.path().join(".agents/skills/review-checks");
    assert_eq!(
        fs::read(installed.join("references/example.txt")).unwrap(),
        b"asset bytes\n"
    );
    assert!(installed.join(".xmlsquish-managed").is_file());
    assert!(!sentinel.exists(), "skill script ran during installation");
    let lock_before = fs::read(root.path().join("xmlsquish.lock")).unwrap();
    let manifest_before = fs::read(&manifest).unwrap();

    success(cli(root.path(), &["sync-skills", "--frozen", "--plain"]));
    assert_eq!(fs::read(&manifest).unwrap(), manifest_before);
    assert_eq!(
        fs::read(root.path().join("xmlsquish.lock")).unwrap(),
        lock_before
    );
    assert!(!sentinel.exists(), "skill script ran during sync");

    success(cli(
        root.path(),
        &["remove-skill", "review-checks", "--dry-run", "--plain"],
    ));
    assert!(installed.exists(), "dry-run removed projection");
    success(cli(
        root.path(),
        &["remove-skill", "review-checks", "--plain"],
    ));
    assert!(!installed.exists(), "remove retained owned projection");
    assert!(source.exists(), "remove deleted source");
    assert!(!sentinel.exists(), "skill script ran during removal");
}

#[test]
fn sync_prunes_stale_managed_projection_after_branch_switch_only() {
    let root = project("branch-switch-");
    // A build produces the old branch's legitimate, skill-free lockfile. Restoring both
    // authoritative files models a checkout rather than inventing an invalid lock.
    success(cli(root.path(), &["build", "--plain"]));
    let old_manifest = fs::read(root.path().join("xmlsquish.toml")).unwrap();
    let old_lock = fs::read(root.path().join("xmlsquish.lock")).unwrap();

    skill(root.path(), "review-checks", "Instructions");
    success(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--path",
            "source/review-checks",
            "--plain",
        ],
    ));
    let projected = root.path().join(".agents/skills/review-checks");
    assert!(projected.join(".xmlsquish-managed").is_file());

    // These siblings have distinct owners: one manual, one the bundled self-install.
    let manual = root.path().join(".agents/skills/manual-helper");
    fs::create_dir_all(&manual).unwrap();
    fs::write(manual.join("SKILL.md"), "manual user content").unwrap();
    success(cli(root.path(), &["install-skill", "--project", "--plain"]));
    let bundled = root.path().join(".agents/skills/prompt-squish");
    let bundled_bytes = fs::read(bundled.join("SKILL.md")).unwrap();

    fs::write(root.path().join("xmlsquish.toml"), &old_manifest).unwrap();
    fs::write(root.path().join("xmlsquish.lock"), &old_lock).unwrap();
    success(cli(root.path(), &["sync-skills", "--frozen", "--plain"]));

    assert!(
        !projected.exists(),
        "stale dependency-owned projection survived sync"
    );
    assert_eq!(
        fs::read(manual.join("SKILL.md")).unwrap(),
        b"manual user content"
    );
    assert_eq!(fs::read(bundled.join("SKILL.md")).unwrap(), bundled_bytes);
    assert_eq!(
        fs::read(root.path().join("xmlsquish.toml")).unwrap(),
        old_manifest
    );
    assert_eq!(
        fs::read(root.path().join("xmlsquish.lock")).unwrap(),
        old_lock
    );
}

#[test]
fn collision_and_user_edit_refuse_destructive_mutation() {
    let root = project("collision-");
    skill(root.path(), "review-checks", "Instructions");
    let installed = root.path().join(".agents/skills/review-checks");
    fs::create_dir_all(&installed).unwrap();
    fs::write(installed.join("note.txt"), "user owned").unwrap();
    failure(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--path",
            "source/review-checks",
            "--plain",
        ],
    ));
    assert_eq!(
        fs::read_to_string(installed.join("note.txt")).unwrap(),
        "user owned"
    );
    assert!(
        !fs::read_to_string(root.path().join("xmlsquish.toml"))
            .unwrap()
            .contains("[skills]")
    );

    fs::remove_dir_all(&installed).unwrap();
    success(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--path",
            "source/review-checks",
            "--plain",
        ],
    ));
    let lock_before = fs::read(root.path().join("xmlsquish.lock")).unwrap();
    fs::write(installed.join("references/example.txt"), "user edit").unwrap();
    failure(cli(
        root.path(),
        &["remove-skill", "review-checks", "--plain"],
    ));
    assert_eq!(
        fs::read_to_string(installed.join("references/example.txt")).unwrap(),
        "user edit"
    );
    assert_eq!(
        fs::read(root.path().join("xmlsquish.lock")).unwrap(),
        lock_before,
        "failed preflight committed lock mutation"
    );
}

#[test]
fn invalid_frontmatter_and_skill_name_mismatch_do_not_commit() {
    let root = project("frontmatter-");
    let source = skill(root.path(), "review-checks", "Instructions");
    fs::write(
        source.join("SKILL.md"),
        "---\nname: another-name\ndescription: Test\n---\n",
    )
    .unwrap();
    failure(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--path",
            "source/review-checks",
            "--plain",
        ],
    ));
    assert!(!root.path().join("xmlsquish.lock").exists());
    assert!(!root.path().join(".agents/skills/review-checks").exists());
    fs::write(source.join("SKILL.md"), "---\nname: review-checks\n---\n").unwrap();
    failure(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--path",
            "source/review-checks",
            "--plain",
        ],
    ));
    assert!(!root.path().join("xmlsquish.lock").exists());
}

#[test]
fn package_add_does_not_discard_existing_skill_lock() {
    let root = project("package-preservation-");
    skill(root.path(), "review-checks", "Instructions");
    success(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--path",
            "source/review-checks",
            "--plain",
        ],
    ));
    let original = skill_digest(&fs::read_to_string(root.path().join("xmlsquish.lock")).unwrap());
    let dependency = root.path().join("dep");
    fs::create_dir_all(&dependency).unwrap();
    fs::write(
        dependency.join("xmlsquish.toml"),
        "manifest-version = 1\n[package]\nname = \"dep\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    success(cli(
        root.path(),
        &["add", "dep", "--path", "dep", "--plain"],
    ));
    let lock = fs::read_to_string(root.path().join("xmlsquish.lock")).unwrap();
    assert_eq!(
        skill_digest(&lock),
        original,
        "package mutation discarded or changed skill pin"
    );
    success(cli(root.path(), &["remove", "dep", "--plain"]));
    let lock = fs::read_to_string(root.path().join("xmlsquish.lock")).unwrap();
    assert_eq!(
        skill_digest(&lock),
        original,
        "package removal discarded skill pin"
    );
}

#[test]
fn bundled_project_install_is_independent_idempotent_and_refuses_manual_collision() {
    let root = project("bundled-");
    let manifest = fs::read(root.path().join("xmlsquish.toml")).unwrap();
    success(cli(
        root.path(),
        &["install-skill", "--project", "--dry-run", "--plain"],
    ));
    let installed = root.path().join(".agents/skills/prompt-squish");
    assert!(!installed.exists());
    assert!(!root.path().join("xmlsquish.lock").exists());
    fs::create_dir_all(&installed).unwrap();
    fs::write(installed.join("SKILL.md"), "manual").unwrap();
    failure(cli(
        root.path(),
        &["install-skill", "--project", "--force", "--plain"],
    ));
    assert_eq!(
        fs::read_to_string(installed.join("SKILL.md")).unwrap(),
        "manual"
    );
    fs::remove_dir_all(&installed).unwrap();
    success(cli(root.path(), &["install-skill", "--project", "--plain"]));
    let first = fs::read(installed.join("SKILL.md")).unwrap();
    assert!(String::from_utf8_lossy(&first).contains("name: prompt-squish"));
    success(cli(root.path(), &["install-skill", "--project", "--plain"]));
    assert_eq!(fs::read(installed.join("SKILL.md")).unwrap(), first);
    assert_eq!(
        fs::read(root.path().join("xmlsquish.toml")).unwrap(),
        manifest
    );
    assert!(!root.path().join("xmlsquish.lock").exists());
}

#[test]
fn bundled_global_install_needs_no_project_and_protects_user_content() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp/skills-tests");
    fs::create_dir_all(&scratch).unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("global-install-")
        .tempdir_in(&scratch)
        .unwrap();
    let home = fixture.path().join("home");
    let unrelated_cwd = fixture.path().join("unrelated-cwd");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&unrelated_cwd).unwrap();
    assert!(!unrelated_cwd.join("xmlsquish.toml").exists());
    let installed = home.join(".agents/skills/prompt-squish");
    let invoke = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_xmlsquish"))
            .current_dir(&unrelated_cwd)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .args(args)
            .output()
            .expect("start global install-skill")
    };

    fs::create_dir_all(&installed).unwrap();
    fs::write(installed.join("SKILL.md"), "manual user content").unwrap();
    failure(invoke(&["install-skill", "--force", "--plain"]));
    assert_eq!(
        fs::read(installed.join("SKILL.md")).unwrap(),
        b"manual user content"
    );

    fs::remove_dir_all(&installed).unwrap();
    success(invoke(&["install-skill", "--plain"]));
    let original = fs::read(installed.join("SKILL.md")).unwrap();
    assert!(String::from_utf8_lossy(&original).contains("name: prompt-squish"));
    assert!(installed.join(".xmlsquish-managed").is_file());
    success(invoke(&["install-skill", "--plain"]));
    assert_eq!(fs::read(installed.join("SKILL.md")).unwrap(), original);

    fs::write(installed.join("SKILL.md"), "edited owned content").unwrap();
    failure(invoke(&["install-skill", "--force", "--plain"]));
    assert_eq!(
        fs::read(installed.join("SKILL.md")).unwrap(),
        b"edited owned content"
    );
    assert!(!unrelated_cwd.join("xmlsquish.toml").exists());
    assert!(!unrelated_cwd.join("xmlsquish.lock").exists());
}

#[test]
fn frozen_rejects_mutated_local_source_without_changing_pin_or_projection() {
    let root = project("drift-");
    let source = skill(root.path(), "review-checks", "First");
    success(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--path",
            "source/review-checks",
            "--plain",
        ],
    ));
    let lock = fs::read(root.path().join("xmlsquish.lock")).unwrap();
    let installed = root.path().join(".agents/skills/review-checks/SKILL.md");
    let original = fs::read(&installed).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: review-checks\ndescription: Test skill\n---\nSecond\n",
    )
    .unwrap();
    failure(cli(root.path(), &["sync-skills", "--frozen", "--plain"]));
    assert_eq!(fs::read(root.path().join("xmlsquish.lock")).unwrap(), lock);
    assert_eq!(fs::read(installed).unwrap(), original);
}

/// Runs Git without inheriting machine-specific identity configuration.
fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("Git must be installed for Git-source integration tests");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn git_tag_is_pinned_to_exact_commit_when_ref_moves() {
    let root = project("git-pin-");
    let repo = root.path().join("skill-repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    skill(&repo, "review-checks", "First revision");
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Skill Test",
            "-c",
            "user.email=skill@example.invalid",
            "commit",
            "-qm",
            "first",
        ],
    );
    let first_commit = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["tag", "chosen"]);
    let url = url::Url::from_file_path(&repo)
        .expect("repository path has file URL")
        .to_string();
    success(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--git",
            &url,
            "--tag",
            "chosen",
            "--subdir",
            "source/review-checks",
            "--plain",
        ],
    ));
    let lock_before = fs::read(root.path().join("xmlsquish.lock")).unwrap();
    let lock_text = String::from_utf8_lossy(&lock_before);
    assert!(
        lock_text.contains(&first_commit),
        "lock did not pin commit: {lock_text}"
    );
    let installed = root.path().join(".agents/skills/review-checks/SKILL.md");
    let first_bytes = fs::read(&installed).unwrap();

    fs::write(
        repo.join("source/review-checks/SKILL.md"),
        "---\nname: review-checks\ndescription: Test skill\n---\nSecond revision\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Skill Test",
            "-c",
            "user.email=skill@example.invalid",
            "commit",
            "-qm",
            "second",
        ],
    );
    git(&repo, &["tag", "-f", "chosen"]);
    success(cli(root.path(), &["sync-skills", "--frozen", "--plain"]));
    assert_eq!(
        fs::read(root.path().join("xmlsquish.lock")).unwrap(),
        lock_before
    );
    assert_eq!(fs::read(installed).unwrap(), first_bytes);
}

#[test]
fn git_subdir_traversal_is_rejected_before_authoritative_writes() {
    let root = project("git-traversal-");
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    skill(&repo, "review-checks", "Instructions");
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Skill Test",
            "-c",
            "user.email=skill@example.invalid",
            "commit",
            "-qm",
            "first",
        ],
    );
    let url = url::Url::from_file_path(&repo).unwrap().to_string();
    failure(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--git",
            &url,
            "--subdir",
            "../source/review-checks",
            "--plain",
        ],
    ));
    assert!(!root.path().join("xmlsquish.lock").exists());
    assert!(!root.path().join(".agents/skills/review-checks").exists());
}

/// Creates a directory alias to exercise real filesystem traversal rules.
#[cfg(unix)]
fn directory_alias(target: &Path, alias: &Path) {
    std::os::unix::fs::symlink(target, alias).unwrap();
}

/// Junctions require no developer-mode symbolic-link privilege on Windows.
#[cfg(windows)]
fn directory_alias(target: &Path, alias: &Path) {
    let quoted = |path: &Path| path.to_string_lossy().replace('\'', "''");
    let command = format!(
        "New-Item -ItemType Junction -Path '{}' -Target '{}' | Out-Null",
        quoted(alias),
        quoted(target)
    );
    let output = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(command)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "New-Item -ItemType Junction: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn local_source_directory_alias_is_not_followed() {
    let root = project("source-alias-");
    let source = skill(root.path(), "review-checks", "Instructions");
    let alias = root.path().join("aliased-review-checks");
    directory_alias(&source, &alias);
    failure(cli(
        root.path(),
        &[
            "add-skill",
            "review-checks",
            "--path",
            "aliased-review-checks",
            "--plain",
        ],
    ));
    assert!(!root.path().join("xmlsquish.lock").exists());
    assert!(!root.path().join(".agents/skills/review-checks").exists());
}
