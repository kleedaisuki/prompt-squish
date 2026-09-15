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
            name: OutputName::new("prompt").unwrap(),
            kind: ArtifactKind::Prompt,
            digest,
            size,
        },
        destination: destination.to_owned(),
    }
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
    assert_eq!(artifact.digest, request.output.digest);
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
        assert!(!root.path().join(&request.destination).exists());

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

    let published = publisher.publish_generation("app", &requests).unwrap();
    assert_eq!(
        publisher.current_generation("app").unwrap(),
        Some(published.clone())
    );
    assert_eq!(published.artifacts.len(), 2);
    assert_eq!(
        fs::read(root.path().join(&published.artifacts[0].uri)).unwrap(),
        b"prompt-v1"
    );
    assert_eq!(
        fs::read(root.path().join(&published.artifacts[1].uri)).unwrap(),
        b"debug-v1"
    );
}

#[test]
fn rejects_collisions_across_the_complete_destination_set() {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let digest = store.insert(b"x");
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    for destinations in [
        ["A.prompt", "a.prompt"],
        ["name", "name."],
        ["tree", "tree/child"],
        [r"dir\file", "dir/file"],
    ] {
        let requests = destinations.map(|destination| publication(digest.clone(), 1, destination));
        assert!(matches!(
            publisher.publish_generation("app", &requests),
            Err(PublishError::AliasConflict(_))
        ));
    }
    assert!(publisher.current_generation("app").unwrap().is_none());
}

#[test]
fn failure_while_staging_one_file_never_switches_the_generation() {
    let root = tempfile::tempdir().unwrap();
    let initial = MemoryStore::default();
    let old = initial.insert(b"old");
    let publisher = FileArtifactPublisher::open(root.path(), initial).unwrap();
    let old_generation = publisher
        .publish_generation("app", &[publication(old, 3, "app.prompt")])
        .unwrap();

    let store = MemoryStore::default();
    let good = store.insert(b"new");
    let missing = Digest::new(DigestAlgorithm::Blake3, vec![0x42; 32]).unwrap();
    let publisher = FileArtifactPublisher::open(root.path(), store).unwrap();
    let error = publisher
        .publish_generation(
            "app",
            &[
                publication(good, 3, "app.prompt"),
                publication(missing, 3, "app.psdbg"),
            ],
        )
        .unwrap_err();
    assert!(matches!(error, PublishError::MissingBlob));
    assert_eq!(
        publisher.current_generation("app").unwrap(),
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
                "app",
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
            publisher.publish_generation("app", &requests).unwrap();
        }));
        assert!(crashed.is_err(), "fault point {point:?} was not reached");
        drop(publisher);

        let recovery_store = MemoryStore::default();
        recovery_store.insert(b"new-prompt");
        recovery_store.insert(b"new-debug");
        let recovered = FileArtifactPublisher::open(root.path(), recovery_store).unwrap();
        let current = recovered.current_generation("app").unwrap().unwrap();
        let rolls_forward = matches!(
            point,
            DurablePoint::CommitDecision
                | DurablePoint::CurrentSwitched
                | DurablePoint::JournalCleared
        );
        if rolls_forward {
            assert_ne!(current.generation_id, old_generation.generation_id);
            assert_eq!(
                fs::read(root.path().join(&current.artifacts[0].uri)).unwrap(),
                b"new-prompt"
            );
            assert_eq!(
                fs::read(root.path().join(&current.artifacts[1].uri)).unwrap(),
                b"new-debug"
            );
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
