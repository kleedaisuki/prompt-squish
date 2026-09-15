//! 根二进制的进程边界合同。 / Process-boundary contracts for the root binary.

use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xmlsquish"))
}

fn project(name: &str) -> tempfile::TempDir {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp");
    fs::create_dir_all(&scratch).unwrap();
    let project = tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(scratch)
        .unwrap();
    fs::create_dir_all(project.path().join("src")).unwrap();
    fs::create_dir_all(project.path().join(".xmlsquish")).unwrap();
    fs::write(
        project.path().join(".xmlsquish/config.toml"),
        "[source]\ncache-root = \"cache/sources\"\n[manager]\nstorage-root = \"cache/state\"\n",
    )
    .unwrap();
    fs::write(
        project.path().join("xmlsquish.toml"),
        r#"manifest-version = 1
[package]
name = "fixture"
version = "1.0.0"

[target.chat]
entry = "src/main.xml"
"#,
    )
    .unwrap();
    fs::write(
        project.path().join("src/main.xml"),
        r#"<xs:entry  xmlns:xs = 'https://xmlsquish.moesegfault.dev/ns' ><message>Hello world</message></xs:entry >"#,
    )
    .unwrap();
    project
}

/// 创建不覆盖全局存储的项目。 / Creates a project that does not override global storage.
fn globally_stored_project(
    name: &str,
    package: &str,
    target: &str,
    message: &str,
) -> tempfile::TempDir {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp");
    fs::create_dir_all(&scratch).unwrap();
    let project = tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(scratch)
        .unwrap();
    fs::create_dir_all(project.path().join("src")).unwrap();
    fs::write(
        project.path().join("xmlsquish.toml"),
        format!(
            "manifest-version = 1\n[package]\nname = \"{package}\"\nversion = \"1.0.0\"\n\n[target.{target}]\nentry = \"src/main.xml\"\n"
        ),
    )
    .unwrap();
    fs::write(
        project.path().join("src/main.xml"),
        format!(
            "<xs:entry xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><message>{message}</message></xs:entry>"
        ),
    )
    .unwrap();
    project
}

#[test]
fn bare_help_is_stdout_success_and_lists_only_direct_commands() {
    let output = binary().output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    for command in ["build", "fmt", "add", "remove", "inspect"] {
        assert!(stdout.contains(command), "missing direct command {command}");
    }
}

#[test]
fn version_reports_the_installed_root_package_version() {
    let output = binary().arg("--version").output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("xmlsquish {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn parse_failure_is_stderr_with_usage_exit_status() {
    let output = binary().arg("legacy.xml").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn json_parse_failures_are_diagnostic_plus_one_terminal_record() {
    for args in [
        vec!["legacy.xml", "--message-format=json"],
        vec!["build", "--emit=debug", "--message-format=json"],
    ] {
        let output = binary().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.is_empty());
        let lines: Vec<_> = output
            .stdout
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(lines.len(), 2);
        let value: serde_json::Value = serde_json::from_slice(lines[0]).unwrap();
        let terminal: serde_json::Value = serde_json::from_slice(lines[1]).unwrap();
        assert_eq!(value["schema"], "xmlsquish-bootstrap-v1");
        assert_eq!(value["kind"], "diagnostic");
        assert_eq!(value["phase"], "parse");
        assert_eq!(value["exit_code"], 2);
        assert_eq!(terminal["kind"], "finished");
        assert_eq!(terminal["status"], "failed");
        assert_eq!(terminal["exit_code"], 2);
        assert!(terminal.get("invocation").is_none());
    }
}

#[test]
fn invalid_registry_environment_credential_is_redacted_in_machine_output() {
    let project = project("invalid-registry-credential");
    fs::write(
        project.path().join(".xmlsquish/config.toml"),
        "[source]\ncache-root = 'cache/sources'\n[manager]\nstorage-root = 'cache/state'\n\
         [registries.corp]\nid = 'https://registry.example/v1'\nindex = 'sparse+https://index.example/'\nauth-scope = 'corp-read'\n",
    )
    .unwrap();
    let secret = "Bearer distinctive-secret\nInjected: yes";
    for args in [
        vec!["build", "--message-format=json"],
        vec!["build", "--message-format=json", "--verbose"],
        vec!["build", "--message-format=json", "--verbose", "--verbose"],
        vec!["build"],
    ] {
        let output = binary()
            .current_dir(project.path())
            .args(&args)
            .env("XMLSQUISH_REGISTRY_CORP_READ_AUTHORIZATION", secret)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(combined.contains("XMLSQUISH_REGISTRY_CORP_READ_AUTHORIZATION"));
        assert!(!combined.contains("distinctive-secret"));
        assert!(!combined.contains("Injected"));
        if args.contains(&"--message-format=json") {
            assert!(output.stderr.is_empty());
            assert!(String::from_utf8_lossy(&output.stdout).contains("xmlsquish-bootstrap-v1"));
        } else {
            assert!(output.stdout.is_empty());
        }
    }
}

#[test]
fn failed_project_discovery_is_a_domain_failure() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp/no-project");
    fs::create_dir_all(&scratch).unwrap();
    let output = binary().current_dir(scratch).arg("build").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("PROJECT001"));
}

#[test]
fn json_pre_dispatch_failures_are_diagnostic_plus_terminal_on_stdout() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp/no-json-project");
    fs::create_dir_all(&scratch).unwrap();
    let discovery = binary()
        .current_dir(&scratch)
        .args(["build", "--message-format=json"])
        .output()
        .unwrap();
    assert_bootstrap_failure(discovery, "discover");

    let invalid_config = project("root-invalid-config-");
    fs::write(
        invalid_config.path().join(".xmlsquish/config.toml"),
        "unknown-key = true\n",
    )
    .unwrap();
    let config = binary()
        .current_dir(invalid_config.path())
        .args(["build", "--message-format=json"])
        .output()
        .unwrap();
    assert_bootstrap_failure(config, "config");

    let invalid_host = project("root-invalid-host-");
    fs::write(
        invalid_host.path().join(".xmlsquish/config.toml"),
        "[source]\ncache-root = \"shared\"\n[manager]\nstorage-root = \"shared\"\n",
    )
    .unwrap();
    let host = binary()
        .current_dir(invalid_host.path())
        .args(["build", "--message-format=json"])
        .output()
        .unwrap();
    assert_bootstrap_failure(host, "host");
}

fn assert_bootstrap_failure(output: std::process::Output, phase: &str) {
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let lines: Vec<_> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(
        lines.len(),
        2,
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let diagnostic: serde_json::Value = serde_json::from_slice(lines[0]).unwrap();
    let terminal: serde_json::Value = serde_json::from_slice(lines[1]).unwrap();
    assert_eq!(diagnostic["kind"], "diagnostic");
    assert_eq!(diagnostic["phase"], phase);
    assert_eq!(terminal["kind"], "finished");
    assert_eq!(terminal["status"], "failed");
    assert!(diagnostic.get("invocation").is_none());
}

#[test]
fn failed_planning_uses_domain_exit_and_stderr_only() {
    let project = project("root-plan-failure-");
    let output = binary()
        .current_dir(project.path())
        .args(["build", "--target", "missing", "--plain"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("error["));
}

#[test]
fn format_check_and_write_preserve_stream_contract() {
    let project = project("root-fmt-");
    let original = fs::read(project.path().join("src/main.xml")).unwrap();
    let checked = binary()
        .current_dir(project.path())
        .args(["fmt", "--check", "--plain"])
        .output()
        .unwrap();
    assert_eq!(checked.status.code(), Some(1));
    assert!(checked.stdout.is_empty());
    assert_eq!(
        fs::read(project.path().join("src/main.xml")).unwrap(),
        original
    );

    let written = binary()
        .current_dir(project.path())
        .args(["fmt", "--plain"])
        .output()
        .unwrap();
    assert!(written.status.success());
    assert!(written.stdout.is_empty());
    assert_ne!(
        fs::read(project.path().join("src/main.xml")).unwrap(),
        original
    );
}

#[test]
fn json_build_is_canonical_ndjson_on_stdout() {
    let project = project("root-json-");
    let output = binary()
        .current_dir(project.path())
        .args([
            "build",
            "--emit=prompt",
            "--emit=ir",
            "--emit=debug",
            "--message-format=json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let lines: Vec<_> = output.stdout.split(|byte| *byte == b'\n').collect();
    assert!(lines.len() > 2);
    for line in lines.into_iter().filter(|line| !line.is_empty()) {
        serde_json::from_slice::<serde_json::Value>(line).unwrap();
    }
}

#[test]
fn quiet_is_silent_and_non_tty_human_output_is_linear() {
    let project = project("root-presentation-");
    let quiet = binary()
        .current_dir(project.path())
        .args(["build", "--quiet"])
        .output()
        .unwrap();
    assert!(
        quiet.status.success(),
        "{}",
        String::from_utf8_lossy(&quiet.stderr)
    );
    assert!(quiet.stdout.is_empty());
    assert!(quiet.stderr.is_empty());

    let verbose = binary()
        .current_dir(project.path())
        .args(["build", "--verbose"])
        .output()
        .unwrap();
    assert!(
        verbose.status.success(),
        "{}",
        String::from_utf8_lossy(&verbose.stderr)
    );
    assert!(verbose.stdout.is_empty());
    assert!(!verbose.stderr.is_empty());
    assert!(!verbose.stderr.contains(&b'\r'));
    assert!(!verbose.stderr.windows(2).any(|bytes| bytes == b"\x1b["));
}

#[test]
fn layered_config_drives_registry_storage_and_presentation_with_cli_precedence() {
    let project = project("root-config-");
    let home = project.path().join("user-home");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join("config.toml"),
        "[term]\nmessage-format = \"json\"\n",
    )
    .unwrap();
    let workspace = project.path().join(".xmlsquish");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(
        workspace.join("config.toml"),
        r#"[term]
message-format = "human"

[source]
cache-root = "source-cache"

[manager]
storage-root = "manager-state"

[registries.community]
id = "https://registry.example.test/identity"
index = "sparse+https://registry.example.test/index/"
auth-scope = "community"
"#,
    )
    .unwrap();

    let workspace_wins = binary()
        .current_dir(project.path())
        .env("XMLSQUISH_HOME", &home)
        .args(["build", "--plain"])
        .output()
        .unwrap();
    assert!(
        workspace_wins.status.success(),
        "{}",
        String::from_utf8_lossy(&workspace_wins.stderr)
    );
    assert!(workspace_wins.stdout.is_empty());
    assert!(workspace.join("source-cache").is_dir());
    assert!(workspace.join("manager-state").is_dir());

    let environment_wins = binary()
        .current_dir(project.path())
        .env("XMLSQUISH_HOME", &home)
        .env("XMLSQUISH_MESSAGE_FORMAT", "json")
        .arg("build")
        .output()
        .unwrap();
    assert!(environment_wins.status.success());
    serde_json::from_slice::<serde_json::Value>(
        environment_wins
            .stdout
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap(),
    )
    .unwrap();

    let cli_wins = binary()
        .current_dir(project.path())
        .env("XMLSQUISH_HOME", &home)
        .env("XMLSQUISH_MESSAGE_FORMAT", "json")
        .args(["build", "--message-format=human", "--plain"])
        .output()
        .unwrap();
    assert!(cli_wins.status.success());
    assert!(cli_wins.stdout.is_empty());
    assert!(!cli_wins.stderr.is_empty());

    let generic_cli_override = binary()
        .current_dir(project.path())
        .env("XMLSQUISH_HOME", &home)
        .args(["build", "--config=term.message-format=\"json\""])
        .output()
        .unwrap();
    assert!(generic_cli_override.status.success());
    assert!(generic_cli_override.stderr.is_empty());
    serde_json::from_slice::<serde_json::Value>(
        generic_cli_override
            .stdout
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap(),
    )
    .unwrap();
}

#[test]
fn build_warms_cache_and_publishes_all_selected_artifact_kinds() {
    let project = project("root-build-");
    let args = [
        "build",
        "--emit=prompt",
        "--emit=ir",
        "--emit=debug",
        "--plain",
    ];
    let cold = binary()
        .current_dir(project.path())
        .args(args)
        .output()
        .unwrap();
    assert!(
        cold.status.success(),
        "{}",
        String::from_utf8_lossy(&cold.stderr)
    );
    assert!(cold.stdout.is_empty());
    let publication = project.path().join("target/xmlsquish");
    let files: Vec<_> = walk(&publication);
    for extension in ["prompt", "xsir", "psdbg"] {
        assert!(
            files
                .iter()
                .any(|path| path.extension().and_then(|v| v.to_str()) == Some(extension)),
            "missing .{extension} under {}",
            publication.display()
        );
    }
    let warm = binary()
        .current_dir(project.path())
        .args(args)
        .output()
        .unwrap();
    assert!(
        warm.status.success(),
        "{}",
        String::from_utf8_lossy(&warm.stderr)
    );
    assert!(String::from_utf8_lossy(&warm.stderr).contains("Cached"));
}

#[test]
fn bare_build_defaults_to_a_prompt_artifact() {
    let project = project("root-default-build-");
    let output = binary()
        .current_dir(project.path())
        .args(["build", "--plain"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        walk(&project.path().join("target/xmlsquish"))
            .iter()
            .any(|path| path.extension().and_then(|value| value.to_str()) == Some("prompt"))
    );
}

#[test]
fn global_storage_isolates_project_catalogs_while_serving_both_projects() {
    let first = globally_stored_project("root-global-first-", "first", "alpha", "Alpha");
    let second = globally_stored_project("root-global-second-", "second", "beta", "Beta");
    let home = tempfile::Builder::new()
        .prefix("root-global-catalog-home-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp"))
        .unwrap();

    for project in [&first, &second] {
        let built = binary()
            .current_dir(project.path())
            .env("XMLSQUISH_HOME", home.path())
            .args(["build", "--plain"])
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
    }

    for (project, target) in [(&first, "alpha"), (&second, "beta")] {
        let inspected = binary()
            .current_dir(project.path())
            .env("XMLSQUISH_HOME", home.path())
            .args(["inspect", "link", target, "--format=json"])
            .output()
            .unwrap();
        assert!(
            inspected.status.success(),
            "{}",
            String::from_utf8_lossy(&inspected.stderr)
        );
        let document: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
        assert_eq!(document["view"], "link");
        assert_eq!(document["value"]["target"], target);
    }

    assert!(home.path().join("state/cas").is_dir());
    assert!(home.path().join("state/actions.sqlite3").is_file());
    let namespaces = fs::read_dir(home.path().join("state/catalog/projects"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .count();
    assert_eq!(
        namespaces, 2,
        "each canonical project needs its own catalog"
    );
}

#[test]
fn fmt_diff_is_unified_stdout_for_humans_and_artifact_based_json() {
    let human_project = project("root-human-diff-");
    let human = binary()
        .current_dir(human_project.path())
        .args(["fmt", "--diff", "--plain"])
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&human.stdout).contains("--- "));
    assert!(String::from_utf8_lossy(&human.stdout).contains("+++ "));
    assert!(!human.stdout.starts_with(b"{"));

    let json_project = project("root-json-diff-");
    let json = binary()
        .current_dir(json_project.path())
        .args(["fmt", "--diff", "--message-format=json"])
        .output()
        .unwrap();
    assert_eq!(json.status.code(), Some(1));
    assert!(json.stderr.is_empty());
    for line in json
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        serde_json::from_slice::<serde_json::Value>(line).unwrap();
    }
}

#[test]
fn closed_fmt_diff_pipe_is_quiet_and_preserves_check_status() {
    let project = project("root-broken-diff-");
    let huge = format!(
        "<xs:entry  xmlns:xs = 'https://xmlsquish.moesegfault.dev/ns' ><message>{}</message></xs:entry >",
        "x".repeat(512 * 1024)
    );
    fs::write(project.path().join("src/main.xml"), huge).unwrap();
    let mut child = binary()
        .current_dir(project.path())
        .args(["fmt", "--diff", "--plain"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("OUTPUT001"), "{stderr}");
    assert!(!stderr.contains("internal error"), "{stderr}");
}

#[test]
fn add_dry_run_then_add_and_remove_have_truthful_file_effects() {
    let project = project("root-mutate-");
    let dependency = project.path().join("dep");
    fs::create_dir_all(&dependency).unwrap();
    fs::write(
        dependency.join("xmlsquish.toml"),
        "manifest-version = 1\n[package]\nname = \"dep\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let manifest = project.path().join("xmlsquish.toml");
    let before = fs::read(&manifest).unwrap();

    let dry_run = binary()
        .current_dir(project.path())
        .args(["add", "dep", "--path", "dep", "--dry-run", "--plain"])
        .output()
        .unwrap();
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    assert_eq!(fs::read(&manifest).unwrap(), before);

    let added = binary()
        .current_dir(project.path())
        .args(["add", "dep", "--path", "dep", "--plain"])
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert!(fs::read_to_string(&manifest).unwrap().contains("dep"));

    let removed = binary()
        .current_dir(project.path())
        .args(["remove", "dep", "--plain"])
        .output()
        .unwrap();
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(
        !fs::read_to_string(&manifest)
            .unwrap()
            .contains("[dependencies]\ndep")
    );
}

#[test]
fn inspect_json_is_one_stdout_document_after_build() {
    let project = project("root-inspect-");
    let built = binary()
        .current_dir(project.path())
        .args(["build", "--emit=prompt", "--emit=ir", "--plain"])
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let inspected = binary()
        .current_dir(project.path())
        .env("XMLSQUISH_MESSAGE_FORMAT", "json")
        .args(["inspect", "link", "chat", "--format=json"])
        .output()
        .unwrap();
    assert!(
        inspected.status.success(),
        "{}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(value["view"], "link");
    assert!(!inspected.stdout.windows(2).any(|bytes| bytes == b"}\n{"));

    let human = binary()
        .current_dir(project.path())
        .args(["inspect", "link", "chat", "--format=human"])
        .output()
        .unwrap();
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert!(String::from_utf8_lossy(&human.stdout).starts_with("Link\n"));
    assert!(!human.stdout.starts_with(b"{"));

    for format in ["--format=human", "--format=json"] {
        let mut broken = binary()
            .current_dir(project.path())
            .args(["inspect", "link", "chat", format])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(broken.stdout.take());
        let broken = broken.wait_with_output().unwrap();
        assert!(broken.status.success());
        let stderr = String::from_utf8_lossy(&broken.stderr);
        assert!(!stderr.contains("OUTPUT001"), "{stderr}");
        assert!(!stderr.contains("internal error"), "{stderr}");
    }
}

#[test]
fn short_mode_is_compact_and_append_only() {
    let project = project("root-short-");
    let output = binary()
        .current_dir(project.path())
        .args(["build", "--message-format=short", "--progress=always"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.contains(&b'\r'));
    assert!(!output.stderr.windows(2).any(|bytes| bytes == b"\x1b["));
    assert!(String::from_utf8_lossy(&output.stderr).lines().count() > 1);
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path)
            } else {
                files.push(path)
            }
        }
    }
    files
}
