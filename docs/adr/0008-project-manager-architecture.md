# ADR 0008: Cargo-like project manager architecture

- Status: Superseded by [ADR 0009](0009-microkernel-manager-and-reusable-ir.md)
- Date: 2026-09-14
- Related decisions: ADR 0004's one-package principle remains; ADR 0006 supersedes its library/binary target layout and keeps semantic modules private and owned by the binary; ADR 0005 and ADR 0007 define the DSL and frozen-source compiler semantics.

## Context and scope

`xmlsquish` currently accepts loose path operands and compiles each discovered entry independently. This is useful compiler behavior, but it does not describe a project: there is no manifest, package identity, workspace membership, dependency alias, declared build target, project-wide formatting operation, or dependency mutation command. Repeated path discovery is therefore standing in for project management.

This ADR proposes a manager layer, inspired by Cargo's separation of manifest loading, selection, planning, execution, and reporting. It covers:

- a manifest-backed single-package project model;
- `build`, `fmt`, `add`, and `remove` project operations;
- path dependencies and package-qualified XML imports;
- deterministic scheduling, artifact placement, and reporting;
- preservation of the existing path-oriented CLI and compiler contracts during migration.

The first implementation is deliberately local. It supports one root package with exact local path dependencies. User-visible workspaces are deferred. It does **not** include a registry, network resolution, semantic-version solving, a dependency lockfile, build-result cache, daemon, plugin system, or generalized task runner. The ephemeral lock used to serialize manifest mutation is not a dependency lockfile.

The compiler and whitespace squasher remain private, binary-owned leaf engines in accordance with ADR 0006. This proposal does not create a public library facade or a second Cargo package. Project knowledge must not leak into the DSL runtime, and the manager must not duplicate compilation semantics.

## Repository observations

The following are observations about the implementation at the time of this proposal, not assumptions about a future design.

| Observation | Repository evidence | Architectural consequence |
| --- | --- | --- |
| The CLI has one flat argument structure. Every positional token is a `PATH`; there are no subcommands. | `src/cli/mod.rs:22-60` | A top-level `build`, `fmt`, `add`, or `remove` token would conflict with an existing, valid relative path of the same name. Compatibility must be decided before introducing short subcommands. |
| Parsing, discovery, option construction, execution, diagnostics, and summary rendering are composed in one `execute` function. | `src/cli/mod.rs:107-210` | Command decoding must be separated from project location, snapshot construction, planning, execution, and presentation. |
| Loose operands already support files, recursive directories, and globs; results are normalized, deduplicated, sorted through ordered sets, and checked for output collisions. | `src/cli/paths.rs:19-45`, `src/cli/paths.rs:54-102`, `src/cli/paths.rs:129-170`, `src/cli/paths.rs:242-264` | Legacy discovery is valuable policy, but it is not a project model. It should become one input adapter to a common plan rather than a second executor. |
| Logical file identity is lexical and absolute; it deliberately does not dereference symbolic links. | `src/cli/paths.rs:172-197`; `src/compiler/model.rs:216-267` | Project source identity must retain logical identity and must not silently switch to filesystem canonicalization. |
| The pipeline is a sequential loop that continues after an individual entry fails and accumulates failures. | `src/cli/pipeline.rs:96-132` | Keep-going behavior is already user-visible. A scheduler may add concurrency later, but failure isolation and deterministic reporting must remain. |
| One entry compile reads dependencies through an injected loader and retains their text for accurate diagnostics. | `src/cli/pipeline.rs:143-189` | The new source-provider abstraction must return snapshot text and identity; it must not reduce diagnostics to anonymous bytes. |
| Compilation, final squishing, measurement, persistence, cleanup, and terminal logging currently occur inside `process_one`. | `src/cli/pipeline.rs:143-240` | Execution and artifact storage need explicit boundaries. Terminal writes must not occur inside parallelizable work. |
| Artifact writes already use a temporary sibling, flush and synchronize it, then persist it. Inputs are not overwritten. | `src/cli/files.rs:12-36`; `src/cli/pipeline.rs:219-237` | The artifact store and manifest editor should reuse this commit discipline rather than invent weaker writes. |
| Existing loose-file outputs are sibling `*.i.xml` and `*.o.xml` files. | `src/cli/paths.rs:267-277` | Project builds need an isolated target directory, while legacy invocations must retain sibling paths. |
| `Compiler::prepare` freezes and links a complete source closure; `PreparedProgram::expand` performs fresh executions without reloading. | `src/compiler/mod.rs:98-119`, `src/compiler/mod.rs:143-176` | The compiler already exposes the correct prepare/execute seam. The manager should supply sources and invoke it, not absorb its program model. |
| Source closure discovery interns a normalized path before traversing its imports, so cycles terminate and each source is loaded at most once. | `src/compiler/mod.rs:184-228` | Generalizing identity to packages must preserve interning-before-traversal and one read per `SourceId` per snapshot. |
| The parser currently resolves `src` immediately to a `PathBuf`, and a parsed unit stores resolved path references. | `src/compiler/parser.rs:147-160`; `src/compiler/model.rs:194-205` | Package-qualified references cannot be bolted onto the file loader. Parsing must retain an unresolved `ImportSpec`; a resolver/provider must bind it in an ownership context. |
| The current file resolver rejects non-`file:` schemes, query strings, and fragments. | `src/compiler/model.rs:235-258` | `pkg:` is a project-level source-reference syntax, not an invitation to add arbitrary URI I/O to the compiler. |
| The semantic AST stores decoded text, attributes, and executable nodes, not a lossless XML concrete syntax representation. | `src/compiler/model.rs:38-147`; `src/compiler/parser.rs:12-60` | `fmt` cannot safely serialize the semantic compiler AST. It requires an independent, lossless formatter representation that retains lexical trivia. |
| The squasher intentionally transforms whitespace after semantic compilation. | `src/cli/pipeline.rs:190-208`; `src/squish.rs:7-35` | `squish` is not a source formatter. Reusing it for `fmt` would conflate product semantics with source preservation. |

## Decision

### 1. Manager pipeline

All commands are routed through one explicit orchestration pipeline:

```text
Command
  -> ProjectLocator
  -> ProjectSnapshot
  -> Selector
  -> BuildPlan
  -> Scheduler
  -> Executor + ArtifactStore
  -> OrderedReport
```

This is a sequence of data transformations, not a chain of objects that each prints or mutates global state.

| Component | Owns | Must not own |
| --- | --- | --- |
| `Command` | Parsed user intent, selection flags, compiler budgets, output mode | Filesystem discovery or terminal output |
| `ProjectLocator` | Upward manifest search, explicit `--manifest-path`, and legacy-path mode | Manifest interpretation or compilation |
| `ProjectSnapshot` loader | Read-once manifest/source metadata, local dependency graph, validated immutable configuration | Running targets or updating manifests |
| `Selector` | Default targets and explicit target filtering | Filesystem writes |
| `BuildPlan` builder | Stable job IDs, source roots, options, dependency/source-provider context, artifact paths | Threads, terminal output, or compiler execution |
| `Scheduler` | Readiness, bounded concurrency policy, keep-going decisions, event collection | XML semantics or artifact naming |
| `Executor` | Invoke a leaf engine for one planned job and return structured events/results | Selection, global ordering, or direct terminal rendering |
| `ArtifactStore` | Collision checking and atomic artifact commit | Compilation or manifest mutation |
| `OrderedReport` | Deterministic ordering, aggregation, rendering, exit status | Re-running work to obtain diagnostics |

The initial scheduler may execute sequentially. The boundary is still required: it prevents today's loop from becoming the permanent architecture and makes later bounded parallelism an internal change. Workers return buffered structured events. Only `OrderedReport` writes to stdout/stderr, sorted by stable job key rather than completion time.

### 2. Project data model

The project manifest is `xmlsquish.toml`. Its first schema represents only data required by the four commands:

```toml
manifest-version = 1
style-version = 1

[package]
name = "agent"
language = "0.3"
source-root = "src"
target-dir = "target/xmlsquish"

[targets.prompt]
entry = "src/prompt.xml"
output = "prompt.o.xml"

[targets.prompt.args]
locale = "zh-CN"

[limits]
max-depth = 1024
max-expansions = 100000
max-output-bytes = 67108864

[dependencies]
common = { path = "../common" }

[exports]
main = "src/lib.xml"
```

The root manifest describes exactly one package. User-visible `[workspace]` membership and multi-package selection are outside the MVP. `manifest-version` versions project syntax independently from the XML `language` version, and unsupported versions are errors. `style-version` pins formatter output so a tool upgrade does not silently rewrite a repository. Named `[targets.<name>]` tables are explicit: the manager does not guess an entry from filenames, macro counts, or document order. `source-root`, target entries, and named exports are normalized against the owning manifest and remain inside the package root. `target-dir` is also manifest-relative but must not overlap `source-root`. A target's `output` is relative to `target-dir/<package>/`; it defaults to `<target>.o.xml` and cannot escape that directory. Package and target names must be unique in the scope in which the CLI accepts them. Unknown keys are diagnosed rather than silently ignored; schema evolution must follow documented compatibility rules.

The in-memory model is immutable after snapshot construction:

```text
ProjectSnapshot {
    manifest_version: ManifestVersion,
    style_version: StyleVersion,
    language_version: LanguageVersion,
    root: ProjectRoot,
    root_manifest: ManifestPath,
    limits: CompileLimits,
    packages: Map<PackageId, Package>,
    names: Map<PackageName, PackageId>,
}

Package {
    id: PackageId,
    manifest: ManifestPath,
    root: PackageRoot,
    source_root: SourceRoot,
    target_dir: TargetDir,
    targets: Map<TargetName, Target>,
    exports: Map<ExportName, SourceId>,
    dependencies: Map<DependencyAlias, PackageId>,
}

Target {
    id: TargetId,
    owner: PackageId,
    entry: SourceId,
    output: ArtifactPath,
    arg_defaults: Map<ArgName, String>,
}
```

For the path-only MVP, the validated package name is the stable `PackageId`; exactly one package with that name may appear in a resolved project. A manifest path locates package content but is not its observable identity. This lets project-mode source URIs remain stable when a checkout moves while detecting two different paths that claim the same package identity. The snapshot records both logical identity and normalized locator. It does not watch for changes; a later invocation reloads a new snapshot.

`[limits]` provides project defaults for compiler budgets. A CLI budget overrides the manifest, which overrides the existing compiler default. `[targets.<name>.args]` contains only non-secret default strings. A CLI argument overrides the target default; duplicate CLI assignments remain errors. Unqualified `--arg NAME=VALUE` is valid only when one target is selected. Multi-target requests use `--arg TARGET.NAME=VALUE`, so an argument is never broadcast into a target that did not declare it.

### 3. `SourceId`, `ImportSpec`, and `SourceProvider`

Project ownership becomes part of source identity:

```text
SourceId {
    owner: Legacy | Package(PackageId),
    path: NormalizedLogicalPath,
}

ImportSpec = RelativePath(text) | PackageExport { alias, export }
```

- A legacy root and all of its relative imports use `owner = Legacy`; this preserves today's logical file behavior.
- A project source uses its owning package. Relative imports remain in that owner.
- `pkg:common/main` resolves `common` through the importing package's **direct** dependency-alias map, then resolves `main` through that dependency package's declared `[exports]`. Dependencies are not implicitly transitive: after entering the exported dependency source, its relative imports and package aliases resolve in the dependency's own package context.
- Consumers cannot traverse arbitrary dependency internals. Cross-package access is expressed through a direct dependency alias and declared export; an export target itself must normalize inside its owning package root.
- Content hashes may later assist invalidation, but never define source identity. Symbolic links are not dereferenced merely to establish identity.

Project mode gives each source a portable canonical identity URI, for example `xmlsquish://agent/src/prompt.xml` or `xmlsquish://common/src/lib.xml`. The existing `file.uri`, `file.dir`, and `file.name` bindings and provenance use that logical URI, never a checkout or cache absolute path. This identity URI is **not** import syntax and cannot be pasted into `xs:import`: `pkg:<dependency-alias>/<export>` is an access request in the importing package, while `xmlsquish://<package-name>/<source-path>` identifies the resolved source. Tests must cover an alias that differs from its package name. Diagnostics may additionally display a physical locator, but it is not program data. Legacy mode retains its existing canonical `file:` URI behavior byte-for-byte. This is what makes relocation-invariant project artifacts possible without changing direct-file userspace.

The compiler-facing abstraction is conceptually:

```text
SourceProvider {
    resolve(base: SourceId, spec: ImportSpec) -> SourceId
    load(id: SourceId) -> SourceText
}
```

`SourceText` includes the immutable UTF-8 text and diagnostic display identity. Project mode uses one invocation-wide `BuildInputSnapshot`/source store for all selected targets. It memoizes the complete read outcome—success or failure—by `SourceId`; an identity is never retried against changing disk state within the project invocation. Planning materializes every selected target's complete closure into this store and seals it before any project action executes. Each target still receives its own `PreparedProgram`; only immutable source outcomes are shared. Legacy direct-file mode instead creates one source store per entry, preserving today's rule that independently processed inputs do not implicitly share a frozen dependency snapshot. Both modes use the same provider, planner, executor, and reporter types; snapshot scope is request policy, not a second implementation. Tests may inject an in-memory provider.

The XML parser must retain `ImportSpec` and its source span. It must not resolve imports to `PathBuf` during syntax parsing. Closure discovery performs `resolve -> intern SourceId -> load -> parse`, preserving the existing cycle-termination rule. The DSL runtime receives already-linked definitions and remains unaware of manifests, aliases, and providers.

`pkg:` is a closed project reference form. It does not enable HTTP, registries, arbitrary URI schemes, or network access. Those require separate identity, reproducibility, and failure-semantics decisions.

### 4. Command routing and compatibility

The compatibility decision for the first release is:

```text
xmlsquish [LEGACY_OPTIONS] PATH...          # existing loose-file behavior
xmlsquish --project build [OPTIONS]
xmlsquish --project fmt [OPTIONS]
xmlsquish --project add ALIAS --path PATH
xmlsquish --project remove ALIAS
```

The `--project` mode option is intentional. Under the current grammar, every non-option token may be a path: both `xmlsquish build` and `xmlsquish project build` are valid legacy path requests. Immediately claiming either positional token as a command namespace would break existing scripts and interactive use. `--project` was not a valid option in the legacy grammar, while a literal path beginning with `--project` already requires the standard `--` argument boundary. No filesystem-existence heuristic decides whether a word is a command or a path.

Both forms normalize into the same internal request and plan types. Legacy mode creates a synthetic snapshot/package and one target per discovered entry, then uses the common scheduler, executor, and ordered reporter. It retains current discovery rules, option meanings, keep-going behavior, exit status, diagnostic ordering, and sibling artifact paths. There must be no separate “old pipeline” whose semantics drift.

Whether a later major release should reserve direct `xmlsquish build|fmt|add|remove` is an open compatibility decision, not part of this ADR's first migration. The option-based form is the stable compatibility path unless that breaking reservation is explicitly accepted.

### 5. `build` semantics

`--project build` locates `xmlsquish.toml` by `--manifest-path` or upward search from the working directory, loads and validates the complete project snapshot, selects the root package's default targets, and constructs a plan. Explicit target selectors change selection but not execution semantics.

For each selected target, execution:

1. takes its declared entry and complete frozen closure from the invocation-wide source store populated during planning;
2. prepares or uses its per-target linked program through the existing compiler seam without rereading disk;
3. expands with that target's command-line arguments and budgets;
4. produces provenance IR and, unless intermediate-only was requested, applies the existing final `squish` leaf engine;
5. validates budgets and all destination collisions before commit;
6. atomically commits successful artifacts.

Project artifacts are isolated below the root package's declared `target-dir`, which defaults to `target/xmlsquish/`. Each target has an explicit relative output name; the planner rejects duplicate, input-overwriting, or escaping destinations before execution. The conventional optimized path is `target/xmlsquish/<package>/<target>.o.xml`; retained/debug IR is the corresponding `.i.xml`. Exact platform encoding of package and target names must be reversible or collision-checked. Legacy artifacts remain siblings of their inputs as `*.o.xml`/`*.i.xml`.

Build is keep-going across independent selected targets. During planning, a source load, XML syntax, or compiler-prepare failure is captured as that target's planned failure/job result; it does not abort preparation or later execution of an unrelated target. Only errors that make the common model ambiguous or unsafe—invalid project/manifest structure, invalid selection, duplicate identity, or artifact-path collision—abort the plan globally before target actions start. Execution or commit failure likewise remains local to its target unless another target truly depends on that action. The final exit status is nonzero if discovery, planning, execution, or artifact commit produced any failure. Statistics count only successfully committed artifacts, as today.

The MVP performs no incremental cache lookup and writes no dependency lockfile. `target/xmlsquish` is an artifact destination, not authoritative state. Deleting it and rebuilding must be correct.

### 6. `fmt` semantics

`--project fmt` formats manager-selected, package-owned XML sources; `--check` performs the same parse and formatting computation but writes nothing and fails when a change would be required. It does not invoke macro expansion or `squish` and does not require build artifacts.

Formatting uses a separate lossless concrete syntax tree (CST) or token stream that retains XML lexical trivia and source ranges. The semantic compiler AST is not reused. The formatter may normalize syntactic trivia proven non-semantic, such as spacing internal to markup, but it must preserve byte-for-byte:

- character data in ordinary mixed content;
- scalar bodies and literal values used by `xs:arg`, `xs:insert`, captures, and related DSL constructs;
- decoded attribute values, CDATA character content, comment content, and processing-instruction data;
- namespace-expanded element and attribute identities and document order.

In particular, indentation-looking character data is not automatically “just whitespace.” The formatter never inserts character-data whitespace where no character-data token existed. Where it cannot prove trivia is non-semantic, it leaves the bytes unchanged. This is a partial normal form: intentionally more conservative than a pretty-printer that corrupts prompts. All selected files are parsed, formatted in memory, and checked against the preservation invariants before the first replacement. Each changed file is then replaced atomically. A commit-time failure can therefore leave earlier files validly formatted and later files unchanged; the operation is not presented as a project-wide transaction.

### 7. `add` and `remove` semantics

`--project add ALIAS --path PATH` performs a typed upsert of one local path dependency in the selected package: it inserts an absent alias, changes an existing alias to the requested path after validation, and is idempotent when the declaration is already equivalent. The CLI path is resolved against the invocation's current working directory, including when `--manifest-path` selects another project; the stored spelling is then rewritten relative to the owning manifest when representable. `--project remove ALIAS` removes exactly that alias. Both support `--dry-run`, which performs parsing, candidate editing, and complete validation and reports the prospective change without writing. The MVP accepts no registry name or version requirement.

Manifest mutation follows one transaction-like sequence:

1. locate the selected package manifest;
2. acquire an ephemeral project mutation lock shared by `add` and `remove`;
3. re-read the manifest under the lock;
4. edit it with `toml_edit` so unrelated ordering, whitespace, and comments survive;
5. load and validate the complete candidate project snapshot, including alias uniqueness, target identity, dependency path existence, and package identity;
6. write, flush, synchronize, and atomically replace the manifest;
7. release the lock and report the committed change.

Validation failure leaves the original manifest untouched. `remove` of an absent alias is an explicit user error with a clear diagnostic; `add` reports whether it inserted, updated, or left an equivalent declaration unchanged. Paths are stored relative to the owning manifest when representable, so a project can move as a unit. Mutation does not create or update a dependency lockfile or transaction journal, fetch remote content, build the project, or opportunistically rewrite XML imports. A persistent resolution lock or journal is deferred until a future remote-resolution design supplies a real reproducibility or multi-file transaction requirement.

## Scheduler and executor boundaries

The planner emits immutable jobs with all paths and options already decided. An executor receives one job and returns a value:

```text
JobResult {
    job_id,
    buffered_events,
    measurements,
    committed_artifacts,
    outcome,
}
```

It never prints. This rule is required even while execution is sequential: otherwise future concurrency would interleave diagnostics and change observable behavior. The scheduler owns only job state (`pending`, `ready`, `running`, `complete`) and keep-going policy. The artifact store owns destination reservations and commits; it is the only production component allowed to publish build artifacts.

Stable plan order is `(root package name, target name, source identity)`. Reports, diagnostics, logs, and summaries use that order regardless of worker completion order. Bounded concurrency, if later enabled, is an execution parameter and cannot change artifacts, diagnostic content, summary counts, or exit status.

Path dependencies provide source namespaces, not build-script side effects. Consequently, an imported package does not need to emit an artifact before a consumer can compile; the provider reads its frozen sources directly. This keeps the actual dependency relationship normal and avoids manufacturing target-level ordering edges where none exist. Four graphs remain deliberately distinct: the package dependency graph controls alias visibility; the source import graph controls source closure and may contain cycles; macro recursion is runtime behavior controlled by execution budgets; and the build-action plan contains only work that must actually execute. None may be reused as a convenient approximation for another.

Exact local package-dependency cycles are permitted in the MVP. Project loading interns a `PackageId` before traversing its dependency declarations, so a cycle terminates just like the existing source-closure discovery. This does not create an action-graph cycle because path dependencies are read-only source providers, not prerequisite build actions. A future version resolver may revisit this policy only through a separate compatibility decision.

## Invariants

1. **One architecture:** legacy and project requests become the same plan/job types and use the same executor and reporter.
2. **Leaf engines:** compiler and `squish` know nothing about CLI routing, manifests, project selection, scheduling, or artifact directories.
3. **Frozen input:** a build job observes one immutable source snapshot. Re-run to observe edits.
4. **Stable identity:** `SourceId` is `(owner, normalized logical path)`; content and physical symlink targets do not define identity.
5. **Read/intern once:** closure discovery interns a `SourceId` before traversing its edges and loads it at most once per frozen program.
6. **Declared crossing:** relative imports stay inside their owner; cross-package imports require `pkg:<alias>/...` and a manifest dependency.
7. **No input overwrite:** build artifacts never replace source or manifest files. `fmt`, `add`, and `remove` replace only their explicit mutation targets after validation.
8. **Publication boundary:** project mode stages every requested artifact before commit and publishes the final `.o.xml` last; a compilation, budget, or pre-commit failure cannot replace the prior final artifact. Each file replacement is atomic, but a debug IR plus final output is not falsely described as one filesystem transaction. Legacy mode retains its established write/cleanup ordering.
9. **Compatibility:** legacy argument meanings, sibling artifact names, failure isolation, and exit behavior remain supported throughout the compatibility window.
10. **Deterministic observation:** scheduling order and completion timing cannot change report order or committed bytes.
11. **Formatter preservation:** formatting never changes scalar values, mixed character data, namespace identity, or node order.
12. **Disposable outputs:** correctness never depends on an undeclared cache or contents already present under `target/xmlsquish`.
13. **Serialized mutation:** `add` and `remove` re-read and validate under a shared project mutation lock before atomic replacement.

## Phased migration

### Phase 0: Characterize the current contract

- Add golden process tests for ambiguous path names such as `build`, `fmt`, `add`, and `remove`.
- Preserve current path/glob discovery, output collisions, BOM handling, keep-going behavior, log order, statistics, and exit codes.
- Record existing sibling artifact behavior as legacy compatibility tests.

### Phase 1: Extract the common execution model

- Introduce command request, synthetic legacy snapshot, selection, plan, job result, artifact store, and ordered report types.
- Move terminal writes out of per-file execution and buffer structured events.
- Route the existing CLI through the new pipeline without adding project commands or changing output.

This phase is successful only if legacy end-to-end tests remain byte-for-byte compatible where output is contractual.

### Phase 2: Generalize source identity

- Replace parser-time `PathBuf` resolution with spanned `ImportSpec` retention.
- Introduce `SourceId` and injectable `SourceProvider`.
- Implement the legacy filesystem provider first and prove existing closure, cycle, diagnostic, and logical-symlink tests still pass.

This avoids combining a compiler identity migration with manifest behavior in one unreviewable change.

### Phase 3: Add read-only project build

- Parse and validate `xmlsquish.toml`, package IDs, explicit targets, exports, and local path dependencies.
- Add `pkg:<alias>/...` resolution.
- Add `xmlsquish --project build`, with project artifacts under `target/xmlsquish`.
- Keep execution sequential initially; validate stable reporting and clean rebuilds before considering concurrency.

### Phase 4: Add conservative formatting

- Introduce the independent lossless XML CST/token stream.
- Implement preservation checks, atomic per-file writes, and `--check`.
- Start with transformations whose semantic safety is mechanically testable; expand style coverage only with adversarial scalar/mixed-content fixtures.

### Phase 5: Add dependency mutation

- Add `toml_edit`, the mutation lock, candidate-snapshot validation, and atomic manifest replacement.
- Implement path-only `--project add` and `--project remove`.
- Exercise concurrent mutation and interrupted-write tests without introducing a dependency lockfile.

### Phase 6: Optimize only from measurements

- Measure project load, repeated source parsing, compilation, and reporting separately.
- Add bounded parallel scheduling only if real projects demonstrate useful independent targets.
- Propose caching, registries, and lockfile semantics in separate ADRs; do not smuggle them into executor internals.

## Consequences and risk controls

The primary benefit is that project behavior becomes explicit data rather than conditionals spread across CLI code. Source ownership removes the special case where dependency aliases must be translated into fake filesystem paths. A common plan preserves existing users while making project mode a normal input to the same executor.

The cost is a larger internal model and a required compiler-source seam change. The riskiest parts are source identity migration, formatter correctness, and manifest mutation. They are deliberately separated into phases with old behavior characterized first. The proposal does not claim faster builds: without measured need, a cache and concurrent scheduler would add invalidation and ordering complexity without demonstrated user value.

## Alternatives considered and rejected

### Add four branches to the current `execute` function

Rejected. The current function already combines parsing, discovery, option assembly, pipeline invocation, rendering, and exit status. More branches would create four orchestration paths and ensure semantic drift.

### Immediately reserve top-level `build`, `fmt`, `add`, and `remove`

Rejected for the compatibility phase. These tokens are valid path operands today. Routing based on whether such a path exists is also rejected because ambient filesystem state must not change command grammar.

### Keep a separate legacy executor

Rejected. Compatibility wrappers are acceptable; duplicate planners/executors are not. A synthetic legacy snapshot makes the old case ordinary while preserving its external artifact policy.

### Resolve `pkg:` imports inside the parser or compiler with global project state

Rejected. Syntax parsing should retain an `ImportSpec`; resolution depends on the owning package snapshot. Global state would make tests, frozen-source semantics, and concurrent jobs fragile.

### Encode package aliases as mounted filesystem directories

Rejected. It confuses logical ownership with physical layout, permits accidental `..` crossing, and makes diagnostics and identity dependent on staging paths.

### Use the compiler AST or `squish` for `fmt`

Rejected. The compiler AST is not lossless, while `squish` intentionally changes whitespace. Neither can satisfy scalar and mixed-content preservation.

### Introduce a registry, resolver lockfile, and cache in the MVP

Rejected. Path dependencies solve the first real project-management need. Registry trust, version solving, reproducible lock semantics, cache keys, invalidation, and garbage collection are distinct design problems requiring evidence and separate decisions.

### Parallelize immediately

Rejected. The repository has stable sequential keep-going behavior but no representative multi-target workload measurements. First isolate scheduling from execution and make events deterministic; then benchmark.

## Open questions

1. Should a future major release reserve direct top-level subcommands, and what deprecation mechanism can prove that path operands named `build`, `fmt`, `add`, or `remove` are no longer materially used?
2. What measured user need would justify a later `[workspace]` ADR, and should a future workspace default to the current package or explicit default members?
3. Which target-name and package-name grammar gives portable, collision-free artifact paths on Windows, macOS, and Linux? The planner must reject collisions before execution.
4. Should a later schema add named argument profiles beyond the MVP's one set of non-secret per-target defaults, and how should CI select them without implicit environment capture?
5. What ignore syntax, if any, should refine `fmt` enumeration below the explicit `source-root` without accidentally including generated XML? The target directory and path dependencies are always excluded.
6. What cross-process locking primitive and stale-lock behavior are sufficiently portable for manifest mutation? The contract requires serialization and recovery, not a particular crate.
7. Does `fmt --check` report the first difference or every selected file, and what stable diff format is appropriate for CI?
8. When bounded concurrency is justified, should the limit apply to targets, prepared programs, or aggregate source bytes? Measurements must precede the choice.
9. Which manifest compatibility policy applies to unknown future keys and schema revisions? The first parser should not accidentally make every internal representation a permanent public contract.

## References and external grounding

- Cargo's [workspace reference](https://doc.rust-lang.org/cargo/reference/workspaces.html) demonstrates production-proven separation between packages and a workspace, upward workspace discovery, explicit member/default-member selection, and one shared target directory. This ADR borrows those management concepts but does not copy Cargo's registry or lockfile machinery into a path-only MVP.
- Cargo's [`cargo add` reference](https://doc.rust-lang.org/cargo/commands/cargo-add.html) defines add as an operation that can add **or modify** a dependency and supports a read-only `--dry-run`. That behavior motivates typed, idempotent upsert semantics rather than raw TOML insertion or a surprising duplicate-alias failure.
- Cargo's [`cargo remove` reference](https://doc.rust-lang.org/cargo/commands/cargo-remove.html) provides the mature precedent for dependency removal as a manifest operation, separate from source-code edits.
- Bazel's [rule model and build phases](https://bazel.build/extending/rules) distinguishes loading and analysis from action execution. The systems are not equivalent, but the proven boundary supports constructing and validating an explicit plan before publishing outputs.
- Mokhov, Mitchell, and Peyton Jones, [“Build Systems à la Carte,”](https://doi.org/10.1145/3236774) *Proceedings of the ACM on Programming Languages* 2 (ICFP), 2018, separates task scheduling from rebuilding policy. It supports keeping the scheduler boundary explicit while declining to invent an unmeasured cache/rebuild policy in this proposal.

These references are design evidence, not proofs that Cargo or Bazel semantics fit XML prompts unchanged. Repository compatibility constraints, especially flat positional-path routing and whitespace-sensitive XML, take precedence where the analogy fails.

## Validation required before acceptance

- Existing binary, diagnostic, path-discovery, source-cycle, logical-identity, output-collision, BOM, atomic-write, and keep-going tests pass through the common manager pipeline.
- Legacy invocations with operands named after proposed commands remain legacy path requests.
- Equivalent legacy and synthetic-plan executions produce the same semantic bytes, ordered diagnostics, statistics, and exit status.
- Project fixtures cover package-name collisions, target collisions, alias resolution, `..` escape rejection, dependency edits, missing manifests, and interrupted commits.
- `pkg:` imports preserve definition-site resolution, one-load-per-`SourceId`, import-cycle termination, complete static validation, and diagnostic source spans; fixtures include a dependency alias different from its package name.
- Project targets share one sealed invocation snapshot, while legacy entries retain independent per-entry snapshots through the same executor.
- Formatter property/golden tests prove byte-unchanged semantically significant character data and scalar bodies, unchanged decoded attribute values and expanded names, and preserved comments, processing instructions, CDATA character content, and node order, including adversarial mixed-content inputs.
- A clean project build succeeds after deleting `target/xmlsquish`; no undeclared cache or lockfile is required.
- Any concurrency experiment demonstrates identical ordered reports and artifacts across repeated schedules before it may become the default.
