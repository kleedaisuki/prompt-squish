# Project Manager Product Scope

- Status: Historical scope, superseded by [ADR 0009](../adr/0009-microkernel-manager-and-reusable-ir.md) and [CLI experience](cli-experience.md)
- Audience: product, CLI, compiler, and dependency-management implementers
- Scope: evolve `xmlsquish` from a path-oriented compiler into a project manager
- Compatibility-period commands: `xmlsquish --project fmt|build|add|remove`

## Executive Summary

Adding four subcommands is not sufficient to make `xmlsquish` a project manager. The commands need to operate on one versioned project model and one observable orchestration pipeline:

```text
Project discovery
  -> Manifest intent
  -> Exact local dependency validation
  -> Immutable source snapshot
  -> Build plan
  -> Scheduler
  -> Existing compiler
  -> Artifacts and diagnostics
```

The manager owns project discovery, dependency intent, local dependency validation, target selection, scheduling, artifact placement, and user-facing events. The compiler should continue to own XML language semantics and expansion. This separation keeps the existing compiler useful and prevents `fmt`, `add`, and `remove` from becoming unrelated file-editing scripts.

The smallest complete release should support one root package, explicitly named entry targets, exact local path dependencies, a semantics-preserving formatter, a project build plan, and the existing direct-file compiler mode. It deliberately has no dependency lockfile: an exact editable path has no version choice to resolve, so a generated lockfile would merely duplicate the manifest without making the dependency immutable. Git dependencies, version ranges, registries, workspaces, publishing, incremental rebuilds, and plugins are later concerns.

## Evidence from the Current Product

The current repository establishes several product constraints:

- `src/cli/mod.rs` exposes options followed by file, directory, or glob operands. It has no project model or subcommands.
- `src/cli/paths.rs` recursively discovers XML files, excludes generated `.i.xml` and `.o.xml` files, and rejects sibling-output collisions. This is batch discovery, not target planning.
- `src/cli/pipeline.rs` compiles each discovered entry independently and writes sibling artifacts. A project build needs a plan covering all entries and one invocation-wide source snapshot.
- `src/compiler/model.rs` resolves imports as local file URLs. There is no package identity, dependency source, package export, or stable package import address.
- `src/compiler/runtime.rs` rejects both missing and unknown root arguments. One undifferentiated argument map therefore does not work for heterogeneous multi-entry projects.
- `src/cli/files.rs` implements BOM-aware UTF-8 reads and atomic artifact replacement.
- Existing tests require that a failure in one input does not prevent independent inputs from running. Compilation and budget failures occur before final-output publication, while each artifact replacement is individually atomic; the existing IR/final/cleanup sequence is not a multi-file transaction.
- ADR 0006 treats the compiler as private binary-owned implementation. Internal architecture can evolve aggressively, but established CLI and artifact behavior remain external contracts.

## Product Principles

1. **One project model.** Every project command uses the same discovery, manifest, package identity, dependency, target, and diagnostic definitions.
2. **Explicit source semantics.** Adding a dependency makes it available; it never injects macros or edits XML imports implicitly.
3. **Do not invent resolution state.** In the MVP, an exact local path in the manifest is the entire dependency selection. A lockfile is introduced only when a future dependency source creates a real resolution choice.
4. **Plan before execution.** Detect invalid targets, missing arguments, unavailable dependencies, and output collisions before publishing artifacts.
5. **One build snapshot.** All targets in one invocation observe the same frozen source contents and dependency resolution.
6. **Observable orchestration.** A user or external tool can identify the project, target, stage, source, artifact, and result for every event.
7. **No source damage.** Source XML is never overwritten by `build`; edits by `fmt`, `add`, and `remove` use preflight validation and atomic replacement.
8. **Compatibility is a feature.** Existing direct-file invocations, exit behavior, artifact semantics, and failure isolation are preserved.
9. **Correct scheduling precedes caching.** Incremental fingerprints and caches are separate rebuilding policies, not prerequisites for a project manager.

## Users and Jobs to Be Done

### Prompt author

When maintaining several entries and shared modules, the author wants to build the intended products with one command so that paths, budgets, arguments, and artifact destinations are not duplicated across shell history and scripts.

### Team maintainer

When reviewing or merging prompt changes, the maintainer wants a single formatting and build contract so that all contributors produce comparable source and artifacts.

### CI and release maintainer

When validating or releasing a project, the maintainer wants explicit local dependencies, deterministic target selection, stable exit codes, and versioned machine-readable events so that automation does not depend on terminal prose.

### Reusable macro package author

When sharing a macro package, the author wants to declare stable exported modules so that consumers do not depend on the package's internal directory layout.

### Existing script or integration owner

When upgrading `xmlsquish`, the owner wants an existing `xmlsquish [OPTIONS] PATH...` invocation to keep its meaning, output, and failure behavior.

## Project Model

### Manifest

The proposed project manifest is `xmlsquish.toml`. Its schema version and XML language version are separate concepts: changing project metadata must not silently change language semantics.

An illustrative initial shape is:

```toml
manifest-version = 1

[package]
name = "agent-prompts"
language = "0.3"
source-root = "src"
target-dir = "target/xmlsquish"

[targets.agent]
entry = "src/agent.xml"
output = "agent.o.xml"

[targets.agent.args]
locale = "zh-CN"

[limits]
max-depth = 128
max-expansions = 10000
max-output-bytes = 1048576

[dependencies]
common = { path = "../common" }
```

The exact TOML field names are provisional. The required semantics are not:

- entries are explicitly named targets rather than all XML discovered under a directory;
- project-owned source scope and generated target scope are distinct;
- each target output is relative to `target-dir/<package>/`, defaults to `<target>.o.xml`, and is checked for collisions or escape before compilation;
- entry defaults and global execution budgets are declarative;
- dependency aliases are explicit.

### No dependency lockfile in the MVP

An MVP dependency is an exact local path, for example `common = { path = "../common" }`. It has no version range, branch, registry candidate set, or other choice for a resolver to make. The path remains editable by design. A generated `xmlsquish.lock` would therefore only copy the manifest while creating a false impression of immutability and reproducibility.

The MVP does not generate, read, repair, or expose commands for a dependency lockfile. It also does not expose `--locked`, `--offline`, or `--frozen`: local path dependency validation performs no network access, and none of those flags would add a distinct guarantee.

Git dependencies, registries, or version ranges would introduce genuine resolution state. Before adding any of them, a separate ADR must define package identity, resolver policy, checksums, cache and offline behavior, lockfile ownership, update rules, and migration. A real future lockfile should record an exact resolution such as a Git commit or registry package checksum; it must not contain build-cache fingerprints or target artifacts.

Because local path contents are mutable and outside the project root, the MVP must describe builds as deterministic over the source snapshot actually read, not hermetic or reproducible solely from the manifest.

### Package exports and imports

`add` is incomplete until XML can refer to package content without depending on cache paths. A dependency package should declare named module exports:

```toml
[exports]
main = "src/macros.xml"
format = "src/format.xml"
```

A consumer can then import an explicit project-resolved address:

```xml
<xs:import src="pkg:common/main"/>
```

Required identity rules:

- package name, dependency alias, XML namespace URI, and macro expanded name are different identities;
- `add` makes a dependency resolvable but does not insert an `xs:import`;
- dependency-internal relative imports keep their current definition-site semantics;
- current file import semantics stay unchanged in direct-file mode;
- consumers import declared exports and do not reach through arbitrary package internals.

Cargo-style simultaneous incompatible package versions cannot be copied blindly. `xmlsquish` currently registers macros by expanded QName in a global compilation closure, so two package versions may define the same names. The first package model should allow only one resolved version of a package per build closure. Multi-version isolation is a future language and resolver design problem.

## Command Contracts

All commands use common project discovery, `--manifest-path`, color behavior, human diagnostics, and exit-code policy. Human diagnostics go to stderr. A versioned machine event stream goes to stdout when requested.

### `xmlsquish --project build`

Examples:

```text
xmlsquish --project build
xmlsquish --project build --entry agent
xmlsquish --project build --manifest-path prompts/xmlsquish.toml
xmlsquish --project build --message-format json-v1
```

Contract:

- search from the current directory upward for `xmlsquish.toml`, unless `--manifest-path` is supplied;
- build all configured entries by default; a repeatable `--entry` selects a subset;
- validate the manifest and exact local dependency paths, freeze project sources, and produce a complete build plan before target execution;
- reject duplicate target names and output collisions before writing any artifact;
- preserve independent-target failure isolation: a failed target does not prevent an unrelated valid target from completing;
- preserve per-target publication safety: a failed target never replaces its previous final artifact;
- place project artifacts under the configured target directory and print their paths;
- keep intermediate provenance only when requested, with the same final-output semantics as the current compiler;
- support `--message-format human|json-v1` with a versioned event schema;
- perform no network access in the MVP.

Arguments require target scope. For one selected entry, existing `--arg NAME=VALUE` syntax remains sufficient. A multi-entry invocation requires an explicit target-qualified form such as `--arg ENTRY.NAME=VALUE`, or manifest defaults. The manager must not broadcast an argument to entries that do not declare it.

No environment variable is implicitly converted into an entry argument. Secrets and volatile deployment inputs remain explicit CLI or future argument-file inputs.

### `xmlsquish --project fmt`

Examples:

```text
xmlsquish --project fmt
xmlsquish --project fmt --check
xmlsquish --project fmt src/agent.xml
```

Contract:

- default to project-owned sources and exclude dependency caches, the target directory, and `.i.xml` or `.o.xml` artifacts;
- format explicit project-owned paths when operands are supplied;
- use an XML/DSL-aware formatter, not the current product `squish` transformation;
- preserve every semantically significant text node and scalar body byte-for-byte;
- modify only syntax and whitespace positions proven to be trivia;
- preserve BOM policy and avoid gratuitous namespace or attribute reordering;
- preflight all selected files before editing any project file;
- atomically replace each changed file;
- be idempotent: a second run produces no changes;
- make `--check` read-only and return failure when a file would change;
- pin a formatter `style-version` so a tool upgrade does not create an unrequested repository-wide diff.

The formatter is the highest-risk command because mixed XML content and scalar bodies can contain meaningful whitespace. A generic XML pretty-printer is not an acceptable MVP.

### `xmlsquish --project add`

Examples:

```text
xmlsquish --project add common --path ../common
xmlsquish --project add common --path ../common --dry-run
```

Contract:

- treat the operation as a typed dependency upsert, not string insertion into TOML;
- resolve a relative CLI `--path` from the invocation's current working directory, then store it relative to the owning manifest when representable;
- resolve and validate a candidate before mutating project state;
- add a new declaration or update the existing declaration for the same alias;
- preserve unrelated manifest comments, ordering, and formatting;
- make `--dry-run` show the proposed manifest effect without writing;
- never insert or edit `xs:import` in source XML;
- never run a build as an implicit side effect.

The MVP supports only one typed source, `--path`, whose value is stored as an exact local path. It has no dependency resolver or lockfile. Git, registry, and version-range dependencies must not be exposed until a separate ADR defines the resolver, lockfile, caching, and offline contract.

### `xmlsquish --project remove`

Examples:

```text
xmlsquish --project remove common
xmlsquish --project remove common --dry-run
```

Contract:

- remove one exact local dependency edge by alias;
- preserve all unrelated manifest content;
- require no network access;
- make `--dry-run` read-only;
- never delete dependency source files or caches as an implicit side effect;
- never delete or rewrite `xs:import` in source XML;
- report project sources that statically import the removed alias, when this can be determined reliably;
- return a clear error with close-name suggestions for an unknown alias.

The manifest is the complete dependency record in the MVP. `add` and `remove` each have one atomic manifest edit to commit, so there is no cross-file manifest/lock transaction or stale derived state to repair.

## Scheduling and Failure Model

The manager must not collapse four different graphs into one assumed directed acyclic graph:

| Graph | Meaning | Cycle policy |
| --- | --- | --- |
| Package dependency graph | Direct alias visibility among exact local source packages | Cycles are permitted and terminate by package-identity interning |
| XML import graph | Frozen source closure and definition loading | Existing import cycles remain legal |
| Macro call relation | Runtime expansion | Recursion is governed by current budgets |
| Build target plan | Entries, arguments, dependencies, and outputs | Output conflicts are rejected before execution |

The MVP scheduler may execute targets sequentially. It still needs an explicit build plan, stable event order, and one immutable project snapshot. Internally, this snapshot may be named `Project` or `WorkspaceSnapshot` if that is the clearest architecture, but in the MVP it always degenerates to exactly one root package plus its local path dependencies. This internal name does not imply a user-visible `[workspace]` manifest, workspace member selection, or monorepo support. Parallelism is not required for product completeness and must not be added before diagnostics, filesystem publication, and resource budgets have defined concurrency semantics.

Expected failure behavior:

| Failure | Behavior |
| --- | --- |
| No manifest for an explicit project command | Identify the searched path and offer the exact future `xmlsquish --project init` or manifest action; do not fall into arbitrary XML scanning |
| Invalid manifest | Report file, field, expected value, and schema version; write nothing |
| Missing or invalid local dependency path | Identify the dependency alias, manifest location, and resolved local path; write nothing |
| Dependency candidate cannot be validated during `add` | Leave the manifest unchanged |
| Target output collision | Reject the complete plan before compiling or writing |
| One target fails before final commit | Preserve its old final artifact and continue independent targets; debug/intermediate files are individually atomic, not a multi-file transaction |
| Formatter parse failure | Report all preflight failures and leave all selected project files unchanged |
| Removed dependency is still imported | `remove` reports affected source locations; a later build produces a source diagnostic rather than guessing an edit |

## MVP Scope

The first project-manager release is complete only when it includes:

1. A versioned, single-package `xmlsquish.toml` model.
2. Shared upward project discovery and `--manifest-path` handling.
3. Explicit named entries, target output mapping, root argument defaults, and execution budgets.
4. Exact local path dependencies with explicit package exports and project-resolved import addresses; no dependency lockfile.
5. A `--project build` command that creates and executes a preflighted build plan over one immutable snapshot.
6. A semantics-preserving `--project fmt` command and read-only `--project fmt --check`.
7. Typed, comment-preserving `--project add --path` and `--project remove`, both with `--dry-run`.
8. Human diagnostics and a versioned `json-v1` event format.
9. Preservation of established direct-file compiler behavior.

`xmlsquish --project init` is a necessary onboarding follow-up and should be scheduled immediately after the MVP. It is not part of the MVP scope above. Without it, users must hand-author the manifest before the project workflow becomes discoverable. It need only create a minimal manifest around an existing entry; `new`, templates, and scaffolding are not required.

## Explicit Non-goals

- A public package registry, authentication, publishing, yanking, or package discovery.
- Git dependencies, dependency version ranges, a resolver, or a dependency lockfile until a separate ADR defines them.
- General semantic-version solving or simultaneous incompatible versions of one package.
- User-visible workspaces or monorepo membership. An XML import graph is not a workspace, and an internal `WorkspaceSnapshot` name does not expose workspace behavior.
- Incremental compilation, filesystem watching, a daemon, or remote execution.
- A build-script, arbitrary subprocess, plugin, or lifecycle-hook system.
- Automatic source edits that add or remove `xs:import`.
- Formatting dependency caches or generated artifacts.
- Changing XML macro identity, recursion budgets, or final `.o.xml` lowering semantics. Legacy provenance remains byte-compatible; new project mode deliberately uses portable `xmlsquish://<package>/<path>` source identities instead of checkout paths.
- Replacing the existing compiler with a second browser or scripting implementation.
- Claiming performance or reproducibility improvements without measurements that cover the project-manager path.

## Compatibility and Migration Experience

### Contracts to preserve

The following current behavior is established userspace:

- `xmlsquish [OPTIONS] PATH...` accepts files, directories, and globs;
- generated `.i.xml` and `.o.xml` inputs are excluded from discovery;
- directory/glob discovery skips valid module libraries while an explicit module operand is an error;
- `-I`, `-O`, `--debug`, `--explain`, budgets, `--arg`, and color behavior keep their meanings;
- no operands prints help and exits successfully;
- argument-parse failures and build/discovery failures retain stable distinct exit behavior;
- inputs are never overwritten;
- final output and intermediate provenance retain their current byte and whitespace semantics;
- compilation or budget failure does not publish a new final output; existing per-file atomic replacement and write/cleanup ordering remain unchanged; unrelated inputs continue;
- BOM and non-UTF-8 path behavior remain supported.

### Subcommand ambiguity

There is an unavoidable positional grammar conflict: today `xmlsquish build` can mean a path named `build`, and `xmlsquish project build` can mean two paths. A positional subcommand or namespace parser would reinterpret valid legacy requests. Checking whether a path happens to exist is not acceptable because the same arguments would change meaning with filesystem state. The new `--project` option avoids positional ambiguity because it was not valid in the legacy option grammar.

During the compatibility period, the recommended and normative manager commands are therefore:

```text
xmlsquish --project build
xmlsquish --project fmt
xmlsquish --project add
xmlsquish --project remove
```

The existing `xmlsquish [OPTIONS] PATH...` grammar remains unchanged. Project mode is entered only through the explicit `--project` option. Unlike any positional command word, that option was not accepted by the legacy parser; a literal path beginning with `--project` already requires the standard `--` argument boundary. A bare direct-file invocation must not silently switch behavior merely because a manifest appears in an ancestor directory.

Top-level short names such as `xmlsquish build` are a future goal, not an MVP alias. They may be introduced only in the next major version or after the project explicitly accepts and documents the breaking reservation of `build`, `fmt`, `add`, and `remove`. That migration must provide `xmlsquish -- ./build` for an explicit legacy path and must not be described as fully backward compatible.

### Version migration

- Manifest schema version and XML language version evolve independently.
- The MVP neither creates nor expects a dependency lockfile; a future lockfile requires its own ADR and migration plan.
- Future formatting changes require an explicit style-version change.
- Future language-breaking changes require an explicit project-selected language edition/version and a source migration path.
- Direct-file mode remains available for scripts and one-off compilation even after project mode becomes the documented default for new projects.

## Acceptance Criteria

### Compatibility

- The current direct-file process, pipeline, path, diagnostic, BOM, and artifact tests continue to pass.
- Representative legacy invocations produce byte-identical `.i.xml` and `.o.xml` artifacts.
- A failed legacy input continues to allow an independent input to complete.

### Build correctness

- Two clean checkouts with identical project-relative logical source identities, project sources, local dependency contents, manifest, arguments, and tool version produce byte-identical project artifacts. Project-mode `file.*` and provenance use portable `xmlsquish://<package>/<path>` identities rather than checkout paths; `pkg:<alias>/<export>` is import access syntax only. Legacy direct-file mode retains existing `file:` identities and does not promise relocation invariance.
- All targets in one invocation observe one frozen source snapshot even if a file changes during execution.
- Output collisions and invalid plans result in no new target artifacts.
- A target that fails before final commit preserves its previous final artifact. Debug/intermediate artifacts use individual atomic replacements and are not claimed to commit transactionally with the final output.

### Formatter correctness

- Running `fmt` twice produces no second diff.
- Formatting preserves the compiler-relevant parsed structure and produces identical expanded output for a differential corpus.
- The corpus covers mixed content, scalar bodies, CDATA, comments, processing instructions, namespace aliases, BOMs, and deep XML.
- `fmt --check` never writes.

### Dependency mutation correctness

- `add` followed by `remove` restores an equivalent manifest dependency graph.
- Unrelated comments and ordering survive both operations.
- A failed dependency validation changes no manifest.
- `--dry-run` changes no file and reports the same planned change a real run would apply.

### Observability

- Human diagnostics identify project, target, stage, source, and artifact when applicable.
- `json-v1` is documented, versioned, contains no terminal control sequences, and has deterministic event order for sequential execution.
- Human presentation changes do not break the JSON contract.

## Success Metrics

Initial metrics are release gates and opt-in repository evidence, not default telemetry.

| Dimension | Metric | Initial target |
| --- | --- | --- |
| Compatibility | Existing direct-file behavioral regression pass rate | 100% |
| Determinism | Equal project and local dependency snapshots producing byte-identical artifacts | 100% |
| Format safety | Differential corpus cases with unchanged compiler output | 100% |
| Format stability | Second-run formatting diffs | 0 |
| Mutation safety | Fault-injection cases that corrupt source, manifest, or prior artifact | 0 |
| Dependency UX | `add`/`remove` round-trip fixture success | 100% |
| Build-plan safety | Output-collision cases detected before artifact writes | 100% |
| Automation | Project operations expressible without parsing human prose | 100% through `json-v1` |
| Activation | Commands after `init` required to produce the first artifact | 1 (`xmlsquish --project build`) |
| Performance | Single-entry manager overhead versus measured direct mode | Set a fixed millisecond budget after baseline; do not use an unstable percentage alone |

Adoption may later be evaluated through opt-in feedback, example-project migration, and issue data. The CLI should not add default telemetry merely to measure product success.

## Consequential Open Decisions

Only three product decisions block a coherent implementation:

1. **Future short command names:** the MVP uses the compatibility-safe `xmlsquish --project ...` mode; decide separately whether the next major version will reserve top-level `build`, `fmt`, `add`, and `remove`.
2. **Package address contract:** confirm versioned package exports and the `pkg:`-style resolver boundary before implementing dependency mutation.
3. **Formatter semantic boundary:** formally classify compiler trivia and meaningful text before promising in-place formatting.

Registry design, parallel scheduling, workspaces, caching, and publishing do not block the MVP and should not expand its critical path.

## References

- Cargo, [Why Cargo Exists](https://doc.rust-lang.org/cargo/guide/why-cargo-exists.html).
- Cargo, [Cargo.toml vs Cargo.lock](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html).
- Cargo, [`cargo build`](https://doc.rust-lang.org/cargo/commands/cargo-build.html).
- Cargo, [`cargo add`](https://doc.rust-lang.org/cargo/commands/cargo-add.html).
- Cargo, [`cargo remove`](https://doc.rust-lang.org/cargo/commands/cargo-remove.html).
- Cargo, [Dependency Resolution](https://doc.rust-lang.org/cargo/reference/resolver.html).
- Cargo, [External Tools](https://doc.rust-lang.org/cargo/reference/external-tools.html).
- rustfmt, [repository and user documentation](https://github.com/rust-lang/rustfmt).
- Rust RFC 3338, [Style evolution](https://rust-lang.github.io/rfcs/3338-style-evolution.html).
- Andrey Mokhov, Neil Mitchell, and Simon Peyton Jones, [Build Systems a la Carte](https://doi.org/10.1145/3236774), PACMPL/ICFP 2018.
