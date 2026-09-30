//! Independent process-boundary acceptance contracts for v1.2 distribution.
//! Fixtures stay in repository `.temp`; ZIP inspection does not reuse backend code.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

thread_local! {
    /// Retains process evidence per test thread without creating project inputs.
    static CLI_EVIDENCE: RefCell<BTreeMap<PathBuf, String>> = const { RefCell::new(BTreeMap::new()) };
}

/// Runs the real CLI with inherited tracing disabled unless explicitly requested.
fn cli(root: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_xmlsquish"))
        .current_dir(root)
        .env_remove("XMLSQUISH_TRACE")
        .args(args)
        .output()
        .expect("start xmlsquish");
    CLI_EVIDENCE.with(|evidence| {
        evidence.borrow_mut().insert(
            root.to_path_buf(),
            format!(
                "cwd={}\nargs={args:?}\nexit={:?}\nstdout={}\nstderr={}",
                root.display(),
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        );
    });
    output
}

/// Keeps temporary project trees on the repository volume.
fn project(label: &str, backend: &str, body: &str) -> tempfile::TempDir {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join(".temp/pack-tests");
    fs::create_dir_all(&scratch).unwrap();
    let root = tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(scratch)
        .unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("xmlsquish.toml"), format!(
        "manifest-version = 1\n[package]\nname = \"fixture\"\nversion = \"1.0.0\"\n[target.chat]\nentry = \"src/main.xml\"\nbackend = \"{backend}\"\n"
    )).unwrap();
    fs::write(root.path().join("src/main.xml"), body).unwrap();
    root
}

/// Preserves full diagnostics for failed expected-success assertions.
fn success(output: Output) {
    assert!(
        output.status.success(),
        "exit={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Requires a rejected operation without coupling to unstable diagnostic wording.
fn failure(output: Output) {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Reads the user-facing product rather than private generation/cache files.
fn artifact(root: &Path, suffix: &str) -> Vec<u8> {
    let path = root.join(format!("target/xmlsquish/artifacts/chat.{suffix}"));
    fs::read(&path).unwrap_or_else(|error| {
        let evidence = CLI_EVIDENCE.with(|evidence| evidence.borrow().get(root).cloned().unwrap_or_default());
        panic!("cannot read expected product {}: {error}\nartifact directory: {}\nproject metadata directory: {}\nlast CLI invocation:\n{evidence}",
            path.display(), directory_contents(&root.join("target/xmlsquish/artifacts")), directory_contents(&root.join("target/xmlsquish")));
    })
}

/// Reports actual published filenames when a successful CLI omitted its product.
fn directory_contents(path: &Path) -> String {
    match fs::read_dir(path) {
        Ok(entries) => {
            let mut entries: Vec<_> = entries
                .map(|entry| match entry {
                    Ok(entry) => format!(
                        "{} ({:?})",
                        entry.file_name().to_string_lossy(),
                        entry.file_type()
                    ),
                    Err(error) => format!("directory entry error: {error}"),
                })
                .collect();
            entries.sort();
            format!("{}: {entries:?}", path.display())
        }
        Err(error) => format!("{}: {error}", path.display()),
    }
}

/// Reads a checked little-endian ZIP field.
fn word(bytes: &[u8], offset: usize) -> usize {
    usize::from(u16::from_le_bytes(
        bytes[offset..offset + 2].try_into().unwrap(),
    ))
}

/// Reads a checked little-endian ZIP field.
fn dword(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Computes the ZIP CRC independently of the implementation under test.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// Validates the agreed canonical STORED ZIP structure and extracts raw members.
/// Local headers, central headers and CRCs are checked without using archive APIs.
fn zip_members(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    assert!(bytes.len() >= 22, "missing ZIP end record");
    let end = bytes.len() - 22;
    assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
    assert_eq!(word(bytes, end + 4), 0, "multi-disk ZIP");
    assert_eq!(word(bytes, end + 6), 0);
    assert_eq!(word(bytes, end + 8), word(bytes, end + 10));
    assert_eq!(word(bytes, end + 20), 0, "volatile archive comment");
    let mut cursor = dword(bytes, end + 16) as usize;
    let start = cursor;
    let mut members = BTreeMap::new();
    let mut prior = String::new();
    for _ in 0..word(bytes, end + 10) {
        assert_eq!(&bytes[cursor..cursor + 4], b"PK\x01\x02");
        assert_eq!(word(bytes, cursor + 10), 0, "not STORED");
        assert_eq!(word(bytes, cursor + 12), 0, "noncanonical DOS time");
        assert_eq!(word(bytes, cursor + 14), 33, "noncanonical DOS date");
        let size = dword(bytes, cursor + 24) as usize;
        assert_eq!(dword(bytes, cursor + 20) as usize, size);
        let name_len = word(bytes, cursor + 28);
        let extra_len = word(bytes, cursor + 30);
        let comment_len = word(bytes, cursor + 32);
        assert_eq!(extra_len, 0, "extra fields are not canonical");
        assert_eq!(comment_len, 0);
        let name = std::str::from_utf8(&bytes[cursor + 46..cursor + 46 + name_len])
            .unwrap()
            .to_owned();
        assert!(!name.starts_with('/') && !name.contains('\\') && !name.contains(':'));
        assert!(
            name.split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
        );
        assert!(prior < name, "members not sorted or duplicate: {name}");
        prior = name.clone();
        let local = dword(bytes, cursor + 42) as usize;
        assert_eq!(&bytes[local..local + 4], b"PK\x03\x04");
        assert_eq!(word(bytes, local + 8), 0);
        assert_eq!(word(bytes, local + 10), 0);
        assert_eq!(word(bytes, local + 12), 33);
        assert_eq!(word(bytes, local + 26), name_len);
        assert_eq!(&bytes[local + 30..local + 30 + name_len], name.as_bytes());
        let data_start = local + 30 + name_len + word(bytes, local + 28);
        let data = bytes[data_start..data_start + size].to_vec();
        assert_eq!(
            crc32(&data),
            dword(bytes, cursor + 16),
            "bad CRC for {name}"
        );
        assert_eq!(dword(bytes, local + 14), dword(bytes, cursor + 16));
        assert!(members.insert(name, data).is_none());
        cursor += 46 + name_len + extra_len + comment_len;
    }
    assert_eq!(cursor, end);
    assert_eq!(cursor - start, dword(bytes, end + 12) as usize);
    members
}

/// Binary fixtures deliberately cannot be decoded as XML or UTF-8.
const BINARY_ASSET: &[u8] = b"\x00\xff\xfe\r\nasset\x00bytes";

#[test]
fn pack_preserves_binary_assets_includes_entries_and_is_reproducible_across_roots() {
    let body = "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:asset path='blob.bin' name='z/blob.bin'/><xs:include path='instruction.xml' name='a/instruction.prompt'/></xs:pack>";
    let roots = [
        project("first-", "pack", body),
        project("relocated-", "pack", body),
    ];
    for (index, root) in roots.iter().enumerate() {
        let files: [(&str, &[u8]); 2] = [
            ("src/blob.bin", BINARY_ASSET),
            ("src/instruction.xml", b"<xs:entry xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><message> Hello   world </message></xs:entry>"),
        ];
        for offset in 0..2 {
            let (path, bytes) = files[(index + offset) % 2];
            fs::write(root.path().join(path), bytes).unwrap();
        }
        success(cli(root.path(), &["build", "--plain"]));
    }
    let first = artifact(roots[0].path(), "pack");
    assert_eq!(
        first,
        artifact(roots[1].path(), "pack"),
        "absolute root or file creation order leaks into ZIP"
    );
    let members = zip_members(&first);
    assert_eq!(members.len(), 2);
    assert_eq!(members["z/blob.bin"], BINARY_ASSET);
    assert_eq!(
        members["a/instruction.prompt"],
        b"<message> Hello world </message>"
    );
    success(cli(roots[0].path(), &["build", "--plain"]));
    assert_eq!(
        artifact(roots[0].path(), "pack"),
        first,
        "warm build changes product bytes"
    );
}

#[test]
fn existing_entry_prompt_bytes_are_unchanged() {
    let root = project(
        "prompt-",
        "squish",
        "<xs:entry xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><message> Hello   world </message></xs:entry>",
    );
    success(cli(root.path(), &["build", "--plain"]));
    assert_eq!(
        artifact(root.path(), "prompt"),
        b"<message> Hello world </message>"
    );
}

#[test]
fn include_module_and_sopack_include_are_rejected_without_publishing() {
    let cases = [
        ("pack", "<xs:include path='module.xml'/>", "pack"),
        ("sopack", "<xs:include path='instruction.xml'/>", "sopack"),
        (
            "sopack",
            "<xs:macro name='m:bad'><xs:include path='instruction.xml'/></xs:macro>",
            "sopack",
        ),
    ];
    for (backend, content, suffix) in cases {
        let root = project(
            "invalid-include-",
            backend,
            &format!(
                "<xs:{backend} xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:macros'>{content}</xs:{backend}>"
            ),
        );
        fs::write(
            root.path().join("src/module.xml"),
            "<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns'/>",
        )
        .unwrap();
        fs::write(root.path().join("src/instruction.xml"), "<xs:entry xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><message>Hello</message></xs:entry>").unwrap();
        failure(cli(root.path(), &["build", "--plain"]));
        assert!(
            !root
                .path()
                .join(format!("target/xmlsquish/artifacts/chat.{suffix}"))
                .exists()
        );
    }
}

#[test]
fn unsafe_and_duplicate_pack_member_names_are_rejected() {
    for names in [
        vec!["../escape.bin"],
        vec!["/escape.bin"],
        vec!["same.bin", "same.bin"],
    ] {
        let directives: String = names
            .iter()
            .map(|name| format!("<xs:asset path='blob.bin' name='{name}'/>"))
            .collect();
        let root = project(
            "unsafe-member-",
            "pack",
            &format!(
                "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'>{directives}</xs:pack>"
            ),
        );
        fs::write(root.path().join("src/blob.bin"), BINARY_ASSET).unwrap();
        failure(cli(root.path(), &["build", "--plain"]));
        assert!(
            !root
                .path()
                .join("target/xmlsquish/artifacts/chat.pack")
                .exists()
        );
    }
}

/// Lists persistent traces without inspecting ordinary build metadata.
fn traces(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let directory = root.join("target/xmlsquish/metadata/traces");
    if !directory.exists() {
        return BTreeMap::new();
    }
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read(path).unwrap(),
            )
        })
        .collect()
}

#[test]
fn tracing_is_opt_in_persistent_and_does_not_change_products() {
    let root = project(
        "telemetry-",
        "squish",
        "<xs:entry xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><message>Hello</message></xs:entry>",
    );
    success(cli(root.path(), &["build", "--plain"]));
    assert!(
        traces(root.path()).is_empty(),
        "default invocation persisted telemetry"
    );
    let prompt = artifact(root.path(), "prompt");
    for _ in 0..2 {
        success(cli(root.path(), &["build", "--trace=events", "--plain"]));
        assert_eq!(artifact(root.path(), "prompt"), prompt);
    }
    let recorded = traces(root.path());
    assert_eq!(
        recorded.len(),
        2,
        "a new invocation must not overwrite a previous trace"
    );
    for bytes in recorded.values() {
        let records: Vec<serde_json::Value> = bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert!(records.len() >= 3);
        assert_eq!(records.first().unwrap()["kind"], "command_start");
        assert_eq!(records.last().unwrap()["kind"], "command_finish");
        assert_eq!(records.last().unwrap()["data"]["exit_code"], 0);
        let invocation = &records[0]["invocation"];
        assert!(
            records
                .iter()
                .all(|record| &record["invocation"] == invocation)
        );
        assert!(
            records.iter().any(|record| record["kind"] == "event"),
            "events mode omitted end-to-end lifecycle events"
        );
        let times: Vec<u64> = records
            .iter()
            .map(|record| record["elapsed_us"].as_u64().unwrap())
            .collect();
        assert!(times.windows(2).all(|pair| pair[0] <= pair[1]));
    }
    success(cli(root.path(), &["build", "--trace=off", "--plain"]));
    assert_eq!(
        traces(root.path()),
        recorded,
        "disabled tracing changed previous run metadata"
    );
}

/// Writes a provider whose public macro emits an asset owned by its source unit.
fn library(label: &str) -> tempfile::TempDir {
    let root = project(
        label,
        "sopack",
        "<xs:sopack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:import src='lib/macros.xml'/></xs:sopack>",
    );
    let manifest = root.path().join("xmlsquish.toml");
    let mut text = fs::read_to_string(&manifest)
        .unwrap()
        .replace("name = \"fixture\"", "name = \"library\"");
    text.push_str("\n[exports]\nmain = \"src/lib/macros.xml\"\n");
    fs::write(manifest, text).unwrap();
    fs::create_dir(root.path().join("src/lib")).unwrap();
    fs::write(root.path().join("src/lib/macros.xml"), "<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:library'><xs:import src='helper.xml'/><xs:macro name='m:bundle'><xs:expand ref='m:asset'/></xs:macro></xs:module>").unwrap();
    fs::write(root.path().join("src/lib/helper.xml"), "<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:library'><xs:macro name='m:asset'><xs:asset path='owned.bin' name='provider.bin'/></xs:macro></xs:module>").unwrap();
    fs::write(root.path().join("src/lib/owned.bin"), BINARY_ASSET).unwrap();
    root
}

#[test]
fn sopack_is_reproducible_relocated_and_uses_provider_assets_without_producer_sources() {
    let producers = [library("provider-a-"), library("provider-b-")];
    for producer in &producers {
        success(cli(producer.path(), &["build", "--plain"]));
    }
    let archive = artifact(producers[0].path(), "sopack");
    assert_eq!(
        artifact(producers[1].path(), "sopack"),
        archive,
        "producer checkout location leaks into SOPack"
    );
    let members = zip_members(&archive);
    assert!(!members.is_empty());
    assert!(
        !members
            .keys()
            .any(|name| name.ends_with(".prompt") || name.ends_with(".pack")),
        "SOPack contains a finished product"
    );
    let consumer = project(
        "consumer-",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:library'><xs:import src='pkg:library/main'/><xs:expand ref='m:bundle'/></xs:pack>",
    );
    fs::write(consumer.path().join("library.sopack"), &archive).unwrap();
    fs::write(
        consumer.path().join("src/owned.bin"),
        b"CALLER BYTES MUST NOT BE USED",
    )
    .unwrap();
    for producer in producers {
        producer.close().unwrap();
    }
    success(cli(
        consumer.path(),
        &["add", "library", "--path", "library.sopack", "--plain"],
    ));
    let manifest = fs::read_to_string(consumer.path().join("xmlsquish.toml")).unwrap();
    assert!(
        manifest.contains("sopack"),
        "add did not record an immutable SOPack dependency"
    );
    success(cli(consumer.path(), &["build", "--plain"]));
    assert_eq!(
        zip_members(&artifact(consumer.path(), "pack"))["provider.bin"],
        BINARY_ASSET
    );

    let manifest_before = fs::read(consumer.path().join("xmlsquish.toml")).unwrap();
    let lock_before = fs::read(consumer.path().join("xmlsquish.lock")).unwrap();
    let product_before = artifact(consumer.path(), "pack");
    let mut corrupted = archive.clone();
    corrupted.push(0);
    fs::write(consumer.path().join("library.sopack"), corrupted).unwrap();
    failure(cli(consumer.path(), &["build", "--plain"]));
    assert_eq!(
        fs::read(consumer.path().join("xmlsquish.toml")).unwrap(),
        manifest_before
    );
    assert_eq!(
        fs::read(consumer.path().join("xmlsquish.lock")).unwrap(),
        lock_before,
        "archive drift was silently ratified"
    );
    assert_eq!(
        artifact(consumer.path(), "pack"),
        product_before,
        "failed build destroyed prior published pack"
    );
    fs::write(consumer.path().join("library.sopack"), archive).unwrap();
    success(cli(consumer.path(), &["remove", "library", "--plain"]));
    assert!(
        !fs::read_to_string(consumer.path().join("xmlsquish.toml"))
            .unwrap()
            .contains("library.sopack")
    );
    failure(cli(consumer.path(), &["build", "--plain"]));
}

#[test]
fn malformed_sopack_add_is_atomic() {
    let root = project(
        "bad-library-",
        "squish",
        "<xs:entry xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><message>Hello</message></xs:entry>",
    );
    success(cli(root.path(), &["build", "--plain"]));
    let manifest = fs::read(root.path().join("xmlsquish.toml")).unwrap();
    let lock = fs::read(root.path().join("xmlsquish.lock")).unwrap();
    fs::write(root.path().join("invalid.sopack"), b"not a ZIP library").unwrap();
    failure(cli(
        root.path(),
        &["add", "invalid", "--path", "invalid.sopack", "--plain"],
    ));
    assert_eq!(
        fs::read(root.path().join("xmlsquish.toml")).unwrap(),
        manifest
    );
    assert_eq!(fs::read(root.path().join("xmlsquish.lock")).unwrap(), lock);
}

#[test]
fn included_entries_keep_independent_module_symbol_scopes() {
    let root = project(
        "include-scopes-",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:include path='first.xml'/><xs:include path='second.xml'/></xs:pack>",
    );
    for (name, value) in [("first", "FIRST"), ("second", "SECOND")] {
        fs::write(root.path().join(format!("src/{name}.xml")), format!("<xs:entry xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:scope'><xs:import src='{name}-macros.xml'/><message><xs:expand ref='m:value'/></message></xs:entry>")).unwrap();
        fs::write(root.path().join(format!("src/{name}-macros.xml")), format!("<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:scope'><xs:macro name='m:value'>{value}</xs:macro></xs:module>")).unwrap();
    }
    success(cli(root.path(), &["build", "--plain"]));
    let members = zip_members(&artifact(root.path(), "pack"));
    assert_eq!(members["first.prompt"], b"<message> FIRST </message>");
    assert_eq!(members["second.prompt"], b"<message> SECOND </message>");
}

#[test]
fn asset_only_mutation_invalidates_warm_pack_product() {
    let root = project(
        "asset-invalidation-",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:asset path='blob.bin'/></xs:pack>",
    );
    fs::write(root.path().join("src/blob.bin"), b"FIRST").unwrap();
    success(cli(root.path(), &["build", "--plain"]));
    assert_eq!(
        zip_members(&artifact(root.path(), "pack"))["blob.bin"],
        b"FIRST"
    );
    fs::write(root.path().join("src/blob.bin"), b"SECOND").unwrap();
    success(cli(root.path(), &["build", "--plain"]));
    assert_eq!(
        zip_members(&artifact(root.path(), "pack"))["blob.bin"],
        b"SECOND",
        "asset bytes are missing from the build action inputs"
    );
}

#[test]
fn included_entry_only_mutation_invalidates_warm_pack_product() {
    let root = project(
        "include-invalidation-",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:include path='instruction.xml'/></xs:pack>",
    );
    for value in ["FIRST", "SECOND"] {
        fs::write(root.path().join("src/instruction.xml"), format!("<xs:entry xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><message>{value}</message></xs:entry>")).unwrap();
        success(cli(root.path(), &["build", "--plain"]));
        assert_eq!(
            zip_members(&artifact(root.path(), "pack"))["instruction.prompt"],
            format!("<message> {value} </message>").as_bytes(),
            "included entry IR is missing from the backend action inputs"
        );
    }
}

#[test]
fn formatted_module_asset_macro_accepts_xml_whitespace_comments_and_processing_instructions() {
    let root = project(
        "pretty-asset-macro-",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:pretty'><xs:import src='macros.xml'/><xs:expand ref='m:bundle'/></xs:pack>",
    );
    fs::write(root.path().join("src/macros.xml"), "<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:pretty'>\n  <xs:macro name='m:bundle'>\n    <!-- Asset documentation is not a product member. -->\n    <?asset-owner provider?>\n    <xs:asset path='owned.bin' name='pretty.bin'/>\n  </xs:macro>\n</xs:module>").unwrap();
    fs::write(root.path().join("src/owned.bin"), BINARY_ASSET).unwrap();
    success(cli(root.path(), &["build", "--plain"]));
    let members = zip_members(&artifact(root.path(), "pack"));
    assert_eq!(members.len(), 1);
    assert_eq!(members["pretty.bin"], BINARY_ASSET);
}

/// Exercises path identity independently of package names, versions and source basenames.
fn distinct_dependency_sources(same_package_name: bool) {
    let provider = project(
        "multi-package-provider-",
        "sopack",
        "<xs:sopack xmlns:xs='https://xmlsquish.moesegfault.dev/ns'><xs:import src='facade.xml'/></xs:sopack>",
    );
    let manifest = provider.path().join("xmlsquish.toml");
    let mut text = fs::read_to_string(&manifest)
        .unwrap()
        .replace("name = \"fixture\"", "name = \"library\"");
    text.push_str("\n[exports]\nmain = \"src/facade.xml\"\n");
    fs::write(manifest, text).unwrap();
    fs::write(provider.path().join("src/facade.xml"), "<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:facade' xmlns:l='urn:test:left' xmlns:r='urn:test:right'><xs:import src='pkg:left/main'/><xs:import src='pkg:right/main'/><xs:macro name='m:both'><xs:expand ref='l:value'/><xs:expand ref='r:value'/></xs:macro></xs:module>").unwrap();
    for name in ["left", "right"] {
        let package_name = if same_package_name { "library" } else { name };
        let package = provider.path().join(name);
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(package.join("xmlsquish.toml"), format!("manifest-version = 1\n[package]\nname = \"{package_name}\"\nversion = \"1.0.0\"\n[exports]\nmain = \"src/lib.xml\"\n")).unwrap();
        fs::write(package.join("src/lib.xml"), format!("<xs:module xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:{name}'><xs:macro name='m:value'><xs:asset path='owned.bin' name='{name}.bin'/></xs:macro></xs:module>")).unwrap();
        fs::write(package.join("src/owned.bin"), name.as_bytes()).unwrap();
        success(cli(
            provider.path(),
            &[
                "add",
                package_name,
                "--rename",
                name,
                "--path",
                name,
                "--plain",
            ],
        ));
    }
    success(cli(provider.path(), &["build", "--plain"]));
    let archive = artifact(provider.path(), "sopack");
    let consumer = project(
        "multi-package-consumer-",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:facade'><xs:import src='pkg:library/main'/><xs:expand ref='m:both'/></xs:pack>",
    );
    fs::write(consumer.path().join("library.sopack"), archive).unwrap();
    provider.close().unwrap();
    success(cli(
        consumer.path(),
        &["add", "library", "--path", "library.sopack", "--plain"],
    ));
    success(cli(consumer.path(), &["build", "--plain"]));
    let members = zip_members(&artifact(consumer.path(), "pack"));
    assert_eq!(members["left.bin"], b"left");
    assert_eq!(members["right.bin"], b"right");
}

#[test]
fn sopack_retains_distinct_dependency_sources_with_identical_relative_paths() {
    distinct_dependency_sources(false);
}

#[test]
fn sopack_retains_same_name_version_dependency_packages_and_root_without_identity_collision() {
    distinct_dependency_sources(true);
}

#[test]
fn sopack_root_macros_are_publicly_importable_after_relocation() {
    let provider = project(
        "root-macro-provider-",
        "sopack",
        "<xs:sopack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:root-library'><xs:macro name='m:bundle'><xs:asset path='owned.bin' name='root-provider.bin'/></xs:macro></xs:sopack>",
    );
    let manifest = provider.path().join("xmlsquish.toml");
    let mut text = fs::read_to_string(&manifest)
        .unwrap()
        .replace("name = \"fixture\"", "name = \"library\"");
    text.push_str("\n[exports]\nmain = \"src/main.xml\"\n");
    fs::write(manifest, text).unwrap();
    fs::write(provider.path().join("src/owned.bin"), BINARY_ASSET).unwrap();
    success(cli(provider.path(), &["build", "--plain"]));
    let archive = artifact(provider.path(), "sopack");
    let consumer = project(
        "root-macro-consumer-",
        "pack",
        "<xs:pack xmlns:xs='https://xmlsquish.moesegfault.dev/ns' xmlns:m='urn:test:root-library'><xs:import src='pkg:library/main'/><xs:expand ref='m:bundle'/></xs:pack>",
    );
    fs::write(consumer.path().join("library.sopack"), archive).unwrap();
    fs::write(consumer.path().join("src/owned.bin"), b"NOT PROVIDER").unwrap();
    provider.close().unwrap();
    success(cli(
        consumer.path(),
        &["add", "library", "--path", "library.sopack", "--plain"],
    ));
    success(cli(consumer.path(), &["build", "--plain"]));
    assert_eq!(
        zip_members(&artifact(consumer.path(), "pack"))["root-provider.bin"],
        BINARY_ASSET
    );
}
