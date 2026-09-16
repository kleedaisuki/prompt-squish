# Issues 1–3: Industry Practice and Research Basis

Status: implementation guidance, not an as-built specification  
Research date: 2026-09-16  
Scope: [issue 1](https://github.com/kleedaisuki/prompt-squish/issues/1),
[issue 2](https://github.com/kleedaisuki/prompt-squish/issues/2), and
[issue 3](https://github.com/kleedaisuki/prompt-squish/issues/3)

## 1. Question and evidence standard

The three issues are one architectural problem viewed at three boundaries:

1. Who is allowed to choose concrete infrastructure?
2. Who is allowed to interpret persisted artifact layout?
3. How is the intended answer kept true as the workspace changes?

This note compares the repository's proposed direction with primary standards,
official platform documentation, original architecture writing, and peer-reviewed
systems/software-engineering research. It deliberately does not treat popularity or
a passing benchmark as evidence of architectural fitness. The conclusions below are
recommendations for this small, statically linked Rust workspace; they are not claims
that the cited systems have the same requirements.

### Evidence map

| Source | What is directly established | Strength and limitation |
| --- | --- | --- |
| Cockburn's original [Hexagonal Architecture technical report](https://alistair.cockburn.us/hexagonal-architecture/) | A port represents a purposeful application conversation; multiple production and in-memory/test adapters can implement the same port while the application remains ignorant of the device. | Original pattern description, not controlled empirical evidence and not Rust-specific. |
| The official Rust Book on [traits and trait objects](https://doc.rust-lang.org/book/ch18-02-trait-objects.html) and the Rust Reference on [trait-object types](https://doc.rust-lang.org/reference/types/trait-object.html) | Rust can express a common capability through generics or `dyn Trait`; generics use static dispatch after monomorphization, while trait objects use virtual dispatch and permit runtime substitution. | Normative/language-authoritative mechanism; it does not decide where a project should put a boundary. |
| OCI Image Specification [content descriptors v1.1.1](https://github.com/opencontainers/image-spec/blob/v1.1.1/descriptor.md) and OCI Distribution Specification [v1.1.1](https://github.com/opencontainers/distribution-spec/blob/v1.1.1/spec.md) | A typed descriptor carries media type, digest, and size; content is addressed and verified through that descriptor; a human-readable tag can resolve to a manifest without exposing registry storage layout. | Mature open standard and strong analogy. OCI does not specify this project's local crash protocol. |
| Bazel [Remote Execution API v2.12.0](https://github.com/bazelbuild/remote-apis/blob/v2.12.0/build/bazel/remote/execution/v2/remote_execution.proto) | Logical output paths and typed output records are separate from digest-addressed bytes in CAS; the action-cache API returns results that reference available CAS content. | Production-grade protocol, but remote execution and gRPC are outside this project's scope. |
| SQLite's official [atomic commit protocol](https://www.sqlite.org/atomiccommit.html) | Durable commit needs an explicit protocol and recovery evidence; SQLite flushes data and, where required, directory state and uses a journal to choose recovery behavior. | Detailed production reference. The repository is not SQLite and must not copy its database protocol mechanically. |
| Pillai et al., [“All File Systems Are Not Created Equal”](https://www.usenix.org/conference/osdi14/technical-sessions/presentation/pillai), OSDI 2014 | Application crash-consistency protocols depend on subtle persistence properties that vary across file systems; apparently simple update sequences can be incorrect. | Peer-reviewed primary systems evidence from six Linux file systems. It does not directly establish Windows behavior. |
| Cargo's official [`cargo metadata`](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html) and [External Tools](https://doc.rust-lang.org/cargo/reference/external-tools.html) documentation | Cargo exposes a stable, versioned JSON interface for workspace packages and resolved dependencies; consumers must request a format version and tolerate compatible additions. | Authoritative interface. Metadata describes dependency facts, not project-specific architectural intent. |
| Bazel's official [visibility](https://bazel.build/concepts/visibility) documentation | A production build system can reject a dependency edge during analysis using an explicit allowlist of permitted consumers. | Strong industry precedent for executable dependency policy; migrating to Bazel would be disproportionate. |
| Murphy, Notkin, and Sullivan, [“Software Reflexion Models”](https://doi.org/10.1109/32.917525), IEEE TSE 27(4), 2001 | A high-level design model can be mapped to an extracted source model and differences classified as convergence, divergence, or absence; the method was applied to systems including Microsoft Excel. | Peer-reviewed primary evidence for architecture-conformance checking. The paper does not prescribe Cargo implementation details. |

The project-local normative inputs remain
[ADR 0009](../adr/0009-microkernel-manager-and-reusable-ir.md), the
[refactor map](../design/refactor-map.md), the
[storage namespace decision](../design/project-storage-namespaces.md), and the
[process-death recovery contract](../design/process-recovery-testing.md). External
evidence can justify mechanisms and expose risks, but it cannot override those
project decisions.

## 2. Combined recommendation

Use one dependency-inversion seam, one artifact-resolution seam, and one executable
graph policy:

```text
production composition root (squish-host / xmlsquish)
        |
        | constructs concrete adapters once
        v
typed BuildRuntimePorts
        |
        | capability calls; no paths or constructors cross inward
        v
squish-manager orchestration
        |
        | TargetId + ArtifactName
        v
ArtifactCatalog port
        |
        | descriptor / reader, verified by implementation
        v
publisher-owned journal + immutable generations + CAS

Cargo.toml facts --cargo metadata--> canonical internal edge set
                                           |
ADR edge policy ---------------------------+--> CI diff and actionable failure
```

| Issue | Recommended decision | Principal reason |
| --- | --- | --- |
| 1 | Constructor-inject a small, typed runtime bundle; assemble production adapters only in the host/root; use traits only at capabilities that need substitution. | Keeps orchestration independent of infrastructure without introducing a runtime plugin framework. |
| 2 | Resolve typed domain identities through a catalog/publisher API that returns verified descriptors or readable handles, never a recipe for generation-directory paths. | Makes the physical layout and recovery protocol one implementation invariant instead of duplicated caller knowledge. |
| 3 | Compare a canonical workspace-internal declared-edge set from `cargo metadata --format-version 1 --no-deps --locked` with a committed exact policy in every CI run. | Converts ADR prose into a deterministic conformance test using Cargo's supported machine interface. |

These decisions reinforce one another. Merely moving `Cas::open` into another manager
helper does not solve issue 1. Merely wrapping a physical URI in a newtype does not
solve issue 2. Merely checking that the graph is acyclic does not solve issue 3.

## 3. Issue 1 — inject runtime ports at the composition root

### 3.1 What the pattern does and does not require

Cockburn's original ports-and-adapters formulation emphasizes the **inside/outside
asymmetry**: application policy talks through a stable port, while SQL, files,
interactive drivers, batch drivers, and in-memory tests are adapters. It does not
require a service container, runtime discovery, a trait for every function, or
dynamic libraries. The issue's proposed statically composed bundle is therefore a
direct application of the pattern rather than a weakened version of it.

For this repository, the useful boundary is a _capability boundary_, not a wrapper
around every concrete type. Storage, action indexing, generation publication, and
fault injection cross a nondeterministic I/O or lifecycle boundary and need ports.
Pure value transformations with one supported implementation need not acquire an
interface solely for visual symmetry. A frontend/backend does need a port when the
manager is intended to select among implementations or tests must replace failure,
cancellation, or scheduling behavior; otherwise an owned concrete, deterministic
service can remain inside the injected bundle without violating the root-ownership
rule.

### 3.2 Rust-specific composition choice

Rust offers two legitimate implementation strategies:

| Strategy | Best fit | Cost |
| --- | --- | --- |
| Generic bundle, such as `BuildRuntimePorts<B, A, P, ...>` | A small number of fixed compositions, performance-sensitive fine-grained calls, and tests that can tolerate generic fixture types. | Type signatures and compile times grow with the product of service types; public generic details can infect orchestration types. |
| Concrete bundle containing `Arc<dyn BlobStore>`, `Arc<dyn ActionStore>`, and similarly coarse ports | One long-lived manager type, heterogeneous adapter selection, faulting/in-memory test substitution, and concurrent worker ownership. | Virtual dispatch and loss of inlining at the boundary; object-safety constraints. |

The official Rust documentation establishes that this is an explicit trade-off, not
a correctness hierarchy. For filesystem, SQLite, hashing, compilation-stage, and
publication calls, I/O and transformation costs dominate one vtable lookup. A
concrete `BuildRuntimePorts` struct holding owned `Arc<dyn Port + Send + Sync>` fields
is consequently a reasonable default when workers need shared ownership. Generics
remain preferable for a hot, homogeneous inner loop that evidence shows to be
dispatch-sensitive. Do not optimize this choice without measurement.

The bundle itself is important: it is a typed list of the capabilities required by a
build. It makes missing dependencies a construction error and avoids the ambient,
string-keyed lookup semantics of a service locator. The manager constructor should
receive the bundle as an ordinary value. No `global()`, lazy singleton, or
`BuildRuntimePorts::production_default()` should be callable from the manager.

### 3.3 Ownership and dependency direction

The port trait must be owned by the inward-facing crate that states the required
conversation. The adapter crate implements it and therefore depends inward. Putting
the port beside `FileArtifactPublisher` would invert only the spelling, not the
dependency. A practical split is:

```text
squish-build or a narrow manager API crate
    owns BlobStore / ActionStore / ArtifactCatalog / compile-stage capabilities

squish-store, squish-publish, frontend/backend adapter crates
    implement those capabilities

squish-host (or the executable root)
    knows both sides, opens resources, chooses lifetimes, and constructs the bundle

squish-manager
    receives the bundle and owns planning, policy, event order, cancellation,
    cache eligibility, and finalization
```

The composition root should be the only place allowed to know the complete object
graph. This is also the only layer that should translate startup configuration into
filesystem roots, database options, observer adapters, and concrete frontends or
backends. Keeping those decisions together makes production lifetime review possible:
for example, one can see whether a CAS is process-wide, project-wide, or build-wide
without searching manager branches.

### 3.4 Contract shape

The bundle should express semantic roles, not libraries:

```rust,ignore
/// Capabilities required to execute a planned build.
///
/// This is illustrative: names and ownership must follow the final crate DAG.
pub struct BuildRuntimePorts {
    pub blobs: Arc<dyn BlobStore>,
    pub actions: Arc<dyn ActionStore>,
    pub artifacts: Arc<dyn ArtifactCatalog>,
    pub frontend: Arc<dyn Frontend>,
    pub linker: Arc<dyn Linker>,
    pub instantiator: Arc<dyn Instantiator>,
    pub backend: Arc<dyn Backend>,
}
```

This is not a recommendation to expose fields publicly; constructor validation may
be preferable. The important contract properties are:

- ownership is explicit and sufficient for the scheduler's thread/lifetime model;
- each capability returns domain values and typed failures rather than paths,
  database handles, or formatted messages;
- the same constructor accepts production, deterministic memory, and faulting
  implementations;
- cancellation and observation are capabilities or explicit request context, never
  hidden process globals;
- construction performs only wiring/validation; build I/O begins when the use case
  is invoked;
- the manager cannot recover production adapters by importing their crates.

If the current crate graph makes this bundle cyclic, the correct repair is to move
the capability vocabulary inward, not to let the manager instantiate an adapter as a
temporary exception. This is exactly the kind of special case that later becomes a
second composition root.

### 3.5 Verification that would falsify the implementation claim

The claim “the manager is infrastructure-independent” is false if any of these is
observed:

- manager production code imports `Cas`, `VerifiedActionIndex`, or
  `FileArtifactPublisher`;
- a manager constructor accepts a storage root and opens infrastructure itself;
- tests need a special manager execution route rather than the production
  constructor plus alternate adapters;
- fault injection is selected by a manager environment variable or global singleton;
- changing the publisher's private directory layout requires editing manager code;
- a build initiated through an injected faulting port has different event ordering or
  cancellation rules from production.

Focused compile-time/API tests should construct the manager with memory adapters.
Behavioral tests should inject a fault at each already documented durability boundary
and reuse the normal orchestration route. Existing determinism, cache verification,
event ordering, and process-death tests remain outcome evidence; dependency injection
must not become a reason to duplicate them into a second pipeline.

### 3.6 Complexity deliberately not adopted

| Rejected option | Why it is unnecessary here |
| --- | --- |
| Runtime-loaded native plugins | Rust does not promise a stable native ABI for arbitrary trait objects; issue 1 only requires test/production substitution in one statically linked executable. |
| Reflection-based DI container or service locator | It moves missing dependencies from construction/type checking to runtime lookup and obscures lifetimes. Plain constructor injection is smaller. |
| One trait per helper function | It fragments a purposeful conversation and creates mocks that assert implementation choreography rather than domain outcomes. |
| A single universal `Runtime::call(String, Value)` | It erases domain types and recreates stringly typed message passing rejected by ADR 0009. |
| Generic parameters threaded through every plan/event/domain type | Adapter choice is a boundary concern; allowing it to parameterize stable domain values produces type noise without increasing isolation. |
| “Production defaults” inside manager | Convenience would recreate the second composition root and make tests exercise a different path. |

## 4. Issue 2 — a typed catalog must hide physical publication layout

### 4.1 The proven separation: logical name, immutable identity, physical storage

OCI and the Bazel Remote Execution API converge on a useful three-part model even
though their products differ:

1. a caller supplies or receives a logical name (`tag`, output path, target);
2. a typed record binds it to immutable content identity and metadata;
3. the storage implementation locates bytes and verifies the digest.

OCI's descriptor requires `mediaType`, `digest`, and `size`; the distribution API
allows a tag or digest to resolve a manifest. Bazel's `ActionResult` contains typed
output records with logical paths and CAS digests. In neither model must the consumer
construct the server's object-directory path. The applicable principle is narrower
than either standard: **a catalog record is a verified semantic reference, not a
leaked storage recipe**.

For this project, the following identities must remain distinct:

| Identity | Meaning | May change when |
| --- | --- | --- |
| `ProjectId` / canonical project identity | Namespace owning publication state | The canonical project identity changes. |
| `TargetId` | Domain identity of one declared build target | Target definition/identity changes, according to project rules. |
| `GenerationId` | Immutable identity of one complete published target result | Identity-bearing published membership, metadata, or bytes change. Distinct build inputs that produce the same publication may legitimately retain the same ID. |
| `ArtifactName` or `ArtifactId` | Stable logical member name within the target/generation | The declared artifact identity changes. |
| `Digest` + byte length + kind | Verifiable identity and interpretation of the bytes | The bytes or representation kind changes. |
| Physical path | Publisher implementation detail | Layout, storage engine, migration, compaction, or platform changes. |

Wrapping `".squish-publish/generations/..."` in `ArtifactUri(String)` is not this
separation. A type is useful only if its constructors and operations prevent callers
from manufacturing or parsing the hidden representation.

### 4.2 Recommended port

The catalog/publisher boundary should own publication, current-generation selection,
recovery, lookup, enumeration, and verification. An illustrative surface is:

```rust,ignore
pub trait ArtifactCatalog: Send + Sync {
    fn current_generation(
        &self,
        target: &TargetId,
    ) -> Result<Option<GenerationDescriptor>, CatalogError>;

    fn resolve_artifact(
        &self,
        target: &TargetId,
        name: &ArtifactName,
    ) -> Result<Option<ArtifactDescriptor>, CatalogError>;

    fn read_verified_artifact(
        &self,
        artifact: &ArtifactDescriptor,
    ) -> Result<VerifiedArtifactBytes, CatalogError>;

    fn list_artifacts(
        &self,
        target: &TargetId,
    ) -> Result<Vec<ArtifactDescriptor>, CatalogError>;
}
```

The exact Rust signatures may instead return verified bytes or an immutable local
materialization handle whose construction completes verification. A streaming reader
is safe only with an explicit finish/EOF contract: ordinary `Read` permits a caller to
consume a prefix before the final length and digest can be checked. The semantic
requirements are more important:

- `ArtifactDescriptor` contains typed target/generation/artifact identities, kind,
  digest, and size; it does not contain a generation-directory-relative URI for the
  caller to join to a root;
- `resolve_artifact` accepts a logical identity and itself resolves the current
  generation;
- the read/materialize capability verifies all bytes against the descriptor before it
  returns a value that callers may treat as valid;
- enumeration has a documented deterministic order;
- absence, corrupt durable state, stale/historical generation, and I/O failure remain
  distinguishable typed outcomes;
- publisher opening invokes recovery, so ordinary reads converge without a separate
  user repair command;
- validation of target/generation relationships occurs exactly once in this layer.

A CLI may still display a stable user-facing destination. That field should be a
logical destination declared by the build, not the backing generation path. If a
local path must be exposed for interoperability, return an explicit
`MaterializedArtifactPath` produced by the catalog and document its lifetime. Do not
make consumers reconstruct it from hashes.

### 4.3 Crash-consistent publication model

SQLite and the OSDI study support two cautious conclusions. First, an atomic rename
alone is not a complete durability protocol: data, directory entries, and the commit
decision have ordering and synchronization requirements. Second, recovery should be
driven by durable evidence, not by guessing which files happen to exist.

The current project design already chooses the appropriate small local protocol:

```text
prepare immutable generation completely
  -> verify each artifact digest/size
  -> persist generation manifest and directory entry as supported by the platform
  -> persist a journaled commit decision
  -> atomically replace one small "current" manifest
  -> persist the switch where the platform supports it
  -> retire the journal
```

On open, a pre-decision staged generation is uncommitted and the previous `current`
remains authoritative; a post-decision journal is rolled forward to the new complete
generation. The claimed old-or-new, never-mixed result depends on four explicit
assumptions: published generation contents remain immutable; replacement of `current`
is atomic on the supported platform/filesystem; journal replay is idempotent; and
recovery, readers, and writers coordinate through the same cross-process locking
protocol. Under those assumptions, readers follow only a committed descriptor and see
the old complete generation or the new complete generation, never a mixture. The
process-death test matrix is evidence for those named implementations and boundaries,
not a proof for arbitrary storage stacks.

The platform adapter must own the exact filesystem operations. Rust's official
[`std::fs::rename`](https://doc.rust-lang.org/std/fs/fn.rename.html) documentation
explicitly describes different Unix and Windows behavior and says cross-mount rename
does not work. Linux documents atomic replacement at
[`rename(2)`](https://man7.org/linux/man-pages/man2/rename.2.html); Windows exposes
different replacement/flush flags through
[`MoveFileEx`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).
Those references establish platform differences and justify keeping platform details
below the port. In particular, Microsoft's `MOVEFILE_WRITE_THROUGH` description is
limited to the flush of a move implemented as copy-and-delete; neither it nor Rust's
rename documentation proves general replacement durability after power loss. The
references therefore do **not** justify claiming identical power-loss guarantees on
every filesystem. The repository's existing contract is process-death recovery;
broader power-loss claims require platform-specific experiments and a stronger
specification.

### 4.4 Concurrency and consistency boundary

For this repository's single-machine, single-namespace writer model, one cross-process
publication lock plus immutable generation directories and a single current-pointer
switch is sufficient only when the lock covers recovery through commit and journal
cleanup. Read-side code should not inspect directories independently; it should ask
the catalog for a snapshot/descriptor whose internal identities were validated under
the catalog's coordination protocol. “Readers need not reproduce the writer protocol”
does not mean that the catalog implementation itself needs no lock.

If concurrent writers are allowed, publication must detect a stale expected current
generation or serialize the whole commit. Silent last-writer-wins behavior can make a
catalog record point at a generation different from the one a build finalized. This
does not require distributed consensus: all state is local and already has a
publisher/repository lock. It requires making the existing local serialization rule
part of the catalog contract. If future garbage collection can remove generations
between descriptor lookup and open, readers additionally need a lease/pin or an
atomic resolve-and-open operation to avoid that time-of-check/time-of-use race.

### 4.5 Tests with high evidential value

| Test | What it establishes | What it does not establish |
| --- | --- | --- |
| Build target and generation IDs whose hashes do not resemble logical target names; resolve by `(TargetId, ArtifactName)` | Callers do not assume the physical directory recipe. | Full crash safety. |
| Change the publisher's internal directory recipe in a test adapter without changing manager tests | The manager depends on catalog semantics rather than layout. | Production adapter correctness. |
| Corrupt one materialized byte while keeping the descriptor | Digest verification rejects inconsistent content. | Protection against malicious replacement of all trusted metadata and bytes. |
| Process exit before and after each durable commit decision, followed by an ordinary open/read | Recovery returns the old or new complete generation according to the decision record. | Arbitrary power-loss/media behavior on untested filesystems. |
| Concurrent publication attempts under the real lock | The selected serialization/conflict rule is enforced. | Distributed/multi-host behavior. |
| Enumerate after failed publication | An incomplete generation is not visible through the public API. | Reclamation of all unreachable bytes. |

Tests should assert only typed API results and public logical destinations. A test that
opens `generations/<hash>/artifacts/...` directly perpetuates the coupling it is meant
to detect. Private adapter unit tests may inspect private layout because layout is
their subject.

### 4.6 Complexity deliberately not adopted

| Rejected option | Why it is unnecessary here |
| --- | --- |
| Full OCI registry/media-type protocol | OCI supplies a strong descriptor analogy, but HTTP distribution, content negotiation, uploads, and referrers do not solve a local build catalog problem. |
| Bazel Remote Execution/gRPC APIs | Logical-output/CAS separation is useful prior art; remote workers, tenants, quotas, and RPC retry semantics are not requirements. |
| SQLite as the catalog of truth | A database can provide transactions, but the existing immutable-generation/current-pointer protocol already matches the small data model and public files. Adding a second authoritative state risks reconciliation complexity. |
| Directory scanning as recovery | Incidental files do not encode a commit decision. Scanning invites ambiguous partial generations and duplicates layout knowledge. |
| Symlink-based `current` | Cross-platform semantics, privileges, and replacement behavior are less uniform than a small validated manifest; the project already has manifest/journal machinery. |
| Returning a raw publisher root plus relative URI | That is the current coupling with a different method name. It lets every caller become a second layout parser. |
| Distributed consensus or MVCC | Publication is local and serialized. Introduce these only if a real multi-host writer model appears. |

## 5. Issue 3 — enforce the crate DAG from Cargo's machine model

### 5.1 Why an executable check is the right level

Cargo already rejects dependency cycles, but a forbidden inward-to-adapter edge can
be acyclic. Architectural validity is therefore a stricter predicate than Cargo graph
validity. Bazel's visibility enforcement is production precedent for rejecting an
otherwise buildable edge. The reflexion-model literature supplies the conceptual
model: compare an intended high-level relation set with relations extracted from the
implementation, then report unexpected and missing relations.

For this workspace, the source model is exceptionally small: Cargo packages and
direct dependency edges. A purpose-built check is preferable to a general architecture
language because the ADR already defines the vocabulary and the exact intended graph.

### 5.2 Supported metadata input

Use Cargo's versioned external-tool interface, not Cargo internals and not terminal
text:

```text
cargo metadata --format-version 1 --no-deps --locked
```

The flags are semantically significant:

- `--format-version 1` opts into the documented compatibility contract;
- `--no-deps` keeps the input to workspace member package definitions; Cargo states
  that each `packages` definition is an unaltered reproduction of its `Cargo.toml`, so
  declared optional and target-conditional dependencies remain visible without
  feature or host-platform activation;
- `--locked` makes CI reject an implicit lockfile resolution change;
- do not add `--offline` merely for determinism: Cargo documents that offline mode can
  produce a different resolution depending on locally available index/cache state.

This uses the declared dependency graph rather than one activated resolve graph. That
is the better match for an architectural rule: an inactive optional edge is still a
source-level permission to depend on another context. It also avoids fetching and
walking third-party packages solely to reject workspace-internal edges.

Cargo permits new JSON fields and enum-like values within a format version and treats
some identifiers as opaque. The parser must ignore unknown fields and compare package
IDs for identity. Human-readable names are for diagnostics and policy after the code
has verified that workspace member names are unique.

### 5.3 Extraction algorithm

Let `W` be the set of package IDs in `workspace_members`. Select the corresponding
entries from `packages`, verify that every member is described, and build a map from
each canonical workspace package directory (the parent of `manifest_path`) to its
unique package name. For each member's declared `dependencies`, canonicalize a
non-null dependency `path`; if that path is a workspace package directory, emit an
internal edge. Resolving by package directory rather than the dependency key handles
renamed crates and does not confuse a same-named external package with a workspace
member.

A robust canonical key is:

```text
(from_workspace_name, dependency_kind, to_workspace_name)
```

Then sort lexicographically and remove exact duplicates. Cargo documents `null` as an
ordinary/normal dependency kind; map only `null` to `normal`. A new non-null kind is a
format evolution that the policy does not understand and must cause an actionable
unsupported-kind failure rather than being silently weakened.

The three-part key deliberately collapses target predicates: target-conditional
declarations are included, but the same source/kind/destination permission applies on
every platform. This is simpler and stricter than platform-specific policy. A fixture
must demonstrate this behavior. If the architecture later genuinely permits an edge
on only one target family, extend the key and policy explicitly rather than letting
the CI host choose which edge exists.

There are two defensible policy modes:

| Mode | CI comparison | Appropriate when |
| --- | --- | --- |
| Allowlist | Fail only on `actual - allowed` | The architecture permits optional direct edges that are not always present. |
| Exact contract | Fail on both `actual - expected` and `expected - actual` | The ADR says its direct-edge list is exact and obsolete policy entries should not survive silently. |

ADR 0009 explicitly calls its direct-edge list the dependency contract, so the exact
mode is the better match. Diagnostics should use the reflexion-model vocabulary
without requiring it from users:

```text
forbidden internal dependency (implementation divergence):
  squish-manager --normal--> squish-store
  expected: no such direct edge; construct the adapter in squish-host/xmlsquish

missing documented dependency (policy absence):
  squish-host --normal--> squish-publish
  update code if accidental, or update ADR and policy together if intentional
```

The policy file or constant must be the single executable list linked from ADR 0009.
Do not duplicate a second edge table in workflow YAML and a third in a test fixture.

### 5.4 Determinism and failure behavior

The checker should:

1. invoke the `cargo` selected by the repository toolchain rather than a hard-coded
   user path;
2. run from or explicitly target the workspace root;
3. require successful JSON output and a complete description of every workspace
   member, without requiring a resolved third-party graph;
4. compare canonical sorted sets;
5. print every unexpected/missing edge in one run, in stable order;
6. exit nonzero for a policy mismatch and distinguish it from Cargo invocation or
   malformed-input failure;
7. never render a graph or require Graphviz;
8. avoid writing source/manifest files.

An implementation in the repository's existing CI scripting language using only its
standard JSON/subprocess libraries is enough. A Rust tool using the Cargo-recommended
[`cargo_metadata`](https://crates.io/crates/cargo_metadata) crate is also valid, but it
adds build time and another package solely for a small edge-set comparison. The choice
should follow the repository's maintenance preference, not a belief that architecture
checks must themselves be Rust.

### 5.5 Focused test design

Separate parsing/policy logic from invoking Cargo:

- a small checked-in metadata fixture contains two or three workspace packages;
- one fixture/test proves the accepted exact graph passes;
- one injects a forbidden but acyclic edge and asserts that both crate names,
  dependency kind, and a stable explanatory label appear;
- one covers an inactive optional internal declaration without enabling its feature;
- one covers a target-conditional declaration and proves that its predicate is
  deliberately collapsed rather than hidden by the Linux CI host;
- one verifies an external path dependency is ignored because its canonical package
  directory is not a workspace member;
- one verifies unknown extra JSON fields are ignored;
- one verifies an unknown non-null dependency kind fails clearly;
- one verifies malformed metadata or duplicate workspace names fail clearly rather
  than weakening the policy.

At least one integration/self-test should execute the checked policy against the real
workspace. Unit fixtures establish rejection behavior; the real invocation establishes
that the committed policy and current manifests converge today.

### 5.6 CI and documentation integration

Run the architecture check as an ordinary Linux CI step using the same locked
workspace and toolchain as `cargo check`. One Linux CI invocation is enough to check
the declared graph because package definitions retain target-conditional dependency
declarations, and fixtures must guard that extraction rule. This does not prove the
Python checker's path canonicalization or execution behavior on Windows/macOS; those
are separate cross-platform implementation questions if the check is later required
there.

ADR 0009 should link to the executable policy. A graph change then requires one
reviewable patch containing:

1. the `Cargo.toml` dependency change;
2. the policy change;
3. ADR rationale if architectural intent changed; and
4. any port/ownership migration needed to make the new edge legitimate.

This workflow turns the checker into a change-review instrument, not a naming-style
test. The useful review question is “why should this context know that context?”, not
“how do we make CI green?”.

### 5.7 Complexity deliberately not adopted

| Rejected option | Why it is unnecessary here |
| --- | --- |
| Parse every `Cargo.toml` directly | Reimplementing workspace inheritance, renamed dependencies, feature activation, target conditions, and resolution is error-prone; Cargo already exposes the model. |
| Parse `cargo tree` terminal output | `cargo tree` is a human visualization with deduplication and formatting options; `cargo metadata` is the versioned machine interface. |
| Depend only on Cargo cycle detection | A forbidden adapter edge can be acyclic and compile successfully. |
| General license/supply-chain policy tools | Their primary model is versions, sources, licenses, or advisories, not this repository's bounded-context edge contract. |
| Bazel migration solely for visibility | Bazel demonstrates the value of enforcement but would duplicate the existing Cargo build model and impose disproportionate maintenance cost. |
| Source-level import analysis | Crate dependencies are declared facts and the issue is specifically the crate DAG. Module/import conformance can be added only if a concrete intra-crate boundary problem appears. |
| Transitive-closure allowlist | The ADR specifies direct ownership edges. Policing every transitive path creates noisy derived policy and obscures the direct edge that should be fixed. |

## 6. Blind spots and adversarial review

### 6.1 “Injection” can preserve the same leak

Moving a `StorageLayout` or publisher root into `BuildRuntimePorts` while leaving the
manager to join generation paths is dependency injection in syntax only. Issue 1 is
complete only when issue 2's semantic catalog operation is injected. The strongest
counterexample is a test publisher with a completely different internal layout: the
same manager build and inspection tests must still pass.

### 6.2 A broad port can become an untyped service locator

One `Runtime` trait with dozens of unrelated methods centralizes construction but
couples every use case to every capability and makes test doubles enormous. Group by
cohesive conversation (`BlobStore`, `ActionStore`, `ArtifactCatalog`, compiler stage),
then aggregate those ports in one bundle. The bundle is the composition convenience;
it is not itself an excuse to erase capability boundaries.

### 6.3 Exact graph checks can fossilize accidents

An exact allowlist makes current structure executable, including mistakes. The
mitigation is not a weaker checker; it is a review rule that every policy addition
must name the ownership reason and update the ADR when intent changes. Missing-edge
diagnostics also remove stale accidents from the contract.

### 6.4 Dev and build dependencies are real edges, but not identical risks

Ignoring dev-dependencies lets tests couple core crates to adapters and encourages
production helpers to migrate later. Treating all kinds as identical can, however,
forbid legitimate black-box test support. Retain dependency kind in extraction and
make any test-only exception explicit. Build-dependencies execute during builds and
deserve at least the same scrutiny as normal edges even though they are not linked
into the library artifact.

### 6.5 Digest verification is not authenticity

A digest proves bytes match a trusted descriptor; it does not prove that the
descriptor was authored by a trusted party. Issues 1–3 concern local integrity and
architecture, not a remote adversary. Do not add signatures, transparency logs, or
OCI trust machinery without a separately stated attacker and distribution model.

### 6.6 Process-death evidence is not power-loss proof

The repository's process-recovery tests intentionally exercise abrupt process exit.
The OSDI evidence warns against generalizing that result to storage-device or kernel
failure. Documentation and errors should preserve that scope. Stronger durability
claims need platform/filesystem matrices and real crash or fault-injection evidence.

## 7. Implementation sequence

The lowest-risk order follows information ownership rather than issue number:

1. Define the catalog query/read contract and typed identities in the inward-facing
   port crate. Keep the existing publisher protocol behind its implementation.
2. Move catalog recovery, generation resolution, URI/path validation, and digest
   verification behind that port; convert manager callers to logical identities.
3. Define `BuildRuntimePorts` around the now-semantic catalog plus storage/action and
   compilation capabilities.
4. Construct production adapters in `squish-host` or the executable root and delete
   all manager-side production constructors/imports.
5. Add memory and faulting implementations through the same constructor and rerun
   the existing determinism, cancellation, event-order, cache, and process-death
   suites.
6. Extract the resulting real direct-edge set through `cargo metadata`, review it
   against ADR 0009, commit it as the executable exact policy, and add the CI gate.
7. Link ADR 0009 and the refactor/manager documentation to the final ports, catalog
   semantics, and policy checker; remove historical “known coupling” text rather than
   leaving contradictory specifications.

This sequence avoids a temporary port that merely transports publisher paths and
avoids encoding today's known-bad crate edges into the CI baseline.

## 8. Decision checklist

### Issue 1

- [ ] The manager constructor requires an explicit typed runtime bundle.
- [ ] The manager crate cannot name or construct production store/publisher adapters.
- [ ] The host/root visibly owns configuration-to-adapter construction and lifetime.
- [ ] In-memory and faulting adapters use the production orchestration path.
- [ ] No runtime plugin loader, DI container, or ambient service locator was added.

### Issue 2

- [ ] Lookup accepts typed logical target/artifact identities.
- [ ] Returned descriptors carry kind, digest, size, and immutable identities.
- [ ] Reading/verifying bytes is a catalog capability; callers do not join roots and
  URIs.
- [ ] Recovery and physical layout interpretation occur only inside the publisher.
- [ ] Successful build results expose an unambiguous stable logical destination.
- [ ] Process-death tests prove old-or-new complete generations at decision boundaries.

### Issue 3

- [ ] The check consumes `cargo metadata --format-version 1 --no-deps --locked`.
- [ ] Both endpoints are filtered by opaque package ID membership in
  `workspace_members`.
- [ ] Dependency kind is retained; target-conditional declarations are included and
  their predicates are deliberately collapsed by documented policy.
- [ ] Exact-set differences are stable and actionable.
- [ ] Fixtures reject a forbidden acyclic edge and cover optional/target-specific
  edges.
- [ ] ADR 0009 links to the one executable policy used by CI.

## 9. Provisional judgment

The issues' proposed directions are well supported, but the important unit of change
is the whole boundary, not three local patches. The strongest industry analogies
favor typed descriptors and explicit dependency visibility; the strongest academic
evidence warns that architectural drift and crash protocols both require executable
evidence rather than prose. For this repository, the simplest adequate design is
static constructor composition, a semantic artifact catalog over immutable
generations, and a small Cargo-metadata set comparison in CI.

This judgment should be revised if the product acquires runtime third-party plugins,
multi-host concurrent publishers, untrusted remote artifact distribution, or a crate
graph too dynamic for a direct-edge policy. None of those conditions is established
by issues 1–3, so designing for them now would add mechanisms without removing a
present failure mode.

## 10. Highest-value remaining evidence

No further literature search is required to choose the three boundaries. The most
valuable next evidence is project-specific and empirical:

1. run the existing process-death matrix after the catalog boundary changes, on both
   Linux and Windows, and record the actual filesystem/platform under test;
2. keep a `cargo metadata` fixture captured with the minimum supported Cargo version
   and exercise the checker with the current toolchain, so compatibility assumptions
   are observable rather than inferred;
3. change the private publisher layout in an adapter-focused test and prove manager
   tests remain unchanged; and
4. profile only if trait-object dispatch appears in a measured hot path—otherwise the
   I/O-bound boundary does not justify generic-type proliferation.

A future multi-host publisher or untrusted artifact source would trigger a new search
on distributed commit/CAS authenticity protocols. Importing that literature and
machinery before such a requirement exists would not improve the present design.
