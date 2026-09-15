//! 根二进制的进程边界合同。 / Process-boundary contracts for the root binary.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
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
    for command in ["new", "build", "fmt", "add", "remove", "inspect"] {
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

/// 仅由公开进程输出发现的构建产物身份。 / Build identities discovered only through public process output.
struct InspectFixture {
    project: tempfile::TempDir,
    ir_id: String,
    prompt_id: String,
    prompt_digest: String,
    debug_digest: String,
    cache_key: String,
}

/// 构建完整产物集并从规范 NDJSON 中提取后续查询句柄。 / Builds the complete artifact set and extracts subsequent query handles from canonical NDJSON.
fn built_inspect_fixture(name: &str) -> InspectFixture {
    let project = project(name);
    let manifest = project.path().join("xmlsquish.toml");
    let first = binary()
        .current_dir(project.path())
        .args([
            "build",
            "--manifest-path",
            manifest.to_str().unwrap(),
            "--emit=prompt",
            "--emit=ir",
            "--emit=debug",
            "--message-format=json",
        ])
        .output()
        .unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(first.stderr.is_empty());
    let first = ndjson_documents(&first.stdout);
    let artifacts = first
        .iter()
        .find_map(|event| {
            (event["payload"]["type"] == "operation_completed")
                .then(|| {
                    event["payload"]["data"]["result"]["result"]["published"][0]["artifacts"]
                        .as_array()
                })
                .flatten()
        })
        .expect("successful build exposes published artifacts in operation_completed");
    let artifact = |kind: &str| {
        artifacts
            .iter()
            .find(|artifact| artifact["kind"]["type"] == kind)
            .unwrap_or_else(|| panic!("build did not publish {kind}"))
    };
    let ir = artifact("binary_ir");
    let prompt = artifact("prompt");
    let debug = artifact("debug_info");

    // A second identical build publicly reports the materialized action keys as cache-hit events.
    // 第二次相同构建通过 cache-hit 事件公开已物化的动作键，无需读取私有 SQLite。
    let second = binary()
        .current_dir(project.path())
        .args([
            "build",
            "--manifest-path",
            manifest.to_str().unwrap(),
            "--emit=prompt",
            "--emit=ir",
            "--emit=debug",
            "--message-format=json",
        ])
        .output()
        .unwrap();
    assert!(second.status.success());
    let second = ndjson_documents(&second.stdout);
    let cache_key = second
        .iter()
        .find_map(|event| {
            (event["payload"]["type"] == "cache_hit")
                .then(|| event["payload"]["data"]["action_key"].as_str())
                .flatten()
        })
        .expect("warm build exposes at least one cache action key")
        .to_owned();

    InspectFixture {
        project,
        ir_id: json_string(ir, "id"),
        prompt_id: json_string(prompt, "id"),
        prompt_digest: digest_string(prompt),
        debug_digest: digest_string(debug),
        cache_key,
    }
}

fn ndjson_documents(bytes: &[u8]) -> Vec<serde_json::Value> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

fn json_string(value: &serde_json::Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("missing string field {key} in {value}"))
        .to_owned()
}

fn digest_string(artifact: &serde_json::Value) -> String {
    format!(
        "blake3:{}",
        artifact["digest"]["hex"]
            .as_str()
            .expect("artifact has a digest hex")
    )
}

fn inspect(
    fixture: &InspectFixture,
    subject: &str,
    identifier: &str,
    format: &str,
) -> std::process::Output {
    binary()
        .current_dir(fixture.project.path())
        .args([
            "inspect",
            "--manifest-path",
            fixture
                .project
                .path()
                .join("xmlsquish.toml")
                .to_str()
                .unwrap(),
            subject,
            identifier,
            format,
        ])
        .output()
        .unwrap()
}

fn assert_inspect_document(output: std::process::Output, view: &str) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["view"], view);
    document
}

#[test]
fn inspect_ir_link_source_and_cache_have_typed_human_and_single_json_views() {
    let fixture = built_inspect_fixture("root-inspect-matrix-");
    let subjects = [
        ("ir", fixture.ir_id.as_str(), "ir", "IR\n"),
        ("link", "chat", "link", "Link\n"),
        (
            "source",
            "xmlsquish://fixture/src/main.xml",
            "source",
            "Source\n",
        ),
        ("cache", fixture.cache_key.as_str(), "cache", "Cache\n"),
    ];
    let mut documents = BTreeMap::new();
    for (subject, identifier, view, heading) in subjects {
        let human = inspect(&fixture, subject, identifier, "--format=human");
        assert!(human.status.success());
        assert!(human.stderr.is_empty());
        assert!(
            String::from_utf8(human.stdout)
                .unwrap()
                .starts_with(heading),
            "{subject} did not use its typed human renderer"
        );
        let json = assert_inspect_document(
            inspect(&fixture, subject, identifier, "--format=json"),
            view,
        );
        documents.insert(subject, json);
    }

    assert_eq!(documents["ir"]["value"]["artifact"]["id"], fixture.ir_id);
    assert_eq!(
        documents["ir"]["value"]["artifact"]["kind"]["type"],
        "binary_ir"
    );
    assert_eq!(documents["link"]["value"]["target"], "chat");
    assert_eq!(
        documents["link"]["value"]["link_map"]["kind"]["name"],
        "static-link-map"
    );
    assert_eq!(
        documents["source"]["value"]["source"],
        "xmlsquish://fixture/src/main.xml"
    );
    assert_eq!(
        documents["cache"]["value"]["actions"][0]["action_key"],
        fixture.cache_key
    );
}

#[test]
fn inspect_artifact_resolves_the_public_path_and_exact_prompt_debug_relation() {
    let fixture = built_inspect_fixture("root-inspect-artifact-");
    let locator = "target/xmlsquish/chat.prompt";
    let human = inspect(&fixture, "artifact", locator, "--format=human");
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert!(human.stderr.is_empty());
    assert!(
        String::from_utf8(human.stdout)
            .unwrap()
            .starts_with("Provenance\n")
    );

    let document = assert_inspect_document(
        inspect(&fixture, "artifact", locator, "--format=json"),
        "provenance",
    );
    let artifact = &document["value"]["artifact"];
    assert_eq!(artifact["id"], fixture.prompt_id);
    assert_eq!(artifact["kind"]["type"], "prompt");
    assert_eq!(digest_string(artifact), fixture.prompt_digest);
    let evidence = document["value"]["evidence"]
        .as_array()
        .expect("provenance evidence is an array");
    let debug = evidence
        .iter()
        .find(|item| item["kind"]["type"] == "debug_info")
        .expect("prompt provenance includes its psdbg artifact");
    assert_eq!(digest_string(debug), fixture.debug_digest);
}

#[test]
fn inspect_failures_are_read_only_and_never_emit_partial_query_documents() {
    let fixture = built_inspect_fixture("root-inspect-invalid-");
    let before_missing = project_bytes(fixture.project.path());
    let missing = inspect(
        &fixture,
        "artifact",
        "target/xmlsquish/missing.prompt",
        "--format=json",
    );
    assert_inspect_failure(missing, "XS3420");
    assert_eq!(project_bytes(fixture.project.path()), before_missing);

    let before_wrong_kind = project_bytes(fixture.project.path());
    let wrong_kind = inspect(&fixture, "ir", &fixture.prompt_id, "--format=json");
    assert_inspect_failure(wrong_kind, "XS3421");
    assert_eq!(project_bytes(fixture.project.path()), before_wrong_kind);

    fs::write(
        fixture.project.path().join("xmlsquish.lock"),
        b"this is not a lockfile",
    )
    .unwrap();
    let before_corrupt = project_bytes(fixture.project.path());
    let corrupt = inspect(
        &fixture,
        "source",
        "xmlsquish://fixture/src/main.xml",
        "--format=json",
    );
    assert_inspect_failure(corrupt, "XS3402");
    assert_eq!(project_bytes(fixture.project.path()), before_corrupt);
}

fn assert_inspect_failure(output: std::process::Output, code: &str) {
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "failed query leaked a partial document"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains(code));
}

fn project_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    walk(root)
        .into_iter()
        .map(|path| {
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            (relative, fs::read(path).unwrap())
        })
        .collect()
}

#[test]
fn absolute_manifest_path_builds_and_inspects_the_same_project_from_outside() {
    let project = project("root-absolute-manifest-");
    let manifest = fs::canonicalize(project.path().join("xmlsquish.toml")).unwrap();
    let outside = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp");
    let built = binary()
        .current_dir(&outside)
        .args([
            "build",
            "--manifest-path",
            manifest.to_str().unwrap(),
            "--emit=prompt",
            "--plain",
        ])
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert!(
        walk(&project.path().join("target/xmlsquish"))
            .iter()
            .any(|path| path.extension().and_then(|value| value.to_str()) == Some("prompt"))
    );

    let inspected = binary()
        .current_dir(&outside)
        .args([
            "inspect",
            "--manifest-path",
            manifest.to_str().unwrap(),
            "link",
            "chat",
            "--format=json",
        ])
        .output()
        .unwrap();
    let document = assert_inspect_document(inspected, "link");
    assert_eq!(document["value"]["target"], "chat");

    let artifact = binary()
        .current_dir(&outside)
        .args([
            "inspect",
            "--manifest-path",
            manifest.to_str().unwrap(),
            "artifact",
            "target/xmlsquish/chat.prompt",
            "--format=json",
        ])
        .output()
        .unwrap();
    let artifact = assert_inspect_document(artifact, "provenance");
    assert_eq!(artifact["value"]["artifact"]["id"], "fixture:chat:prompt");
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

/// Allocates an isolated process-test root under the repository-owned scratch directory.
/// 在仓库自有暂存目录下分配隔离的进程测试根目录。
fn new_scratch(name: &str) -> tempfile::TempDir {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp");
    fs::create_dir_all(&scratch).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("new-{name}-"))
        .tempdir_in(scratch)
        .unwrap()
}

/// Runs `new` with user configuration isolated from the developer or CI account.
/// 在隔离用户配置的前提下运行 `new`，避免开发机或 CI 账户污染合同。
fn run_new(root: &tempfile::TempDir, destination: &Path, args: &[&str]) -> std::process::Output {
    let home = root.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let mut command = binary();
    command
        .current_dir(root.path())
        .env("XMLSQUISH_HOME", home)
        .arg("new")
        .arg(destination)
        .args(args);
    command.output().unwrap()
}

fn assert_process_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn relative_files(root: &Path) -> Vec<String> {
    let mut files = walk(root)
        .into_iter()
        .map(|path| {
            path.strip_prefix(root)
                .unwrap()
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[test]
fn new_vcs_none_has_exact_public_tree_and_is_immediately_usable() {
    let root = new_scratch("none-buildable");
    let destination = root.path().join("deep/prompts/support");
    let output = run_new(&root, &destination, &["--vcs=none", "--quiet"]);
    assert_process_success(&output);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        relative_files(&destination),
        ["src/prompt.xml", "xmlsquish.toml"]
    );
    assert!(!destination.join("xmlsquish.lock").exists());
    assert!(!destination.join("target").exists());

    let manifest = destination.join("xmlsquish.toml");
    let formatted = binary()
        .args(["fmt", "--manifest-path"])
        .arg(&manifest)
        .args(["--check", "--plain"])
        .output()
        .unwrap();
    assert_process_success(&formatted);
    let built = binary()
        .args(["build", "--manifest-path"])
        .arg(&manifest)
        .args(["--offline", "--plain"])
        .output()
        .unwrap();
    assert_process_success(&built);
}

#[test]
fn new_default_initializes_git_outside_an_enclosing_worktree() {
    let root = new_scratch("default-git");
    let destination = root.path().join("standalone");
    let home = root.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let output = binary()
        .current_dir(root.path())
        .env("XMLSQUISH_HOME", &home)
        // The test itself must live below `.temp`; the ceiling makes that scratch root a genuine
        // standalone Git context instead of inheriting this repository's worktree.
        .env(
            "GIT_CEILING_DIRECTORIES",
            root.path().parent().expect("scratch root has a parent"),
        )
        .args(["new"])
        .arg(&destination)
        .arg("--quiet")
        .output()
        .unwrap();
    assert_process_success(&output);
    assert!(destination.join(".git").is_dir());
    assert_eq!(
        fs::read(destination.join(".gitignore")).unwrap(),
        b"/target/\n"
    );
    for path in ["xmlsquish.toml", "src/prompt.xml"] {
        assert!(destination.join(path).is_file(), "missing {path}");
    }
    assert!(!destination.join("xmlsquish.lock").exists());
    assert!(!destination.join("target").exists());
}

#[test]
fn new_rejects_invalid_names_at_the_correct_boundary() {
    let explicit = new_scratch("bad-explicit");
    let explicit_destination = explicit.path().join("valid-name");
    let output = run_new(
        &explicit,
        &explicit_destination,
        &["--name=Not Valid!", "--vcs=none"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(!explicit_destination.exists());

    let inferred = new_scratch("bad-inferred");
    let inferred_destination = inferred.path().join("Not Valid!");
    let output = run_new(&inferred, &inferred_destination, &["--vcs=none"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--name"));
    assert!(!inferred_destination.exists());
}

#[test]
fn new_refuses_every_existing_destination_kind() {
    let root = new_scratch("existing");
    let directory = root.path().join("directory");
    fs::create_dir(&directory).unwrap();
    let file = root.path().join("file");
    fs::write(&file, b"occupied").unwrap();
    for destination in [&directory, &file] {
        let output = run_new(&root, destination, &["--vcs=none"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
    }
    assert_eq!(fs::read(file).unwrap(), b"occupied");
}

#[cfg(unix)]
#[test]
fn new_refuses_an_existing_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let root = new_scratch("existing-symlink");
    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    let destination = root.path().join("link");
    symlink(&target, &destination).unwrap();
    let output = run_new(&root, &destination, &["--vcs=none"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        fs::symlink_metadata(destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

fn workspace_manifest(members: &str, exclude: &str) -> String {
    format!("manifest-version = 1\n[workspace]\nmembers = [{members}]\nexclude = [{exclude}]\n")
}

#[test]
fn new_joins_an_enclosing_workspace_once_and_preserves_existing_membership() {
    let root = new_scratch("workspace");
    let manifest = root.path().join("xmlsquish.toml");
    fs::write(&manifest, workspace_manifest("", "")).unwrap();
    let destination = root.path().join("packages/new-member");
    assert_process_success(&run_new(&root, &destination, &["--vcs=none", "--quiet"]));
    let updated = fs::read_to_string(&manifest).unwrap();
    assert_eq!(updated.matches("packages/new-member").count(), 1);

    let effective = new_scratch("workspace-effective");
    let effective_manifest = effective.path().join("xmlsquish.toml");
    fs::write(
        &effective_manifest,
        workspace_manifest("\"packages/already\"", ""),
    )
    .unwrap();
    let before = fs::read(&effective_manifest).unwrap();
    assert_process_success(&run_new(
        &effective,
        &effective.path().join("packages/already"),
        &["--vcs=none", "--quiet"],
    ));
    assert_eq!(fs::read(effective_manifest).unwrap(), before);
}

#[test]
fn new_rejects_workspace_exclusion_and_duplicate_package_name_without_publication() {
    let excluded = new_scratch("workspace-excluded");
    fs::write(
        excluded.path().join("xmlsquish.toml"),
        workspace_manifest("", "\"packages/*\""),
    )
    .unwrap();
    let excluded_destination = excluded.path().join("packages/new");
    let output = run_new(&excluded, &excluded_destination, &["--vcs=none"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!excluded_destination.exists());

    let duplicate = new_scratch("workspace-duplicate");
    fs::create_dir_all(duplicate.path().join("packages/existing")).unwrap();
    fs::write(
        duplicate.path().join("xmlsquish.toml"),
        workspace_manifest("\"packages/existing\"", ""),
    )
    .unwrap();
    fs::write(
        duplicate.path().join("packages/existing/xmlsquish.toml"),
        "manifest-version = 1\n[package]\nname = \"same-name\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let duplicate_destination = duplicate.path().join("packages/new");
    let output = run_new(
        &duplicate,
        &duplicate_destination,
        &["--name=same-name", "--vcs=none"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(!duplicate_destination.exists());
}

#[test]
fn new_human_short_quiet_and_ndjson_preserve_stream_contracts() {
    let human = new_scratch("human");
    let output = run_new(
        &human,
        &human.path().join("human-project"),
        &["--vcs=none", "--plain"],
    );
    assert_process_success(&output);
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());

    let short = new_scratch("short");
    let output = run_new(
        &short,
        &short.path().join("short-project"),
        &["--vcs=none", "--message-format=short"],
    );
    assert_process_success(&output);
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    assert!(!output.stderr.contains(&b'\r'));

    let quiet = new_scratch("quiet");
    let output = run_new(
        &quiet,
        &quiet.path().join("quiet-project"),
        &["--vcs=none", "--quiet"],
    );
    assert_process_success(&output);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());

    let json = new_scratch("json");
    let output = run_new(
        &json,
        &json.path().join("json-project"),
        &["--vcs=none", "--message-format=json"],
    );
    assert_process_success(&output);
    assert!(output.stderr.is_empty());
    let documents = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(documents.len() > 2);
    assert!(documents.iter().any(|document| {
        document["payload"]["type"] == "operation_completed"
            && document["payload"]["data"]["result"]["type"] == "new"
    }));
    assert_eq!(
        documents.last().unwrap()["payload"]["data"]["status"],
        "success"
    );
}

#[test]
fn concurrent_new_processes_serialize_same_and_shared_parent_destinations() {
    use std::sync::{Arc, Barrier};

    let root = new_scratch("concurrent");
    let same = root.path().join("same");
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let destination = same.clone();
            let cwd = root.path().to_path_buf();
            std::thread::spawn(move || {
                barrier.wait();
                binary()
                    .current_dir(cwd)
                    .arg("new")
                    .arg(destination)
                    .args(["--vcs=none", "--quiet"])
                    .output()
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let outputs = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        outputs
            .iter()
            .filter(|output| output.status.success())
            .count(),
        1
    );
    assert_eq!(
        outputs
            .iter()
            .filter(|output| output.status.code() == Some(1))
            .count(),
        1
    );

    let shared = root.path().join("missing-parent");
    let barrier = Arc::new(Barrier::new(2));
    let handles = ["left", "right"]
        .into_iter()
        .map(|leaf| {
            let barrier = Arc::clone(&barrier);
            let destination = shared.join(leaf);
            let cwd = root.path().to_path_buf();
            std::thread::spawn(move || {
                barrier.wait();
                binary()
                    .current_dir(cwd)
                    .arg("new")
                    .arg(destination)
                    .args(["--vcs=none", "--quiet"])
                    .output()
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    for output in handles.into_iter().map(|handle| handle.join().unwrap()) {
        assert_process_success(&output);
    }
    assert!(shared.join("left/xmlsquish.toml").is_file());
    assert!(shared.join("right/xmlsquish.toml").is_file());
}

#[cfg(unix)]
#[test]
fn first_interrupt_during_git_preparation_cancels_before_publication() {
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };

    let root = new_scratch("interrupt-precommit");
    let tools = root.path().join("tools");
    fs::create_dir(&tools).unwrap();
    let marker = root.path().join("git-init-entered");
    let release = root.path().join("release-git-init");
    let shim = tools.join("git");
    fs::write(
        &shim,
        b"#!/bin/sh\n\
if [ \"$1\" = \"rev-parse\" ]; then printf 'false\\n'; exit 0; fi\n\
if [ \"$1\" = \"init\" ]; then\n\
  : > \"$XMLSQUISH_TEST_GIT_MARKER\"\n\
  while [ ! -f \"$XMLSQUISH_TEST_GIT_RELEASE\" ]; do sleep 0.01; done\n\
  mkdir .git\n\
  exit 0\n\
fi\n\
exit 64\n",
    )
    .unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    let destination = root.path().join("cancelled");
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(tools.clone()).chain(std::env::split_paths(&inherited_path)),
    )
    .unwrap();
    let child = binary()
        .current_dir(root.path())
        .env("PATH", path)
        .env("XMLSQUISH_HOME", root.path().join("home"))
        .env("XMLSQUISH_TEST_GIT_MARKER", &marker)
        .env("XMLSQUISH_TEST_GIT_RELEASE", &release)
        .arg("new")
        .arg(&destination)
        .arg("--quiet")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.is_file() {
        assert!(
            Instant::now() < deadline,
            "Git preparation barrier timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let interrupted = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(interrupted.success());
    fs::write(&release, b"release").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(130),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!destination.exists());
    assert!(walk(root.path()).iter().all(|path| {
        !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(".xmlsquish-published-"))
    }));
}
