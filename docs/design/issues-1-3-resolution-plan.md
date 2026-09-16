# Resolution Plan for Issues #1, #2, and #3

## Status and scope

This document is the implementation plan for:

- [#1: Inject build runtime ports instead of constructing adapters in manager](https://github.com/kleedaisuki/prompt-squish/issues/1);
- [#2: Hide publisher URI layout behind a typed catalog API](https://github.com/kleedaisuki/prompt-squish/issues/2); and
- [#3: Enforce the documented crate dependency graph in CI](https://github.com/kleedaisuki/prompt-squish/issues/3).

The three issues have one architectural root: a documented inward-pointing
dependency model is not yet the executable model. Issue #2 is the type-boundary
prerequisite for the publication portion of issue #1. Issue #3 is independently
shippable, but its policy must change atomically with the dependency changes made
for issues #1 and #2.

This plan does not preserve internal constructor, wire-schema, or on-disk catalog
compatibility. It does preserve the product invariants that matter: deterministic
actions, verified cache hits, cancellation semantics, ordered lifecycle events,
crash-recoverable publication, and stable logical artifact lookup.

## Decision summary

1. `squish-manager` remains the owner of build planning, action/work
   correspondence, cache eligibility, reconciliation policy, lifecycle events,
   and finalization. It stops constructing or naming production adapters.
2. A host-supplied `BuildRuntimeProvider` opens one invocation-scoped,
   thread-safe `BuildRuntimePorts` bundle during the existing planning recovery
   step. This preserves lifecycle ordering while moving concrete composition to
   `squish-host`.
3. The runtime bundle exposes two cohesive capabilities rather than a trait per
   function: persistence/publication and the pure compilation toolchain. The
   bundle is statically composed; it is not a runtime plugin system.
4. Publication target, logical path, and generation identities become distinct
   types owned by the port-defining `squish-build` crate. `squish-publish` alone
   owns target hashes, journal names, `current.json`, generation directories,
   retry policy, filesystem locks, and physical paths.
5. The build catalog moves directly to `BuildRecordV3`. It stores semantic
   descriptors and typed identities, never publisher manifest URIs or physical
   generation paths.
6. The executable dependency policy freezes the actual consolidated workspace,
   not ADR-0009's unimplemented aspirational crate split. Dependency kind is part
   of an edge, so a dev-only edge cannot silently become a production edge.

## Current-state evidence

The following are observations from the current repository, not desired-state
assumptions.

| Concern | Repository evidence | Consequence |
|---|---|---|
| Manager is a second composition root | `BuildExecutor` in [`crates/squish-manager/src/build.rs`](../../crates/squish-manager/src/build.rs) owns `Cas`, `VerifiedActionIndex`, and two `FileArtifactPublisher<Cas>` values; `BuildExecutor::new` opens all of them. | Supplying a test or alternative adapter requires reproducing production layout assumptions. |
| Pure engines are selected inside the executor | The same file directly calls XML `compile`, `StaticLinker`, `Instantiator`, and `SquishBackend`. | The proposed runtime boundary is incomplete if it only wraps storage. |
| Concrete coupling exists outside `BuildExecutor` | `read_base_build_catalog`, `read_current_build_catalog`, `recover_build_catalog`, and `restore_recorded_generation` reopen CAS/publishers directly. | Replacing only the executor fields cannot satisfy issue #1. |
| Manager duplicates publisher layout logic | `generation_locator_recipe` reconstructs `.squish-publish/targets/.../current.json` and `.squish-publish/generations/...`, while `recover_current_generation` parses artifact URI prefixes. | Layout is a shared secret rather than an adapter implementation detail. |
| Publisher exposes its physical layout | `PublishedGeneration.manifest` and each returned `Artifact.uri` in [`crates/squish-publish/src/lib.rs`](../../crates/squish-publish/src/lib.rs) contain private current/generation paths. | Wrapping the present return type in a trait would preserve the defect. |
| Presentation also knows the layout | `published_artifact_path` in [`crates/squish-presentation/src/lib.rs`](../../crates/squish-presentation/src/lib.rs) recognizes generation-path hashes to recover a logical path. | Issue #2 spans manager, protocol projection, presentation, and process tests. |
| The documented ownership already rejects this state | [`crates/squish-manager/ARCHITECTURE.md`](../../crates/squish-manager/ARCHITECTURE.md) says concrete CAS/index/artifact adapters are composition-root concerns and records the catalog URI coupling as debt. | The change implements an existing boundary rather than inventing a new subsystem. |
| ADR prose and Cargo manifests differ | ADR-0009 names contexts that were consolidated during implementation, while the repository currently has 22 workspace packages and 67 internal dependency records (60 normal/optional and 7 dev). | Issue #3 must enforce the reviewed implemented graph, not force an unrelated crate split. |

The particularly dangerous duplication is:

```text
squish-publish::target_key/current_manifest_uri/generation_artifact_uri
                               ^
                               | duplicated hashing and path recipe
                               v
squish-manager::generation_locator_recipe/recover_current_generation
```

Changing either copy independently can make valid generations look corrupt or
make inspection unable to find a successful build.

## Required invariants

The implementation is accepted only if the following remain true.

### Build and lifecycle

- A sealed plan has an exact one-to-one correspondence between actions and
  typed work.
- Only transform work is persistently cacheable. Publication and finalization
  never become cache hits.
- A cache hit is used only after blob presence, digest, size, codec, and
  stage-specific semantic checks succeed.
- Worker completion order cannot change the emitted action-event order.
- Cancellation before work starts prevents that work. Once publication crosses
  its durable commit decision, cancellation is deferred until publication has
  completed or is recoverable; the user is not asked to repair storage.
- A failed target does not erase the prior good generation of another target.

### Publication and catalog

- A generation becomes current through one atomic current-pointer switch after
  every member and its manifest have been staged, synchronized, and verified.
- Physical journal recovery is a normal publisher operation and occurs under the
  publisher's cross-process lock.
- The manager decides semantic reconciliation (adopt, restore, or rebuild), but
  never constructs, parses, joins, or validates a publisher-private path.
- `ArtifactId`, content `Digest`, publication target identity, logical artifact
  path/name, and immutable generation identity remain distinct concepts.
- A successful build exposes an unambiguous `(target, logical artifact)` query.
  Resolving that query may yield a current readable location, but the physical
  location is not a persisted identity.

### Composition and concurrency

- Runtime ports are `Send + Sync` because the manager executes scheduler waves
  on scoped worker threads.
- Production opens one coherent CAS/action-index/publication graph per composed
  runtime. Publishers do not silently use a second blob store.
- Manager state locks are never held across an external port call. Publisher
  serialization remains inside the publisher adapter; catalog finalization runs
  only after the action graph closes.
- Runtime adapters return data and typed failures. They do not print, emit kernel
  lifecycle events, or terminate the process.

## Target architecture

```text
src/main.rs
    |
    +-- constructs ProductionHost + ProductionBuildRuntimeProvider
    |                       |
    |                       +-- Cas + VerifiedActionIndex
    |                       +-- target FileArtifactPublisher
    |                       +-- catalog FileArtifactPublisher
    |                       +-- XML/link/instantiate/backend engines
    |
    v
ManagerCapability(Services, BuildRuntimeProvider)
    |
    +-- planning / policy / lifecycle / finalization
    |
    +-- BuildExecutor
            |
            +-- BuildRuntimePorts.persistence
            +-- BuildRuntimePorts.toolchain
```

`squish-host` is allowed to know concrete adapter types. `squish-manager` is
allowed to know domain request/result types and port contracts. Neither the
manager nor presentation is allowed to know the publisher's directory recipe.

### Runtime opening and lifetime

Opening all storage eagerly before kernel dispatch would move failures from the
existing `open-build-state` planning observation into bootstrap and would change
event ordering. Therefore the injected object is a provider/factory:

```rust,ignore
pub trait BuildRuntimeProvider: Send + Sync {
    fn open(
        &self,
        scope: &BuildScope,
    ) -> Result<BuildRuntimePorts, BuildRuntimeOpenError>;
}
```

The manager calls this abstract operation inside the existing recover planning
step. The host implementation performs concrete opens and automatic journal
recovery. Tests inject an in-memory or faulting provider through the same API.
The returned bundle lives for the complete build and is shared by all workers.

### Runtime bundle

The exact Rust factoring may reuse existing domain traits, but the semantic
surface is fixed:

| Capability group | Required operations | Must not own |
|---|---|---|
| Persistence | blob read/write; verified action lookup/record; publish/query/read target generations; publish/query/read build-catalog generations | action eligibility, build-record meaning, lifecycle events |
| Toolchain | compile frozen source; static link; instantiate with budgets; negotiate and render backend output; expose stable cache identity | project discovery, CAS paths, publication, terminal output |

`squish-build::{BlobStore, ActionIndex}` remain the primitive inward-facing
storage ports. The new typed generation port also belongs in `squish-build`,
because placing that contract in the concrete `squish-publish` crate would retain
an adapter dependency in manager. An object-safe manager-facing facade may
normalize associated adapter errors into one `BuildRuntimeError`; that facade is
an orchestration boundary, not a new storage implementation.

Where domain-only traits already exist, reuse them. In particular,
`squish-backend::Backend` is already a pure engine contract. Frontend, linker,
and instantiator contracts should live beside their domain request/result types,
with the host selecting their production implementations. Avoid an eight- or
ten-parameter generic `BuildExecutor`: it would optimize irrelevant dispatch
cost while making construction, diagnostics, and tests harder to read.

### Typed publication model

The public port must not return the existing layout-bearing
`PublishedGeneration`. Its minimum type model is:

```rust,ignore
pub struct PublicationTargetId(String);       // stable owner identity
pub struct PublicationPath(String);           // portable logical destination
pub struct LogicalArtifactName(String);       // target-local query name
pub struct GenerationId([u8; 32]);             // opaque outside publisher API

pub struct GenerationRef {
    pub target: PublicationTargetId,
    pub generation: GenerationId,
}

pub struct ArtifactDescriptor {
    pub id: ArtifactId,
    pub name: LogicalArtifactName,
    pub kind: ArtifactKind,
    pub digest: Digest,
    pub size: u64,
}

pub struct GenerationArtifact {
    pub descriptor: ArtifactDescriptor,
    pub destination: PublicationPath,
}

pub struct CommittedGeneration {
    pub identity: GenerationRef,
    pub artifacts: Vec<GenerationArtifact>,
}
```

The generation port provides at least:

```rust,ignore
pub trait GenerationRepository: Send + Sync {
    type Error;

    fn publish_generation(
        &self,
        target: &PublicationTargetId,
        publications: &[Publication],
    ) -> Result<CommittedGeneration, Self::Error>;

    fn current_generation(
        &self,
        target: &PublicationTargetId,
    ) -> Result<Option<CommittedGeneration>, Self::Error>;

    fn read_generation_artifact(
        &self,
        generation: &GenerationRef,
        destination: &PublicationPath,
        sink: &mut dyn std::io::Write,
    ) -> Result<ArtifactRead, Self::Error>;
}
```

`read_generation_artifact` resolves the private layout, checks manifest
membership, reads the immutable member, and verifies size and digest. Absence is
represented explicitly and is not an empty byte stream. If historical activation
is needed, add a typed `activate_generation(GenerationRef)` operation; never add
a method taking a manifest path.

Generation hashing uses a domain separator and a canonical ordering by logical
destination and artifact identity. It cannot depend on input slice order.
`GenerationId` has one canonical lowercase encoding at serialization boundaries.
The target's SHA-256 layout key remains a private publisher value.

### Stable artifact lookup

The stable user contract is a logical query, not a stable generation directory:

```text
project identity + TargetName + LogicalArtifactName
    -> current GenerationRef
    -> verified ArtifactDescriptor + PublicationPath
    -> optional current readable location/stream
```

If the protocol retains an `Artifact.uri` field temporarily, its value is a
publisher-supplied stable logical location and is treated as opaque. It must not
contain `.squish-publish`, `current.json`, a target hash, or a generation path.
The preferred cleanup is to split `ArtifactDescriptor` from an explicit logical
location in `squish-protocol`, so one string can no longer ambiguously mean a CAS
URI, a user path, and a private publisher path.

### Build catalog V3

`BuildRecordV3` records only:

- plan and terminal action facts;
- `GenerationRef` for every successfully retained target;
- each artifact's semantic descriptor and `PublicationPath`; and
- the typed build-catalog target identity.

It removes `RecordedGeneration.manifest` and physical `Artifact.uri` values.
Manager validation continues to own semantic membership: unique target and
artifact identities, required static link map, target-record schema, artifact
kind, digests, and plan/action consistency. Publisher validation owns journal,
current pointer, generation membership, physical bytes, and filesystem aliases.

There is no V2 compatibility decoder in the new path. An old catalog is stale,
rebuildable state: it is ignored for reuse and replaced by the next successful
V3 finalization. Inspecting before that rebuild returns a typed unavailable/stale
result rather than panicking or asking the user to edit storage.

## Failure and recovery semantics

| Result | Owner | Required behavior |
|---|---|---|
| No current generation | Publisher | Return `None`; this is not corruption. |
| Missing/corrupt cache blob | Verified storage adapter | Remove or ignore the rebuildable action record and report a cache miss; manager reruns the transform. |
| Malformed journal/current manifest, missing member, digest mismatch | Publisher | Attempt normal locked recovery; if unrecoverable, return typed publication integrity failure and retain evidence for diagnosis. |
| Recorded generation differs from current | Manager reconciliation | Compare typed `GenerationId` values; adopt only a fully verified newer generation, otherwise republish/activate the verified recorded manifest from CAS. |
| Catalog schema or semantic membership invalid | Manager | Reject catalog reuse; do not misclassify it as I/O absence. |
| Permission, disk, or database failure | Host adapter | Return a storage/availability error. Do not convert it into `None` or destructive recovery. |
| Engine rejects input | Toolchain adapter plus manager mapping | Return the stage's structured error; manager preserves action failure, dependency blocking, and ordered events. |
| Cancellation after publication commit decision | Publisher/runtime | Finish or leave a journal that opens can roll forward; report cancellation as deferred, not as user-owned repair work. |

The current Windows `PermissionDenied` retry in manager's
`publish_generation` helper moves into `squish-publish`, beside the operation and
its locking semantics.

## Dependency policy (issue #3)

### Policy decision

Enforce the actual consolidated 22-package graph. ADR-0009's conceptual drawing
mentions separate diagnostic, syntax, codec, runtime, backend API, backend
implementation, and artifact crates that do not exist. Creating those crates is
not required to close an enforcement gap and would hide issues #1 and #2 inside
an unrelated decomposition.

The executable contract is:

- [`scripts/architecture_edges.txt`](../../scripts/architecture_edges.txt):
  explicit internal direct edges including dependency kind; and
- [`scripts/check_architecture.py`](../../scripts/check_architecture.py):
  deterministic extraction and comparison using
  `cargo metadata --format-version 1 --no-deps --locked`.

Workspace membership is established from Cargo package IDs and manifest paths,
not dependency display names. This handles renamed dependencies and ignores a
same-named third-party/path package outside the workspace. Normal, dev, and
build dependencies are different policy edges. Optional and target-qualified
declarations are still inspected, so they cannot bypass the contract. Sorted
diagnostics name both endpoints and the kind.

The current exact-policy behavior rejects both a forbidden actual edge and a
stale allowlisted edge. Therefore every intentional manifest change in issues #1
and #2 updates the policy in the same commit. At minimum, the final graph removes
normal `squish-manager -> squish-store` and
`squish-manager -> squish-publish`; production composition adds the required
`squish-host -> ...` adapter/engine edges. The precise host additions are derived
from the chosen domain-trait signatures rather than guessed in advance.

The checker and its standard-library fixture tests run in the Ubuntu/MSRV quality
job before formatting and Clippy. It requires neither network access beyond what
Cargo metadata already needs nor graph-rendering software. ADR-0009 links to the
policy and checker so prose cannot silently claim enforcement without an
executable source of truth.

## Migration and integration order

Each numbered slice is independently reviewable. Slices 2 through 5 may share a
feature branch, but they should retain the listed commit boundaries.

### Slice 1 — Executable graph policy (issue #3, independent)

1. Commit the exact kind-aware edge policy, metadata checker, and focused
   forbidden-edge fixture/self-test.
2. Invoke the self-test and real checker in the Ubuntu quality job.
3. Update ADR-0009 to distinguish its conceptual context drawing from the
   executable consolidated graph and link the checked policy.

Acceptance: the current graph passes; adding a forbidden acyclic edge fails with
`source -> destination (kind)`; third-party dependencies are ignored; no graph
renderer is installed.

### Slice 2 — Typed publication identities and adapter API (issue #2 foundation)

1. Add `PublicationTargetId`, `PublicationPath`, `LogicalArtifactName`,
   `GenerationId/Ref`, `ArtifactDescriptor`, and `CommittedGeneration` to
   `squish-build`.
2. Change `Publication.destination` from `String` to `PublicationPath`.
3. Implement `GenerationRepository` for the filesystem publisher. Keep all
   target-key/path helpers private, canonicalize generation hashing, and move the
   Windows retry into this adapter.
4. Add publisher-only tests for typed current/read, ordering-independent identity,
   corrupt members, and the complete crash-point matrix.

Do not expose a temporary `String manifest` port. That would make the later
inversion cosmetic.

### Slice 3 — Catalog V3 and logical inspection (issue #2 completion)

1. Replace `BuildRecordV2` with V3 typed generation/artifact records.
2. Rewrite strict catalog load and planning recovery against typed
   `current_generation`/`read_generation_artifact` operations.
3. Delete `generation_locator_recipe`, generation-hex validation from manager,
   URI-prefix stripping, and every `publication_root.join(artifact.uri)` call.
4. Make inspection index `(TargetName, LogicalArtifactName)` and
   `PublicationPath`; remove presentation's `published_artifact_path` heuristic.
5. Update README/process assertions so public output never contains a private
   publisher path.

Acceptance: content-derived target/generation IDs are tested without callers
knowing hashes or directories; a successful build is found through a logical
query; strict inspection stays read-only while planning recovery may reconcile.

### Slice 4 — Runtime-provider seam (issue #1 foundation)

1. Add `BuildRuntimeProvider`, `BuildRuntimePorts`, normalized runtime errors,
   and in-memory/faulting implementations.
2. Inject the provider into `ManagerCapability`. Keep runtime open inside the
   current planning recovery step.
3. Remove `PreparedBuild.storage`; retain only project/snapshot/domain state.
4. Route blob, action-index, generation, and catalog operations through the
   runtime bundle. Route the four pure build stages through the toolchain
   capability.

Tests for unrelated commands use an explicit unavailable/no-build provider;
build tests use the same in-memory port bundle as production uses conceptually,
not project-local `StorageLayout` shortcuts.

### Slice 5 — Production composition and concrete dependency removal

1. Add a `squish-host` production runtime provider owning the coherent CAS,
   verified index, two publisher namespaces, and default engines.
2. Split publisher durability/fault hooks out of manager's `DurabilityPorts` and
   pass them to host/runtime construction. Repository-creation fault injection
   remains on its existing repository boundary.
3. Delete concrete store/publisher imports and helpers from manager, then remove
   its `squish-store` and `squish-publish` Cargo dependencies.
4. Update `scripts/architecture_edges.txt` atomically with every manifest edge
   change.
5. Update manager/host architecture documents and resolve the historical debt
   note in [`docs/design/refactor-map.md`](refactor-map.md).

Acceptance: neither `BuildExecutor` nor any manager catalog helper names or opens
`Cas`, `VerifiedActionIndex`, or `FileArtifactPublisher`; the host integration
test uses real adapters; manager unit tests inject deterministic memory/fault
ports.

### Slice 6 — End-to-end regression evidence

Run and retain evidence for:

1. manager build/cache/catalog/inspect suites;
2. publisher crash and corruption suites;
3. scheduler concurrency, single-flight, cancellation, and event-order suites;
4. process-death recovery tests with fault injection;
5. workspace tests and Clippy on all features;
6. architecture checker self-tests and the real graph check; and
7. composed CLI smoke on Linux, Windows, and macOS.

## File ownership for parallel implementation

The rows below are single-writer regions. Separate workers may proceed in
parallel only where rows do not overlap.

| Slice/owner | Primary files | Contract with other slices |
|---|---|---|
| Architecture policy worker | `scripts/check_architecture.py`, `scripts/architecture_edges.txt`, `scripts/tests/**` | Publishes the policy format before any Cargo edge update. |
| CI worker | `.github/workflows/ci.yml` | Only invokes the checker/self-test; does not encode edges in YAML. |
| ADR/documentation worker | `docs/adr/0009-*.md`, `docs/design/refactor-map.md`, manager/host architecture docs | Links executable policy and records final ownership after code lands. |
| Publication API worker | `crates/squish-build/src/ports.rs` or a new sibling module; `crates/squish-publish/src/lib.rs` and its tests | Freezes typed identities and query semantics before catalog/runtime work. |
| Catalog/inspection worker | manager `build.rs`, `inspect.rs`; protocol/presentation projections; manager/process tests | Consumes typed generation API; owns BuildRecordV3 and semantic validation. |
| Runtime seam worker | new manager `runtime.rs`, manager capability plumbing, build executor calls, manager test doubles | Does not implement filesystem/CAS adapters. |
| Production runtime worker | new host `build_runtime.rs`, host configuration/tests, root composition and fault-hook wiring | Implements the frozen runtime and publication contracts; owns concrete opens. |
| Dependency manifest worker | affected `Cargo.toml` files plus the matching policy update | Runs the real architecture checker after every edge change. |

No worker should simultaneously redesign artifact wire types and publisher
layout. The former is a cross-context contract; the latter remains private and
may stay physically unchanged.

## Acceptance mapping

### Issue #1

| Acceptance criterion | Evidence |
|---|---|
| Manager does not open CAS/index/publishers | Search guard over manager source plus removal of its store/publish manifest edges. |
| `BuildExecutor` names no concrete adapter types | Compiler-enforced runtime fields and a focused source/manifest assertion. |
| Production adapters assembled at root/host | Host construction integration test and root smoke test. |
| Deterministic memory/fault injection | Runtime-provider unit tests exercise cache miss/corruption, engine failure, publication failure, and finalization failure. |
| Determinism, cancellation, ordering, crash safety preserved | Existing manager/scheduler/process recovery suites, updated to use typed ports. |
| Dependency contract updated | ADR/architecture docs plus the executable edge policy. |

### Issue #2

| Acceptance criterion | Evidence |
|---|---|
| Manager has no URI-layout recipe | Deleted recipe/parser helpers; forbidden private substrings test over V3 JSON/public output. |
| Inspection uses logical identity | Tests query target + logical name/path and resolve through `GenerationRepository`. |
| Recovery stays crash-safe and digest-verified | Publisher durable-point matrix and corrupt/missing member tests. |
| Hash-derived identities are opaque to callers | Manager/inspect tests use typed IDs and never build target/generation hashes. |
| Successful artifacts have stable lookup | Build result followed by typed logical query returns the declared descriptor and bytes. |
| Debt documentation resolved | Manager architecture and refactor-map note point to the new boundary. |

### Issue #3

| Acceptance criterion | Evidence |
|---|---|
| CI extracts internal graph | Real `cargo metadata --format-version 1 --no-deps --locked` checker run. |
| Every internal edge checked | Exact kind-aware committed policy; optional/target declarations included. |
| Forbidden acyclic edge is actionable | Fixture exits nonzero and names source, destination, and kind. |
| Current graph passes without renderer | Ubuntu/MSRV real-policy job. |
| Focused self-test exists | Standard-library unit test with synthetic metadata and third-party control. |
| ADR cannot drift silently | Direct links from ADR-0009 to the checker and policy. |

## Rejected alternatives

| Alternative | Why rejected |
|---|---|
| Inject `StorageLayout` and let manager keep opening adapters | It injects paths, not capabilities, so the manager remains the composition root. |
| Put typed catalog contracts in `squish-publish` | It makes the use-case layer depend on a concrete adapter crate and defeats dependency inversion. |
| Wrap the existing `PublishedGeneration` in a runtime trait | Its manifest and artifact URIs already expose the physical layout; the bad boundary would become permanent. |
| One trait object for every trivial method | It adds ceremony without a replaceable semantic boundary. Two cohesive runtime capabilities are easier to reason about and fault-inject. |
| Fully generic `BuildExecutor<B,A,P,C,F,L,I,O>` | It spreads construction types through manager and tests for no meaningful performance benefit. |
| Open runtime eagerly at bootstrap | It changes planning observations and failure/event ordering. |
| Preserve BuildRecord V2 with optional new fields | It retains ambiguous URI semantics and creates dual recovery logic. The catalog is rebuildable. |
| Make a stable symlink/current filesystem path the identity | Windows behavior, aliasing, and atomic replacement differ; a logical typed query is the portable stable contract. |
| Enforce ADR-0009's original aspirational crate list | It requires a large unrelated split and does not directly close the three reported defects. |
| Check only cycles or layer names | A forbidden acyclic adapter edge still compiles. Exact direct edges are the stated contract. |
| Treat allowed edges as untyped pairs | A dev dependency could be promoted to a normal production dependency without review. |

## Material risks and adversarial checks

1. **Split-brain blob stores.** Two independently opened CAS roots could make a
   publication descriptor valid in one store but unreadable to its publisher.
   The production runtime must construct index and both publishers from the same
   authoritative CAS configuration, with a host integration test proving a blob
   written through the runtime can be published and read through both namespaces.
2. **Layout leakage through a renamed field.** Calling a private generation path
   `location` does not fix issue #2. Persisted records and public events should be
   scanned for `.squish-publish`, `current.json`, and generation path fragments.
3. **Recovery policy accidentally moves into host.** The publisher owns physical
   recovery, but only manager has plan/target semantics. Host must not decide
   which target generation is authoritative.
4. **Lifecycle regression from eager open.** Preserve the planning recover step
   and compare event traces before/after on success, storage failure, and
   cancellation.
5. **Lock inversion.** Do not hold `BuildExecutionState` while calling storage,
   toolchain, or publisher ports. Add a parallel build test that would deadlock if
   runtime callbacks re-enter observation code under a manager lock.
6. **False cache success.** In-memory ports must model corruption and disappearance,
   not only happy-path maps. Semantic codec validation remains manager-side after
   storage integrity verification.
7. **Graph policy blocks the intended migration.** Land issue #3 independently,
   then change manifests and the exact policy together. A policy-only relaxation
   without corresponding code is not an acceptable preparatory step.
8. **Tests keep asserting private paths.** Publisher unit tests may inspect its
   private layout. Cross-crate and process tests must use the typed public query.
9. **Observability loss.** Moving retries and recovery into adapters must retain
   structured durability/source events and fault-injection points; hiding layout
   does not mean hiding operational evidence.

## External grounding

The design applies information hiding to a change-prone decision: Parnas's
classic modularization criterion is to isolate design decisions behind module
interfaces, which directly supports moving generation layout out of manager
([Communications of the ACM, 1972](https://doi.org/10.1145/361598.361623)).

The architecture checker deliberately consumes Cargo's versioned machine
interface. Cargo recommends passing `--format-version`, documents
`workspace_members`, and notes that `--no-deps` retains workspace package
information without fetching dependency packages
([Cargo metadata reference](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)).
Dependency kind is material because Cargo has distinct normal, development,
build, and target-specific declarations
([Cargo dependency reference](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html)).

Production build systems similarly enforce dependency visibility during analysis
rather than relying on diagrams; Bazel rejects a target whose dependency is not
visible and presents visibility as a way to distinguish public API from
implementation details
([Bazel visibility](https://bazel.build/concepts/visibility)). The custom Cargo
checker is the smallest project-specific equivalent. This is also consistent
with research describing architectural violations and structural problems as
forms of architecture erosion rather than merely documentation drift
([Li et al., *Journal of Software: Evolution and Process*, 2022](https://onlinelibrary.wiley.com/doi/10.1002/smr.2423)).

