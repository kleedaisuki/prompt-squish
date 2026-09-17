use super::*;
use squish_build::{OutputName, ProducedOutput};
use squish_protocol::ArtifactKind;
use std::{collections::HashMap, convert::Infallible, io::Read, sync::Mutex};

#[derive(Default)]
struct MemoryStore(Mutex<HashMap<Vec<u8>, Vec<u8>>>);
impl MemoryStore {
    fn insert(&self, bytes: &[u8]) -> Digest {
        let digest = Digest::new(
            DigestAlgorithm::Blake3,
            blake3::hash(bytes).as_bytes().to_vec(),
        )
        .unwrap();
        self.0
            .lock()
            .unwrap()
            .insert(digest.bytes().to_vec(), bytes.to_vec());
        digest
    }
}
impl BlobStore for MemoryStore {
    type Error = Infallible;
    fn copy_to(&self, digest: &Digest, sink: &mut dyn Write) -> Result<bool, Self::Error> {
        let guard = self.0.lock().unwrap();
        let Some(bytes) = guard.get(digest.bytes()) else {
            return Ok(false);
        };
        sink.write_all(bytes).unwrap();
        Ok(true)
    }
    fn write_from(&self, source: &mut dyn Read) -> Result<Digest, Self::Error> {
        let mut bytes = Vec::new();
        source.read_to_end(&mut bytes).unwrap();
        Ok(self.insert(&bytes))
    }
}

#[derive(Default)]
struct RecordingObserver(Mutex<Vec<PublishEvent>>);
impl RecordingObserver {
    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }

    fn events(&self) -> Vec<PublishEvent> {
        self.0.lock().unwrap().clone()
    }
}
impl PublishObserver for RecordingObserver {
    fn observe(&self, event: &PublishEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

struct CrashAt(DurablePoint);
impl PublishObserver for CrashAt {
    fn observe(&self, event: &PublishEvent) {
        if event == &PublishEvent::DurablePoint(self.0) {
            panic!("simulated process crash at {:?}", self.0);
        }
    }
}

fn publication(digest: Digest, size: u64, destination: &str) -> Publication {
    Publication {
        output: ProducedOutput {
            name: OutputName::new(destination).unwrap(),
            kind: ArtifactKind::Prompt,
            digest,
            size,
        },
        name: LogicalArtifactName::new(destination).unwrap(),
        destination: PublicationPath::new(destination).unwrap(),
    }
}

fn target(value: &str) -> PublicationTargetId {
    PublicationTargetId::new(value).unwrap()
}

fn read_member(
    publisher: &FileArtifactPublisher<MemoryStore>,
    generation: &CommittedGeneration,
    index: usize,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    assert!(matches!(
        publisher
            .read_generation_artifact(
                &generation.identity,
                &generation.artifacts[index].path,
                &mut bytes,
            )
            .unwrap(),
        ArtifactRead::Verified(_)
    ));
    bytes
}

#[test]
fn publishes_atomically_and_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let digest = store.insert(b"hello");
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    let request = publication(digest, 5, "nested/result.prompt");
    let artifact = publisher.publish_artifact(&request).unwrap();
    publisher.publish(&request).unwrap();
    assert_eq!(
        fs::read(root.path().join("nested/result.prompt")).unwrap(),
        b"hello"
    );
    assert_eq!(artifact.descriptor.digest, request.output.digest);
    assert!(!root.path().join(STATE_DIR).join(JOURNAL).exists());
}

#[test]
fn replaces_existing_file_and_verifies_sha256() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("result.prompt"), b"stale").unwrap();
    let store = MemoryStore::default();
    let bytes = b"fresh";
    let digest = Digest::new(DigestAlgorithm::Sha256, Sha256::digest(bytes).to_vec()).unwrap();
    store
        .0
        .lock()
        .unwrap()
        .insert(digest.bytes().to_vec(), bytes.to_vec());
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    publisher
        .publish(&publication(digest, bytes.len() as u64, "result.prompt"))
        .unwrap();
    assert_eq!(fs::read(root.path().join("result.prompt")).unwrap(), bytes);
}

#[test]
fn rejects_portable_lexical_escape_forms() {
    for path in [
        "../outside",
        "a/../outside",
        r"a\..\outside",
        "/absolute",
        r"C:\absolute",
        ".squish-publish/x",
    ] {
        assert!(parse_destination(path).is_err(), "accepted {path}");
    }
}

#[test]
fn rejects_case_collision_even_on_case_sensitive_hosts() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("Result.prompt"), b"old").unwrap();
    let store = MemoryStore::default();
    let digest = store.insert(b"new");
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    assert!(matches!(
        publisher.publish(&publication(digest, 3, "result.prompt")),
        Err(PublishError::AliasConflict(_))
    ));
}

#[test]
fn rejects_hardlink_alias() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("one"), b"old").unwrap();
    fs::hard_link(root.path().join("one"), root.path().join("two")).unwrap();
    let store = MemoryStore::default();
    let digest = store.insert(b"new");
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    assert!(matches!(
        publisher.publish(&publication(digest, 3, "two")),
        Err(PublishError::AliasConflict(_))
    ));
}

#[test]
fn verifies_complete_digest_and_size() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let digest = store.insert(b"content");
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    assert!(matches!(
        publisher.publish(&publication(digest, 6, "bad")),
        Err(PublishError::IntegrityMismatch)
    ));
}

#[test]
fn invalid_sources_leave_no_journal_and_do_not_poison_later_publications() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let valid_digest = store.insert(b"content");
    let missing_digest = Digest::new(DigestAlgorithm::Blake3, vec![0x5a; 32]).unwrap();
    let unsupported_digest =
        Digest::new(DigestAlgorithm::Other("future-hash".into()), vec![0x7b]).unwrap();
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    let failures = [
        publication(valid_digest.clone(), 6, "bad-size.prompt"),
        publication(missing_digest, 7, "missing.prompt"),
        publication(unsupported_digest, 7, "unsupported.prompt"),
    ];

    for (index, request) in failures.iter().enumerate() {
        let error = publisher.publish(request).unwrap_err();
        assert!(
            matches!(
                (index, error),
                (0, PublishError::IntegrityMismatch)
                    | (1, PublishError::MissingBlob)
                    | (2, PublishError::UnsupportedDigest(_))
            ),
            "unexpected failure for case {index}"
        );
        assert!(!publisher.state.join(JOURNAL).exists());
        assert!(!root.path().join(request.destination.as_str()).exists());

        let recovery_destination = format!("valid-after-{index}.prompt");
        publisher
            .publish(&publication(valid_digest.clone(), 7, &recovery_destination))
            .unwrap();
        assert_eq!(
            fs::read(root.path().join(recovery_destination)).unwrap(),
            b"content"
        );
        assert!(!publisher.state.join(JOURNAL).exists());
    }
}

#[test]
fn nested_directory_metadata_is_durable_before_journal_commit() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let digest = store.insert(b"nested");
    let observer = Arc::new(RecordingObserver::default());
    let publisher =
        FileArtifactPublisher::with_observer(root.path(), store, observer.clone()).unwrap();
    observer.clear();

    publisher
        .publish(&publication(digest, 6, "one/two/result.prompt"))
        .unwrap();

    let first = publisher.root.join("one");
    let second = first.join("two");
    let events = observer.events();
    let position = |expected: &PublishEvent| {
        events
            .iter()
            .position(|event| event == expected)
            .unwrap_or_else(|| panic!("missing event {expected:?} in {events:?}"))
    };
    let guarantee = if cfg!(unix) {
        DirectoryDurability::Synced
    } else {
        DirectoryDurability::Unsupported
    };
    let first_created = position(&PublishEvent::DirectoryCreated(first.clone()));
    let root_synced = position(&PublishEvent::DirectoryDurability {
        path: publisher.root.clone(),
        guarantee,
    });
    let second_created = position(&PublishEvent::DirectoryCreated(second));
    let first_synced = position(&PublishEvent::DirectoryDurability {
        path: first,
        guarantee,
    });
    // 首个状态目录持久性事件是 write_journal 的提交；后一个在替换目标后删除 journal。
    // The first state-directory durability event is write_journal's commit; the later
    // one removes the journal after destination replacement.
    let journal_committed = position(&PublishEvent::DirectoryDurability {
        path: publisher.state.clone(),
        guarantee,
    });

    assert!(first_created < root_synced);
    assert!(root_synced < second_created);
    assert!(second_created < first_synced);
    assert!(first_synced < journal_committed);
}

#[cfg(windows)]
#[test]
fn windows_atomic_replace_overwrites_an_existing_destination() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("existing.prompt");
    fs::write(&destination, b"old").unwrap();
    let mut temporary = NamedTempFile::new_in(root.path()).unwrap();
    temporary.write_all(b"new").unwrap();
    temporary.as_file().sync_all().unwrap();

    // 专门防止用 std::fs::rename 取代 persist_replace：前者不能覆盖 Windows 既有目标。
    // This guards against replacing persist_replace with std::fs::rename, which cannot
    // overwrite an existing Windows destination.
    persist_replace::<Infallible>(temporary, &destination).unwrap();
    assert_eq!(fs::read(destination).unwrap(), b"new");
}

#[test]
fn recovers_durable_journal_without_user_repair() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let digest = store.insert(b"recovered");
    fs::create_dir_all(root.path().join(STATE_DIR)).unwrap();
    let journal = Journal {
        destination: "out.prompt".into(),
        temporary: ".interrupted".into(),
        size: 9,
        digest,
    };
    fs::write(
        root.path().join(STATE_DIR).join(JOURNAL),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
    let _publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    assert_eq!(
        fs::read(root.path().join("out.prompt")).unwrap(),
        b"recovered"
    );
    assert!(!root.path().join(STATE_DIR).join(JOURNAL).exists());
}

#[test]
fn publishes_a_complete_multi_artifact_generation() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let prompt = store.insert(b"prompt-v1");
    let debug = store.insert(b"debug-v1");
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    let requests = [
        publication(prompt, 9, "app.prompt"),
        publication(debug, 8, "app.psdbg"),
    ];

    let target = target("app");
    let published = publisher.publish_generation(&target, &requests).unwrap();
    assert_eq!(
        publisher.current_generation(&target).unwrap(),
        Some(published.clone())
    );
    assert_eq!(published.artifacts.len(), 2);
    assert_eq!(read_member(&publisher, &published, 0), b"prompt-v1");
    assert_eq!(read_member(&publisher, &published, 1), b"debug-v1");
}

#[test]
fn separated_layout_exposes_only_stable_user_artifacts_and_cleans_stale_paths() {
    let root = tempfile::tempdir().unwrap();
    let artifacts = root.path().join("target/xmlsquish");
    let state = root.path().join("private/publication-state");
    let store = MemoryStore::default();
    let mut metadata = publication(
        store.insert(b"private map"),
        11,
        "target/xmlsquish/app.xsmap",
    );
    metadata.output.kind = ArtifactKind::Metadata;
    let publisher =
        FileArtifactPublisher::open_with_layout(&artifacts, &state, "target/xmlsquish", store)
            .unwrap();
    let first = publisher
        .publish_generation(
            &target("app"),
            &[
                publication(
                    publisher.store.insert(b"prompt-v1"),
                    9,
                    "target/xmlsquish/app.prompt",
                ),
                publication(
                    publisher.store.insert(b"debug-v1"),
                    8,
                    "target/xmlsquish/debug/app.psdbg",
                ),
                metadata,
            ],
        )
        .unwrap();

    assert_eq!(
        fs::read(artifacts.join("app.prompt")).unwrap(),
        b"prompt-v1"
    );
    assert_eq!(
        fs::read(artifacts.join("debug/app.psdbg")).unwrap(),
        b"debug-v1"
    );
    assert!(!artifacts.join("app.xsmap").exists());
    assert!(!artifacts.join("target").exists());
    assert!(!artifacts.join(STATE_DIR).exists());
    assert!(state.join(GENERATIONS).exists());
    let metadata_index = first
        .artifacts
        .iter()
        .position(|artifact| artifact.path.as_str().ends_with("app.xsmap"))
        .unwrap();
    assert_eq!(
        read_member(&publisher, &first, metadata_index),
        b"private map"
    );

    publisher
        .publish_generation(
            &target("app"),
            &[publication(
                publisher.store.insert(b"prompt-v2"),
                9,
                "target/xmlsquish/app.prompt",
            )],
        )
        .unwrap();
    assert_eq!(
        fs::read(artifacts.join("app.prompt")).unwrap(),
        b"prompt-v2"
    );
    assert!(!artifacts.join("debug/app.psdbg").exists());
    assert!(!artifacts.join("debug").exists());
}

#[test]
fn separated_layout_recovery_repairs_the_stable_projection() {
    let root = tempfile::tempdir().unwrap();
    let artifacts = root.path().join("target/xmlsquish");
    let state = root.path().join("private/publication-state");
    let store = MemoryStore::default();
    let publisher =
        FileArtifactPublisher::open_with_layout(&artifacts, &state, "target/xmlsquish", store)
            .unwrap();
    let generation = publisher
        .publish_generation(
            &target("app"),
            &[publication(
                publisher.store.insert(b"recover me"),
                10,
                "target/xmlsquish/app.prompt",
            )],
        )
        .unwrap();
    fs::remove_file(artifacts.join("app.prompt")).unwrap();
    fs::write(
        state.join(GENERATION_JOURNAL),
        serde_json::to_vec(&GenerationJournal {
            target_key: target_key(&generation.identity.target),
            generation_id: generation.identity.generation.to_hex(),
        })
        .unwrap(),
    )
    .unwrap();
    drop(publisher);

    let recovered = FileArtifactPublisher::open_with_layout(
        &artifacts,
        &state,
        "target/xmlsquish",
        MemoryStore::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read(artifacts.join("app.prompt")).unwrap(),
        b"recover me"
    );
    assert!(!recovered.state_root().join(GENERATION_JOURNAL).exists());
}

#[test]
fn separated_publishers_share_an_external_lock_that_does_not_pin_state() {
    let root = tempfile::tempdir().unwrap();
    let catalog = root.path().join("catalog");
    let lock = project_lock_path(&catalog);
    let first_state = catalog.join("target-state");
    let second_state = catalog.join("catalog-state");
    let first = FileArtifactPublisher::open_with_layout_and_lock(
        root.path().join("artifacts"),
        &first_state,
        Path::new(""),
        &lock,
        MemoryStore::default(),
    )
    .unwrap();
    let second = FileArtifactPublisher::open_with_layout_and_lock(
        root.path().join("catalog-artifacts"),
        &second_state,
        Path::new(""),
        &lock,
        MemoryStore::default(),
    )
    .unwrap();
    assert_eq!(first.lock_path(), second.lock_path());
    assert!(!first.lock_path().starts_with(&catalog));

    let renamed = catalog.join("target-state-renamed");
    first
        .with_lock(|_| {
            let contender = OpenOptions::new()
                .read(true)
                .write(true)
                .open(second.lock_path())?;
            assert!(contender.try_lock_exclusive().is_err());
            fs::rename(&first_state, &renamed)?;
            Ok(())
        })
        .unwrap();
    assert!(renamed.exists());
    assert!(!first_state.exists());
    assert!(second_state.exists());
}

#[test]
fn explicit_lock_constructor_does_not_create_roots_while_clean_holds_lock() {
    let root = tempfile::tempdir().unwrap();
    let catalog = root.path().join("catalog");
    let lock = project_lock_path(&catalog);
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .unwrap();
    held.lock_exclusive().unwrap();

    let artifacts = root.path().join("target/xmlsquish");
    let state = catalog.join("target-state");
    let thread_artifacts = artifacts.clone();
    let thread_state = state.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = FileArtifactPublisher::open_with_layout_and_lock(
            thread_artifacts,
            thread_state,
            "target/xmlsquish",
            lock,
            MemoryStore::default(),
        )
        .map(|_| ())
        .map_err(|error| error.to_string());
        done_tx.send(result).unwrap();
    });
    started_rx.recv().unwrap();
    assert!(
        done_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err()
    );
    assert!(!artifacts.exists());
    assert!(!state.exists());

    FileExt::unlock(&held).unwrap();
    done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    handle.join().unwrap();
    assert!(artifacts.exists());
    assert!(state.exists());
}

#[test]
fn pending_clean_blocks_cached_external_publisher_without_recreating_roots() {
    let root = tempfile::tempdir().unwrap();
    let artifacts = root.path().join("target/xmlsquish");
    let state = root.path().join("catalog/target-state");
    let lock = project_lock_path(root.path().join("catalog"));
    let publisher = FileArtifactPublisher::open_with_layout_and_lock(
        &artifacts,
        &state,
        "target/xmlsquish",
        &lock,
        MemoryStore::default(),
    )
    .unwrap();
    fs::remove_dir_all(&artifacts).unwrap();
    fs::remove_dir_all(&state).unwrap();
    let marker = publisher.lock_path().with_extension("clean.json");
    fs::write(&marker, b"pending").unwrap();
    let request = publication(
        publisher.store.insert(b"after clean"),
        11,
        "target/xmlsquish/app.prompt",
    );

    assert!(matches!(
        publisher.publish_generation(&target("app"), std::slice::from_ref(&request)),
        Err(PublishError::MaintenancePending(_))
    ));
    assert!(matches!(
        publisher.current_generation(&target("app")),
        Err(PublishError::MaintenancePending(_))
    ));
    assert!(!artifacts.exists());
    assert!(!state.exists());

    fs::remove_file(marker).unwrap();
    publisher
        .publish_generation(&target("app"), &[request])
        .unwrap();
    assert_eq!(
        fs::read(artifacts.join("app.prompt")).unwrap(),
        b"after clean"
    );
    assert!(state.exists());
}

#[test]
fn state_local_compatibility_api_ignores_clean_named_ordinary_file() {
    let root = tempfile::tempdir().unwrap();
    let publisher = FileArtifactPublisher::open(root.path(), MemoryStore::default()).unwrap();
    fs::write(
        publisher.lock_path().with_extension("clean.json"),
        b"ordinary",
    )
    .unwrap();
    publisher
        .publish(&publication(publisher.store.insert(b"ok"), 2, "out.prompt"))
        .unwrap();
    assert_eq!(fs::read(root.path().join("out.prompt")).unwrap(), b"ok");
}

#[test]
fn changed_epoch_supersedes_cached_publishers_and_a_new_publisher_accepts_it() {
    let root = tempfile::tempdir().unwrap();
    let catalog = root.path().join("catalog");
    let epoch_path = project_epoch_path(&catalog);
    let initial = read_project_epoch(&catalog).unwrap();
    let layout = ExternalPublisherLayout::new(
        root.path().join("artifacts"),
        catalog.join("state"),
        "target/xmlsquish",
        project_lock_path(&catalog),
    )
    .with_epoch(&epoch_path, initial);
    let publisher = FileArtifactPublisher::with_external_layout(
        layout,
        MemoryStore::default(),
        Arc::new(NoopObserver),
    )
    .unwrap();
    let next = initial.checked_next().unwrap();
    write_project_epoch(&epoch_path, &next).unwrap();
    fs::remove_dir_all(publisher.root()).unwrap();
    fs::remove_dir_all(publisher.state_root()).unwrap();
    let request = publication(
        publisher.store.insert(b"new epoch"),
        9,
        "target/xmlsquish/app.prompt",
    );

    assert!(matches!(
        publisher.publish_generation(&target("app"), std::slice::from_ref(&request)),
        Err(PublishError::Superseded(_))
    ));
    assert!(matches!(
        publisher.current_generation(&target("app")),
        Err(PublishError::Superseded(_))
    ));
    assert!(!publisher.root().exists());
    assert!(!publisher.state_root().exists());

    let fresh_layout = ExternalPublisherLayout::new(
        publisher.root(),
        publisher.state_root(),
        "target/xmlsquish",
        publisher.lock_path(),
    )
    .with_epoch(&epoch_path, read_project_epoch(&catalog).unwrap());
    let fresh = FileArtifactPublisher::with_external_layout(
        fresh_layout,
        MemoryStore::default(),
        Arc::new(NoopObserver),
    )
    .unwrap();
    let fresh_request = publication(
        fresh.store.insert(b"new epoch"),
        9,
        "target/xmlsquish/app.prompt",
    );
    fresh
        .publish_generation(&target("app"), &[fresh_request])
        .unwrap();
    assert_eq!(
        fs::read(fresh.root().join("app.prompt")).unwrap(),
        b"new epoch"
    );
}

#[test]
fn separated_layout_migrates_and_validates_a_legacy_generation() {
    let root = tempfile::tempdir().unwrap();
    let artifacts = root.path().join("target/xmlsquish");
    let store = MemoryStore::default();
    let legacy = FileArtifactPublisher::open(&artifacts, store).unwrap();
    let generation = legacy
        .publish_generation(
            &target("app"),
            &[publication(
                legacy.store.insert(b"legacy"),
                6,
                "target/xmlsquish/app.prompt",
            )],
        )
        .unwrap();
    drop(legacy);
    // v1.0.1 only retained the private generation; remove the compatibility publisher's
    // projection so this fixture has the same observable shape.
    fs::remove_dir_all(artifacts.join("target")).unwrap();

    let private = root.path().join("catalog/target-publication-state");
    let publisher = FileArtifactPublisher::with_layout_observer_lock_and_legacy(
        &artifacts,
        &private,
        "target/xmlsquish",
        project_lock_path(root.path().join("catalog")),
        artifacts.join(STATE_DIR),
        MemoryStore::default(),
        Arc::new(NoopObserver),
    )
    .unwrap();
    assert_eq!(
        publisher.current_generation(&target("app")).unwrap(),
        Some(generation.clone())
    );
    assert_eq!(read_member(&publisher, &generation, 0), b"legacy");
    assert_eq!(fs::read(artifacts.join("app.prompt")).unwrap(), b"legacy");
    assert!(!artifacts.join(STATE_DIR).exists());
}

#[test]
fn separated_layout_finishes_a_switched_legacy_migration_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let artifacts = root.path().join("target/xmlsquish");
    let legacy = FileArtifactPublisher::open(&artifacts, MemoryStore::default()).unwrap();
    let generation = legacy
        .publish_generation(
            &target("app"),
            &[publication(
                legacy.store.insert(b"legacy"),
                6,
                "target/xmlsquish/app.prompt",
            )],
        )
        .unwrap();
    drop(legacy);
    fs::remove_dir_all(artifacts.join("target")).unwrap();

    // Simulate a crash after the copied state became authoritative but before the legacy
    // tree was removed. The phase marker must make the next open validate and finish cleanup.
    let private = root.path().join("catalog/target-publication-state");
    fs::create_dir_all(&private).unwrap();
    copy_directory_tree::<Infallible>(&artifacts.join(STATE_DIR), &private).unwrap();
    write_new_synced::<Infallible>(&private.join(LEGACY_MIGRATION), b"copied\n").unwrap();

    let publisher = FileArtifactPublisher::with_layout_observer_lock_and_legacy(
        &artifacts,
        &private,
        "target/xmlsquish",
        project_lock_path(root.path().join("catalog")),
        artifacts.join(STATE_DIR),
        MemoryStore::default(),
        Arc::new(NoopObserver),
    )
    .unwrap();
    assert_eq!(
        publisher.current_generation(&target("app")).unwrap(),
        Some(generation)
    );
    assert_eq!(fs::read(artifacts.join("app.prompt")).unwrap(), b"legacy");
    assert!(!artifacts.join(STATE_DIR).exists());
    assert!(!private.join(LEGACY_MIGRATION).exists());
}

#[test]
fn generation_identity_is_order_independent_and_reads_are_typed() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let one = publication(store.insert(b"one"), 3, "one.prompt");
    let two = publication(store.insert(b"two"), 3, "two.prompt");
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    let target = target("opaque-target");
    let forward = publisher
        .publish_generation(&target, &[one.clone(), two.clone()])
        .unwrap();
    let reverse = publisher.publish_generation(&target, &[two, one]).unwrap();
    assert_eq!(forward.identity.generation, reverse.identity.generation);
    assert_eq!(forward.artifacts, reverse.artifacts);

    let mut bytes = Vec::new();
    assert!(matches!(
        publisher
            .read_generation_artifact(
                &forward.identity,
                &PublicationPath::new("missing.prompt").unwrap(),
                &mut bytes
            )
            .unwrap(),
        ArtifactRead::NotFound
    ));
}

#[test]
fn member_read_verifies_only_selected_bytes_after_structural_manifest_validation() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let one = publication(store.insert(b"one"), 3, "one.prompt");
    let two = publication(store.insert(b"two"), 3, "two.prompt");
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    let target = target("selective-read");
    let generation = publisher.publish_generation(&target, &[one, two]).unwrap();
    let corrupt = publisher.generation_artifact_path(
        &target_key(&target),
        generation.identity.generation,
        &generation.artifacts[1].path,
    );
    fs::write(corrupt, b"bad").unwrap();

    assert_eq!(read_member(&publisher, &generation, 0), b"one");
    assert!(matches!(
        publisher.current_generation(&target),
        Err(PublishError::IntegrityMismatch)
    ));
}

#[test]
fn rejects_collisions_across_the_complete_destination_set() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let digest = store.insert(b"x");
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    for destinations in [["A.prompt", "a.prompt"], ["tree", "tree/child"]] {
        let requests = destinations.map(|destination| publication(digest.clone(), 1, destination));
        assert!(matches!(
            publisher.publish_generation(&target("app"), &requests),
            Err(PublishError::AliasConflict(_))
        ));
    }
    assert!(
        publisher
            .current_generation(&target("app"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn failure_while_staging_one_file_never_switches_the_generation() {
    let root = tempfile::tempdir().unwrap();
    let initial = MemoryStore::default();
    let old = initial.insert(b"old");
    let publisher = FileArtifactPublisher::open(root.path(), initial).unwrap();
    let old_generation = publisher
        .publish_generation(&target("app"), &[publication(old, 3, "app.prompt")])
        .unwrap();

    let store = MemoryStore::default();
    let good = store.insert(b"new");
    let missing = Digest::new(DigestAlgorithm::Blake3, vec![0x42; 32]).unwrap();
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    let error = publisher
        .publish_generation(
            &target("app"),
            &[
                publication(good, 3, "app.prompt"),
                publication(missing, 3, "app.psdbg"),
            ],
        )
        .unwrap_err();
    assert!(matches!(error, PublishError::MissingBlob));
    assert_eq!(
        publisher.current_generation(&target("app")).unwrap(),
        Some(old_generation)
    );
    assert!(!publisher.state.join(GENERATION_JOURNAL).exists());
}

#[test]
fn crash_at_every_durable_point_preserves_or_rolls_forward_a_whole_generation() {
    let points = [
        DurablePoint::ArtifactStaged,
        DurablePoint::ManifestStaged,
        DurablePoint::GenerationStaged,
        DurablePoint::CommitDecision,
        DurablePoint::CurrentSwitched,
        DurablePoint::JournalCleared,
    ];
    for point in points {
        let root = tempfile::tempdir().unwrap();
        let old_store = MemoryStore::default();
        let old_prompt = old_store.insert(b"old-prompt");
        let old_debug = old_store.insert(b"old-debug");
        let old_publisher = FileArtifactPublisher::open(root.path(), old_store).unwrap();
        let old_generation = old_publisher
            .publish_generation(
                &target("app"),
                &[
                    publication(old_prompt, 10, "app.prompt"),
                    publication(old_debug, 9, "app.psdbg"),
                ],
            )
            .unwrap();
        drop(old_publisher);

        let new_store = MemoryStore::default();
        let new_prompt = new_store.insert(b"new-prompt");
        let new_debug = new_store.insert(b"new-debug");
        let publisher =
            FileArtifactPublisher::with_observer(root.path(), new_store, Arc::new(CrashAt(point)))
                .unwrap();
        let requests = [
            publication(new_prompt.clone(), 10, "app.prompt"),
            publication(new_debug.clone(), 9, "app.psdbg"),
        ];
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            publisher
                .publish_generation(&target("app"), &requests)
                .unwrap();
        }));
        assert!(crashed.is_err(), "fault point {point:?} was not reached");
        drop(publisher);

        let recovery_store = MemoryStore::default();
        recovery_store.insert(b"new-prompt");
        recovery_store.insert(b"new-debug");
        let recovered = FileArtifactPublisher::open(root.path(), recovery_store).unwrap();
        let current = recovered
            .current_generation(&target("app"))
            .unwrap()
            .unwrap();
        let rolls_forward = matches!(
            point,
            DurablePoint::CommitDecision
                | DurablePoint::CurrentSwitched
                | DurablePoint::JournalCleared
        );
        if rolls_forward {
            assert_ne!(
                current.identity.generation,
                old_generation.identity.generation
            );
            assert_eq!(read_member(&recovered, &current, 0), b"new-prompt");
            assert_eq!(read_member(&recovered, &current, 1), b"new-debug");
        } else {
            assert_eq!(current, old_generation);
        }
        assert!(!recovered.state.join(GENERATION_JOURNAL).exists());
    }
}

#[cfg(unix)]
#[test]
fn rejects_symlink_component() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join("linked")).unwrap();
    let store = MemoryStore::default();
    let digest = store.insert(b"x");
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    assert!(matches!(
        publisher.publish(&publication(digest, 1, "linked/x")),
        Err(PublishError::Symlink(_))
    ));
}

#[cfg(unix)]
#[test]
fn constructor_rejects_an_existing_intermediate_symlink() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let linked = root.path().join("linked");
    symlink(outside.path(), &linked).unwrap();

    assert!(matches!(
        FileArtifactPublisher::open_with_layout_and_lock(
            linked.join("artifacts"),
            root.path().join("state"),
            Path::new(""),
            root.path().join("locks/project.lock"),
            MemoryStore::default(),
        ),
        Err(PublishError::Symlink(path)) if path == linked
    ));
    assert!(!outside.path().join("artifacts").exists());
}
