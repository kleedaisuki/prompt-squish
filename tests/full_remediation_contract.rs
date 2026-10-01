//! Independent end-to-end contracts for the complete v1.2 performance remediation.
//! These fixtures test integrity and portability, not elapsed-time thresholds.

use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Runs a fresh process so verified in-memory snapshots cannot leak between invocations.
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xmlsquish"))
        .current_dir(root)
        .env_remove("XMLSQUISH_TRACE")
        .args(args)
        .output()
        .expect("start CLI")
}

/// Retains complete process diagnostics rather than treating a zero exit as a product oracle.
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "exit={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Creates all experiment files beneath the repository scratch directory.
fn project(label: &str, package: &str, backend: &str, source: &str) -> tempfile::TempDir {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp/full-remediation");
    fs::create_dir_all(&scratch).unwrap();
    let root = tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(scratch)
        .unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("xmlsquish.toml"), format!("manifest-version = 1\n[package]\nname = \"{package}\"\nversion = \"1.0.0\"\n[target.main]\nentry = \"src/main.xml\"\nbackend = \"{backend}\"\n")).unwrap();
    fs::write(root.path().join("src/main.xml"), source).unwrap();
    root
}

/// Reads only the published product, never a staged generation or CAS entry.
fn product(root: &Path, suffix: &str) -> Vec<u8> {
    let path = root.join(format!("target/xmlsquish/artifacts/main.{suffix}"));
    fs::read(&path)
        .unwrap_or_else(|error| panic!("missing public artifact {}: {error}", path.display()))
}

/// Extracts agreed STORED local records independently of the production archive reader.
fn members(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut cursor = 0;
    let mut result = BTreeMap::new();
    while bytes.get(cursor..cursor + 4) == Some(b"PK\x03\x04".as_slice()) {
        let word =
            |offset| u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()) as usize;
        let size = u32::from_le_bytes(bytes[cursor + 18..cursor + 22].try_into().unwrap()) as usize;
        assert_eq!(word(cursor + 8), 0, "fixture requires STORED ZIP");
        let name_len = word(cursor + 26);
        let name_start = cursor + 30;
        let name = std::str::from_utf8(&bytes[name_start..name_start + name_len])
            .unwrap()
            .to_owned();
        let data_start = name_start + name_len + word(cursor + 28);
        assert!(
            result
                .insert(name, bytes[data_start..data_start + size].to_vec())
                .is_none()
        );
        cursor = data_start + size;
    }
    assert_eq!(
        bytes.get(cursor..cursor + 4),
        Some(b"PK\x01\x02".as_slice())
    );
    result
}

/// Decodes actual emitted protocol events for work-count assertions.
fn events(output: &Output) -> Vec<Value> {
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

/// Extracts structured diagnostics from both action failures and diagnostic events.
fn diagnostics(output: &Output) -> Vec<Value> {
    events(output)
        .into_iter()
        .filter_map(|event| {
            let payload = &event["payload"];
            if payload["type"] == "diagnostic" {
                Some(payload["data"].clone())
            } else {
                payload["data"].get("diagnostic").cloned()
            }
        })
        .collect()
}

/// Returns concrete compile actions advertised by the real process protocol.
fn compile_actions(output: &Output) -> usize {
    events(output)
        .iter()
        .filter(|event| {
            event["payload"]["type"] == "action_declared"
                && event["payload"]["data"]["kind"] == "compile"
        })
        .count()
}

/// Returns all regular files below one project-owned directory without following aliases.
fn files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut result = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                pending.push(entry.path());
            }
            if kind.is_file() {
                result.push(entry.path());
            }
        }
    }
    result
}

#[test]
fn large_library_small_export_shared_skill_assets_survive_relocation_and_warm_reuse() {
    let imports: String = (0..64)
        .map(|index| format!("<xs:import src='module{index}.xml'/>"))
        .collect();
    let provider = project(
        "large-provider-",
        "library",
        "sopack",
        &format!(
            "<xs:sopack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'>{imports}</xs:sopack>"
        ),
    );
    let shared: Vec<u8> = (0..262_144).map(|index| (index % 251) as u8).collect();
    fs::write(provider.path().join("src/shared.bin"), &shared).unwrap();
    for index in 0..64 {
        let content = if index == 0 {
            "<xs:asset path='shared.bin' name='scripts/tool.bin'/><xs:asset path='shared.bin' name='references/tool.bin'/><xs:asset path='shared.bin' name='assets/tool.bin'/>".to_owned()
        } else {
            "<xs:asset path='shared.bin'/>".to_owned()
        };
        fs::write(provider.path().join(format!("src/module{index}.xml")), format!("<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:module{index}'><!--{}--><xs:macro name='m:skill'>{content}</xs:macro></xs:module>", "unused authoring text ".repeat(800))).unwrap();
    }
    let manifest = provider.path().join("xmlsquish.toml");
    let mut text = fs::read_to_string(&manifest).unwrap();
    text.push_str("\n[exports]\nsmall = \"src/module0.xml\"\n");
    fs::write(manifest, text).unwrap();
    success(&cli(provider.path(), &["build", "--plain"]));
    let archive = product(provider.path(), "sopack");
    assert_eq!(
        members(&archive)
            .keys()
            .filter(|path| path.starts_with("assets/"))
            .count(),
        1,
        "identical asset bytes should have one physical archive member"
    );
    let consumer = project(
        "large-consumer-",
        "consumer",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:module0'><xs:import src='pkg:library/small'/><xs:expand ref='m:skill'/></xs:pack>",
    );
    fs::write(consumer.path().join("library.sopack"), archive).unwrap();
    provider.close().unwrap();
    success(&cli(
        consumer.path(),
        &["add", "library", "--path", "library.sopack", "--plain"],
    ));
    let build = cli(
        consumer.path(),
        &["build", "--trace=events", "--message-format=json"],
    );
    success(&build);
    let compile_count = compile_actions(&build);
    assert!(
        (1..=2).contains(&compile_count),
        "small export scheduled {compile_count} compile actions from a 64-module library"
    );
    let expected = product(consumer.path(), "pack");
    let packed = members(&expected);
    assert_eq!(packed.len(), 3);
    for path in ["scripts/tool.bin", "references/tool.bin", "assets/tool.bin"] {
        assert_eq!(packed[path], shared);
    }
    let warm = cli(
        consumer.path(),
        &["build", "--trace=off", "--message-format=json"],
    );
    success(&warm);
    assert_eq!(product(consumer.path(), "pack"), expected);
    assert!(
        events(&warm)
            .iter()
            .any(|event| event["payload"]["type"] == "cache_hit"),
        "warm run has no actual cache-hit evidence"
    );
}

#[test]
fn fresh_invocation_rejects_corrupt_product_blob_and_repairs_without_changing_zip() {
    let root = project(
        "cas-corruption-",
        "consumer",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:asset path='bytes.bin'/></xs:pack>",
    );
    fs::write(
        root.path().join("src/bytes.bin"),
        b"stable binary data\x00\xff",
    )
    .unwrap();
    success(&cli(root.path(), &["build", "--plain"]));
    let expected = product(root.path(), "pack");
    let cas = root.path().join("target/xmlsquish/cache/cas");
    let stored = files(&cas)
        .into_iter()
        .filter(|path| fs::read(path).unwrap() == expected)
        .collect::<Vec<_>>();
    assert!(
        !stored.is_empty(),
        "fixture did not locate the actual cached product"
    );
    for path in &stored {
        fs::write(path, vec![0xa5; expected.len()]).unwrap();
    }
    let rebuilt = cli(
        root.path(),
        &["build", "--trace=events", "--message-format=json"],
    );
    success(&rebuilt);
    assert_eq!(
        product(root.path(), "pack"),
        expected,
        "corrupt cached bytes became a successful product"
    );
    assert!(
        stored
            .iter()
            .all(|path| fs::read(path).unwrap() == expected),
        "corrupt addressed blobs were not repaired"
    );
    success(&cli(root.path(), &["clean", "--plain"]));
    assert!(!root.path().join("target/xmlsquish").exists());
    assert_eq!(
        fs::read(root.path().join("src/bytes.bin")).unwrap(),
        b"stable binary data\x00\xff"
    );
    success(&cli(root.path(), &["build", "--trace=off", "--plain"]));
    assert_eq!(
        product(root.path(), "pack"),
        expected,
        "clean/rebuild changes deterministic product"
    );
}

#[test]
fn invalid_action_index_warns_rebuilds_without_cache_authority_and_clean_recovers() {
    let root = project(
        "invalid-index-",
        "consumer",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:asset path='bytes.bin'/></xs:pack>",
    );
    fs::write(root.path().join("src/bytes.bin"), b"unchanged").unwrap();
    success(&cli(root.path(), &["build", "--plain"]));
    let expected = product(root.path(), "pack");
    let index = root.path().join("target/xmlsquish/cache/actions.sqlite3");
    assert!(index.is_file());
    // Every CLI child has exited. Remove the isolated fixture's recovery files:
    // otherwise SQLite may legitimately reconstruct the old database from WAL.
    for suffix in ["-wal", "-shm"] {
        let sidecar = index.with_file_name(format!("actions.sqlite3{suffix}"));
        if sidecar.exists() {
            fs::remove_file(sidecar).unwrap();
        }
    }
    let invalid = b"invalid derived cache; not a SQLite database";
    fs::write(&index, invalid).unwrap();
    assert_eq!(
        fs::read(&index).unwrap(),
        invalid,
        "fixture failed to corrupt the complete persisted index"
    );
    let rebuilt = cli(
        root.path(),
        &["build", "--trace=events", "--message-format=json"],
    );
    // Cache lookup/record errors are advisory in run_dispatch: execute verified
    // work and report a warning, never turn invalid index rows into cache hits.
    success(&rebuilt);
    let recorded = events(&rebuilt);
    assert!(
        recorded
            .iter()
            .any(|event| event["payload"]["type"] == "diagnostic"
                && event["payload"]["data"]["severity"] == "warning"
                && event["payload"]["data"]["code"] == "host_action_index_open"),
        "invalid index lost its actual storage warning: {}",
        String::from_utf8_lossy(&rebuilt.stdout)
    );
    assert!(
        !recorded
            .iter()
            .any(|event| event["payload"]["type"] == "cache_hit"),
        "corrupt index supplied successful cache authority"
    );
    assert_eq!(
        product(root.path(), "pack"),
        expected,
        "invalid index destroyed prior public product"
    );
    success(&cli(root.path(), &["clean", "--plain"]));
    success(&cli(root.path(), &["build", "--trace=off", "--plain"]));
    assert_eq!(
        product(root.path(), "pack"),
        expected,
        "clean recovery changes output"
    );
}

#[test]
fn output_budget_failure_preserves_previous_pack_generation_without_partial_product() {
    let root = project(
        "budget-atomicity-",
        "consumer",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:asset path='bytes.bin'/></xs:pack>",
    );
    fs::write(root.path().join("src/bytes.bin"), b"small").unwrap();
    success(&cli(root.path(), &["build", "--plain"]));
    let expected = product(root.path(), "pack");
    let manifest = root.path().join("xmlsquish.toml");
    let mut text = fs::read_to_string(&manifest).unwrap();
    text.push_str("\n[target.main.limits]\nmax-output-bytes = 64\n");
    fs::write(manifest, text).unwrap();
    fs::write(root.path().join("src/bytes.bin"), vec![0x5a; 4096]).unwrap();
    let failed = cli(
        root.path(),
        &["build", "--trace=events", "--message-format=json"],
    );
    assert!(!failed.status.success(), "output budget was ignored");
    assert_eq!(
        product(root.path(), "pack"),
        expected,
        "failed budget replaced the previous generation"
    );
    let published = root.path().join("target/xmlsquish/artifacts");
    assert_eq!(
        files(&published)
            .into_iter()
            .filter(|path| path
                .extension()
                .is_some_and(|extension| extension == "pack"))
            .count(),
        1
    );
}

/// Builds same-name providers with a genuine import cycle and source-owned assets.
fn cyclic_library(label: &str) -> tempfile::TempDir {
    let root = project(
        label,
        "library",
        "sopack",
        "<xs:sopack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:cycle-root' xmlns:l='urn:test:cycle-left' xmlns:r='urn:test:cycle-right'><xs:import src='pkg:left/main'/><xs:import src='pkg:right/main'/><xs:macro name='m:bundle'><xs:asset path='owned.bin' name='root.bin'/><xs:expand ref='l:asset'/><xs:expand ref='r:asset'/></xs:macro></xs:sopack>",
    );
    fs::write(root.path().join("src/owned.bin"), b"ROOT").unwrap();
    let manifest = root.path().join("xmlsquish.toml");
    let mut text = fs::read_to_string(&manifest).unwrap();
    text.push_str("\n[exports]\nmain = \"src/main.xml\"\n\n[dependencies]\nleft = { package = \"library\", path = \"left\" }\nright = { package = \"library\", path = \"right\" }\n");
    fs::write(manifest, text).unwrap();
    for (name, other, value) in [
        ("left", "right", b"LEFT".as_slice()),
        ("right", "left", b"RIGHT".as_slice()),
    ] {
        let package = root.path().join(name);
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(package.join("xmlsquish.toml"), format!("manifest-version = 1\n[package]\nname = \"library\"\nversion = \"1.0.0\"\n[exports]\nmain = \"src/lib.xml\"\n[dependencies]\n{other} = {{ package = \"library\", path = \"../{other}\" }}\n")).unwrap();
        fs::write(package.join("src/lib.xml"), format!("<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:cycle-{name}'><xs:import src='pkg:{other}/main'/><xs:macro name='m:asset'><xs:asset path='owned.bin' name='{name}.bin'/></xs:macro><xs:macro name='m:recurse'><xs:expand ref='m:recurse'/></xs:macro></xs:module>")).unwrap();
        fs::write(package.join("src/owned.bin"), value).unwrap();
    }
    root
}

#[test]
fn cyclic_same_name_providers_and_root_assets_have_stable_portable_identity_and_diagnostics() {
    let providers = [cyclic_library("cycle-a-"), cyclic_library("cycle-b-")];
    for provider in &providers {
        success(&cli(provider.path(), &["build", "--plain"]));
    }
    let archive = product(providers[0].path(), "sopack");
    assert_eq!(
        product(providers[1].path(), "sopack"),
        archive,
        "cyclic identity depends on physical provider roots"
    );
    let old_roots: Vec<_> = providers
        .iter()
        .map(|provider| provider.path().to_string_lossy().into_owned())
        .collect();
    let consumer = project(
        "cycle-consumer-",
        "consumer",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:cycle-root'><xs:import src='pkg:library/main'/><xs:expand ref='m:bundle'/></xs:pack>",
    );
    fs::write(consumer.path().join("library.sopack"), archive).unwrap();
    fs::write(consumer.path().join("src/owned.bin"), b"CALLER").unwrap();
    for provider in providers {
        provider.close().unwrap();
    }
    success(&cli(
        consumer.path(),
        &["add", "library", "--path", "library.sopack", "--plain"],
    ));
    success(&cli(consumer.path(), &["build", "--plain"]));
    let expected = product(consumer.path(), "pack");
    let packed = members(&expected);
    assert_eq!(packed["root.bin"], b"ROOT");
    assert_eq!(packed["left.bin"], b"LEFT");
    assert_eq!(packed["right.bin"], b"RIGHT");
    fs::write(consumer.path().join("src/main.xml"), "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:l='urn:test:cycle-left'><xs:import src='pkg:library/main'/><xs:expand ref='l:recurse'/></xs:pack>").unwrap();
    let manifest = consumer.path().join("xmlsquish.toml");
    let mut text = fs::read_to_string(&manifest).unwrap();
    text.push_str("\n[target.main.limits]\nmax-expansions = 8\n");
    fs::write(manifest, text).unwrap();
    let failed = cli(
        consumer.path(),
        &["build", "--trace=events", "--message-format=json"],
    );
    assert!(
        !failed.status.success(),
        "cyclic macro ignored expansion budget"
    );
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&failed.stdout),
        String::from_utf8_lossy(&failed.stderr)
    );
    assert!(
        diagnostics.contains("lib.xml"),
        "archived definition-source diagnostics lost: {diagnostics}"
    );
    assert!(
        old_roots.iter().all(|root| !diagnostics.contains(root)),
        "diagnostics retained a deleted producer-device root"
    );
    assert_eq!(
        product(consumer.path(), "pack"),
        expected,
        "failed archived recursion destroyed the old generation"
    );
}

#[test]
fn empty_document_singleflight_targets_keep_distinct_directive_identity() {
    let root = project(
        "target-sidechannel-",
        "consumer",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:asset path='a.bin'/></xs:pack>",
    );
    fs::write(root.path().join("src/a.bin"), b"A").unwrap();
    fs::write(root.path().join("src/b.bin"), b"B").unwrap();
    fs::write(root.path().join("src/other.xml"), "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:asset path='b.bin'/></xs:pack>").unwrap();
    let manifest = root.path().join("xmlsquish.toml");
    let mut text = fs::read_to_string(&manifest).unwrap();
    text.push_str("\n[target.second]\nentry = \"src/main.xml\"\nbackend = \"pack\"\n\n[target.third]\nentry = \"src/other.xml\"\nbackend = \"pack\"\n");
    fs::write(manifest, text).unwrap();
    success(&cli(
        root.path(),
        &["build", "--jobs=4", "--trace=off", "--plain"],
    ));
    let original: BTreeMap<_, _> = ["main", "second", "third"]
        .into_iter()
        .map(|target| {
            let bytes = fs::read(
                root.path()
                    .join(format!("target/xmlsquish/artifacts/{target}.pack")),
            )
            .unwrap();
            (target, bytes)
        })
        .collect();
    assert_eq!(original["main"], original["second"]);
    assert_eq!(
        members(&original["main"]),
        BTreeMap::from([("a.bin".to_owned(), b"A".to_vec())])
    );
    assert_eq!(
        members(&original["third"]),
        BTreeMap::from([("b.bin".to_owned(), b"B".to_vec())])
    );
    success(&cli(
        root.path(),
        &["build", "--jobs=4", "--trace=events", "--plain"],
    ));
    for (target, expected) in original {
        assert_eq!(
            fs::read(
                root.path()
                    .join(format!("target/xmlsquish/artifacts/{target}.pack"))
            )
            .unwrap(),
            expected,
            "tracing or warm hydration rebound directives for {target}"
        );
    }
}

#[test]
fn reachable_archive_planning_does_not_hide_malformed_local_source_units() {
    let root = project(
        "malformed-local-",
        "consumer",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:asset path='bytes.bin'/></xs:pack>",
    );
    fs::write(root.path().join("src/bytes.bin"), b"asset").unwrap();
    fs::write(
        root.path().join("src/uncalled.xml"),
        "<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:macro/></xs:module>",
    )
    .unwrap();
    let failed = cli(root.path(), &["build", "--message-format=json"]);
    assert!(
        !failed.status.success(),
        "lazy archive planning incorrectly suppressed validation of malformed local XML units"
    );
    assert!(
        !root
            .path()
            .join("target/xmlsquish/artifacts/main.pack")
            .exists()
    );
    let failures = diagnostics(&failed);
    let primary = failures
        .iter()
        .find_map(|diagnostic| {
            let primary = &diagnostic["primary"];
            primary["source"]
                .as_str()
                .filter(|source| source.contains("uncalled.xml"))
                .map(|_| primary)
        })
        .unwrap_or_else(|| {
            panic!(
                "frontend failure lost its actual local source span: {}",
                String::from_utf8_lossy(&failed.stdout)
            )
        });
    let start = primary["start"]
        .as_u64()
        .expect("frontend primary span start");
    let end = primary["end"].as_u64().expect("frontend primary span end");
    let source_len = fs::read(root.path().join("src/uncalled.xml"))
        .unwrap()
        .len() as u64;
    assert!(
        start < end && end <= source_len,
        "frontend primary span is not a real byte range in the malformed source"
    );
}

#[test]
fn conflicting_sopack_symbols_report_relocated_provider_primary_and_preserve_old_pack() {
    let mut archives = Vec::new();
    for package in ["first-library", "second-library"] {
        let provider = project(
            "collision-provider-",
            package,
            "sopack",
            "<xs:sopack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:import src='lib.xml'/></xs:sopack>",
        );
        let manifest = provider.path().join("xmlsquish.toml");
        let mut text = fs::read_to_string(&manifest).unwrap();
        text.push_str("\n[exports]\nmain = \"src/lib.xml\"\n");
        fs::write(manifest, text).unwrap();
        fs::write(provider.path().join("src/lib.xml"), format!("<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:collision'><xs:macro name='m:shared'><xs:asset path='owned.bin' name='{package}.bin'/></xs:macro></xs:module>")).unwrap();
        fs::write(provider.path().join("src/owned.bin"), package.as_bytes()).unwrap();
        success(&cli(provider.path(), &["build", "--plain"]));
        archives.push((
            package,
            product(provider.path(), "sopack"),
            provider.path().to_string_lossy().into_owned(),
        ));
        provider.close().unwrap();
    }
    let consumer = project(
        "collision-consumer-",
        "consumer",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:asset path='baseline.bin'/></xs:pack>",
    );
    fs::write(consumer.path().join("src/baseline.bin"), b"OLD PRODUCT").unwrap();
    success(&cli(consumer.path(), &["build", "--plain"]));
    let old = product(consumer.path(), "pack");
    for (package, archive, _) in &archives {
        let file = format!("{package}.sopack");
        fs::write(consumer.path().join(&file), archive).unwrap();
        success(&cli(
            consumer.path(),
            &["add", package, "--path", &file, "--plain"],
        ));
    }
    fs::write(consumer.path().join("src/main.xml"), "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:collision'><xs:import src='pkg:first-library/main'/><xs:import src='pkg:second-library/main'/><xs:expand ref='m:shared'/></xs:pack>").unwrap();
    let failed = cli(
        consumer.path(),
        &["build", "--trace=events", "--message-format=json"],
    );
    assert!(
        !failed.status.success(),
        "two providers silently overwrote the same macro symbol"
    );
    let failures = diagnostics(&failed);
    let duplicate = failures
        .iter()
        .find(|diagnostic| {
            diagnostic["code"] == "MGB070"
                && format!("{} {}", diagnostic["message"], diagnostic["help"]).contains("LNK023")
        })
        .unwrap_or_else(|| {
            panic!(
                "duplicate symbol failure lost stable MGB070 code or compiler LNK023 cause: {}",
                String::from_utf8_lossy(&failed.stdout)
            )
        });
    let source = duplicate["primary"]["source"]
        .as_str()
        .expect("duplicate provider primary source identity");
    assert!(
        source.contains("sopack") && source.contains("lib.xml"),
        "duplicate provider identity was not relocated: {source}"
    );
    let description = format!("{} {}", duplicate["message"], duplicate["help"]);
    assert!(
        description.contains("urn:test:collision") && description.contains("shared"),
        "duplicate macro QName is not helpful to the caller: {duplicate}"
    );
    let transcript = String::from_utf8_lossy(&failed.stdout);
    assert!(
        archives
            .iter()
            .all(|(_, _, root)| !transcript.contains(root)),
        "diagnostic captured a deleted producer checkout"
    );
    assert_eq!(
        product(consumer.path(), "pack"),
        old,
        "failed duplicate-symbol link replaced the old public pack"
    );
}

#[test]
fn shipped_v1_2_library_with_legacy_provider_namespaces_remains_importable_after_relocation() {
    let fixture = std::env::var_os("XMLSQUISH_LEGACY_SOPACK_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1.2.0-library.sopack")
        });
    let archive = fs::read(&fixture).unwrap_or_else(|error| panic!(
        "required shipped-v1.2.0 compatibility fixture missing at {}: {error}; generate with the released writer, never the candidate writer", fixture.display()));
    assert!(
        members(&archive)
            .keys()
            .any(|name| name.contains("_providers/")),
        "legacy fixture must exercise old foreign-provider namespace spelling"
    );
    let consumers = [
        project(
            "legacy-a-",
            "consumer",
            "pack",
            "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:legacy-library'><xs:import src='pkg:legacy-library/main'/><xs:expand ref='m:bundle'/></xs:pack>",
        ),
        project(
            "legacy-b-",
            "consumer",
            "pack",
            "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:legacy-library'><xs:import src='pkg:legacy-library/main'/><xs:expand ref='m:bundle'/></xs:pack>",
        ),
    ];
    for consumer in &consumers {
        fs::write(consumer.path().join("legacy.sopack"), &archive).unwrap();
        fs::write(
            consumer.path().join("src/owned.bin"),
            b"CALLER NOT LEGACY PROVIDER",
        )
        .unwrap();
        success(&cli(
            consumer.path(),
            &[
                "add",
                "legacy-library",
                "--path",
                "legacy.sopack",
                "--plain",
            ],
        ));
        success(&cli(
            consumer.path(),
            &["build", "--trace=events", "--plain"],
        ));
        assert_eq!(
            members(&product(consumer.path(), "pack")),
            BTreeMap::from([("legacy.bin".to_owned(), b"legacy\x00\xffprovider".to_vec())])
        );
    }
    assert_eq!(
        product(consumers[0].path(), "pack"),
        product(consumers[1].path(), "pack")
    );
}
