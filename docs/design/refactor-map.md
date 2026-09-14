# Refactoring Map: Compiler-to-Manager Architecture

## Status and intent

This document maps the current implementation onto the target `xmlsquish`
architecture. It is an implementation guide for deliberate contract replacement.
The new product contract replaces the loose-file compiler interface,
the sibling `*.i.xml` / `*.o.xml` artifact convention, and the assumption that
the intermediate representation is printable XML.

The design is driven top-down by the information that must survive each
transformation and by the invariants required for deterministic builds,
reusable artifacts, diagnostics, caching, and machine inspection. Work should
be delivered as coherent vertical slices of this final architecture; temporary
adapters may connect slices during the refactor, but they must not become a
second execution path.

## Target system

`xmlsquish` is a microkernel-style executable. Its root owns command routing,
domain lifecycle, scheduling, and presentation while domain implementations
communicate through typed data and narrow ports:

```text
                              +--------------------+
XML source -----------------> | lossless XML CST   | -----> fmt
        |                     +--------------------+
        v
+----------------------+      +-----------------------------+
| semantic XML frontend| ---> | relocatable binary Module IR|
+----------------------+      | SourceId, Span, debug tables |
                              +-----------------------------+
                                             |
dependency closure + entry ------------------+
                                             v
                              +-----------------------------+
                              | static linker               |
                              | relocation + verification   |
                              +-----------------------------+
                                             | LinkedProgram
entry arguments + budgets -------------------+
                                             v
                              +-----------------------------+
                              | instantiator                 |
                              | expansion + provenance      |
                              +-----------------------------+
                                  | Linked Document IR
                                  | Expansion Trace
                                  v
                              +-----------------------------+
                              | squish backend              |
                              +-----------------------------+
                                             |
                                             v
                                        *.prompt
```

The semantic frontend is XML-based, and the initial Module IR deliberately
contains an extensible XML-derived data dialect with QNames and attributes.
Backend output media is not constrained to XML. The existing DSL primitives
and their semantics remain the language foundation. The current evaluator is
principally an entry instantiator; static linking is a separate cache boundary,
and the whitespace squasher is the first output backend.

### Domain boundaries

| Domain | Owns | Does not own |
| --- | --- | --- |
| `kernel` | command dispatch, domain construction, lifecycle, cancellation, exit status | XML semantics, manifest details, terminal escape sequences |
| `project` | manifest model and editing, dependency graph, target selection, project snapshot | macro expansion, output rendering |
| `source` | `SourceId`, source envelopes, immutable content store, import resolution | DSL nodes, artifact publication |
| `frontend::xml` | XML syntax, DSL validation and lowering into relocatable modules | project lookup, dependency loading policy, execution |
| `format` | lossless CST/token tape and conservative source rewrites | semantic-IR serialization, squishing |
| `ir` | versioned schemas, typed IDs, codecs, stable hashing and validation | filesystem discovery, terminal reporting |
| `linker` | source-closure linking, symbol relocation, static verification | runtime arguments, backend options, artifact naming, direct terminal output |
| `instantiate` | entry evaluation with arguments and budgets, linked document construction, expansion trace | static dependency resolution, backend rendering |
| `backend` | lowering `LinkedDocument` into a requested product format | source loading, manifest mutation |
| `cache` | content keys, binary module storage, index/database policy, invalidation | deciding source semantics |
| `scheduler` | immutable plans, readiness, bounded work, keep-going policy, structured results | compiler/linker internals, presentation |
| `artifact` | collision validation, staging, durable atomic publication | computing products |
| `diagnostic` | structured diagnostics, source labels, spans, causal context | styling or writing streams |
| `presentation` | color, progress, spinners, stable non-TTY output, summaries | performing jobs or mutating state |

Dependency direction follows the table: the kernel and adapters depend on
domain ports; semantic domains do not import CLI or presentation code. There
must be one planner/scheduler/executor path for `fmt`, `build`, `add`, and
`remove`, with action-specific job payloads rather than action-specific copies
of orchestration.

## Representation model

### Lossless syntax and semantic frontend

Formatting requires a lossless concrete syntax tree (CST) or token tape that
retains lexical trivia and exact source ranges. `roxmltree` is suitable for
semantic parsing, but its tree is not a source-rewriting representation. The
formatter must therefore never serialize the semantic IR back into source.

The semantic XML frontend produces one relocatable `Module` per source. Imports
remain unresolved in that artifact:

```text
ImportSpec {
    raw_uri: StringId,
    span: SpanId,
}
```

Resolution belongs to the source/project context. This removes `PathBuf` from
syntax lowering and permits package exports, content-addressed sources, and
future non-filesystem source providers without teaching the DSL parser about
storage.

### Relocatable Module IR

The module format is a deterministic, machine-readable binary schema. It must
retain information rather than trim it for one backend. At minimum it contains:

- a magic value and independently versioned schema;
- compiler build identity and DSL language version;
- normalized source identity and source-content hash;
- expanded names, definitions, imports, entry declarations, arguments, fills,
  values, and executable operations;
- `SourceId`, byte spans, line mapping, debug/source tables, and provenance;
- symbol and relocation tables sufficient to link without reparsing XML;
- feature/capability flags and validation metadata;
- a canonical encoding from which a stable content key can be computed.

Use semantic newtypes such as `SourceId`, `SpanId`, `DefId`, `StringId`, and
`ExpandedName`; raw strings and collection indices must not be interchangeable.
Prefer flat or arena-backed tables over recursively owned trees. Pure module IR
contains no `Rc`, `OnceCell`, session cache, filesystem path handle, or rendered
XML string. Runtime memoization lives in a build/link session keyed by typed IR
identities.

The canonical storage contract is an immutable content-addressed store (CAS)
outside the disposable target view. It owns blobs such as
`ir/<content-key>.xsir`; `target/` only materializes or exports selected
artifacts. A rebuildable SQLite index may map action keys and graph metadata to
CAS digests when it improves lookup and garbage collection, but it is never the
only copy of semantic IR. The codec and logical schema remain independent from
both stores. Explicit `--emit-ir` and `inspect` operations expose artifacts for
tooling and debugging; cache internals are not the inspection API.

### Linked Document IR and trace

Static linking resolves imports and symbols over a frozen dependency closure
and verifies signatures, producing a `LinkedProgram` whose cache key excludes
runtime arguments and backend configuration. A separate instantiation step
evaluates an entry with arguments and budgets. It produces:

1. `LinkedDocument`, a backend-neutral ordered document/event representation;
2. `ExpansionTrace`, structured occurrence-level provenance and diagnostic
   frames; and
3. metrics and cache dependencies used by the manager.

The trace is not the module IR. The current intermediate XML is generated after
execution and therefore cannot cache parsed/lowered modules. Backend policy such
as namespace-attribute removal, element-name lowering, XML escaping, and text
serialization must be removed from linker tokens and implemented behind a
backend interface consuming `LinkedDocument`.

## Current-to-target code map

Line ranges below describe the inspected tree on 2026-09-14. Symbol names are
the durable locator when later edits move the lines.

| Current implementation evidence | Destination and required change |
| --- | --- |
| `src/main.rs:2-4` declares private `cli`, `compiler`, and `squish` modules. | Retain one executable and reorganize its private modules around `kernel`, domain packages, adapters, and ports. No public library facade is required merely to obtain boundaries. |
| `src/cli/mod.rs:32-60` defines the flat arguments; `src/cli/mod.rs:107-211` mixes decoding, discovery, option construction, execution, and reporting. | Replace with a `Command` enum for `fmt`, `build`, `add`, and `remove`. Reduce the root CLI to parsing plus microkernel dispatch. |
| `src/cli/mod.rs:214-331` renders summaries and stage-specific prose. | Move to `presentation`, consuming ordered structured events and job results. |
| `src/cli/console.rs:12-61` implements color policy and safe clap styling. | Preserve as presentation infrastructure; extend it with TTY-aware progress/spinners and stable redirected output. |
| `src/cli/diagnostics.rs:5-140`, `:161-188` buffer diagnostic rendering and sanitize displayed text. | Retain these observable qualities, but render a shared structured diagnostic type rather than compiler-formatted messages. |
| `src/cli/pipeline.rs:96-132` is the sequential batch loop; `:143-240` combines loading, compilation, squishing, metrics, writes, cleanup, and log output. | Replace with `ProjectSnapshot -> BuildPlan -> Scheduler -> JobResult -> ArtifactStore`. Executors return data; they never print. |
| `src/cli/pipeline.rs:172-189` builds a read snapshot separately per entry. | Use one immutable invocation `ContentStore`, sealed before execution, so all selected targets observe the same source outcomes. |
| `src/cli/pipeline.rs:219-237` publishes sibling XML IR/output files. | Persist binary module IR through `cache`; publish final products through `ArtifactStore` as `*.prompt`. |
| `src/cli/files.rs:14-36` reads XML and atomically replaces a file. | Split into `SourceStore` and `ArtifactStore` ports. BOM is metadata of an XML source envelope, not an IR property. Retain durable staging/replacement semantics. |
| `src/cli/paths.rs:10-45`, `:129-198`, `:242-277` owns loose discovery, identity normalization, collision checks, and sibling naming. | Move project lookup and selection into `project`, identity rules into `source`, and destinations into `artifact`. Remove glob-as-project, implicit file precedence, library skipping, and sibling artifact policy. |
| `src/compiler/parser.rs:13-60`, `:74-145`, `:164-465`, `:466-503` performs XML parsing, spans/QNames/attributes, DSL lowering, and XML helpers. | Move to `frontend::xml` and make its output a relocatable `ir::Module`. |
| `src/compiler/parser.rs:148-162` resolves an import immediately to `PathBuf`. | Emit unresolved `ImportSpec { raw_uri, span }`; resolve only within a source/project ownership context. |
| `src/compiler/model.rs:194-205` defines `Unit`. | Replace with the relocatable module/parsed-unit boundary. |
| `src/compiler/model.rs:8-37`, `:96-193`, `:206-215` defines names, locations, executable variants, declarations, and the program. | Move semantic data into `ir`, using typed identities and flat tables. Separate module-local data from a linked program. |
| `src/compiler/model.rs:38-60` combines payload, runtime render events, `OnceCell`, and `Rc`; `:62-95` implements special deep-tree destruction. | Split immutable serializable IR from session caches. Flat ownership removes the custom recursive-drop workaround. |
| `src/compiler/model.rs:216-277` normalizes paths, resolves references, and constructs file URIs. | Move to `source` resolver and identity code; source display and stable program identity must remain distinct. |
| `src/compiler/mod.rs:22-43` returns `CompileResult` containing rendered intermediate/final strings. | Return typed `IrArtifact`, `LinkedDocument`, `ExpansionTrace`, metrics, and diagnostics at their actual phase boundaries. |
| `src/compiler/mod.rs:68-89` defines monolithic `CompileOptions`. | Split linker/instantiation limits from manifest build profiles and command presentation settings. |
| `src/compiler/mod.rs:184-238` discovers a source closure; `:239-299` verifies expansions; `:300-337` resolves expansion targets. | Split into source graph construction, static/signature verification, and linker relocation passes. Preserve intern-before-traverse cycle handling. |
| `src/compiler/runtime.rs:89-159`, `:179-568`, `:571-645` defines environments, frames, tasks, evaluator, and expansion. | Move to `linker::instantiate`; distinguish immutable linked definitions from invocation-local environments and buffers. |
| `src/compiler/runtime.rs:8-86` lowers tokens/payloads; `:625-637` serializes XML and reparses it for validation. | Replace with `LinkedDocument` construction and backend validation. Do not validate an internal document by round-tripping through one backend's string. |
| `src/compiler/runtime.rs:513-567` emits post-expansion provenance XML. | Convert to structured `ExpansionTrace`; it supplements, but never substitutes for, reusable module IR. |
| `src/squish.rs:96-335` implements lexical whitespace squishing. | Move under `backend::squish` and adapt it to consume `LinkedDocument`. Keep the behavior as the first backend while avoiding a permanent string-only backend interface. |
| `src/compiler/mod.rs:12-65`, `src/compiler/model.rs:21-35`, and CLI diagnostics divide error construction, embedded location prose, and rendering. | Introduce one structured cross-domain diagnostic model with typed codes, labels/spans, causes, notes, and source identities. Formatting occurs only in presentation. |

## External contract replacement

The new contract is deliberately direct:

```text
xmlsquish fmt [selection] [--check]
xmlsquish build [selection] [profile/options]
xmlsquish add <dependency> [source/options]
xmlsquish remove <dependency>
xmlsquish inspect <artifact-or-key>
```

`src/cli/mod.rs:33-52` currently describes loose XML operands and
`*.i.xml`/`*.o.xml`; `src/cli/paths.rs:200-207` and `:269-276` hard-code the
corresponding input filtering and output names. Replace all of these together.
The product artifact suffix is `*.prompt`. Binary IR lives in the target/cache
layout and is exported only on explicit request. Examples, release smoke tests,
site documentation, and packaging text must change in the same contract slice
so users never receive a half-old, half-new interface.

No filesystem-existence heuristic or hidden fallback selects an obsolete loose
compiler route. If direct single-source compilation remains desirable, express
it as an explicit project/command concept using the same plan and executor,
rather than retaining the existing pipeline.

## Scheduling and state model

The planner creates a closed, immutable plan before scheduling:

```text
BuildPlan {
    snapshot_id,
    jobs: [Job],
    artifact_set,
    stable_order,
}

JobResult {
    job_id,
    status,
    products,
    diagnostics,
    events,
    metrics,
}
```

The plan validates source identities, target/product collisions, dependency
edges, cache keys, and output destinations before workers run. The scheduler
may execute ready independent jobs concurrently with an explicit bound. A
failure prevents only jobs that depend on its unavailable product; unrelated
jobs continue. Workers write neither terminal streams nor final artifacts.
They return buffered structured results, and the artifact store commits staged
products. Presentation sorts by stable plan/job identity, never by completion
time, so colored interactive output and redirected logs communicate the same
facts.

Project and dependency mutation uses typed manifest editing and full candidate
resolution. `add` and `remove` serialize mutation only for the selected project,
compute coherent candidate manifest and lock bytes, recheck the observed
generation/digests, and publish them with a journaled recoverable transaction.
Structured events describe recovery and commit state. Builds consume a coherent
frozen manifest/lock generation and never observe a half-mutated project state.

## Test migration and new proof obligations

Tests move with semantics rather than filenames. Do not copy the entire current
suite into a facade around the old pipeline.

| Target suite | Evidence to migrate or replace | Additional obligations |
| --- | --- | --- |
| XML frontend | `src/compiler/parser.contract.test.rs:42-156`; `src/compiler/parser.rs:505-626` | unresolved-import spans, stable IDs, complete DSL lowering, lossless/semantic representation separation |
| Source graph and linker | `src/compiler/source.test.rs:49-270`; `src/compiler/mod.test.rs:21-184` | frozen closure, package ownership, relocation, cycles, duplicate symbols, cache-loaded modules |
| Instantiation and document IR | `src/compiler/runtime.contract.test.rs:42-260`; `src/compiler/runtime.xml.test.rs:16-126`; `src/compiler/runtime.rs:647-881` | compare structured documents/traces/spans; backend-independent expansion equivalence |
| IR codec and cache | upgrade `src/compiler/reuse.test.rs:19-83` | serialize-deserialize-link equivalence, canonical deterministic bytes, schema rejection/migration, complete cache keys, corruption recovery, relocation after checkout movement |
| Squish backend | `src/squish.test.rs`; process coverage from `src/cli/binary.integration.test.rs:61` | identical semantic output through `LinkedDocument`, `*.prompt` publication, backend diagnostics |
| Manager and transactions | `src/cli/mod.pipeline_tests.test.rs:96-217`; `src/cli/pipeline.semantic.test.rs:25-82` | all four commands, immutable plans, dependency blocking, independent keep-going jobs, atomic publication, interrupted/candidate manifest mutations |
| Presentation | existing console and diagnostics unit/integration tests | color modes, TTY progress lifecycle, spinner cleanup, safe user text, stable non-TTY snapshots, deterministic concurrent results |
| Source identity | retain only relevant cases from `src/cli/paths.test.rs` | lexical identity, symlinks, non-UTF-8 platform paths, source display versus program identity |
| Performance | retain and divide `src/compiler/perf.test.rs:121` onward | parse, encode/decode, cache hit/miss, relocation/link, instantiation/backend, cold and warm end-to-end measurements |

Formatter tests require separate properties: idempotence; preservation of every
semantic text/attribute value; no insertion of character-data tokens; unchanged
frontend Module IR before and after formatting; and corpus tests for mixed
content, CDATA, comments, processing instructions, entities, namespaces, and
deep inputs.

Each architecture slice needs negative tests. Particularly important are
unknown IR schema/features, truncated or corrupted artifacts, stale content
keys, source changes during a frozen invocation, duplicate output paths,
dependency cycles, job failure during concurrent execution, terminal
redirection, and publication failure after staging.

## Three-platform GitHub Actions plan

The current Rust job runs only on Ubuntu (`.github/workflows/ci.yml:18-46`).
Replace it with a matrix over `ubuntu-latest`, `windows-latest`, and
`macos-latest`. Every platform runs:

1. locked dependency resolution and `cargo check --all-targets`;
2. the complete unit and process/integration suite;
3. manager smoke scenarios for `fmt`, `build`, `add`, and `remove`;
4. a cold build followed by a warm cache build, then `*.prompt` byte/semantic
   equivalence checks;
5. IR deterministic-encoding and deserialize-link checks;
6. redirected-output snapshots with color/progress disabled automatically; and
7. artifact-path, replacement, non-UTF-8/Unicode, separator, and symlink/junction
   behavior appropriate to that runner.

Run formatting and strict Clippy once on Linux to avoid redundant lint work;
retain the Rust 1.88 MSRV check as its own Linux job. Add a case-sensitive
filesystem job on macOS because the release workflow already establishes the
necessary APFS technique. Cross-platform success means the same logical
`SourceId`, canonical IR bytes, cache key, linked document, and final prompt for
the portable fixture set; platform-native display paths may differ only in
structured diagnostic fields designated for display.

Upload failure-only diagnostic logs and minimal test artifacts for inspection.
Do not upload a shared writable cache whose contents can mask codec or cache-key
errors; cache behavior is tested from fixtures created within each isolated
job.

## Implementation order

The order follows information dependencies rather than a reduced product scope:

1. **Specify IR and ports.** Define typed identities, Module/LinkedDocument/
   Trace schemas, codec versioning, canonical hash rules, validation, source and
   artifact ports, and architecture dependency checks.
2. **Extract the XML frontend.** Produce relocatable modules and unresolved
   imports; introduce the independent lossless formatter representation.
3. **Split graph construction, verification, relocation, and instantiation.**
   Remove rendering and filesystem knowledge from these phases.
4. **Implement backend boundaries.** Make squish consume linked document IR and
   publish `*.prompt`; provide inspectable structured traces.
5. **Build the manager kernel.** Add manifests, snapshots, content store,
   planner, scheduler, cache, artifact store, and structured event/report path.
6. **Install product commands and micro-interactions.** Route `fmt`, `build`,
   `add`, and `remove`; finish color, progress, non-TTY behavior, diagnostics,
   help, and corrective suggestions.
7. **Remove superseded paths and names.** Delete the loose-file pipeline,
   sibling XML artifacts, intermediate-string contracts, and any temporary
   adapter once its final slice is connected.
8. **Enforce the three-platform gate.** Turn the matrix and deterministic
   cross-platform fixtures into required CI evidence; benchmark cold/warm and
   phase-level behavior before accepting the refactor.

At every step, the checked-in architecture should have one owner for each
invariant and one route for each operation. Adding conditionals around the old
pipeline is not progress toward this design; converting an old responsibility
into a typed domain boundary is.
