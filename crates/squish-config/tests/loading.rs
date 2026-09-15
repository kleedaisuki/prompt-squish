use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use squish_config::{
    ColorPolicy, ConfigError, ConfigHome, ConfigLayer, ConfigLoader, MessageFormat, Verbosity,
};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(".temp")
            .join("squish-config-tests");
        let path = root.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("test path has parent")).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn precedence_is_defaults_user_workspace_then_ordered_cli() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    let workspace = tmp.path().join("work");
    write(
        &home.join("config.toml"),
        "[build]\njobs=2\n[term]\ncolor='always'\n",
    );
    write(
        &workspace.join(".xmlsquish/config.toml"),
        "[build]\njobs=3\n[term]\ncolor='never'\n",
    );

    let loaded = ConfigLoader::new(ConfigHome::new(&home))
        .workspace_root(&workspace)
        .cli_overrides(["build.jobs=4", "build.jobs=5", "term.color='auto'"])
        .load()
        .unwrap();

    assert_eq!(loaded.config.build.jobs, 5);
    assert_eq!(loaded.config.term.color, ColorPolicy::Auto);
    let chain = loaded.explain("build.jobs").unwrap();
    assert_eq!(chain.len(), 5);
    assert!(matches!(chain[0].layer, ConfigLayer::Defaults));
    assert!(matches!(chain[4].layer, ConfigLayer::Cli { index: 1 }));
}

#[test]
fn relative_paths_use_their_declaring_file_and_explicit_cli_base() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    let workspace = tmp.path().join("work");
    let cli = tmp.path().join("invocation");
    write(
        &home.join("config.toml"),
        "[source]\ncache-root='user-cache'\n",
    );
    write(
        &workspace.join(".xmlsquish/config.toml"),
        "[manager]\nstorage-root='manager-state'\n",
    );
    let loaded = ConfigLoader::new(ConfigHome::new(&home))
        .workspace_root(&workspace)
        .cli_base(&cli)
        .cli_overrides(["source.cache-root='cli-cache'"])
        .load()
        .unwrap();
    assert_eq!(loaded.config.source.cache_root, cli.join("cli-cache"));
    assert_eq!(
        loaded.config.manager.storage_root,
        workspace.join(".xmlsquish").join("manager-state")
    );
}

#[test]
fn registry_layers_merge_by_stable_alias_and_keep_identity_typed() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    let workspace = tmp.path().join("work");
    write(
        &home.join("config.toml"),
        "[registries.corp]\nid='https://PACKAGES.example:443/a/../xmlsquish'\nindex='sparse+https://old.example/index/'\nauth-scope='corp-read'\n",
    );
    write(
        &workspace.join(".xmlsquish/config.toml"),
        "[registries.corp]\nindex='sparse+https://MIRROR.example:443/a/../index/'\n",
    );
    let loaded = ConfigLoader::new(ConfigHome::new(&home))
        .workspace_root(workspace)
        .load()
        .unwrap();
    let registry = &loaded.config.registries["corp"];
    assert_eq!(registry.id.as_str(), "https://packages.example/xmlsquish");
    assert_eq!(
        registry.index.as_str(),
        "sparse+https://mirror.example/index/"
    );
    assert_eq!(registry.auth_scope.as_str(), "corp-read");
    assert_eq!(loaded.explain("registries.corp.index").unwrap().len(), 2);
    assert!(matches!(
        loaded.explain("registries.corp.auth-scope").unwrap()[0].layer,
        ConfigLayer::User(_)
    ));
}

#[test]
fn same_stable_id_aliases_share_a_domain_when_settings_match() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    let workspace = tmp.path().join("work");
    write(
        &home.join("config.toml"),
        "[registries.one]\nid='https://id.example/r'\nindex='sparse+https://index.example/'\nauth-scope='read'\n",
    );
    write(
        &workspace.join(".xmlsquish/config.toml"),
        "[registries.two]\nid='https://ID.example:443/r'\nindex='sparse+https://INDEX.example:443/'\nauth-scope='read'\n",
    );
    let loaded = ConfigLoader::new(ConfigHome::new(&home))
        .workspace_root(&workspace)
        .load()
        .unwrap();
    assert_eq!(loaded.config.registries.len(), 2);
    assert_eq!(
        loaded.config.registries["one"].id,
        loaded.config.registries["two"].id
    );
}

#[test]
fn same_stable_id_rejects_conflicting_endpoint_or_auth_at_later_layer() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    let workspace = tmp.path().join("work");
    write(
        &home.join("config.toml"),
        "[registries.one]\nid='https://id.example/r'\nindex='sparse+https://index.example/'\nauth-scope='read'\n",
    );
    write(
        &workspace.join(".xmlsquish/config.toml"),
        "[registries.two]\nid='https://id.example/r'\nindex='sparse+https://other.example/'\nauth-scope='read'\n",
    );
    match ConfigLoader::new(ConfigHome::new(&home))
        .workspace_root(&workspace)
        .load()
        .unwrap_err()
    {
        ConfigError::RegistryCollision { kind, location, .. } => {
            assert_eq!(kind, "stable-id endpoint");
            assert!(matches!(location.layer, ConfigLayer::Workspace(_)));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    write(
        &workspace.join(".xmlsquish/config.toml"),
        "[registries.two]\nid='https://id.example/r'\nindex='sparse+https://index.example/'\nauth-scope='write'\n",
    );
    assert!(matches!(
        ConfigLoader::new(ConfigHome::new(&home))
            .workspace_root(&workspace)
            .load(),
        Err(ConfigError::RegistryCollision {
            kind: "stable-id auth scope",
            ..
        })
    ));
}

#[test]
fn case_folded_alias_tokens_are_ambiguous() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");

    write(
        &home.join("config.toml"),
        "[registries.Corp]\nid='https://id.example/a'\nindex='sparse+https://one.example/'\n[registries.corp]\nid='https://id.example/b'\nindex='sparse+https://two.example/'\n",
    );
    assert!(matches!(
        ConfigLoader::new(ConfigHome::new(&home)).load(),
        Err(ConfigError::RegistryCollision { kind: "alias", .. })
    ));
}

#[test]
fn spans_are_exact_original_value_and_key_slices() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    let valid = "# jobs = 'comment decoy'\n[build]\njobs = 7 # value decoy: jobs\n";
    write(&home.join("config.toml"), valid);
    let loaded = ConfigLoader::new(ConfigHome::new(&home)).load().unwrap();
    let entry = loaded.explain("build.jobs").unwrap().last().unwrap();
    assert_eq!(&valid[entry.span.clone().unwrap()], "7");

    let unknown_source = "# colour in comment\n[term]\nverbosity='normal' # colour in value/comment\ncolour='always'\n";
    write(&home.join("config.toml"), unknown_source);
    match ConfigLoader::new(ConfigHome::new(&home))
        .load()
        .unwrap_err()
    {
        ConfigError::UnknownKey { location, .. } => {
            assert_eq!(&unknown_source[location.span.unwrap()], "colour");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let cli = "term.color='never'";
    let loaded = ConfigLoader::new(ConfigHome::new(home.join("absent")))
        .cli_overrides([cli])
        .load()
        .unwrap();
    let entry = loaded.explain("term.color").unwrap().last().unwrap();
    assert_eq!(&cli[entry.span.clone().unwrap()], "'never'");
}

#[test]
fn invalid_values_and_types_have_exact_spans() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    let bad_url =
        "[registries.bad]\nid='http://wrong.example/'\nindex='sparse+https://index.example/'\n";
    write(&home.join("config.toml"), bad_url);
    match ConfigLoader::new(ConfigHome::new(&home))
        .load()
        .unwrap_err()
    {
        ConfigError::InvalidValue { location, .. } => {
            assert_eq!(&bad_url[location.span.unwrap()], "'http://wrong.example/'");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let empty_auth = "[registries.bad]\nid='https://id.example/'\nindex='sparse+https://index.example/'\nauth-scope=''\n";
    write(&home.join("config.toml"), empty_auth);
    match ConfigLoader::new(ConfigHome::new(&home))
        .load()
        .unwrap_err()
    {
        ConfigError::InvalidValue { location, .. } => {
            assert_eq!(&empty_auth[location.span.unwrap()], "''");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let bad_type = "[build]\njobs='many'\n";
    write(&home.join("config.toml"), bad_type);
    match ConfigLoader::new(ConfigHome::new(&home))
        .load()
        .unwrap_err()
    {
        ConfigError::Parse { location, .. } => {
            assert_eq!(&bad_type[location.span.unwrap()], "'many'");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn unknown_secret_and_duplicate_keys_report_the_source_layer_and_span() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    write(&home.join("config.toml"), "[term]\ncolour='always'\n");
    match ConfigLoader::new(ConfigHome::new(&home))
        .load()
        .unwrap_err()
    {
        ConfigError::UnknownKey { key, location } => {
            assert_eq!(key, "term.colour");
            assert!(matches!(location.layer, ConfigLayer::User(_)));
            assert!(location.span.is_some());
        }
        other => panic!("unexpected error: {other:?}"),
    }
    write(
        &home.join("config.toml"),
        "[registries.corp]\nid='https://id.example/'\nindex='sparse+https://idx.example/'\ntoken='secret'\n",
    );
    assert!(
        matches!(ConfigLoader::new(ConfigHome::new(&home)).load(), Err(ConfigError::UnknownKey { key, .. }) if key == "registries.corp.token")
    );
    write(&home.join("config.toml"), "[build]\njobs=1\njobs=2\n");
    assert!(
        matches!(ConfigLoader::new(ConfigHome::new(&home)).load(), Err(ConfigError::Parse { location, .. }) if location.span.is_some())
    );
}

#[test]
fn bad_override_is_located_and_valid_overrides_are_typed() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    let bad = ConfigLoader::new(ConfigHome::new(&home))
        .cli_overrides(["term.progress='never'", "term.message-formt='json'"])
        .load();
    assert!(matches!(
        bad,
        Err(ConfigError::UnknownKey {
            location: squish_config::SourceLocation {
                layer: ConfigLayer::Cli { index: 1 },
                span: Some(_)
            },
            ..
        })
    ));
    let loaded = ConfigLoader::new(ConfigHome::new(&home))
        .cli_overrides([
            "term.message-format='json'",
            "term.verbosity='trace'",
            "build.keep-going=false",
        ])
        .load()
        .unwrap();
    assert_eq!(loaded.config.term.message_format, MessageFormat::Json);
    assert_eq!(loaded.config.term.verbosity, Verbosity::Trace);
    assert!(!loaded.config.build.keep_going);
}

#[test]
fn absent_and_empty_files_produce_complete_defaults() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    write(&home.join("config.toml"), "\n# intentionally empty\n");
    let loaded = ConfigLoader::new(ConfigHome::new(&home)).load().unwrap();
    assert_eq!(loaded.config.build.jobs, 0);
    assert!(loaded.config.build.keep_going);
    assert_eq!(loaded.config.source.cache_root, home.join("cache/sources"));
    assert_eq!(loaded.provenance.iter().count(), 8);
}

#[test]
fn incomplete_registry_is_rejected_after_all_layers() {
    let tmp = TestDir::new();
    let home = tmp.path().join("home");
    write(
        &home.join("config.toml"),
        "[registries.corp]\nid='https://id.example/'\n",
    );
    assert!(
        matches!(ConfigLoader::new(ConfigHome::new(&home)).load(), Err(ConfigError::InvalidValue { key, .. }) if key == "registries.corp.index")
    );
}
