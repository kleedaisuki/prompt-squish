# Compiler-manager, reusable IR, and incremental build prior art

Status: research synthesis; inputs for architecture decisions, not an accepted design

Date: 2026-09-15

## 1. Decision question and scope

`xmlsquish` is being repositioned from a single-purpose XML compiler into one manager whose
domains include project/package management, source formatting, compilation, linking, caching,
artifact publication, and terminal presentation. The source language keeps its current
primitives, but XML becomes a frontend syntax rather than the definition of the output format.
The immediate backend still produces a squished document, now named `*.prompt`; later backends
must be able to reuse the same intermediate representation (IR).

This review asks:

1. What should be learned—and not copied—from LLVM bitcode and LLVM debug metadata?
2. How should `rustc` queries, Salsa's red-green algorithm, and Bazel's action/CAS model divide
   responsibility between in-memory incremental computation and persistent build caching?
3. What parts of Cargo's implementation make a coherent manager rather than a bag of commands?
4. Which binary serialization family is an appropriate durable encoding for a reusable IR?
5. What terminal and machine interfaces are part of a product-quality manager contract?

The requested direction deliberately rejects a compatibility wrapper around the old CLI and a
small-MVP-first implementation plan. The recommendations below are therefore driven top down by
semantic invariants and cost models. They do **not** recommend implementing every general build
system feature; unnecessary generality is different from architecturally incomplete foundations.

This is not a security audit. Input validation is discussed only where it is an unavoidable part
of safely reading a binary cache or durable artifact.

### Evidence labels

| Label | Meaning |
| --- | --- |
| **Primary implementation contract** | Current official documentation or upstream source for a production system. Strong evidence of actual behavior, not proof that it is optimal here. |
| **Peer-reviewed model** | A published paper with an explicit model and/or evaluation. Its applicability still depends on matching assumptions. |
| **Project inference** | A conclusion for `xmlsquish` derived from the evidence and stated requirements. It remains a design hypothesis until validated in this repository. |

## 2. Executive synthesis

The strongest combined design is neither “put the current AST in a database” nor “copy LLVM
bitcode.” It is a five-part architecture:

```text
XML source + project snapshot
        │
        ▼
 frontend queries ───────► immutable module IR blobs (`*.xsir`, internal/public tooling artifact)
        │                                      │
        │                         symbols + imports + complete provenance
        │                                      │
        └──────────────────────────────► linker queries
                                               │
                                               ▼
                                      linked executable IR
                                               │
                              ┌────────────────┴────────────────┐
                              ▼                                 ▼
                       squish backend                    future backends
                              │
                              ├── `name.prompt`       (the usable document)
                              └── source-map metadata (machine-readable provenance)

 Manager microkernel: manifest/project snapshot, dependency resolver, query runtime,
 action scheduler, CAS + index, artifact publisher, event stream, terminal/JSON renderers
```

The main conclusions are:

1. **The semantic IR and its wire encoding are separate contracts.** Domain types must not be
   shaped around a serialization library's generated types. A versioned envelope encodes a
   complete module; explicit upgrade code converts older envelopes to the current in-memory IR.
2. **Compile modules, link entries.** An XML file/package becomes a reusable module with symbols,
   unresolved imports, operations, source text, and provenance. An entry selects and links modules;
   the backend executes linked IR. Do not flatten modules into an XML string between phases.
3. **Preserve provenance as first-class data.** A single `(file, line, column)` cannot explain
   imports and macro expansion. Use interned byte spans plus an origin DAG, and generate an output
   source map for `*.prompt`.
4. **Use query incrementality and action caching together, not interchangeably.** A red-green
   query engine avoids recomputation inside a live project database. Persistent action results
   belong in immutable content-addressed blobs. A small transactional database indexes logical
   keys, dependencies, last-use data, and blob digests; it is not the sole representation of IR.
5. **Hash semantic inputs, not arbitrary serialized messages.** Protobuf explicitly states that
   its bytes are not canonical. Cache/action keys require an `xmlsquish`-owned canonical
   fingerprint specification.
6. **Prefer a tagged schema such as Protobuf for the durable IR envelope.** It has the strongest
   ecosystem and explicit field-evolution rules among the compared practical choices. FlatBuffers
   or Cap'n Proto should replace it only after a benchmark demonstrates that zero-copy access is
   material. `rkyv` and Postcard are suitable only for disposable, version-scoped local caches.
7. **The manager owns policy; domain engines own mechanism.** The executable entry point should
   only bootstrap a context and dispatch typed commands. Compilation, formatting, dependency
   editing, linking, storage, and presentation communicate through typed requests/results and a
   common structured event protocol.
8. **Human and machine UX are equally real APIs.** Human progress is TTY-aware and goes to stderr;
   machine events are versioned NDJSON on stdout. Color, Unicode, hyperlinks, and progress have
   `auto` behavior plus explicit overrides. Diagnostics remain useful without any styling.

## 3. The semantic decomposition comes before the storage format

### 3.1 Four graphs must remain distinct

| Graph | Nodes and edges | Cycle policy | What it is for |
| --- | --- | --- | --- |
| Project/package graph | Package identities and declared dependencies | Resolver policy | Acquisition, selection, lock state |
| Source/import graph | Logical source IDs and DSL import/include edges | Existing DSL semantics may permit cycles | Source ownership and closure discovery |
| Query dependency graph | A particular pure computation key and the queries it read | A query invocation cannot depend on itself; cycles are diagnosed or explicitly recovered | Incremental recomputation |
| Build action graph | Materializable frontend, link, backend, and publication actions | A planned execution DAG; cyclic source components are condensed first | Scheduling and persistent action caching |

**Project inference.** A source strongly connected component (SCC) is a normal compilation unit,
not a scheduler failure. Condense the source graph into SCCs, then plan a DAG of `compile SCC ->
link entry -> run backend -> publish`. This removes a special case instead of scattering cycle
checks across loaders, queries, and the scheduler.

### 3.2 Proposed pure semantic functions

The architecture becomes inspectable if its expensive steps are modeled as deterministic
functions:

```text
parse       : SourceSnapshot -> LosslessSyntax
analyze     : LosslessSyntax × ModuleContext -> SemanticModule
link        : Entry × Set<SemanticModule> × LinkOptions -> LinkedProgram
execute     : LinkedProgram × InvocationInputs -> BackendDocument × ProvenanceMap
publish     : BackendDocument × ArtifactPolicy -> PublishedArtifact
```

`publish` is intentionally outside the pure compiler: filesystem state and atomic replacement are
manager concerns. Formatting consumes `LosslessSyntax`; compilation consumes `SemanticModule`.
They may share parsing/tokenization, but the formatter must not serialize the semantic IR and the
backend must not be used as a formatter.

### 3.3 The cache-soundness invariant

For every cacheable action `a`, define an owned canonical key:

```text
K(a) = H(
    "xmlsquish-action-v1" ||
    action_kind || semantic_schema_version || language_version ||
    frontend_or_backend_version || canonical_parameters ||
    sorted[(logical_input_name, content_digest)] ||
    relevant_platform_semantics
)
```

Let `C(a)` be the complete canonical semantic-input projection for action `a`. The logical
correctness invariant is:

```text
C(a1) = C(a2)  =>  observable_result(a1) = observable_result(a2)
```

`K(a) = H(C(a))` is an engineering approximation based on the collision resistance of `H`; no
finite digest makes the inverse implication mathematically exact. A cache requiring stronger
assurance can store and compare the complete canonical projection on a digest hit. This distinction
also makes adversarial collision resistance and ordinary accidental-corruption checks separate
policies.

The semantic implication is a design obligation, not something a hash algorithm supplies. All semantic
inputs must appear at the layer that first consumes them: source bytes and frontend options in the
module key; module behavior/provenance digests, resolution, and entry in the link key; explicit
entry arguments and execution limits in the instantiate key; backend identity/options in the
backend key. UI preferences, destination paths, and progress settings should not be included unless
they change artifact bytes. Section 6.2 specifies these four keys and explicitly excludes
arguments/backend choices from linking. The current DSL has no implicit environment or clock
binding; adding such host inputs would be a future language decision, not an assumed manager feature.

## 4. LLVM: reusable IR, bitcode, linking, and debug metadata

### 4.1 What LLVM actually establishes

LLVM's original CGO paper describes a common, language-independent SSA representation intended
to retain useful high-level information and support transformations at compile time, link time,
runtime, and idle time. The important precedent is not SSA itself; it is the decision to make a
well-specified IR the durable seam between producers, transformations, linkers, and consumers
([Lattner and Adve, CGO 2004](https://llvm.org/pubs/2004-01-30-CGO-LLVM.html)).

The current LLVM language reference explicitly supports three forms of the same IR: an in-memory
representation, an on-disk bitcode representation intended for fast loading, and human-readable
assembly ([LLVM Language Reference](https://llvm.org/docs/LangRef.html)). This is a useful
three-interface model for `xmlsquish`:

| LLVM form | `xmlsquish` analogue |
| --- | --- |
| In-memory IR | Typed Rust domain model used by queries and passes |
| Bitcode | Durable module/linked IR used by cache, linker, and tools |
| Textual assembly | `xmlsquish inspect --format=json|text`, a diagnostic projection rather than the cache key |

LLVM bitcode is itself two layers: a generic bitstream of blocks and records, and a domain-specific
encoding of LLVM IR. Blocks carry lengths so readers can skip whole regions and lazily read function
bodies; the stream contains self-described abbreviations used for compression. The encoding has
separate blocks for modules, functions, type tables, constants, metadata, attachments, symbols, and
strings ([LLVM Bitcode File Format](https://llvm.org/docs/BitCodeFormat.html)).

MLIR is a second, more directly applicable LLVM-family precedent because it supports multiple
domain dialects rather than one low-level instruction set. Its bytecode has a magic value, container
version, producer string, and independently encoded sections for strings, dialects, attributes/types,
resources, and IR. It promises stable older-bytecode reading and back-deployment of older container
versions, but explicitly warns that those promises assume immutable dialect definitions; a dialect
that evolves must own a dialect version and an `upgradeFromVersion` implementation
([MLIR Bytecode Format](https://mlir.llvm.org/docs/BytecodeFormat/)).

MLIR locations also go beyond one source coordinate: `CallSiteLoc` connects callee and caller as a
directed stack, `FileLineColRange` represents a source range, and `FusedLoc` preserves multiple
contributing locations plus optional context metadata
([MLIR builtin location attributes](https://mlir.llvm.org/docs/Dialects/Builtin/#location-attributes)).
Together, LLVM's `DILocation.inlinedAt` and MLIR's call-site/fused locations are strong evidence for
modeling prompt provenance as a graph rather than picking one “best” span.

**Project inference.** Copy the separation and indexing properties, not LLVM's bit-level encoding.
An `xmlsquish` module envelope should separate header, source/string tables, symbol/import tables,
semantic operations, provenance, and optional indexes. A consumer should be able to inspect a
header or symbol table without executing the module. Existing schema libraries already provide
tagged fields, skipping, and generated readers; maintaining a bespoke VBR bitstream is unjustified.

### 4.2 Debug information is not disposable decoration

LLVM's debug design maps source-language AST objects to IR through metadata. Its stated goals are
that debug information minimally intrudes on the rest of the compiler, transformations have
well-defined obligations toward it, and backend-independent IR tools need not understand the
source language. Optimizations may remove or merge program objects, but should preserve accurate
source-level reading and update metadata under defined rules
([Source Level Debugging with LLVM](https://llvm.org/docs/SourceLevelDebugging.html),
[guide for pass authors](https://llvm.org/docs/HowToUpdateDebugInfo.html)).

Concrete LLVM mechanisms directly relevant here include:

- `DIFile` can carry a source checksum;
- `DILocation` contains line, column, lexical scope, and an `inlinedAt` chain;
- metadata attaches to program operations without becoming their runtime value;
- duplicate metadata can be uniqued, while identity-sensitive nodes can remain distinct.

These are documented in the [LLVM Language Reference metadata section](https://llvm.org/docs/LangRef.html#metadata).

For prompt compilation, `inlinedAt` plus MLIR-style fused locations generalize to an **origin graph**:

```text
SourceFile {
    logical_uri, content_digest, exact_bytes
}

Span {
    source_file_id, start_byte, end_byte
}

Origin {
    primary_span,
    kind: Direct | Import | MacroCall | Conditional | Generated | Link | Merge,
    symbol?,
    parents: repeated OriginEdge { role, origin_id }
}

OutputSegment {
    output_start_byte, output_end_byte, origin_id
}
```

Byte offsets are authoritative and remain correct for Unicode; line/column indexes are derived and
may be stored as an acceleration table. Interning spans and origins avoids repeating complete call
chains. A single final byte range may originate from a definition in one file, a call in another,
and an entry link in a third; collapsing it to one file location loses essential explanation.
`parents` is ordered and multi-valued so a `Merge`/fused origin can retain every contributor. Edge
roles distinguish definition, call-site, import, link, and merge contribution instead of forcing
tools to infer meaning from graph position. The verifier requires this origin graph to be acyclic.

Every IR-transforming pass needs a provenance contract analogous to LLVM's debug-info rules:

| Transformation | Provenance obligation |
| --- | --- |
| Preserve/copy an operation | Preserve its origin ID |
| Inline/expand a macro | Add typed parent edges for the call site and definition origin |
| Merge adjacent equivalent text | Create a merge origin with ordered contributor edges |
| Synthesize delimiters/whitespace | Mark `Generated` and point to the causing construct |
| Delete unreachable content | May delete operations, but must not corrupt origin references retained elsewhere |
| Link a module | Retain module-local source IDs through a deterministic relocation table |

### 4.3 LLVM's compatibility policy is a warning, not only a success story

LLVM currently promises that new versions can read bitcode back to LLVM 3.0, but its textual IR is
not promised backward-compatible, newer-to-older reading is not guaranteed, and debug metadata has
historically been special enough to be dropped during upgrades
([LLVM developer policy](https://llvm.org/docs/DeveloperPolicy.html#ir-backwards-compatibility)).

**Project inference.** `xmlsquish` must state separate compatibility policies for:

1. source-language version;
2. semantic IR schema version;
3. cache namespace/toolchain version;
4. `*.prompt` backend behavior;
5. machine event/inspection format.

“The decoder can parse the bytes” is weaker than “the compiler preserves semantics and provenance.”
Golden compatibility fixtures must therefore compare decoded semantics and source maps, not merely
successful decoding.

## 5. `rustc` and Salsa: demand-driven incremental computation

### 5.1 Query systems make dependencies observed data

In `rustc`, a query is identified by a name and key, yields an immutable result, and invokes a
provider when its memo is absent. Providers can access inputs and other results only through the
query context; this makes dependency recording and memoization possible
([query evaluation model](https://rustc-dev-guide.rust-lang.org/queries/query-evaluation-model-in-detail.html),
[`rustc` query overview](https://rustc-dev-guide.rust-lang.org/query.html)).

Salsa packages the same family of ideas for Rust applications: inputs are mutable between
revisions, tracked functions are pure transformations, return values are memoized, dependencies
are recorded from actual reads, and interned values give compact identity
([Salsa overview](https://salsa-rs.github.io/salsa/overview.html)). Its database revision model
also makes a key concurrency rule explicit: readers operate on a stable revision; mutation cancels
or waits for outstanding parallel readers before advancing the revision
([Salsa database/runtime](https://salsa-rs.github.io/salsa/plumbing/database_and_runtime.html)).

This maps naturally onto a manager invocation:

```text
ProjectDatabase revision R
├── inputs: manifest text, lock state, source bytes, selected targets,
│           toolchain identity, explicit entry arguments, backend options
├── tracked: parse, imports, SCCs, analyze, exports, link, backend plan
├── interned: SourceId, PackageId, SymbolId, strings, spans, origins
└── accumulated events: diagnostics and structured observations
```

Workers never read the live filesystem or undeclared host state behind the database. Loader code
creates or updates typed inputs; query providers see immutable values. This gives one coherent
snapshot to formatting, compilation, linking, and diagnostics and prevents a file from changing
halfway through an action.

### 5.2 Red-green is early cutoff, not merely key memoization

The red-green algorithm saves the prior dependency DAG and result fingerprints. On a later
revision, a node is green when its dependencies and result remain semantically unchanged; if an
input changed, the node may be re-executed, compare equal to its previous result, and be “backdated,”
preventing needless recomputation downstream
([`rustc` incremental compilation](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation.html),
[Salsa algorithm](https://salsa-rs.github.io/salsa/reference/algorithm.html)).

This matters for formatting-only or comment-only changes, but only after separating two notions of
equality. Parsing and provenance production rerun because byte spans and line indexes changed. If
the **behavior digest** is unchanged, behavior-only subqueries for semantic binding/instantiation
and prompt bytes may remain green; the **provenance digest** changes, so complete actions carrying
linked debug data, diagnostics, or `*.prompt.map` must be regenerated or relocated. Reusing an old
source map is never a valid early cutoff. A naive cache keyed on whole source bytes cannot exploit
behavior-level cutoff; a single behavior-only digest would instead be unsound for diagnostic/debug
artifacts.

`rustc` also demonstrates three less obvious requirements:

1. **Stable cross-session identities.** In-memory numeric IDs can shift after edits. `rustc`
   persists stable paths/hashes and remaps them to session-local IDs.
2. **Two dependency graphs.** The previous graph is immutable evidence; the current graph is being
   recorded. A current key is mapped to the previous graph via a stable fingerprint.
3. **Fingerprint/value separation.** It is often profitable to persist a compact fingerprint and
   dependency edges without serializing the full query value. Only selected expensive results are
   stored.

The cost and collision trade-offs are explicit in the
[`rustc` incremental detail guide](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html):
stable hashing is not free, and a 128-bit fingerprint has a tiny but nonzero collision probability.
`rustc` notes that fingerprinting can make an incremental build slower than a non-incremental one
when reuse is poor.

**Project inference.** Persistent IR must use logical identities such as package ID, normalized
logical source URI, and qualified symbol—not arena index, traversal number, absolute checkout path,
or pointer-shaped serializer output. Local table indexes are valid only inside one module blob and
must be deterministically relocated at link time.

### 5.3 Query granularity should be chosen mathematically

For a candidate query `i`, let (all probabilities be conditional on the query being part of the
workload under study):

- `c_i` be cold recomputation cost;
- `v_i` be cached-dependency/fingerprint validation cost on a demand;
- `d_i` be deserialization cost for a persistent result;
- `s_i` be serialization/write cost after a result is recomputed;
- `m_i` be per-revision amortized storage and garbage-collection cost;
- `p_i` be the probability the query is demanded in the next revision;
- `g_i` be the conditional probability a demanded cached result is proven reusable (green).

Under a deliberately simple **new-process/cold-memory** model, the next invocation has no live
in-memory memo. “No persist” therefore recomputes, while “persist” can validate and load an on-disk
result. Expected costs are:

```text
C_no_persist(i) = p_i * c_i
C_persist(i)    = p_i * [v_i + g_i*d_i + (1-g_i)*(c_i+s_i)] + m_i

E[saving_i] = C_no_persist(i) - C_persist(i)
            = p_i * [g_i*(c_i-d_i) - v_i - (1-g_i)*s_i] - m_i
```

Persist the value only when this is materially positive under measured workloads. Always-persist
is not “more incremental”; it can be slower and larger. A real decision model should estimate these
terms over an edit-trace horizon, including cold population, eviction, and repeated reuse rather
than treating this new-process equation as a benchmark substitute. For successive revisions in one
live Salsa-style database, compare two in-memory memo-retention policies instead: both sides pay
dependency validation and may remain green, so persistence is not the differentiating variable.
Query boundaries should begin at meaningful
semantic reuse units—source parse, source/module analysis, SCC compilation, entry link, backend
execution—not at every XML token. Finer granularity is justified only when edit traces show that it
reduces expected work more than dependency tracking and hashing cost.

Salsa's durability optimization likewise avoids traversing dependencies of results that depend
only on rarely changing inputs. Registry packages and the toolchain are high-durability; the active
workspace is low-durability
([Salsa durability](https://salsa-rs.github.io/salsa/reference/durability.html)). This is useful
for the live in-memory database but should not be confused with correctness: durability is a
performance hint, never permission to ignore a changed digest.

### 5.4 Salsa is not the persistent cache architecture

Salsa is a strong candidate for the in-process project database, particularly for future watch/LSP
workloads. It does not by itself define a portable, cross-version artifact store. `rustc` has its
own on-disk encoder, stable hashing, previous/current dependency graphs, and selective result cache.

**Recommendation.** Keep these seams explicit even if Salsa is adopted:

```text
QueryRuntime                 PersistentCache
-------------                ----------------
revision and cancellation    action-key -> result metadata
in-memory memo values        digest -> immutable blob
dynamic dependency reads     durable dependency/fingerprint records
interned session handles     stable logical identities
diagnostic accumulation      transactional index and GC state
```

This allows replacing/tuning the query engine without changing the IR wire contract and allows
cache invalidation without changing compiler semantics.

## 6. Bazel: action graphs, action cache, and content-addressable storage

### 6.1 Target intent is lowered into explicit actions

Bazel distinguishes a target graph from an action graph. The latter represents artifacts,
relationships, and concrete build actions
([Bazel overview](https://docs.bazel.build/versions/main/bazel-overview.html)). Each cacheable
action declares its inputs, output names, command line, and environment. Remote caching separates:

- an **action cache**, mapping an action hash to result metadata; and
- a **content-addressable store (CAS)**, mapping content digests to output blobs.

This split and the cache workflow are part of Bazel's official
[remote caching contract](https://bazel.build/remote/caching). The Remote Execution API makes the
identity even more precise: an `Action` references a command digest and input-root digest and also
includes cache-relevant timeout, salt, and platform semantics; canonicalization is necessary so
logically equivalent actions receive the same digest
([Remote Execution protocol](https://github.com/bazelbuild/remote-apis/blob/main/build/bazel/remote/execution/v2/remote_execution.proto)).

The peer-reviewed *Build Systems à la Carte* framework separates scheduler, rebuilder, dependency
model, and store rather than treating “the build system” as one algorithm. It models Bazel-like
constructive traces and early cutoff while showing that scheduler choice is orthogonal to rebuild
policy ([Mokhov, Mitchell, and Peyton Jones, JFP 2020](https://doi.org/10.1017/S0956796820000088)).

**Project inference.** `xmlsquish` should expose these as separate manager domains:

```text
Planner:    target selection -> immutable ActionGraph
Scheduler:  readiness, bounded resources, cancellation, keep-going policy
Rebuilder:  query green check -> local action cache -> execute
CAS:        digest -> immutable bytes
Index:      action key -> result/provenance/log digests and last-use data
Publisher:  materialize selected blobs as user-facing artifacts
Reporter:   render structured action events deterministically
```

The artifact path is not its identity. The backend result first exists as a CAS blob; publication
atomically materializes it at `target/.../name.prompt`. Rebuilding the same bytes should reuse the
blob even if multiple targets publish them under different names.

### 6.2 Four cache-key layers, with deliberately different inputs

A single “build hash” would either invalidate too much or reuse unsoundly. The compiler pipeline
needs four domain-separated action keys:

```text
module_key = H(
  "xmlsquish/module/v1",
  semantic_ir_version,
  frontend_version,
  language_version,
  module_compile_options,
  source_component_graph_with_exact_source_digests
)

link_key = H(
  "xmlsquish/link/v1",
  semantic_ir_version,
  linker_version,
  selected_entry,
  resolved_package_and_symbol_bindings,
  canonically_ordered_module_behavior_and_provenance_digests,
  link_options
)

instantiate_key = H(
  "xmlsquish/instantiate/v1",
  instantiator_version,
  linked_program_behavior_digest,
  linked_program_provenance_digest,
  arguments,
  semantic_execution_limits
)

backend_key = H(
  "xmlsquish/backend/v1",
  backend_id,
  backend_version,
  instantiated_document_semantic_digest,
  instantiated_document_provenance_digest,
  backend_options
)
```

The exact field sets are part of the cache specification, not an implementation detail.

- `module_key` does not include an entry, runtime arguments, or a backend; reusable module IR has
  not linked or executed them.
- **`link_key` does not include arguments, backend identity, or backend options.**
  A linked program is a backend-neutral binding of an entry to semantic modules. Including later
  inputs would destroy valid reuse and quietly re-couple frontend/linker/backend.
- `instantiate_key` owns explicit entry arguments. If an execution limit can change
  successful semantic output, it is part of this key; a presentation-only reporting limit is not.
  Provenance is also required because instantiation maps emitted segments through the linked origin
  graph. A behavior-only memo may be an internal subquery, but it cannot stand in for the complete
  instantiated result after source spans move.
- `backend_key` owns only rendering/lowering choices after neutral instantiation. Adding a new
  backend cannot invalidate module compilation or linking. When a backend action emits both
  `*.prompt` and `*.prompt.map`, its compound key includes provenance. A compound backend action
  therefore misses after provenance-only edits. If measurements justify independent reuse, define
  two explicit subkeys rather than merely splitting the return value:

```text
backend_document_key = H(backend_id, backend_version,
                         instantiated_document_semantic_digest, backend_document_options)

backend_map_key = H(backend_document_key,
                    instantiated_document_provenance_digest, source_map_version)
```

The first may reuse plain prompt bytes after a behavior-stable source edit; the second must
regenerate the map. Their pair forms the compound `backend_key` result above.

There are also several non-interchangeable digest domains:

| Digest | Hashes | Purpose |
| --- | --- | --- |
| Source content digest | Exact source bytes | Detect immutable input identity |
| Behavior semantic digest | Canonical traversal of executable meaning, excluding source coordinates/trivia | Red-green cutoff for binding, instantiation, and prompt bytes |
| Provenance digest | Canonical source tables, spans, origin edges, and their binding to semantic operations | Diagnostics, linked debug data, and source-map invalidation |
| Portable artifact digest | Exact portable artifact bytes (normative prelude plus payload), before transparent CAS/transport compression | CAS address and corruption check |
| Storage checksum | Optional checksum of physical compressed chunks/transport representation | Store-local integrity only; never action identity |
| Action key | Canonical semantic inputs and tool semantics for one transformation | Action-cache lookup |

An IR encoder upgrade may produce a new **portable artifact digest** for the same module behavior and
provenance digests. Conversely, two wire payloads that decode to equal behavior/provenance must not
cause downstream semantic work solely because field order changed. The action cache maps an action
key to result metadata containing semantic and portable artifact digests; the CAS is addressed by
the uncompressed logical container's portable artifact digest. Transparent at-rest or transport
compression does not change identity. This matches the Remote Execution API's digest semantics and
Bazel's separation of ActionCache and CAS; serialization bytes are not program meaning.

### 6.3 Recommended local storage topology

```text
target/xmlsquish/
├── artifacts/<package>/<target>.prompt        # stable user-facing materialization
├── artifacts/<package>/<target>.prompt.map    # output byte ranges -> origin IDs
├── ir/<package>/<module>.xsir                 # optional inspectable materialization
└── cache/
    ├── index.sqlite3                          # action records, graph, last-use, leases
    └── cas/ab/cdef...                         # immutable digest-addressed blobs
```

SQLite is appropriate for the mutable index because it is in-process, cross-platform, and provides
atomic transactions even across crashes. Its own implementation uses rollback journaling or WAL to
make multi-page updates appear atomic
([SQLite atomic commit](https://www.sqlite.org/atomiccommit.html)). A database is less attractive
as the *only* IR representation: blobs then lose natural content identity, copying/importing one
module is awkward, corruption or schema migration has a larger blast radius, and external tools
must coordinate with a live mutable store.

The CAS can use create-temp/write/fsync/rename discipline and verify the digest on read. The SQLite
index may be discarded and rebuilt from manifests/CAS metadata; published artifacts and portable
IR remain independently inspectable. Cross-process build coordination needs leases/transactions at
the action/index layer, not a global lock over the whole manager.

### 6.4 Persistent caching requires source immutability during an action

Bazel documents a concrete failure mode: modifying an input during a build can upload an invalid
result. It recommends guarding against concurrent changes and explicitly declaring relevant
environment variables ([Bazel remote caching, known issues](https://bazel.build/remote/caching#known-issues)).

`xmlsquish` has an easier route because source reads are local and the language has no general
subprocess actions. Load exact bytes into the immutable project revision before executing queries;
hash those bytes, not a later pathname read. Before publication, optionally re-stat or re-hash
source locators and report that a newer revision exists, but never mix new bytes into the running
revision.

## 7. Cargo: one manager context, typed operations, and external contracts

### 7.1 Cargo's architecture is more instructive than its command names

Cargo's implementation overview says that commands are thin wrappers over major operations in
`ops`; `cargo_compile` is the common entry for compilation commands; package sources implement a
`Source` abstraction and have unique `SourceId` values; manifest/lockfile parsing and global
configuration have separate owners
([Cargo crate architecture overview](https://doc.rust-lang.org/nightly/nightly-rustc/cargo/)).

Its `Workspace` is created early and threaded through later operations. Construction discovers the
root and member packages and validates the whole workspace before returning a usable value
([Cargo `Workspace`](https://doc.rust-lang.org/nightly/nightly-rustc/cargo/workspace/workspace/struct.Workspace.html)).
Compilation receives a separate `BuildContext` containing the workspace, global context, selected
packages, profiles, roots, target data, and unit graph
([Cargo `BuildContext`](https://doc.rust-lang.org/nightly/nightly-rustc/cargo/compiler/build_context/struct.BuildContext.html)).

The public model also deliberately separates project intent and resolved state: manifests describe
packages/targets/dependencies, workspaces share resolution and output policy, and `cargo metadata`
exports a versioned machine-readable graph
([manifest format](https://doc.rust-lang.org/cargo/reference/manifest.html),
[workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html),
[`cargo metadata`](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)).

### 7.2 A microkernel interpretation for `xmlsquish`

“Microkernel” should mean a deliberately small policy-neutral orchestration core, not dynamic
plugins everywhere:

```text
main
└── parse top-level CLI
    └── Manager::run(Command, HostCapabilities)
        ├── construct ManagerContext / ProjectSnapshot
        ├── dispatch typed domain operation
        ├── submit plan to shared query/scheduler/storage services
        └── stream typed events to selected presenter
```

Suggested domain ownership:

| Domain | Owns | Must not own |
| --- | --- | --- |
| `project` | manifest/lock schemas, discovery, normalized IDs, immutable snapshot | terminal output, XML parsing |
| `packages` | dependency sources, resolution, add/remove transactions | compilation semantics |
| `syntax` / `fmt` | lossless XML-front syntax and format policy | backend squishing |
| `ir` | semantic module model, verifier, version upgrades, codec adapter | project discovery, filesystem publication |
| `frontend_xml` | current DSL parsing/lowering, source diagnostics | artifact placement |
| `linker` | symbol/import binding, SCC relocation, entry selection | parsing XML or rendering terminal UI |
| `backend_squish` | execute linked IR and produce document + mapping | dependency resolution |
| `engine` | query database, action plan/scheduler, cancellation | domain-specific display strings |
| `store` | CAS, cache index, GC, atomic publication | compiler semantics |
| `ui` | event schema, human/JSON presenters, color/progress policy | running actions |

Commands such as `fmt`, `build`, `add`, and `remove` are adapters over these domains, not domain
boundaries themselves. `build` and future `check/run/inspect` share the same project, IR, link,
engine, store, and UI services. `add/remove` update a typed candidate manifest/lock state
transactionally, validate it, and only then publish it. This follows Cargo's coherent project
model without importing its accumulated resolver or build-script complexity.

### 7.3 Machine-readable interfaces prevent the manager becoming a closed monolith

Cargo provides both `cargo metadata` for project structure and newline-delimited JSON build
messages distinguished by a `reason` field. Its documentation calls the metadata format stable and
versioned and warns clients to request a format version
([Cargo external tools](https://doc.rust-lang.org/cargo/reference/external-tools.html)).

Bazel's Build Event Protocol (BEP) is the stronger precedent for a concurrent build stream. It uses
typed Protobuf events with a unique event ID, announced child IDs, and a payload; the child relation
forms a DAG reflecting command lifecycle. All non-root events are announced by an earlier event,
and a build is complete only when every announced event has either appeared or been replaced by an
`Aborted` payload. Binary, JSON, and text encodings are supported, and external IDE/dashboard tools
consume the protocol instead of parsing terminal prose
([Bazel BEP](https://bazel.build/remote/bep),
[`build_event_stream.proto`](https://github.com/bazelbuild/bazel/blob/master/src/main/java/com/google/devtools/build/lib/buildeventstream/proto/build_event_stream.proto)).

**Project inference.** NDJSON is sufficient as the transport, but event semantics should borrow
BEP's explicit lifecycle rather than assuming arrival order equals dependency order. Each event
needs an invocation-local stable ID and action/target relationship; started actions must eventually
receive success, failure, cancellation, or skipped completion. The terminal renderer may reorder a
final summary, while the machine stream remains append-only and truthful about live scheduling.

`xmlsquish` should therefore provide two stable projections:

```text
xmlsquish metadata --format-version 1       # project/resolution/target graph
xmlsquish build --message-format json       # NDJSON action/diagnostic/artifact events
```

Each event has `schema_version`, `invocation_id`, `event_id`, `sequence`, `kind`, related
action/target IDs, and a typed payload. Additional fields within a version are allowed and enum-like
values must be treated as open by consumers. Human rendering consumes the same events but is not
frozen byte-for-byte.

## 8. Binary schema families and evolution

### 8.1 Comparative evidence

| Format | Strength relevant to IR | Evolution and determinism caveat | Appropriate role |
| --- | --- | --- | --- |
| Protobuf | Mature multi-language schema tooling; unknown tagged fields can be skipped; adding fields is wire-safe | Field numbers cannot change/reuse; removals must reserve numbers; serialization is explicitly not canonical | **Recommended durable envelope**, with an `xmlsquish` canonical semantic hasher |
| FlatBuffers | Direct access without unpacking; compact, cross-platform; `flatc --conform` can check evolution | Tables require append-only fields unless every field has explicit IDs; fields cannot truly be removed; not normally self-describing | Alternative if measured decode/allocation cost dominates |
| Cap'n Proto | Direct traversal and explicit ordinals; documents changes that preserve both compatibility and canonical encoding | Smaller Rust ecosystem and more opinionated layout/runtime; ordinals and type IDs become permanent protocol commitments | Credible alternative where canonical zero-copy interchange is worth added integration cost |
| Deterministic CBOR | IETF-standard deterministic encoding rules, compact generic data model, implementations in many languages | Determinism is an application profile; CBOR does not define the IR schema, field evolution, or semantic equality | Strong candidate for a canonical action/fingerprint projection or self-describing inspection format, not a complete schema strategy by itself |
| `rkyv` | Excellent Rust-native zero-copy access; portable archived primitives and optional validation | Data is readable only if schema and format-control features stay unchanged and the rkyv version is semver-compatible | Version-scoped, disposable local memo blobs only |
| Postcard | Very small, stable Serde wire format | Not self-describing; schema compatibility is explicitly left to users | Small disposable records, never the independently evolvable public IR by itself |
| SQLite | Transactional mutable metadata, indexes, concurrent reads, crash recovery | A database schema is not automatically a portable IR schema; BLOB codecs still need versions | Cache/action index and GC bookkeeping |

Protocol Buffers documents safe additions and the need to reserve removed field numbers/names
([Proto3 updating message types](https://protobuf.dev/programming-guides/proto3/#updating)), but it
also explicitly rejects using generic serialized bytes as a canonical fingerprint
([serialization is not canonical](https://protobuf.dev/programming-guides/serialization-not-canonical/)).
Even deterministic serialization is not canonical across languages or schema builds, as the
[official C++ API contract](https://protobuf.dev/reference/cpp/api-docs/google.protobuf.io.coded_stream/)
also states explicitly.

FlatBuffers documents forward/backward evolution through append-only table fields (or explicit
field IDs), deprecation rather than removal, and stable union discriminants
([FlatBuffers evolution](https://flatbuffers.dev/evolution/)). It offers `--conform` to check that a
new schema is a valid evolution of an older schema
([`flatc`](https://flatbuffers.dev/flatc/)). Cap'n Proto similarly makes member ordinals record
evolution order and enumerates changes that preserve compatibility and canonical message encoding
([Cap'n Proto schema language](https://capnproto.org/language.html#evolving-your-protocol)).

CBOR is useful in a different dimension. RFC 8949 defines core deterministic encoding requirements:
preferred/shortest encodings, no indefinite-length items, and bytewise ordering of deterministically
encoded map keys. It also warns that an application protocol must make additional choices where its
data model admits semantically equivalent representations
([RFC 8949, Section 4.2](https://www.rfc-editor.org/rfc/rfc8949.html#section-4.2)). Thus
deterministic CBOR can be the standardized byte construction for an action-key projection **after**
`xmlsquish` specifies the projection's semantic fields and normalization. It does not make arbitrary
IR objects canonical and does not replace tagged schema-evolution rules.

The Rust-native alternatives have materially weaker durable evolution contracts. `rkyv` says an
archive remains accessible only while the schema and format-control features do not change and the
library version remains semver-compatible
([`rkyv` compatibility](https://docs.rs/rkyv/latest/rkyv/#compatibility)); its byte validation must
also precede checked access when bytes are not already trusted
([`rkyv` validation](https://rkyv.org/validation.html)). Postcard guarantees a stable 1.x wire
format but explicitly leaves forward/backward schema compatibility outside its scope
([Postcard wire format](https://postcard.jamesmunns.com/wire-format#stability)).

### 8.2 Recommended durable envelope

A tagged message schema should begin with a small fixed prelude so wrong files and unsupported
major versions fail with a precise diagnostic before invoking a full decoder:

```text
FixedPrelude {
    magic: "XSIR",
    container_major: u16,
    container_minor: u16,
    flags: u32,
    payload_len: u64,
    payload_digest: [u8; 32]
}

ModuleEnvelope {
    producer: ProducerIdentity,
    language_version: LanguageVersion,
    semantic_ir_version: IrVersion,
    module_id: StableModuleId,
    feature_bits: repeated u64,
    sources: repeated SourceFile,
    strings: repeated bytes,
    symbols: repeated Symbol,
    imports: repeated Import,
    operations: repeated Operation,
    spans: repeated Span,
    origins: repeated Origin,
    indexes: optional Indexes,
    extensions: repeated ExtensionBlock
}
```

`container_major` governs the outer framing/codec; `semantic_ir_version` governs operation
semantics. Minor container changes must remain skippable. Required semantic changes use a new IR
version with a verifier/upgrader rather than overloading an old field's meaning. Feature bits allow
a reader to distinguish “unknown but safely ignorable” from “required to interpret semantics.”
The prelude's `payload_digest` checks only the encoded payload; the CAS portable artifact digest
checks the entire prelude-plus-payload container. Neither identity changes when the store or
transport transparently compresses those logical bytes.

The decoder pipeline is:

```text
bytes -> prelude/length/digest check -> schema decode -> structural limits
      -> version upgrade -> semantic IR verifier -> current in-memory Module
```

Decode success alone is not validation. The verifier checks table bounds, stable-ID uniqueness,
symbol/import resolution preconditions, span bounds and UTF-8 assumptions, origin DAG acyclicity,
operation invariants, and declared resource limits. A durable binary file must never cause panics
because an index happens to be in range in well-formed test fixtures.

### 8.3 Canonical semantic fingerprints

Do not hash `prost::Message::encode_to_vec()` or an equivalent general serializer. Define a
canonical traversal of the semantic model:

- fixed domain-separation prefix and hash algorithm version;
- integers in a fixed endian/varint rule;
- maps sorted by normalized key bytes;
- sets sorted by element fingerprint;
- strings as UTF-8 bytes with explicit length, without locale normalization;
- source identities as logical URI plus exact content digest, never host path formatting;
- exclude producer timestamps, cache paths, presentation indexes, and unknown optional metadata;
- include every field that can change backend bytes, diagnostics, or link resolution according to
  the specific fingerprint's contract.

There should be distinct fingerprints for `syntax`, `semantic module`, `linked program`, and
`backend result`; one universal digest encourages accidental over- or under-invalidation.
An implementation may feed this canonical model into a small purpose-built hasher or a precisely
profiled deterministic-CBOR encoder. Either choice needs cross-implementation golden vectors; the
normative object is the `xmlsquish` fingerprint specification, not the chosen library's default.

## 9. IR contents and linker contract for the current DSL

### 9.1 The module must be lossless with respect to semantics and explanation

“No information pruning” should be made testable. A compiled module must contain or address:

| Information class | Minimum representation |
| --- | --- |
| Exact sources | Logical URI, content digest, exact bytes, line-start index |
| Source structure | Stable spans for parsed constructs; lossless syntax may remain a separate frontend query artifact |
| Semantic operations | Typed opcodes for every current DSL primitive and raw document emission |
| Names and scopes | String table, expanded-QName symbols, definition/invocation scopes, parameters, slots, captures |
| Module boundary | Stable module ID, imports, exports, required language/features |
| Control/data behavior | `import`, `expand`, `arg`, `insert`, `ifr`, `slot`, and `fill` semantics plus budgets |
| Provenance | Interned span/origin DAG including definition, call/import, and generated causes |
| Link data | Unresolved references and relocation sites, not absolute in-memory IDs |
| Verification data | Feature requirements, resource counts/limits, behavior and provenance fingerprints |

Do not store only already-rendered XML fragments. That would freeze the XML backend into the IR,
discard control structure needed by future backends, and make argument-sensitive execution
uncacheable above the string level. Raw source slices may be referenced for ordinary XML
content where byte preservation matters, while DSL primitives lower to typed operations.

### 9.2 Linking is entry selection plus binding, not compilation by another name

```text
Module IR
  = complete reusable package/source semantics + exports + unresolved imports + provenance

Linked IR(entry)
  = ordered module set + symbol resolutions + relocated tables + entry execution plan

Backend result
  = execute Linked IR with immutable invocation inputs, then apply backend-specific rendering
```

The linked representation should retain module boundaries and complete source/debug tables instead
of destructively flattening them. Link-time dead-content analysis may compute a reachable execution
plan, but it must not mutate away information from the underlying module IR. Thus optimization and
debuggability are not forced into a false choice.

The immediate publication pair should be:

```text
<target>.prompt       # plain backend document, directly usable by people and APIs
<target>.prompt.map   # versioned map to IR/source origins, or equivalent indexed metadata
```

Embedding opaque metadata in the document would make arbitrary non-XML backends harder and pollute
the payload. Omitting it would violate the provenance requirement. A sidecar plus CAS-linked build
record keeps the document clean while diagnostics, `inspect`, and future debugger tooling remain
complete. Artifact publication treats the document and its map as one logical result even when the
filesystem cannot atomically replace two independent names at once; a small manifest/pointer can
select a committed generation.

## 10. Product CLI and micro-interactions

### 10.1 Structured events are the single presentation seam

Every operation should emit domain-neutral structured events:

```text
CommandStarted
ProjectLoaded
ActionQueued | ActionStarted | ActionCacheHit | ActionProgress | ActionFinished
Diagnostic
ArtifactPublished
CommandFinished
```

Workers and compiler passes do not print. The event aggregator assigns deterministic logical order
for final diagnostics and artifacts while permitting live progress to reflect actual scheduling.
This avoids interleaved parallel output and makes terminal, NDJSON, tests, IDEs, and future GUI
frontends consumers of one contract.

Output channel rules:

| Mode | stdout | stderr |
| --- | --- | --- |
| Human build/fmt/add/remove | Requested primary data only, normally empty | Status, progress, diagnostics, summary |
| `--message-format=json` | One complete JSON object per line, no ANSI | Process-level failures that prevent creating the JSON stream; preferably also a terminal JSON error event |
| `metadata` / `inspect --format=json` | One documented JSON document | Diagnostics |
| `--quiet` | Explicit requested artifact/data only | Errors; warnings according to documented policy |

### 10.2 Color, Unicode, hyperlinks, and progress are capabilities

Cargo exposes `auto|always|never` color, TTY-aware progress, hyperlink and Unicode capability
settings, and progress width/terminal integration
([Cargo terminal configuration](https://doc.rust-lang.org/cargo/reference/config.html#term)). Clap's
`ColorChoice` uses the same three-way model and its style system centralizes semantic styles
([clap `ColorChoice`](https://docs.rs/clap/latest/clap/enum.ColorChoice.html),
[clap `Styles`](https://docs.rs/clap/latest/clap/builder/struct.Styles.html)). The informal
[`NO_COLOR` standard](https://no-color.org/) asks colorizing tools to honor a non-empty
`NO_COLOR`, while allowing explicit CLI/config choices to override it.

Recommended precedence:

```text
explicit CLI > project/user config > NO_COLOR/TERM capability > auto detection
```

- `--color auto|always|never`; auto requires a terminal and suitable capability.
- `--unicode auto|always|never`; every icon has an ASCII fallback.
- `--hyperlinks auto|always|never`; file links use terminal hyperlink escapes only when enabled.
- `--progress auto|always|never`; auto shows only for an interactive terminal and work long enough
  to avoid flicker. Redirected and JSON output never receives cursor-control sequences.
- Color reinforces `error/warning/success/cache-hit/dim context`; it never carries the only meaning.
- Use stable action labels and concise phase verbs; show cache state and elapsed totals in the final
  summary. Verbose mode exposes action keys, dependency reasons, and timing rather than merely more
  prose.

A useful human interaction sketch is:

```text
  Resolving  project agent.prompt
  Compiling  shared/layout.xsir
      Cached  shared/style.xsir
    Linking  agent::chat
  Rendering  target/xmlsquish/agent/chat.prompt
   Finished  1 target (2 cached, 2 executed) in 0.18s
```

Do not animate successful sub-millisecond work just to appear modern. Micro-interactions should
reduce uncertainty: what is running, why something rebuilt, where the artifact went, and how to
act on a diagnostic.

### 10.3 Diagnostic quality is part of IR quality

A diagnostic event should carry a stable code, severity, message, primary and secondary spans,
origin chain, notes/help, action/module/target IDs, and an optional rendered form. Human rendering
can show snippets and macro/import backtraces; machine clients consume the structure. Because the
source bytes and origin DAG live with IR, a cache hit can reproduce diagnostics without pretending
the originating source is anonymous.

## 11. Implementation consequences and validation program

### 11.1 Architectural decisions supported strongly enough to adopt

| Decision | Evidence strength | Remaining uncertainty |
| --- | --- | --- |
| Separate in-memory IR from durable codec | Strong across LLVM and every schema system | Exact generated-code ergonomics in this Rust tree |
| Module IR -> link entry -> backend | Strong compiler architecture precedent; matches required product semantics | Best module/SCC granularity requires repository prototypes |
| First-class source/provenance tables | Strong LLVM debug precedent and explicit product requirement | Exact output source-map container |
| Query database plus separate persistent CAS/index | Strong complementary `rustc`/Salsa/Bazel evidence | Whether Salsa overhead is justified at current scale |
| Owned canonical semantic fingerprints | Required by Protobuf's non-canonical contract and cache soundness | Hash algorithm and normalization specification |
| Tagged durable schema, Protobuf default | Strong evolution/tooling evidence; engineering recommendation | Must benchmark against FlatBuffers/Cap'n Proto on representative prompt graphs |
| SQLite index, portable immutable IR blobs | Strong transaction/CAS reasoning | Workload-specific WAL/rollback and GC tuning |
| Typed event stream feeding terminal and JSON | Strong Cargo production precedent | Exact schema and exit-code policy |

### 11.2 Experiments that can falsify or revise recommendations

All experiments should live under the repository's `.temp` or `.cache`, not outside the project.

1. **Codec benchmark.** Generate small, median, and pathological IR graphs from real fixtures.
   Measure encoded size, cold encode/decode, peak allocations, selective symbol/header inspection,
   and implementation complexity for Protobuf and the strongest zero-copy challenger. Replace
   Protobuf only if end-to-end manager latency or memory—not a microbenchmark alone—materially
   improves.
2. **Granularity/edit-trace benchmark.** Replay source edits (whitespace/comment, leaf macro,
   shared dependency, manifest option, argument change). Record query invalidations,
   bytes hashed/decoded, cache hit levels, and total latency. Split queries only where expected
   savings are positive.
3. **Full-vs-incremental oracle.** For every edit trace, compare the linked behavior/provenance fingerprints,
   `*.prompt` bytes, diagnostics, and provenance map from a warm incremental build with a clean
   build. Any difference is a cache correctness bug.
4. **Schema compatibility corpus.** Commit one canonical module fixture for every supported IR
   schema version. Current readers must upgrade it to the expected semantic JSON projection and
   preserve provenance. A new writer must pass compatibility checks against the previous schema.
5. **Determinism matrix.** GitHub Actions on Windows, Linux, and macOS builds the same fixture and
   compares semantic/action digests and prompt bytes. Host path separators, checkout roots, hash-map
   iteration, locale, terminal capability, and clock must not leak unless declared semantic inputs.
6. **Cache recovery tests.** Interrupt blob/index writes, leave orphan CAS entries, corrupt a blob,
   and rebuild the index. The result must be a recoverable miss/recomputation, not a successful
   wrong artifact or opaque project failure.
7. **CLI presentation tests.** Snapshot human output for narrow/wide TTY, `color=always/never`,
   Unicode fallback, redirected streams, quiet/verbose, multiple parallel diagnostics, and NDJSON
   schema validation. Terminal decoration must never contaminate machine stdout.
8. **Provenance property tests.** Every emitted output byte is either mapped to at least one valid
   source origin or explicitly marked generated; all referenced spans fit the exact stored source;
   all origin parent chains terminate.

### 11.3 Anti-patterns ruled out by the evidence

- Serializing the current Rust structs with `rkyv` and calling that the stable IR.
- Treating serialized Protobuf bytes as the semantic/action hash.
- Putting all IR only in one mutable SQLite database.
- Flattening imported modules to XML before link resolution.
- Running the squish backend during formatting.
- Reading files or any undeclared host state directly inside memoized providers.
- Using absolute checkout paths or traversal-order integers as cross-session identity.
- Making every XML node a persistent query before workload evidence supports that cost.
- Letting worker threads print progress or diagnostics directly.
- Styling JSON, redirected output, or diagnostics whose meaning disappears without color.
- Reusing the source/import graph as the scheduler DAG.

## 12. Most valuable remaining searches

The prior art is sufficient for the architectural split, but these narrower questions should be
answered alongside implementation:

1. Inspect the current `prost` unknown-field behavior and generated-code/toolchain integration;
   Protobuf's language-level compatibility does not guarantee that every Rust implementation
   preserves unknown fields during decode-modify-reencode.
2. Benchmark `prost`, `flatbuffers`, and `capnp` using actual `xmlsquish` origin-heavy graphs rather
   than vendor examples.
3. Review source-map standards only after backend output categories are explicit. JavaScript source
   maps assume generated line/column mappings and may not represent many-to-one prompt origins
   without an extension.
4. Investigate persistent incremental engines such as `salsa-2022` successors or Adapton only if
   the custom CAS/query bridge becomes the dominant complexity; do not assume an academic engine's
   workload and durability model matches a compiler manager.
5. Study Cargo's job queue and rust-analyzer's Salsa integration at implementation level when the
   action/query scheduler is specified; neither should be copied before resource classes and
   cancellation semantics are known.

## 13. Bottom line

LLVM demonstrates why a durable, information-rich IR can decouple frontends, linking,
transformations, and backends—but also how expensive long-term compatibility and debug preservation
become. `rustc` and Salsa demonstrate that immutable pure queries, stable identity, and red-green
early cutoff are the correct in-process model. Bazel demonstrates that persistent reuse needs
declared actions, an action cache, and immutable CAS rather than filenames. Cargo demonstrates that
one validated project/workspace context and typed operations make commands cohere. Binary schema
systems demonstrate that zero-copy speed, schema evolution, and canonical hashing are separate
properties; no serializer provides all three automatically.

Therefore the recommended foundation for `xmlsquish` is: a small manager kernel; isolated domain
contexts; a complete typed module IR; explicit entry linking; a squish backend producing plain
`*.prompt` plus provenance metadata; a query runtime backed by CAS and a transactional index; an
owned canonical fingerprint specification; and one structured event stream rendered as either a
polished terminal experience or a stable machine protocol. This foundation is broad because the
product is broad, but each mechanism remains small, typed, and independently testable.
