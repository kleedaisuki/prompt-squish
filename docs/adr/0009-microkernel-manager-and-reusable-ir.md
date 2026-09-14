# ADR 0009: Microkernel Manager and Reusable Intermediate Representation

- Status: Accepted
- Date: 2026-09-15
- Supersedes: [ADR 0008](0008-project-manager-architecture.md) in full;
  [ADR 0006](0006-binary-module-layout.md)'s organizational and package-boundary decision
- Preserves: [ADR 0007](0007-unified-macro-expansion.md) and the DSL primitives
  and semantics it defines

## Context

`xmlsquish` is being repositioned from a command-line compiler into the single
entry point for a project system. `fmt`, `build`, `add`, and `remove` are not
four unrelated branches around the existing pipeline. They are operations over
one project model, one resolved dependency state, one action planner, one state
store, and one presentation contract.

The current implementation also conflates four things that have different
lifetimes: XML syntax, reusable compiled modules, one entry instantiation, and
one serialized output. Its printable intermediate XML is produced too late to
reuse parsed and validated modules and is too backend-specific to support
non-XML products. The DSL remains XML-authored, but XML is a frontend syntax,
not the identity of the compiler core or of every future output.

This decision describes the intended complete system. It is not organized
around a reduced first release. Implementation work follows information and
dependency topology; incomplete slices may use explicit temporary adapters,
but the architecture has one final model from the beginning.

Backward compatibility with the loose-file command grammar, sibling
`*.i.xml`/`*.o.xml` products, or the existing internal module layout is not a
constraint. Language compatibility is different: ADR 0007's source primitives,
scope rules, import semantics, expansion behavior, provenance requirements, and
budgets remain the semantic contract.

## Decision summary

1. `xmlsquish` is a statically composed microkernel executable. The kernel owns
   bootstrap, lifecycle, command dispatch, cancellation, and exit status; all
   domain work occurs through typed ports and structured events.
2. The implementation is divided into bounded contexts with an enforced,
   acyclic crate dependency graph. A context owns its invariants and cannot
   reach through another context's adapter or storage layout.
3. The compilation model has four explicit representation domains: lossless
   XML CST, relocatable `ModuleIR`, linkage/execution evidence
   (`StaticLinkMap` plus `ExpansionTrace`), and backend-neutral
   `LinkedDocumentIR`.
4. `ModuleIR` is a deterministic, versioned binary container. It retains all
   semantic, source, symbol, relocation, span, line-map, and debug information;
   it is not a dump of Rust structs.
5. Projects include manifests, workspaces, dependency requirements, a resolver,
   and a machine-managed exact lockfile. `add` and `remove` update manifest and
   lock state as one recoverable logical transaction.
6. Builds are mathematical action graphs. Content-derived action keys make
   caching a property of declared inputs rather than an executor side effect.
7. Immutable blobs live in a content-addressed store (CAS). SQLite in WAL mode
   stores transactional indexes, action results, leases, and garbage-collection
   metadata; it is not the semantic source of truth for blob identity.
8. The first backend is `squish`. Products use `*.prompt`; complete debugging
   data is published as a versioned `.psdbg` companion when requested by the
   selected profile.
9. Human terminal output is colored and interactive when appropriate. NDJSON
   is the stable machine interface. Rendering is downstream of structured
   events and never occurs in workers.

## System model

### Microkernel

The microkernel is a composition root, not a general plugin host and not a
place for business logic:

```text
argv / environment / terminal capabilities
                  |
                  v
+------------------------------------------+
| xmlsquish kernel                         |
| bootstrap | dispatch | lifecycle | exit  |
+------------------------------------------+
       | commands                   ^ events/results
       v                            |
+----------------------------------------------------+
| project | resolver | build | format | presentation |
| source  | frontend | IR    | link   | backend      |
| store   | artifact | diagnostic | scheduler        |
+----------------------------------------------------+
```

Composition is static and auditable. “Microkernel” here means that the root
depends on narrow ports, constructs adapters, and coordinates their lifetimes.
It does not mean runtime-loaded native plugins, an ambient service locator, or
stringly typed message passing. Future frontend and backend registration may
use compile-time registries or an explicitly versioned process protocol; Rust
ABI dynamic loading is outside this decision.

The kernel may perform only these responsibilities:

- parse a top-level command into a typed request;
- construct per-invocation domain services and cancellation scope;
- invoke the manager port;
- forward structured events to the selected presentation sink;
- map the final typed outcome to a documented process exit status; and
- ensure terminal state and staged resources are cleaned up on interruption.

It must not parse XML, interpret a manifest, resolve a version, name an
artifact, execute an action, format a diagnostic, or write ANSI sequences.

### Ports

Ports express capabilities rather than expose concrete objects:

```text
Manager              execute a typed project use case through one orchestration path
ProjectRepository    load/edit manifest and workspace intent
DependencyResolver   resolve intent to an exact graph
SourceProvider       resolve SourceRef and load immutable SourceEnvelope
Frontend             lower one source to relocatable ModuleIR
IrCodec              validate/encode/decode versioned IR containers
Linker                relocate a frozen module closure
Instantiator          evaluate one entry and produce document plus trace
Backend               lower LinkedDocumentIR to product bytes
ActionStore           look up/record action-key results
BlobStore             put/get verified immutable blobs
ArtifactPublisher     reserve, stage, and atomically publish products
EventSink             receive typed lifecycle/diagnostic/progress events
Clock/TerminalProbe   adapter-only capabilities, never implicit semantics
```

Ports return domain values and typed failures. They do not print, terminate the
process, discover global singletons, or smuggle an open database connection
through an IR type. Adapters own filesystem, network, SQLite, and terminal
details. Tests substitute in-memory adapters at the same boundaries.

## Bounded contexts and crate DAG

The repository becomes a Cargo workspace. Crate names below state ownership;
they are private implementation crates unless separately declared public.

| Crate/context | Owns | Forbidden knowledge |
| --- | --- | --- |
| `squish-kernel` | bootstrap contracts, command/result envelope, lifecycle | XML, TOML, SQLite schemas, ANSI |
| `squish-manager` | unified `fmt`/`build`/`add`/`remove`/`inspect` use cases and the `Manager` implementation | concrete terminal rendering, XML parser internals |
| `squish-project` | manifests, workspaces, targets, profiles, typed edits | XML operations, cache layout |
| `squish-resolver` | requirements, exact package graph, lock model | DSL syntax, terminal output |
| `squish-source` | canonical `SourceIdentity` validation, `SourceRef` resolution, source envelopes and snapshots | wire event schemas, macros, artifact publication |
| `squish-syntax-xml` | lossless XML tokens/CST, lexical spans and trivia | DSL meaning, project resolution |
| `squish-ir` | module, static link map, trace and document schemas; typed IDs and validation traits | lossless XML editing, filesystem paths, CLI concepts |
| `squish-ir-codec` | canonical binary container and compatibility checks | linker policy, project discovery |
| `squish-frontend-xml` | XML parsing, DSL validation, CST-to-IR lowering | dependency selection, output bytes |
| `squish-format-xml` | lossless XML rewrite policy and preservation checks | linker and backend policy |
| `squish-link` | graph linking, symbol relocation, static verification | terminal, physical artifact paths |
| `squish-runtime` | entry instantiation, budgets, expansion trace | manifest mutation, serialization style |
| `squish-backend-api` | backend request/result contract | XML frontend types |
| `squish-backend-squish` | current squish product semantics | project resolution, CLI |
| `squish-build` | action model, planning, scheduling, plus `ActionStore`, `BlobStore`, `ArtifactPublisher`, and `FileTransaction` ports | concrete storage, parsing details, ANSI |
| `squish-store` | concrete CAS/SQLite adapter, leases and GC | language semantics |
| `squish-artifact` | concrete destination validation, staging and publication adapters | compilation and resolution |
| `squish-diagnostic` | diagnostic codes, labels, causes, suggestions | rendering and stream ownership |
| `squish-protocol` | opaque cross-context `SourceId` wire newtype; versioned command, diagnostic, event, plan, result and NDJSON envelopes | source-identity validation, domain execution and rendering policy |
| `squish-presentation` | human/NDJSON rendering and micro-interactions | executing or mutating jobs |
| `xmlsquish` | binary composition root and adapters | domain implementations in `main` |

The permitted dependency structure is acyclic. Arrows below mean “depends on”:

```text
squish-protocol ----> diagnostic
source       ----> protocol, diagnostic
syntax-xml   ----> protocol, diagnostic
project      ----> source, protocol, diagnostic
resolver     ----> project, source, protocol, diagnostic
ir           ----> protocol, diagnostic
ir-codec     ----> ir
frontend-xml ----> syntax-xml, source, ir
format-xml   ----> syntax-xml, diagnostic
link         ----> ir, source
runtime      ----> link, ir
backend-api  ----> ir
backend-squish ---> backend-api
kernel       ----> protocol
build        ----> protocol, project, resolver, source, ir, frontend-xml,
                   link, runtime, backend-api, diagnostic
store        ----> build (storage ports), source, diagnostic
artifact     ----> build (publication/transaction ports), diagnostic
manager      ----> kernel (Manager port), protocol, build, project, resolver,
                   format-xml, ir-codec
presentation ----> protocol, diagnostic
xmlsquish    ----> kernel, manager, presentation, concrete adapters
```

The drawing groups many direct edges for readability. The normative ownership
rule is that `squish-protocol` owns only the opaque, serializable `SourceId`
newtype used across context boundaries, while `squish-source` alone validates
and canonicalizes rich `SourceIdentity` values and implements providers. IR
depends on the opaque protocol ID and never reimplements identity rules. Both
frontend and formatter depend on the
XML-specific `squish-syntax-xml`; and only `squish-manager` implements the
kernel's `Manager` port by composing the build, formatting, project mutation,
and inspection use cases. `squish-build` owns the action engine, build plan, and
storage/publication port definitions, not orchestration of every command.
Consequently concrete `squish-store` and `squish-artifact` depend on those ports;
the build engine never depends on either adapter. This exact direct-edge list,
rather than the visual layout of crates, is the dependency contract enforced by
CI. (“`protocol`” in the list means `squish-protocol`.)

The exact visual placement is less important than these enforced rules:

- core identities and immutable schemas point inward;
- adapters point toward port-defining crates, never the reverse;
- frontend and backend do not depend on each other;
- `squish-manager` orchestrates use cases and `squish-build` schedules build
  capabilities without owning their semantics;
- `squish-presentation` consumes public events but cannot call executors; and
- `xmlsquish` is the only crate allowed to assemble the complete graph.

CI checks the crate graph (including forbidden dependencies) so architectural
boundaries cannot silently decay into module naming conventions.

## Four representations

The representations are deliberately not collapsed. Each answers a different
question and has a different identity.

| Representation | Question answered | Required information | Lifetime |
| --- | --- | --- | --- |
| Lossless XML CST | What exact source bytes and trivia did the author write? | tokens, trivia, entities, comments, CDATA, PI data, namespace spelling, byte ranges | editor/format action |
| Relocatable `ModuleIR` | What does this source module declare and execute before imports are bound? | typed operations, symbols, unresolved imports, relocations, source/debug tables, dialect/features | reusable across builds |
| Linkage/execution evidence | How was a module graph bound, and how did one entry invocation produce occurrences? | persistent `StaticLinkMap` with bindings/relocations; per-run `ExpansionTrace` with frames, spans, captures, counters and emitted ranges | link cache plus one instantiation/debug bundle |
| `LinkedDocumentIR` | What ordered backend-neutral document did evaluation produce? | elements, expanded names, attributes, text and structural events, provenance handles | one backend invocation |

The lossless CST is not serialized as `ModuleIR` and `ModuleIR` cannot be used
to rewrite source. Within the third domain, `StaticLinkMap` contains only
canonical resolved import/symbol bindings and relocation results, while
`ExpansionTrace` contains dynamic occurrence provenance. Neither duplicates
module declarations or substitutes for a module. `LinkedDocumentIR` contains
no XML serialization choices: prefixes, escaping, whitespace squishing, and
final byte encoding are backend responsibilities.

Across these schemas, provenance is an acyclic `Origin` graph with the closed
node vocabulary `SourceSpan`, `Expansion`, `Import`, `Fused`, `Synthetic`, and
`Unknown`. A module may contain only its unit-static subset; the static link map
adds cross-module import/binding origins; the trace adds dynamic expansion
origins; and document nodes reference the resulting origin IDs. `Unknown` is an
explicit decoded fact for permitted information absence, never a fallback used
to avoid recording provenance that is available.

The semantic path is:

```text
SourceEnvelope --XML frontend--> ModuleIR
ModuleIR closure --relocate/verify--> StaticLinkMap
ModuleIR closure + StaticLinkMap --reconstruct--> session-only LinkedProgram
LinkedProgram + entry + args --instantiate--> LinkedDocumentIR + ExpansionTrace
LinkedDocumentIR --squish backend--> *.prompt
```

`LinkedProgram` is a session-only view reconstructed from `ModuleIR` plus
`StaticLinkMap`. It may be memoized in process but is never a CAS artifact and
is not a fifth public interchange representation. `K_link` names the persistent
`StaticLinkMap`, not a serialized runtime object.

## Relocatable Module IR

### Logical schema

`ModuleIR` uses semantic newtypes rather than interchangeable strings or array
indices: `ModuleId`, `SourceId`, `StringId`, `SpanId`, `DefId`, `SymbolId`,
`ImportId`, `RelocId`, and `ExpandedNameId`. Tables are flat or arena-backed;
recursive ownership, `Rc`, `OnceCell`, filesystem handles, rendered XML, and
session memoization are forbidden.

Every module retains:

- normalized logical source identity and a digest of the exact source bytes;
- DSL language/dialect version and all required capability bits;
- expanded names, strings, constants, arguments, slots, fills, matches,
  insertions, expansions, and other ADR 0007 operations;
- definitions, imports as unresolved `SourceRef` plus their source spans,
  exports, symbols, and relocation records;
- complete source envelope(s), including exact bytes and digest, byte spans,
  line starts, lexical-to-semantic mappings, and provenance needed to
  reconstruct diagnostics;
- a unit-static origin DAG. Its nodes are explicitly typed as `SourceSpan`,
  `Import`, `Fused`, `Synthetic`, or `Unknown`, so lowering can combine origins
  without inventing one misleading source location. `Expansion` nodes and
  dynamic call/capture facts exist only in `ExpansionTrace`; cross-module
  relocation facts exist only in `StaticLinkMap`;
- validation summaries and limits that affect interpretation; and
- deterministic table order or explicit canonicalization rules.

No optimization may erase source or debug facts from reusable IR. A transform
that removes executable structure must retain a mapping from transformed
entities to the original entities and spans. Stripping is a publication policy
for optional companions, never mutation of authoritative cached IR.

### Wire container and codec

The `.xsir` storage container is specified independently from Rust data
layouts. It begins with a fixed header:

```text
magic | container-schema | fixed-little-endian marker | dialect
semantic-epoch | required-features | producer-metadata-offset
section-directory-offset | container-checksum
```

The section directory gives typed section identifiers, schema revisions,
offsets, lengths, alignment, compression codec, and per-section checksums.
Required sections include identities, strings, symbols, operations,
imports/relocations, sources/spans/line maps, and validation metadata. Unknown
required features or sections are rejected with a typed diagnostic; unknown
optional sections are preserved by inspection/copy tools where feasible and
may be ignored only when their declared capability does not affect the
requested link/backend semantics. Frontend and backend remain crate-independent,
but interoperability is constrained explicitly by IR dialect and capability
negotiation; the IR is not claimed to be a universal document language. All
integer widths, byte order, normalization,
sorting, and hash algorithms are named by the specification.

All multi-byte numeric fields are little-endian; decoders do not accept a
host-native alternative. `semantic-epoch` changes whenever compiler behavior
that can affect meaning changes and participates in the relevant action key.
Producer name/version/build metadata is diagnostic and reproducibility context
in a separate section; it does not invalidate semantically identical artifacts
merely because a build timestamp or revision label changed.

Encoding the same logical module under the same codec/schema produces exactly
the same bytes on Linux, macOS, and Windows. Rust enum discriminants, `usize`,
`PathBuf`, map iteration order, struct padding, and implementation-specific
serializer defaults never enter the wire format. Decoding is bounded, validates
offsets before allocation, verifies checksums and typed references, and either
returns a fully valid module or no module.

Container schema, DSL dialect, feature set, semantic epoch, and producer
metadata are independent dimensions. Schema migration is an explicit decode/upgrade
operation. A newer compiler may read an older supported schema, but it never
lies by relabeling incompatible bytes. `xmlsquish inspect` exposes a stable
machine-readable view without asking tools to query cache internals.

## Project, workspace, resolution, and lock state

### Authoritative states

The system distinguishes three forms of state:

```text
Manifest/workspace intent --> exact lock graph --> derived CAS/index/artifacts
       human-authored          machine-managed       disposable/reconstructible
```

Two immutable records bridge authoritative intent and derived execution state:
`BuildInputSnapshot` records the exact manifest/lock revisions, complete source
envelopes (logical identity, exact bytes, encoding/BOM facts, content digest and
load outcome), remote package blobs, and options observed by one invocation;
`BuildRecord` records the snapshot digest, materialized action keys, result
digests, diagnostics/trace references, publication generations, and final
outcome. The lock answers *which package resolution*; the input snapshot answers
*which bytes this build observed*; the build record answers *what happened*.

`xmlsquish.toml` declares package identity, workspace membership, targets,
exports, dependency requirements and sources, build/format profiles, frontend
dialect, backend selection, arguments, and resource budgets. A workspace has a
single resolution root, lockfile, target directory, and policy set while member
packages retain explicit ownership and manifests.

The lockfile records every selected package identity/version/source, immutable
content digest, transitive edges, resolver version/policy, and relevant source
metadata. Path dependencies record their locator and declared manifest/package
identity, not a content snapshot: they remain deliberately mutable. Exact
content digests for path packages belong to the per-invocation
`BuildInputSnapshot` and durable `BuildRecord`, never the resolution lock.
Registry archives and Git dependencies require an immutable content digest or
commit identity.

The package graph, source import graph, macro expansion graph, and action graph
are distinct. Source import cycles remain legal under ADR 0007. Resolver and
action cycle policy must not be inferred from that fact.

### Resolution modes

- Normal interactive commands reconcile manifest intent and lock state under a
  documented resolver policy and report lock changes.
- `--locked` requires a present, current lock and never modifies it.
- `--frozen` additionally forbids network access and lock mutation. It still
  snapshots the current bytes of workspace and path sources; it does not forbid
  an author from editing local source or pretend path dependencies are frozen.
  Every required remote content blob must already be available.
- `--offline` forbids network access but may resolve using locally available
  index and content data when the command otherwise permits lock changes.

Resolver decisions are deterministic for an identical manifest set, registry
snapshot, resolver version, and prior-lock preference. They are emitted as a
plan before mutation and available as NDJSON. Package aliases affect source
visibility but not package identity. Cross-package imports bind through the
importing package's direct dependency alias and the dependency's declared
export; cache or checkout paths never become `SourceId`.

### `add` and `remove` transaction

`add` and `remove` are typed domain edits, not TOML string manipulation. They:

1. read affected manifests/lock with revision digests and construct candidate
   intent without holding the writer lock;
2. perform registry/network work, resolve, and validate the candidate graph
   optimistically;
3. acquire the selected workspace's short-duration writer lock and re-read the
   authoritative revision digests;
4. if they changed, release the lock and automatically recompute from the new
   state, subject to a bounded, observable retry policy;
5. while revisions still match, stage all affected manifest and lock bytes plus
   a recovery record;
6. atomically commit a transaction marker and replace the staged files;
7. reconcile or recover any interrupted commit on the next manager invocation;
8. publish structured change events and release the lock.

Failure before the commit decision changes no authoritative file. Recovery
makes a decided transaction converge to either the complete old set or the
complete new set; users are not asked to repair partial manager state. Unrelated
dependencies are retained from the prior lock when they still satisfy intent.
`remove` prunes unreachable lock entries but never deletes shared CAS content as
part of the foreground command. `--dry-run` performs the identical candidate
resolution and validation without the commit steps.

The writer lock is never held across ordinary network latency. Exhausting the
conflict retry budget produces a precise concurrent-modification diagnostic and
preserves both authoritative files; routine contention is handled by the
manager rather than immediately delegated to the user.

## Mathematical action model

The scheduler operates on nodes, while the cache operates only on deterministic
transform actions. A schedulable node is

\[
N=(id,k,D,r,e,p),
\]

where `id` is stable within the invocation, `k` is its kind, `D` its
prerequisites, `r` its resource class, `e` is `Transform`, `ReadEffect`,
`WriteEffect`, or `Coordination`, and `p` is a typed payload. Network resolve,
filesystem snapshot, publication, and transaction commit are effect nodes.
They have invocation identities, retry/idempotency contracts, and recovery
rules, but are never treated as pure merely to fit the cache model.

A cacheable transform action is the immutable tuple

\[
A = (k, v, o, R, r, f)
\]

where `k` is action kind, `v` is the semantic implementation/schema epoch (not
diagnostic producer metadata), `o` is a
canonical option vector, `R` is an ordered input recipe whose resolved value
`I` is the ordered vector of declared semantic digests, `r` is a resource class, and
`f` is a deterministic domain transform behind a port. A frozen plan stores
`KeyRecipe { kind, semantic_epoch, options, inputs: [OutputRef | BlobRef] }`,
not a premature key. Once predecessor outputs exist, every `OutputRef` resolves
to its semantic input digest and the action key materializes as

\[
K(A)=H(\mathrm{domain}\parallel k\parallel v\parallel
       \mathrm{encode}(o)\parallel I).
\]

Graph dependencies control readiness but ordinary predecessor action keys are
not hashed merely because an edge exists. Only the semantic digests of outputs
actually consumed by the action enter `I`. This prevents a non-semantic change
to a producer recipe from invalidating a consumer while still invalidating the
consumer whenever consumed meaning changes.

The domain separator and length-delimited canonical encoding prevent ambiguous
concatenation. Ordered vectors use semantic order; unordered input sets are
sorted by canonical typed identity before hashing. Environment variables,
working directory, locale, terminal capability, current time, random state,
and physical cache paths affect a key only when the action explicitly declares
them as semantic inputs. Secrets are never placed in a key or event; actions
whose results depend on secrets are non-cacheable unless a purpose-specific,
non-disclosing identity is defined.

For a deterministic action, equal valid keys imply observationally equivalent
declared outputs:

\[
K(A_1)=K(A_2) \land Valid(A_1,A_2)
\Rightarrow Digest(outputs(A_1))=Digest(outputs(A_2)).
\]

This is an invariant to test, not an assumption licensed by using a hash. Every
implementation or schema change capable of changing output must change `v` or
a declared input. Cache hits are verified result records whose referenced blobs
exist and match their digests; a corrupt or incomplete record becomes a miss
and a diagnostic event, not a poisoned success.

The generic formula is specialized at phase boundaries so reuse is neither too
broad nor accidentally destroyed by downstream options:

\[
\begin{aligned}
K_{module} &= H(frontend\ semantics, dialect, source\ digest,
                 lowering\ options),\\
K_{link} &= H(linker\ semantics, entry, sorted\ module/interface\ digests,
               resolved\ binding\ graph\ digest, link\ options),\\
K_{instantiate} &= H(linked\ semantic\ digest, arguments,
                      evaluator\ identity, policies, budgets),\\
K_{backend} &= H(document\ semantic\ digest, backend\ identity/version,
                  backend\ options).
\end{aligned}
\]

The linked semantic digest consumed by instantiation is

\[
D_{linked}=H(ordered\ ModuleIR\ semantic\ digests,
             StaticLinkMap\ semantic\ digest).
\]

The module order is the canonical linked-module identity order, not discovery
or task-completion order.

In particular, `K_link` never contains runtime arguments or backend
configuration. Changing presentation, output encoding, or squish policy must
not invalidate frontend lowering or symbol relocation. Conversely, evaluator
budgets and policies belong to instantiation even when a particular input does
not happen to exhaust them. The resolved-binding digest canonically encodes
every import edge, importing module and import slot, selected package/export,
target module/symbol, and edge role. Sorting is over complete typed edge tuples;
it cannot discard multiplicity, direction, or which import a binding satisfies.

Every persistable result distinguishes an **artifact digest**, computed over
its exact encoded bytes, from a **semantic digest**, computed over its canonical
logical value. Artifact digests verify storage and publication; semantic
digests connect phase keys across compatible codec revisions. The two may be
equal only when a format explicitly defines its bytes as the canonical semantic
encoding, and code must not assume that equality.

### Action DAG

Planning has an analysis segment followed by a closed execution segment:

```text
Resolve -> Snapshot -> Scan
                         |
                         +--> CompileUnit(source A) --+
                         +--> CompileUnit(source B) --+--> Link
                         +--> CompileUnit(source C) --+      |
                                                            v
                                                       Instantiate
                                                            |
                                                            v
                                                         Backend
                                                            |
                                                            v
                                                         Publish
```

`Resolve`, `Snapshot`, and `Scan` are analysis nodes. `Resolve` fixes the package
graph; `Snapshot` fixes every logical source outcome; `Scan` discovers the
complete static closure. Their results deterministically materialize a closed
execution DAG before any `CompileUnit` becomes runnable. `CompileUnit` creates
relocatable modules independently; `Link` relocates and verifies their graph;
`Instantiate` applies one entry, arguments, and execution budgets; `Backend`
produces product bytes; `Publish` reserves and commits user-visible artifacts.

The graph above is the build specialization, not a build-only orchestrator.
`fmt` plans `Snapshot -> ParseLossless* -> Rewrite* -> VerifyEquivalent* ->
PublishTransaction`; `add` and `remove` plan `LoadIntent -> EditCandidate ->
ResolveCandidate -> ValidateCandidate -> CommitTransaction`. All command plans
use the same action IDs, readiness states, resource permits, cancellation,
event, dry-run, and result model. Mutation commit actions are deliberately
non-cacheable; their pure candidate-computation predecessors may be cached when
their complete inputs are declared.

An invocation therefore has a stable request identity during analysis and a
`PlanId = H(request identity, resolved graph, snapshot digest, canonical closed
execution DAG)` after Scan. The scheduler may execute the known analysis prefix,
but it cannot admit compile, link, backend, or publication work until the closed
execution plan has passed destination, capability, key-completeness, cycle, and
resource validation. `--dry-run` performs analysis and reports that exact closed
plan without executing its transform or effect nodes.

The source import graph may contain strongly connected components. Scan interns
a `SourceId` before traversal; compile actions are per source; link processes
the complete module graph and its cycles. The build DAG therefore does not
pretend that legal import cycles are impossible. Target-to-target producer
edges are present only where a target consumes another target's declared
product.

### Scheduling

The scheduler maintains `pending`, `ready`, `running`, `succeeded`, `failed`,
and `blocked` state for immutable action IDs. A ready action may run when all
required predecessors succeeded and its resource permit is available. Global
and resource-class bounds control CPU, memory-heavy linking, filesystem work,
and network resolution without spawning an unbounded task per source.

Failure blocks only transitive dependents whose required product is missing.
Independent actions continue. Cancellation stops admitting work, propagates a
token to cooperative executors, waits for or safely abandons staged work, and
leaves authoritative project state recoverable. Workers never write terminal
streams or final destinations; they return `ActionResult`, diagnostics,
metrics, and structured events. Reports use stable plan identity, not completion
order. Concurrency can change latency but not product bytes, diagnostic facts,
summary counts, or exit status.

Planning validates destinations, collisions, capabilities, key completeness,
resource declarations, and cycles before execution. `--dry-run` and machine
output expose this same plan; they do not reconstruct a different approximation
of it.

## CAS and SQLite state store

The content-addressed store holds immutable source snapshots, dependency
archives, `.xsir` containers, `StaticLinkMap` and `LinkedDocumentIR` artifacts where profitable,
backend products awaiting publication, and `.psdbg` components. A blob key is
`algorithm:digest(bytes)`. Writes use a temporary file, streaming digest and
length verification, durable close, and atomic placement. Concurrent insertion
of identical content converges on one blob. Blob bytes are never modified in
place.

SQLite in write-ahead logging (WAL) mode provides the mutable coordination
plane:

- action key to result manifest and output blob digests;
- package/source snapshot metadata and resolver indexes;
- schema and codec compatibility metadata;
- leases/pins protecting in-use blobs;
- access/generation data for garbage collection; and
- incomplete staging and recovery records.

Transactions update related index rows atomically. Busy handling uses bounded
retry with observable progress rather than returning routine contention to the
user. Processes that cannot use WAL on a particular filesystem fall back to a
documented compatible journal mode without changing semantic results. Database
loss or corruption discards acceleration state. Only the blob catalog can be
reconstructed by enumerating and verifying CAS objects; action mappings, LRU
history, and access metadata are safely lost and repopulated by clean
computation rather than inferred from anonymous blobs. This cannot change the
correct build result. GC uses reachability from locks, active leases, published
artifacts, and retained action results, and never races an active reader.

Physical layout, hard links, reflinks, and compression are replaceable storage
optimizations. Copy fallback is required across filesystems and on platforms
without link support. No CAS path, row ID, host absolute path, or journal state
may appear in logical source identity or canonical product bytes.

## Linking, backend, products, and debugging

Linking resolves imports and relocations against the frozen project/source
snapshot, validates definitions and signatures over the complete closure, and
persists a canonical `StaticLinkMap`. A session reconstructs its immutable
`LinkedProgram` view from that map and the referenced modules; the view itself
is not serialized. Instantiation evaluates ADR 0007 semantics
with isolated immutable inputs and explicit depth, expansion, and output
budgets. It always produces `LinkedDocumentIR` and a complete occurrence-level
`ExpansionTrace`; the trace is part of the action result and is retained in the
CAS according to the normal action-result retention policy even when no portable
debug companion is published. Instantiation does not serialize XML.

The backend contract consumes `LinkedDocumentIR`, backend options, and relevant
provenance handles. The `squish` backend implements today's final whitespace and
attribute/prefix behavior as the first backend. Its product is named
`<target>.prompt` regardless of whether the content happens to be XML. Backend
identity and options participate in the action key. Additional backends can be
introduced without modifying XML lowering or the linker.

When enabled by the build profile, publication emits:

```text
<target>.prompt
<target>.psdbg
```

`.psdbg` is a deterministic, versioned container, not a directory convention.
It includes the exact source table and source bytes, source/content identities,
spans and line maps, module and lock digests, compiler/dialect/backend identity,
entry and non-secret arguments, relocation maps, `ExpansionTrace`, mappings
from final byte ranges to document occurrences and source frames, metrics, and
diagnostics required for post-build inspection. The debug companion records the
prompt artifact digest, but the prompt cannot record the debug digest: mutual
content references would make hashing circular. A separately committed
`ArtifactManifest` and its SQLite index map a target/build record to both
digests for bidirectional discovery. A portable debug export is self-contained;
ordinary CAS references may supplement but never replace the information
needed to debug the published product.

Publication first reserves every destination globally and rejects cross-target,
source, and manifest collisions before executable target actions start. Each
successful target then commits independently as one recoverable **generation**:
it stages and verifies the prompt, optional portable debug companion, and
`ArtifactManifest`; durably records a per-target journal containing old/new
generation digests; replaces the materialized files; and atomically advances
the target's generation/manifest pointer as the commit record. Recovery uses
that pointer and journal to finish or roll back interrupted multi-file
materialization without guessing from timestamps. The debug object points only
to the prompt digest, while the artifact manifest names both.

A failed compile, backend, or publication action cannot overwrite that target's
previous committed generation and does not prevent independent successful
targets from committing theirs. Reports count only committed generations. A
profile may choose not to publish `.psdbg`, but reusable `ModuleIR` and the
per-execution CAS trace remain complete. The choice is visible in the artifact
manifest; there is no “debug IR” variant with different semantic contents.

## Command and product experience

The direct command surface is:

```text
xmlsquish fmt [selection] [--check | --diff]
xmlsquish build [selection] [--locked] [--frozen] [--offline]
xmlsquish add <requirement> [source and package options] [--dry-run]
xmlsquish remove <dependency> [--dry-run]
xmlsquish inspect <artifact-or-key> [--format human|json]
```

Commands search upward for the workspace/project manifest unless an explicit
manifest path is supplied. Selection, profile, package, target, and dependency
vocabulary is shared rather than independently reimplemented. Help shows the
most relevant examples and defaults at the point of use. Diagnostics carry
stable codes, primary and secondary source labels, causal frames, actionable
suggestions, and documentation links. Unknown names offer edit-distance
suggestions only when unambiguous; automation never depends on prose.

Presentation has two contracts:

- **Human mode:** semantic color, compact phase/status verbs, elapsed time,
  cache-hit visibility, responsive progress bars/spinners, and a final summary.
  `auto`, `always`, and `never` color policies are explicit. Progress occupies
  transient lines only on a capable TTY. Cursor visibility and line state are
  restored on success, error, panic handling, and interruption.
- **NDJSON mode:** one versioned event object per line with stable IDs,
  timestamps/durations where non-semantic, parent action relationships,
  diagnostics, products, and final outcome. It contains no ANSI, spinner
  frames, carriage-return rewriting, or localization-dependent fields.

Redirected human output is stable, non-interactive, and free of ANSI by
default. Workers emit facts to an event channel; presentation may coalesce
high-rate progress but cannot discard diagnostics or change exit meaning. Quiet
mode suppresses progress, not errors. Verbose mode adds causal and cache detail,
not arbitrary internal debug prints. Interrupts yield one clear cancellation
summary rather than leaving a spinner or a forest of duplicated failures.

## Formatter

`fmt` uses its own lossless XML token stream/CST. It never serializes
`ModuleIR`, calls the squish backend, expands macros, or fetches dependencies.
It edits only lexical trivia proven not to alter XML or DSL meaning. Character
data that resembles indentation is semantic until proven otherwise.

For every accepted source `x`, formatter `F`, and semantic frontend `C`, the
required properties are:

\[
F(F(x))=F(x)
\]

and

\[
C(F(x)) \equiv C(x),
\]

where equivalence compares canonical `ModuleIR` semantic tables, decoded
attribute values, expanded names, ordering, and source-to-semantic association
after accounting only for updated byte spans. Tests additionally cover mixed
content, entity spelling, CDATA, comments, processing instructions, namespaces,
line endings, BOM policy, very deep inputs, and malformed documents.

`fmt --check` and `fmt --diff` run the same formatter and validation path as
write mode. Selected files are parsed and candidate results validated before
publication. Replacements use the `FileTransaction` port implemented by the
filesystem artifact adapter;
an interrupted multi-file operation is detected and recovered rather than
silently reported as wholly complete.

## Invariants

1. **One entry, one orchestration model.** Every command enters through the
   kernel and manager; no retained compiler pipeline bypasses action planning.
2. **One owner per invariant.** Domain rules live in their bounded context;
   adapters may not duplicate them.
3. **Frozen observation.** A plan refers to one exact project, lock, dependency,
   source, option, and toolchain snapshot. An invocation never observes half of
   an edit.
4. **Declared inputs.** Every fact capable of changing a cacheable action result
   participates in its key; undeclared ambient state cannot affect semantics.
5. **Portable identity.** `SourceId`, package identity, canonical IR, and output
   bytes do not contain checkout, CAS, or host-specific paths.
6. **Complete reusable IR.** Module artifacts preserve all semantic and debug
   information required for relocation, diagnostics, inspection, and future
   backends.
7. **Representation separation.** CST edits source; `ModuleIR` represents a
   unit; `StaticLinkMap` records binding; `ExpansionTrace` explains an
   instantiation; `LinkedDocumentIR` feeds a backend. None impersonates another.
8. **Frontend/backend independence.** XML is the first frontend and squish the
   first backend. They have no direct dependency, but compatibility is checked
   through explicit IR dialects and capabilities rather than assumed universal.
9. **Determinism.** Canonical inputs produce identical IR, action keys,
   documents, products, and diagnostic facts across supported platforms.
10. **Legal graph cycles remain legal.** Source imports are interned before
    traversal and linked as a graph; scheduler DAG rules do not rewrite DSL
    semantics.
11. **Bounded execution.** Scheduler concurrency and runtime evaluation consume
    explicit resource permits and budgets.
12. **Isolated failure.** A failed action blocks only true dependents; unrelated
    work continues and all facts are reported deterministically.
13. **Single publication authority.** Workers never write final destinations;
    the artifact publisher validates collisions and commits staged bytes.
14. **Recoverable mutation.** Manifest/lock and multi-file formatting
    transactions recover without asking users to repair manager-owned state.
15. **Disposable acceleration.** Deleting CAS indexes, build cache, target
    directory, or SQLite state cannot change the correct clean-build result.
16. **Machine contract first-class.** NDJSON events, IR inspection, lockfiles,
    and debug bundles are versioned interfaces; scripts need not scrape prose.
17. **Semantic formatting.** Formatting is idempotent and frontend-equivalent;
    uncertain whitespace is preserved.
18. **Product contract.** Build products are `*.prompt`; complete optional debug
    companions are `.psdbg`; intermediate XML is not a compiler interface.

## Implementation topology

Implementation proceeds in dependency order, not as reduced product editions.
Each slice establishes part of the final architecture and deletes its temporary
adapter as soon as its consumers are connected:

```text
T0  language invariants + canonical identity specification
 |
T1  protocol/diagnostic IDs + four IR schemas + wire/container specification
 |\
 | +--> T2 source snapshots + provider and storage/publication ports
 |       |
 |       +--> T3 XML CST and relocatable frontend lowering
 |               |
 |               +--> T4 codec, validator, inspect, deterministic fixtures
 |                       |
 |                       +--> T5 link relocation + runtime + trace
 |                               |
 |                               +--> T6 document/backend split + *.prompt/.psdbg
 |
 +--> P1 project/workspace manifest model
         |
         +--> P2 resolver + exact lock + transaction/recovery

T6 + P2 --> P3 action algebra + closed planner + scheduler
T2 + P3 --> P4 concrete CAS/SQLite and artifact transaction adapters

T3 + T6 + P4 --> K1 microkernel/manager composition with all commands wired
              |
              +--> K2 product micro-interactions and presentation refinement
                      |
                      +--> K3 remove old pipeline and enforce crate DAG
                              |
                              +--> K4 cross-platform deterministic CI gates
```

Vertical integration continuously exercises real DSL fixtures through the new
types. Temporary translation from old semantic nodes into new IR is acceptable
only when named, localized, tested, and scheduled for deletion at the next
connected node. There is no dual permanent executor, legacy CLI fallback, or
old artifact policy. A slice is complete when its final owner enforces its
invariants, consumers use its port, negative tests exist, and the superseded
responsibility has been removed.

GitHub Actions runs the full unit and process suite on Ubuntu, Windows, and
macOS. Portable fixtures must produce identical canonical `.xsir`, action keys,
`LinkedDocumentIR`, `*.prompt`, and `.psdbg` digests. CI also exercises cold and
warm builds, lock modes, interrupted manifest/format recovery, corrupt cache
records, concurrent scheduling, TTY-disabled snapshots, formatter properties,
and clean rebuilds after deleting all derived state. Formatting and strict
Clippy may run once on Linux; the documented MSRV remains a separate gate.

## Consequences

The design creates more explicit types and crates than the current compiler,
but removes the more dangerous complexity: implicit phase coupling, duplicated
orchestration, filesystem identities in semantics, backend strings used as IR,
and cache behavior hidden inside workers. The manager can reuse compilation at
source-module granularity and can add frontends/backends without changing the
project model.

The principal engineering risks are canonical codec stability, cache-key
completeness, transactional recovery across platforms, and source-preserving
formatting. These are treated as independently testable contracts rather than
late optimizations. SQLite and a CAS introduce operational state, but that
state is derived and self-verifying; correctness continues to come from
manifest, lock, immutable source bytes, and declared transformations.

The direct command and artifact break is intentional. It avoids carrying an
obsolete compiler-shaped product through every new abstraction. DSL source
meaning is preserved even though command grammar, output suffixes, internal
crates, and intermediate formats are replaced.

## Rejected alternatives

### Extend the existing CLI with four code paths

Rejected. It would preserve the compiler as the real architecture and turn the
manager into conditional glue. Shared state, transactions, scheduling, and
machine output would immediately diverge.

### Retain ADR 0008 and grow it incrementally

Rejected. ADR 0008 deliberately excludes the resolver, lockfile, cache, broad
workspace, binary reusable IR, and immediate bounded scheduling. Those are now
core system properties, not speculative later features. Its compatibility
route and XML product assumptions also contradict this decision.

### Keep all domains as private modules in one binary crate

Rejected as ADR 0006's organizational boundary. A single delivered executable
remains desirable, but module privacy alone does not enforce dependency
direction or permit focused codec, store, resolver, and scheduler testing.
Private workspace crates provide compile-time boundaries without promising a
public Rust API.

### Make the microkernel a runtime plugin framework

Rejected. Native dynamic loading introduces ABI, distribution, lifecycle, and
failure modes unrelated to the current need. Statically composed ports provide
the architectural benefit without an unstable plugin ABI.

### Serialize Rust structs with a general-purpose serializer

Rejected. Rust layout and derived serialization make canonical bytes, bounded
decoding, independent schema evolution, optional features, and non-Rust tooling
accidental. The IR container is a specified protocol.

### Use XML, JSON, or the final prompt as reusable IR

Rejected. Printable trees are bulky, ambiguous about typed identity and
relocation, and tend to lose source/debug structure. Final products occur after
entry instantiation and cannot reuse module compilation. Human-readable views
belong to `inspect`, not to the authoritative codec.

### Put every representation into one universal IR

Rejected. Lossless editing, relocatable semantics, occurrence-level execution
history, and backend input require incompatible identities and lifetimes. A
universal node would accumulate optional fields and phase-invalid states that
the type system could otherwise rule out.

### Use SQLite BLOBs as the only store

Rejected. Large immutable content and transactional indexes have different
access, recovery, and garbage-collection patterns. CAS files make blob identity
independently verifiable; SQLite remains the coordination/index plane.

### Cache from timestamps, paths, or incomplete fingerprints

Rejected. Such caches are invalidation heuristics, not deterministic action
memoization. Cache identity derives from complete declared content and semantic
configuration.

### Make the import graph the scheduler graph

Rejected. Import cycles are legal and source modules do not necessarily produce
independently ordered target artifacts. Scan/compile/link actions model the
real producer relationships without inventing cycle errors.

### Use semantic IR or the squish backend for formatting

Rejected. Neither retains the lexical trivia needed to preserve XML and DSL
character data. Formatting owns a lossless CST and proves semantic equivalence.

### Print directly from workers for responsive UX

Rejected. It makes output timing-dependent, breaks NDJSON, and leaves terminal
cleanup distributed across failure paths. Structured events permit responsive
TTY rendering and deterministic redirected output simultaneously.

### Preserve loose-file behavior and sibling XML artifacts

Rejected. The product is a project manager with explicit commands and targets.
Keeping an invisible legacy route would duplicate planning and keep XML output
assumptions in the core. Direct one-file use, if desired, must be modeled as an
explicit ephemeral project through the same manager.

## Acceptance evidence

Implementation of this accepted decision is complete only when evidence demonstrates:

- the crate graph obeys the declared dependency rules;
- every ADR 0007 primitive lowers to, survives codec round-trip in, and executes
  from `ModuleIR` with equivalent semantics and diagnostics;
- canonical IR and debug containers reject truncation, checksum failure,
  unknown required features, invalid references, and resource-exhausting
  lengths without partial acceptance;
- every semantic operation in every `ModuleIR` has a nonempty unit-static
  `OriginId`; every recorded span is within the exact source envelope whose
  digest it names; and origin edges are acyclic and type-valid;
- final prompt byte-range mappings are sorted, non-overlapping within each
  declared mapping layer, and cover every byte. Each leaf resolves to a valid
  source/expansion chain or to `Synthetic { transform, parent_origins }`; an
  `Unknown` leaf is accepted only where the schema explicitly permits and the
  producer proves that no stronger origin was available;
- cold and warm builds produce byte-identical `*.prompt` and `.psdbg` results;
- changing each semantic input changes the responsible action key, while
  relocation of an unchanged checkout does not;
- manifest/lock edits and multi-file format operations recover correctly from
  interruption at every durable commit point;
- dependency and source cycles follow their separately documented policies;
- bounded concurrent schedules produce identical artifacts, diagnostic facts,
  summary counts, and exit statuses across repeated randomized schedules;
- formatter idempotence and frontend equivalence hold over adversarial and
  generated XML corpora;
- human TTY behavior, redirected human logs, NDJSON schemas, cancellation, and
  terminal restoration have process-level tests; and
- the same portable fixtures pass with identical logical digests in GitHub
  Actions on Linux, Windows, and macOS.

## References

- Mokhov, Mitchell, and Peyton Jones, [“Build Systems à la Carte”](https://doi.org/10.1145/3236774), *Proceedings of the ACM on Programming Languages* 2 (ICFP), 2018. The separation of scheduler and rebuilder motivates an explicit action algebra rather than cache logic inside executors.
- LLVM, [LLVM Language Reference Manual](https://llvm.org/docs/LangRef.html) and [Link Time Optimization](https://llvm.org/docs/LinkTimeOptimization.html). LLVM demonstrates durable frontend/IR/backend boundaries and relocatable cross-module optimization, without implying that LLVM IR itself fits this DSL.
- Cargo, [Workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html), [Cargo.toml versus Cargo.lock](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html), and [External tools](https://doc.rust-lang.org/cargo/reference/external-tools.html). These support unified workspace state, intent/exact-state separation, and versioned machine interfaces.
- Bazel, [Rules and build phases](https://bazel.build/extending/rules) and [Remote cache](https://bazel.build/remote/caching). These are production precedents for analysis before execution and content-addressed action results; they do not require adopting a general build language or remote execution.
- Nix, Dolstra, de Jonge, and Visser, [“Nix: A Safe and Policy-Free System for Software Deployment”](https://doi.org/10.1145/945445.945450), *LISA*, 2004. It grounds derivation identity and immutable content reuse while leaving prompt-specific semantics to this design.
- SQLite, [Write-Ahead Logging](https://sqlite.org/wal.html) and [Atomic Commit](https://sqlite.org/atomiccommit.html). These define the operational assumptions and limitations that the store adapter must test rather than hide.
- rustfmt, [project repository and idempotence tests](https://github.com/rust-lang/rustfmt). Formatter idempotence is treated as a tested property, not an aesthetic expectation.
- pnpm, [Workspace task orchestration](https://pnpm.io/workspace-task-orchestration) and [content-addressable storage layout](https://pnpm.io/symlinked-node-modules-structure). These provide production evidence for ready-queue scheduling and content reuse while their Node-specific linking layout is deliberately not copied.
- uv, [Locking and syncing](https://docs.astral.sh/uv/concepts/projects/sync/) and [Caching](https://docs.astral.sh/uv/concepts/cache/). These motivate explicit locked/frozen modes, transactional project state, append-oriented caching, and portable copy fallback.

These references are evidence and design analogies, not substitutes for
repository-specific tests. Acceptance depends on the invariants and evidence
above, especially where XML whitespace semantics, legal import cycles, and
complete provenance differ from the cited systems.
