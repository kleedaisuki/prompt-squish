# ADR: Project-local `target/xmlsquish` build state

- **Status:** Accepted for v1.0.4 implementation
- **Date:** 2026-09-20
- **Decision owner:** xmlsquish architecture
- **Scope:** build products, build cache, dependency acquisition cache, compilation metadata,
  publication recovery, concurrency, and `xmlsquish clean`
- **Compatibility:** no compatibility with the pre-v1.0.4 storage layout is required
- **Supersedes:** the global-storage decisions in
  [`project-storage-namespaces.md`](../design/project-storage-namespaces.md) and the split
  project/global layout in
  [`v1.0.2-target-layout-and-clean.md`](../design/v1.0.2-target-layout-and-clean.md)

## 1. Decision

Every byte of reusable or inspectable build state owned by an xmlsquish workspace belongs to one
project-local build root. The default root is:

```text
<canonical-workspace-root>/target/xmlsquish
```

The root manifest's relative `workspace.target-dir` remains the spelling override for this root.
It defaults to `target/xmlsquish`, is resolved once against the canonical workspace root, and may
not escape or alias the workspace. All workspace members share the root. Cargo's
`CARGO_TARGET_DIR`, Cargo configuration, and `cargo metadata` do not affect it: xmlsquish owns an
xmlsquish project tree, not Cargo's compiler tree.

The root has four non-overlapping children:

```text
target/xmlsquish/                         # ProjectBuildLayout.root
|-- artifacts/                           # Stable user-facing products only
|   |-- chat.prompt
|   |-- chat.psdbg                       # only when requested
|   `-- ir/                              # only when requested
|       `-- chat/<package>/<source>.xsir
|-- cache/                               # Disposable acceleration state
|   |-- cas/
|   |   `-- blobs/v1/blake3/<hh>/<62-hex>
|   |-- actions.sqlite3                  # plus SQLite -wal/-shm sidecars while open
|   `-- sources/
|       `-- v1/                          # registry, Git, archives, and materializations
|-- metadata/                            # Rebuildable compilation/publication evidence
|   |-- layout.json                      # format discriminator; contains no absolute path
|   |-- publications/
|   |   |-- generations/
|   |   `-- targets/
|   `-- catalog/
|       |-- artifacts/                   # build records and inspectable evidence
|       `-- state/                       # catalog generations/current pointers/journals
`-- work/                                # Same-filesystem staging; never authoritative
    |-- publish/
    |-- fetch/
    `-- trash/
```

The exact descendants below versioned CAS, source-cache, publication, and catalog roots remain
adapter-private. The four first-level names and the meaning of `artifacts/` are the layout
contract. Product locators therefore become, for example,
`target/xmlsquish/artifacts/chat.prompt`. This deliberate break removes collisions between
manifest-selected outputs and internal state; it is preferable to reserving a growing collection
of magic output names.

Two coordination files are the sole exception to the removable-root rule. They are siblings of
the build root so that their inode and recovery evidence survive an atomic detach of the root:

```text
target/
|-- .xmlsquish.xmlsquish.lock            # shared build/inspect; exclusive clean
|-- .xmlsquish.xmlsquish.clean.json      # present only during committed clean recovery
|-- .xmlsquish.xmlsquish-trash-<nonce>/  # detached root, present only during recovery
`-- xmlsquish/
```

For a custom root `<parent>/<leaf>`, the names are
`.<leaf>.xmlsquish.lock`, `.<leaf>.xmlsquish.clean.json`, and
`.<leaf>.xmlsquish-trash-<nonce>`. These are coordination records, not caches or compilation
metadata. No state is placed in a user or machine-global cache.

## 2. Problem and evidence

### 2.1 Current repository facts

The present implementation is structurally global:

| Concern | Current implementation | Consequence |
| --- | --- | --- |
| Defaults | `squish-config` derives `source.cache-root` and `manager.storage-root` from the user config home (`crates/squish-config/src/model.rs:341-351`). | Default cache ownership is the machine user, not the project. |
| Composition | `compose_host` puts CAS and the action database under `manager.storage-root`, hashes the canonical project path for a catalog namespace, and passes the source cache independently (`src/main.rs:638-690`). | One project's complete derived state is spread across three roots. |
| Layout type | `StorageLayout` accepts four unrelated absolute paths and rejects ancestor/descendant overlap (`crates/squish-manager/src/services.rs:98-161,216-260`). | Simply putting the current catalog below the current publication root is invalid by construction. |
| Runtime | Target publication state and build-catalog state are separate global catalog descendants (`crates/squish-host/src/runtime.rs:79-113`). | Clean needs a multi-root transaction and path-derived project identity. |
| Clean | Clean journals, epoch fences, and two tombstones delete product and catalog roots but intentionally preserve global CAS/action state (`crates/squish-host/src/lib.rs:529-923,1670-1752`). | The complexity exists mainly because ownership is split. |
| SQLite | The action index uses WAL and has associated `-wal` and `-shm` state (`crates/squish-store/src/action.rs:231-300`). | Deleting it without excluding live runtimes is invalid. |
| Tests | Process tests assert that clean preserves `.xmlsquish/cache/state/{cas,actions.sqlite3}` and that two projects share a global store (`tests/process.rs:639-795`). | Those tests encode the superseded product model rather than an invariant to preserve. |

The new ownership rule makes the common case the only case: one workspace, one derived root, one
clean boundary.

### 2.2 External practice and research

Cargo puts final and intermediate build products in a workspace-shared `target` directory by
default, and explicitly treats intermediate layout as private. This supports a project-local,
tool-owned namespace without copying Cargo's private directory names. Cargo also documents that
its target directory can be redirected outside a project; following Cargo's resolved target
directory would therefore violate the stronger xmlsquish requirement that state belongs to the
project. See the Cargo Book on [build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html),
[workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html), and
[configuration](https://doc.rust-lang.org/cargo/reference/config.html#buildtarget-dir).

Bazel's documented output-layout requirements include avoiding collisions, supporting concurrent
workspaces/configurations, being easy to clean, and keeping tool output under an unambiguous owned
root. Its concrete global choice is not copied, but those requirements motivate the typed root and
separate artifact/cache/metadata children. See Bazel's
[output directory layout](https://docs.bazel.build/versions/main/output_directories.html).

The action cache remains content-derived. The storage relocation must not change action-key
semantics. This follows the separation between task description, rebuild decision, and execution
strategy developed in Mokhov, Mitchell, and Peyton Jones,
[“Build Systems à la Carte”](https://doi.org/10.1145/3236774), PACMPL/ICFP 2018. The paper is a
design framework, not evidence that this particular physical layout is optimal; the relevant
lesson is that persistence location and rebuild logic are separable concerns.

SQLite documents that WAL uses associated `-wal` and `-shm` files and that unlinking or renaming a
database while it is open can yield undefined and undesirable behavior. The whole-root maintenance
lock is consequently a correctness requirement, not defensive ornamentation. See SQLite's
[WAL documentation](https://www.sqlite.org/wal.html) and
[corruption guidance](https://www.sqlite.org/howtocorrupt.html#unlink).

Rust recommends atomic filesystem operations such as `create_new` instead of check-then-create.
`rename` is a same-filesystem primitive, and `sync_all` is required when callers need errors and
durability beyond `Drop`. See [`std::fs`](https://doc.rust-lang.org/std/fs/),
[`OpenOptions::create_new`](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html#method.create_new),
[`rename`](https://doc.rust-lang.org/std/fs/fn.rename.html), and
[`File::sync_all`](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all).

## 3. Data structure and ownership review

### 3.1 Replace path bags with a typed aggregate

`StorageLayout` is replaced by one aggregate whose constructor accepts only the canonical
workspace root and validated relative target directory:

```text
ProjectBuildLayout
|-- root: OwnedBuildRoot
|-- artifacts: ArtifactRoot
|-- cache: ProjectCacheLayout
|   |-- cas: CasRoot
|   |-- action_index: ActionIndexPath
|   `-- sources: SourceCacheRoot
|-- metadata: CompilationMetadataLayout
|   |-- publications: PublicationStateRoot
|   `-- catalog: BuildCatalogRoot
|-- work: WorkRoot
`-- coordination: CoordinationPaths
    |-- maintenance_lock
    |-- clean_journal
    `-- clean_trash_prefix
```

Only `root` is stored as an independent absolute path. Child accessors append compile-time fixed
relative components. `coordination` is deterministically derived from `root.parent()` and
`root.file_name()`. This removes:

- arbitrary combinations of absolute roots;
- project-path hashing and `catalog/projects/<hash>`;
- pairwise overlap checks among build responsibilities;
- separate source/build-cache configuration;
- the need to reconstruct layout inside manager code.

`ArtifactRoot`, `CasRoot`, and the other semantic wrappers should not be interchangeable
`PathBuf`s at port boundaries. The type system should make “publish metadata as a user artifact”
and “open a source cache at the artifact root” unrepresentable.

### 3.2 Authority and lifetime

| Data | Authority | Validation/failure behavior | Lifetime |
| --- | --- | --- | --- |
| Project manifest, lockfile, XML sources | Authoritative input | Ordinary project validation | Outside build root |
| `artifacts/` | Materialized output | Digest-checked before publication; reproducible from inputs | Until next publication or clean |
| CAS blobs | Disposable cache | Path digest must match content; mismatch is a miss and replacement | Until clean |
| Action SQLite index | Disposable index | Transactional; corruption or incompatible schema creates a fresh index after maintenance exclusion | Until clean/layout reset |
| Source cache | Disposable exact materialization | Complete marker and locked digest/revision must verify | Until clean |
| Publication/catalog metadata | Rebuildable evidence | Versioned and digest-linked; incomplete generation is ignored/recovered | Until clean/layout reset |
| `work/` | Non-authoritative staging | Safe to delete when not referenced by a committed journal | Invocation/recovery |
| External clean journal | Recovery authority for a committed clean only | Strict version and leaf-name validation | Clean transaction |

No file below the build root is required for the semantic correctness of a cold build. Inspect may
report that no completed build exists after clean; it must not treat missing derived metadata as
project corruption.

## 4. Path resolution rules

1. Discover the workspace from `--manifest-path` or the current directory using the existing
   project repository, then canonicalize the workspace root once.
2. Parse `workspace.target-dir`; the default is exactly `target/xmlsquish`.
3. Reject an empty path, absolute path, Windows prefix, root component, `.` or `..` component,
   non-portable component, or a path equal to the workspace root.
4. Join the relative target directory to the canonical workspace root.
5. Canonicalize the nearest existing ancestor and append missing components lexically. Reject the
   result unless it remains physically below the canonical workspace root. This prevents an
   existing symlink/reparse ancestor from redirecting build state outside the project.
6. Construct `ProjectBuildLayout`; callers never concatenate cache/catalog component strings.
7. Resolve manifest target outputs relative to `artifacts/`, not to the build root. Continue to
   reject absolute, escaping, duplicate, and case-folding-colliding outputs.
8. Expose project-relative artifact locators (`<target-dir>/artifacts/...`) in protocol and UI.
   Cache, metadata, work, lock, journal, and trash paths are never logical artifact locators.
9. Workspace member selection never changes the root. Members therefore share project-local cache
   entries and cannot accidentally open per-member action databases.
10. `XMLSQUISH_HOME` remains a user-configuration/credential location only. Remove
    `source.cache-root`, `manager.storage-root`, `XMLSQUISH_SOURCE_CACHE_ROOT`, and
    `XMLSQUISH_STORAGE_ROOT` from build layout resolution. v1.0.4 does not read their former data.

The decision intentionally does **not** invoke Cargo or consume `CARGO_TARGET_DIR`. A project can
contain Rust tooling, but Cargo configuration is not an xmlsquish workspace contract.

## 5. Atomic write and concurrency protocol

### 5.1 Lock hierarchy

The MSRV is Rust 1.88, while standard-library file locking stabilized later. Continue using the
existing `fs2` cross-platform advisory lock adapter rather than raising MSRV solely for this change.
Every xmlsquish process cooperates as follows:

```text
maintenance lock (shared: build/inspect; exclusive: clean/layout reset)
    -> short publication lock (exclusive, only while committing generations)
        -> SQLite's own transaction locks
```

- Build and inspect acquire the shared maintenance lock **before** opening CAS, source cache,
  SQLite, publication, or catalog handles and hold it until all such handles are dropped.
- Clean and incompatible-layout reset acquire the exclusive maintenance lock before inspecting or
  detaching the root. Acquisition waits with cancellation-aware polling; a busy lock is not a
  user-repair failure.
- Concurrent builds may share cache reads and compilation. A short publication lock serializes
  generation/current-pointer changes. It is acquired only after expensive compilation completes.
- No code may acquire maintenance while holding publication or an SQLite transaction. This fixed
  order prevents a clean/build deadlock.
- Lock files are coordination among cooperating xmlsquish processes, not a claim that arbitrary
  external filesystem mutation is prevented.

The source cache must become lazy. In particular, constructing the host for `clean` must not create
or open a descendant of the root that the same invocation is about to detach.

### 5.2 Immutable cache writes

CAS and immutable source objects use this protocol:

1. compute the complete content digest and destination;
2. create a unique sibling temporary with `create_new(true)`;
3. write all bytes, flush, and `sync_all` under the existing durability policy;
4. validate length and digest;
5. publish with no-clobber rename;
6. if another writer won, validate and reuse the winner;
7. treat an invalid winner as a cache miss and replace it under the adapter's existing recovery
   protocol.

There is no global exclusive cache lock. Content identity makes equal concurrent writes converge.

### 5.3 Mutable metadata and artifacts

- SQLite owns action-index atomicity. The application never renames or deletes its database, WAL,
  or SHM files while a runtime holds the shared maintenance lease.
- Versioned generation directories are fully staged, validated, and then renamed into
  `metadata/`. A `current` pointer is a small, canonical, versioned file replaced from a sibling
  temporary.
- Public artifacts are written to a sibling temporary in the final artifact directory, verified,
  synchronized, and atomically replaced one file at a time. The primary `.prompt` is replaced last.
- A durable publication journal in `metadata/publications/` distinguishes an undecided stage from a
  decided transaction. Recovery rolls a decided transaction forward and discards an undecided
  stage. A portable filesystem does not provide an atomic multi-file snapshot, so the contract is
  individual-file atomicity plus deterministic next-invocation recovery, not impossible
  project-wide atomicity.
- Metadata contains relative locators and digests, never canonical absolute project paths. Moving a
  project with its build root therefore cannot redirect recovery to its former location.

## 6. `clean` semantics and recovery

Plain `xmlsquish clean` removes the **entire** project build root: products, CAS, action index,
source cache, compilation metadata, and work files. It does not edit sources, manifests, or the
lockfile. A subsequent offline build may fail for a remote dependency because clean deliberately
removed the project's acquisition cache; that is the honest meaning of “all derived state is under
the clean boundary.”

The transaction is:

1. Resolve and validate `ProjectBuildLayout` without opening a descendant store.
2. Acquire the external maintenance lock exclusively, cancellation-responsively.
3. Recover any prior journal before creating the build root.
4. If the root is absent, return a successful zero result.
5. Choose a random/monotonic unique trash leaf in the same parent; create no destination.
6. Atomically persist and synchronize a versioned clean journal containing only the expected root
   leaf and trash leaf. It contains no absolute path.
7. Rename the root to the journaled trash path with no-clobber semantics. This is the commit point:
   new ordinary readers can no longer see any old generation.
8. Recursively remove the trash without following symlinks or reparse points; account only for
   successfully removed regular files.
9. Remove the journal and synchronize its parent where the platform adapter supports that
   guarantee. Release the lock.

Cancellation is honored before the journal is durable. After that point, the invocation or the next
ordinary invocation completes the committed operation and reports a committed result/failure.
Deletion failure is not converted into instructions for the user to repair hidden state: every
build, inspect, and clean entry path first performs the same idempotent roll-forward while holding
the exclusive maintenance lock.

Recovery cases collapse to one state machine:

| Journal | Root | Trash | Recovery |
| --- | --- | --- | --- |
| absent | absent | absent | already clean |
| present | present | absent | rename root to the exact journaled trash, then delete |
| present | absent | present | finish deleting trash |
| present | present | present | invalid/ambiguous; do not guess or clobber either tree; report a typed storage error with both project-local paths |
| absent | present | absent | ordinary build root |

Unjournaled stale `work/` entries are disposable after taking the shared maintenance lease plus the
publication lock. Unjournaled sibling trash is not deleted by a prefix glob: ownership must be
proven by a valid journal, avoiding destructive guesses.

## 7. Layout-version cutover (not migration)

v1.0.4 performs no import, copy, lookup, or deletion of old machine-global caches/catalogs and no
read of `target/xmlsquish/.squish-publish`. The old roots are outside the new ownership boundary and
are left untouched.

`metadata/layout.json` contains at least a schema version and tool-format epoch. On build/inspect:

- a missing build root is created lazily with the current marker;
- a non-empty build root with a missing, malformed, or unsupported marker is disposable old or
  foreign derived state. Under the exclusive maintenance lock it is detached with the same clean
  protocol, then a fresh current layout is created;
- an empty root may receive the current marker directly;
- no adapter carries a legacy reader or migration branch.

The root manifest declares `workspace.target-dir` as tool-owned derived storage, so reset may remove
unknown files inside it. The implementation and product documentation must state this plainly.
Retired global path configuration must not silently redirect new state. Remove the two environment
overrides and configuration fields; diagnostics for those exact retired keys should say that
v1.0.4 always uses the project build root rather than presenting a generic unknown-key message.

## 8. Four-layer review

### 8.1 Data structure

The critical improvement is one owner and one typed root. Child paths are structural facts, not
configuration. The CAS remains content-addressed, the action index remains an index, and the
catalog remains project-relative. Only their lifetime boundary changes.

### 8.2 Special cases removed or retained

Removed:

- global-versus-project cache policy;
- canonical-project-path hashing for catalog namespaces;
- clean of two authoritative roots plus separate invalid-source pruning;
- clean epochs needed to invalidate cached runtimes after deleting only some roots;
- legacy `.squish-publish` migration;
- configurable physical overlap matrices;
- “clean succeeds but preserves build cache” and offline-rebuild special semantics.

Retained because they represent real platform behavior:

- external coordination survives removable-root detach;
- Windows sharing violations receive bounded retry inside the platform filesystem adapter;
- symlinks/reparse points are removed as entries and never traversed;
- individual artifacts are atomic, but a set of files is recovered rather than falsely claimed to
  be atomically visible.

### 8.3 Complexity

Path construction falls from four independently configured absolute roots plus a project hash to
one root plus fixed child accessors. Clean falls from a two-root journal/epoch transaction and a
separate source-cache sweep to one rename-and-delete transaction. Runtime cache lookup complexity
is unchanged: CAS access is digest-bucketed and SQLite lookup remains indexed. Local caches give up
cross-project hits by design; prompt projects are small, and predictable ownership/cleanability is
the product priority.

### 8.4 Destructive analysis

| Break/change | Intended handling |
| --- | --- |
| Artifact locators gain `/artifacts/` | Update protocol fixtures, CLI output, docs, examples, and site in the same release. No legacy locator alias. |
| Warm global hits disappear | First v1.0.4 build is cold; later builds reuse the local cache. |
| `clean` removes dependency cache and action/CAS state | Document explicitly; test cold online rebuild rather than preserved-cache offline rebuild. |
| Old global state consumes disk | Leave untouched; never guess ownership. Release notes may give an explicit manual path explanation, not automatic deletion. |
| Copying `target/xmlsquish` between projects | Relative metadata cannot escape, but action keys still validate all semantic inputs; invalid entries become misses. Copying is unsupported acceleration, never authority. |
| Network/network-share workspace | SQLite WAL requires same-host shared-memory/locking semantics. Do not claim correctness on filesystems that violate SQLite/file-lock guarantees. |
| Concurrent `cargo clean` removes the parent `target` | External tools are outside the lock protocol. The next xmlsquish invocation treats missing derived state as a cold build. |

## 9. Alternatives rejected

| Alternative | Why rejected |
| --- | --- |
| Keep direct artifacts at root and hide state under `.xmlsquish/` | Preserves old locators but reserves a magic output subtree forever and keeps product/internal collision logic. No-compat v1.0.4 can choose the simpler namespace. |
| Put only compilation metadata locally, retain global CAS/source cache | Violates project ownership and preserves multi-root clean/configuration complexity. |
| Use Cargo's resolved `target_directory` | It may be redirected outside the project by CLI, environment, or config, contradicting the requirement. It also couples a non-Cargo project model to Cargo discovery. |
| Put maintenance lock inside the removable root | After rename/delete, a new process can create a new lock inode and bypass the cleaner. |
| Keep four absolute path fields and relax overlap validation | Makes invalid combinations representable and spreads path knowledge among callers. |
| Delete the root recursively in place | Readers can observe mixed generations; SQLite may remain open; a crash leaves an ambiguous half-clean tree. |
| One exclusive lock for the complete build | Correct but needlessly serializes independent compilation. Shared maintenance plus a short publication lock preserves useful concurrency. |
| Use mtimes as the primary freshness key | Timestamp granularity and copying make it fragile. Existing content/action digests are simpler for small prompt sources. |

## 10. Implementation slices and exact file boundaries

Implement in dependency order; each slice should leave tests compiling and should be committed
separately where practical.

### Slice A — typed project build layout

- `crates/squish-manager/src/services.rs`: replace `StorageLayout` with
  `ProjectBuildLayout` and semantic child-layout accessors; update the `Services` port.
- `crates/squish-manager/src/lib.rs` and `crates/squish-manager/src/build.rs`: consume artifact
  locators rooted at `artifacts/`; do not reconstruct internal paths.
- `crates/squish-project/src/model.rs` and `crates/squish-project/src/manifest.rs`: document and
  validate `workspace.target-dir` as the complete tool-owned project build root.
- `crates/squish-repository/src/repository.rs` and
  `crates/squish-repository/src/snapshot.rs`: resolve public outputs below `artifacts/` exactly once.

### Slice B — remove global build-path configuration and compose local storage

- `crates/squish-config/src/model.rs`, `crates/squish-config/tests/loading.rs`, and
  `crates/squish-config/IMPLEMENTATION.md`: retire global source/manager root settings and add
  actionable diagnostics for their exact old keys.
- `src/main.rs`: derive one `ProjectBuildLayout`; remove project namespace hashing and the two
  storage environment overrides; keep `XMLSQUISH_HOME` for non-build user configuration.
- `crates/squish-host/src/lib.rs`: change `HostConfig` to accept the typed layout, construct source
  acquisition lazily, and validate only the single root/coordination relationship.
- `crates/squish-host/src/runtime.rs`: open CAS, action index, publishers, and catalog from typed
  child paths; remove legacy-state wiring.

### Slice C — publisher and layout cutover

- `crates/squish-publish/src/lib.rs` and `crates/squish-publish/src/tests.rs`: remove
  `with_legacy_state` and global catalog-derived epoch behavior; retain generation recovery under
  local metadata and explicit lock injection.
- `crates/squish-store/src/cas.rs` and `crates/squish-store/src/action.rs`: keep storage formats,
  but ensure layout reset/index recreation occurs only under the maintenance lease and document
  local ownership.
- `crates/squish-fetch/src/lib.rs`, `crates/squish-fetch/src/materialize.rs`, and
  `crates/squish-fetch/src/clean.rs`: point acquisition at `cache/sources`; remove the user-level
  invalid-cache clean pass because whole-root clean supersedes it. Preserve automatic integrity
  recovery during normal fetch.

### Slice D — one-root clean and recovery

- `crates/squish-host/src/lib.rs`: replace the two-root `CleanJournal`, epoch fence, and tombstones
  with the relative one-root protocol in Section 6; ensure handles are unopened/dropped first.
- `crates/squish-manager/src/clean.rs` and `crates/squish-manager/src/services.rs`: update the
  clean contract and result wording to cover all project-derived state.
- `crates/squish-protocol/src/lib.rs` and `crates/squish-presentation/src/lib.rs`: revise typed
  clean statistics/presentation if dependency-specific invalid-entry fields are removed.
- `src/fault_injection.rs`, `tests/recovery_process.rs`, and `tests/pty.rs`: relocate failure probes
  and recovery discovery to the new project-local coordination/layout paths.

### Slice E — integration contracts and documentation

- `crates/squish-host/tests/composition.rs`: assert exact child ownership and lazy creation.
- `crates/squish-manager/tests/build.rs`, `crates/squish-manager/tests/clean.rs`, and
  `crates/squish-manager/tests/inspect.rs`: update port fakes and locators.
- `tests/process.rs`: replace global sharing/preserved-cache assertions with the acceptance tests
  below.
- `README.md`, `CHANGELOG.md`, `docs/product/project-manager-scope.md`,
  `docs/product/cli-experience.md`, `docs/releases/1.0.4.md`, and relevant examples: publish the
  user-visible tree, destructive clean meaning, and cold cutover.

This ADR intentionally assigns no changes to `site/` or `.github/`; those are separate workstreams.

## 11. Test matrix

| Level | Scenario | Required observation |
| --- | --- | --- |
| Unit: layout | Default root | Canonical workspace + `target/xmlsquish`; all child accessors match the tree exactly. |
| Unit: layout | Custom relative target | All artifacts/cache/metadata/work follow the same custom root. |
| Unit: layout | Empty, absolute, `..`, Windows prefix, symlink/reparse escape | Rejected before any directory is created. |
| Unit: layout | Two workspace members | Both derive byte-identical layout paths. |
| Unit: locator | Output, IR, debug | Every public locator contains `/artifacts/` once; no internal locator is accepted. |
| Unit: config | Retired keys and environment overrides | No redirection occurs; exact diagnostic explains project-local storage. |
| Unit: marker | Missing/invalid/current `layout.json` | Empty initializes; non-empty incompatible root resets under exclusive lock; current opens normally. |
| Store | Concurrent equal CAS writes | One verified blob remains; losers reuse it; no partial bytes become visible. |
| Store | CAS corruption | Read reports a miss and a later action repairs it; no corrupt hit. |
| Store | Action DB missing/corrupt | Fresh local index is created under maintenance exclusion; build output remains correct. |
| Fetch | Incomplete/corrupt registry or Git materialization | It is quarantined/replaced inside `cache/sources`; exact locked identity is enforced. |
| Publication | Failure before decision | Old artifacts/current generation remain authoritative; stage is discarded. |
| Publication | Failure after decision at every write/rename/sync boundary | Next invocation rolls forward to one complete generation; prompt is replaced last. |
| Concurrency | Two builds, same/different targets | Compilation/cache sharing proceeds concurrently; publication serializes briefly; final metadata/artifacts agree. |
| Concurrency | Build or inspect versus clean | Clean waits (and can cancel before commit); it never renames an open SQLite/source/publication root. |
| Clean | Empty root and repeated clean | Success with zero counts; no build root is created as a side effect. |
| Clean | Normal populated root | Root becomes absent; artifacts, CAS, DB sidecars, sources, metadata, and work are all gone. |
| Clean recovery | Death before journal, after journal, after rename, during recursive delete, before journal removal | Pre-journal cancellation changes nothing; every committed state automatically converges to absent root. |
| Clean safety | Symlink/reparse entry inside root | Entry is removed without traversing or deleting its target. |
| Clean safety | Journal/root/trash ambiguity | No guessed deletion; typed error identifies the two project-local conflicting paths. |
| Process | Cold build then warm build | First build populates only the project root; second emits real cache hits. |
| Process | Projects A and B under one user home | No shared CAS/index/catalog and no cross-project hit unless inputs are rebuilt independently. |
| Process | Move project directory with target included | Relative metadata opens or safely misses; no access to the old absolute location. |
| Process | Observe filesystem writes | Build writes no cache/catalog state under `XMLSQUISH_HOME`, APPDATA, XDG cache/config, or HOME. |
| Process | Clean then offline build with only local/path deps | Cold rebuild succeeds. Remote deps may produce the documented offline miss. |
| Cross-platform | Linux, macOS, Windows native jobs | Path validation, lock waiting, SQLite close-before-clean, rename retry, and recovery pass natively. |
| Performance | Warm representative prompt project | Local warm build has no material regression versus the same cache/index logic before relocation; measurements report cold and warm separately. |

## 12. Acceptance invariants

The implementation is complete only when all of the following are mechanically demonstrated:

1. Deleting the project build root removes every xmlsquish build artifact, cache entry, and piece of
   compilation metadata needed only for that project.
2. A build with an arbitrary `XMLSQUISH_HOME` creates no build state there.
3. No production API accepts independently configurable CAS, action-index, source-cache, catalog,
   and publication absolute paths.
4. Build correctness after clean depends only on declared project inputs and available dependency
   sources, never on old global state.
5. Clean never renames/unlinks an open SQLite WAL database owned by a cooperating invocation.
6. No current production code reads `.squish-publish`, `catalog/projects/<project-hash>`,
   `source.cache-root`, or `manager.storage-root`.
7. Public artifact paths contain `artifacts/`; internal implementation paths never appear as
   artifact locators.
8. Crash injection at each durable boundary converges automatically without manual repair.

