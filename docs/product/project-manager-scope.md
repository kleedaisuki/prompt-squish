# Project Manager Product Scope

- **Status:** Production scope; `new` implemented and locally exercised, remote CI pending
- **Date:** 2026-09-15
- **Supersedes:** the former MVP and compatibility-period assumptions in this file
- **Authority:** [ADR 0009](../adr/0009-microkernel-manager-and-reusable-ir.md), [CLI experience](cli-experience.md), and the unchanged [XML DSL](../dsl.md)

## 1. Product definition

`xmlsquish` is one command-first project manager, not a loose-file compiler with optional project behavior:

```text
xmlsquish <new|build|fmt|add|remove|inspect> ...
```

Every operation uses the same project discovery, typed manifest and lock state, immutable source snapshot, manager kernel, event protocol, presentation policy, cancellation path, and exit reduction. The old proposal for a separate compatibility-period project mode, sibling generated XML files, and direct path compilation is superseded by ADR 0009 and is not a compatibility requirement.

The XML DSL remains source compatible. `xs:entry` is an explicit link root; `xs:module`, `xs:import`, `xs:macro`, `xs:expand`, scalar arguments, slots, regex conditions, scope, definition-site paths, and resource semantics retain the contracts in `docs/dsl.md`.

## 2. Users and decisions served

| User | Job to be done | Product evidence |
| --- | --- | --- |
| New user | Reach a buildable, version-controlled prompt package without hand-authoring manager metadata | canonical `new` scaffold, workspace placement, immediate format/build acceptance |
| Prompt author | Build named prompt products without repeating paths, arguments, and budgets in shell scripts | manifest targets, profiles, `.prompt` publication |
| Package author | Share exported macro modules without exposing checkout/cache layout | typed package identity, exports, resolver bindings |
| Workspace maintainer | Apply one format/build contract across selected packages | workspace membership and package selectors |
| CI/release owner | Run deterministic, observable operations without scraping terminal prose | locked/offline/frozen modes, NDJSON, stable exits |
| Debugger/tool author | Inspect compiler and build evidence without parsing private storage | typed `inspect` subjects and versioned JSON query documents |

## 3. In-scope production model

### 3.1 Creation

`xmlsquish new PATH [--name NAME] [--vcs git|none]` is the production entry into the project
lifecycle. It atomically creates a canonical package manifest, `src/prompt.xml`, and Git policy at
a destination that does not exist. The generated source uses only the current DSL, is already in
canonical format, and builds offline into a `.prompt` product. When the destination belongs to an
enclosing workspace, package creation and any missing `workspace.members` edit are one recoverable
transaction. The top-level member path `.xmlsquish` is reserved for that workspace's manager state
and is rejected before writes; the same directory name is not reserved for a standalone project
(an explicit valid `--name` is still required because the inferred leaf contains a dot).

The previous decision that dismissed `init/new` together is explicitly reversed for `new`.
Adopting a populated existing directory (`init`) remains a distinct, unresolved product operation;
`new` never merges, overwrites, asks questions, executes remote templates, or hides an implicit
build. The complete syntax, files, workspace/VCS policy, output protocol, cancellation semantics,
and acceptance matrix are normative in [CLI experience Section 3.3](cli-experience.md#33-new).

### 3.2 Project and workspace

A project is discovered by searching upward for `xmlsquish.toml`, or selected explicitly by `--manifest-path`. Manifest schema version and XML dialect are independent. A manifest may contain a package, a workspace, or both.

The production manifest supports:

- package name, semantic version, dialect, and source root;
- named targets with entry, `.prompt` output, backend, arguments, limits, and features;
- named profiles with inheritance, arguments, limits, features, debug-info, and optimization policy;
- direct normal, development, and build-tool dependencies;
- named package exports;
- workspace members, exclusions, shared target directory, and inherited dependencies.

Target entries and exports are package-relative logical paths. Outputs are target-directory relative, use `.prompt`, cannot escape the target root, and cannot collide after lexical normalization.

### 3.3 Dependency sources and exact state

A dependency selects exactly one source:

| Source | Manifest/CLI intent | Exact resolution |
| --- | --- | --- |
| Path | `{ path = "../common" }` / `add --path` | validated package at the declared path and frozen source observation |
| Git | `{ git = "…", rev|tag|branch = "…" }` / `add --git` | immutable commit/tree recorded in lock state |
| Registry | version requirement, optional named registry / `add NAME@REQ --registry` | exact version, stable registry identity, archive/content checksums |
| Workspace | `{ workspace = true }` | matching workspace dependency and member identity |

The resolver owns version choice; the fetch layer owns verified acquisition and materialization; the repository owns the authoritative snapshot. `--locked` forbids lock mutation, `--offline` forbids network access, and `--frozen` combines both. A valid local cache may satisfy locked or offline requests. Manifest and lock changes from `add`/`remove` are committed as one recoverable logical transaction.

Adding a dependency only makes exports resolvable. It never inserts, removes, or rewrites `xs:import`.

### 3.4 Build and artifact model

```text
manifest + exact dependency graph
  -> immutable workspace snapshot
  -> complete action plan
  -> source compile to relocatable IR
  -> static link from one xs:entry root
  -> entry instantiation with args and budgets
  -> backend
  -> atomic target generation
```

A selected target publishes a `.prompt` product. Repeated `--emit` selections may also materialize `.xsir` reusable IR and `.psdbg` self-contained debug evidence. Content-derived action keys support safe cache reuse. One invocation never observes two revisions for the same logical source.

The scheduler may run independent work concurrently up to `--jobs`. The default is keep-going: a root failure blocks only dependent work. `--no-keep-going` stops admitting new work after the first failure. Invalid plans, output collisions, unresolved dependencies, or failed generations do not publish partial success for the affected target.

### 3.5 Formatting and mutation

`fmt` selects project-owned sources, selected packages, or explicit `--path` values. Its lossless XML CST policy preserves compiler-relevant semantics, protects scalar and mixed-content whitespace, validates before committing, and is idempotent. `--check` reports dirty inputs without writing and returns domain failure; `--diff` implies check.

`add` is a typed dependency upsert and `remove` deletes an exact direct alias in the selected dependency class. Both preserve unrelated TOML comments and formatting, validate candidates and lock coherence before commit, and support no-write `--dry-run`.

### 3.6 Inspection, presentation, and configuration

`inspect ir|link|source|cache|artifact` reads one typed manager object and returns one human or versioned JSON document. It does not mutate project state. Operational commands use human, short, or versioned newline-delimited JSON events. Human/short status and diagnostics use stderr; NDJSON uses stdout.

Configuration is merged in deterministic order:

```text
defaults < user config < workspace config < environment < CLI
```

User configuration is `$XMLSQUISH_HOME/config.toml` or the platform config directory; workspace configuration is `<project>/.xmlsquish/config.toml`. Repeatable `--config KEY=VALUE` applies TOML-typed CLI overrides in order. Explicit presentation and execution flags have CLI precedence.

### 3.7 Stable process contract

| Exit | Meaning |
| ---: | --- |
| 0 | success, help, or version |
| 1 | domain failure, including a dirty `fmt --check` |
| 2 | usage/argument parse failure |
| 101 | unmapped internal failure at the composition boundary |
| 130 | cancelled by interruption after cleanup |

Automation consumes exit status and structured output, not human prose. Bare `xmlsquish` prints help and succeeds; unknown commands are usage errors and are never reinterpreted as paths.

## 4. Explicit non-goals that still apply

- No runtime-loaded native plugins, ambient service locator, or unstable Rust ABI extension system. Frontends and backends are statically composed unless a future versioned process protocol is specified.
- No build scripts, arbitrary subprocess hooks, lifecycle hooks, or evaluation of environment/time/randomness inside the language core.
- No automatic source edits for dependency imports and no implicit build as a side effect of `add` or `remove`.
- No formatting of dependency caches, generated artifacts, or manager-owned immutable blobs.
- No general XML transformation guarantee: the `squish` backend intentionally produces prompt-oriented `.prompt` bytes, while XML remains a frontend syntax.
- No simultaneous incompatible versions of a package inside one link closure when their globally keyed macro expanded names would collide.
- No claim of hermeticity for editable path dependencies, or of performance/reproducibility beyond the measured configuration and declared inputs.
- No default telemetry. Adoption evidence must be opt-in or repository-local.
- No second browser/compiler implementation that can silently diverge from the Rust manager.

Incremental caching, parallel scheduling, workspaces, Git, registries, and lockfiles were non-goals of the old MVP proposal; they are now production scope and must not remain described as deferred work.

## 5. Compatibility and migration

The cutover intentionally reserves the command names `build`, `fmt`, `add`, `remove`, and `inspect`. Migrate a loose source invocation by creating `xmlsquish.toml`, declaring its entry under `[target.<name>]`, moving entry arguments into target/profile defaults or passing `--arg <target>.<name>=<value>`, then running `xmlsquish build -t <name>`.

Migrate generated artifacts as follows:

| Old concept | Current contract |
| --- | --- |
| loose input operand | manifest target |
| printable intermediate XML | reusable binary `.xsir` |
| sibling final XML | target-directory `.prompt` |
| optional provenance output | self-contained `.psdbg` |
| compiler entry execution | static link from explicit `xs:entry`, then instantiation |

There is no compatibility shim that guesses whether a command word is a file. Historical release notes remain historical evidence, not the current product contract.

## 6. Acceptance and release gates

### Correctness

- A successful `new` publishes the complete canonical scaffold and any required workspace
  membership as one coherent transaction; a following `fmt --check` and offline `build` succeed.
- An existing destination, workspace conflict, pre-commit failure, or pre-commit cancellation
  overwrites nothing and publishes no partial project. A post-decision process death is recovered
  by the next ordinary invocation.
- Process-kill recovery is required on every supported platform. It does not, by itself, prove
  Windows sudden-power-loss durability where directory flush/sync is unavailable; that narrower
  claim requires a storage-level power-cut or equivalent fault harness.
- All selected targets observe one authoritative frozen snapshot and exact resolved package graph.
- Entry linking validates the complete static source closure, including import cycles and unreachable definitions according to DSL rules.
- Equivalent declared inputs produce byte-identical canonical IR and prompt products under the same tool/schema/backend identities.
- Invalid planning produces no target artifacts; failed publication preserves the previously committed generation.

### Formatting and mutation

- A second formatting run yields no diff; the differential corpus preserves frontend semantics.
- Dirty `fmt --check` writes nothing and exits 1.
- Add/remove round trips preserve unrelated comments and return coherent manifest/lock state.
- Dry runs report the same candidate effect as real runs and write nothing.

### Observability and automation

- Human diagnostics name operation, package/target, stage, source, and artifact when applicable.
- NDJSON is parseable one event per line, contains no terminal escapes, and ends with a final operation result.
- Inspect JSON is one versioned document and is not mixed with operation events.
- Cancellation settles admitted work and manager-owned temporary state before exit 130.

## 7. Durable code/document mapping

This map is an evidence index, not a substitute for executable tests:

| Contract | Primary implementation/evidence |
| --- | --- |
| Transactional package creation | `crates/squish-project/src/scaffold.rs`, `crates/squish-repository/src/creation.rs`, `crates/squish-manager/src/new.rs`, `crates/squish-manager/tests/new.rs`, and the root process/recovery creation cases; remote cross-platform workflow evidence remains required before release |
| Direct command grammar and typed selectors | `crates/squish-cli/src/lib.rs`, `crates/squish-cli/tests/cli_contract.rs` |
| Composition, layered configuration, streams, exits | `src/main.rs`, `tests/process.rs`, `crates/squish-config/` |
| Manifest/workspace/dependency model and edits | `crates/squish-project/src/{manifest,model,edit,transaction}.rs` |
| Discovery and immutable workspace snapshot | `crates/squish-repository/src/` |
| Exact resolution and lock policy | `crates/squish-resolver/src/`, `docs/design/dependency-source-protocol.md` |
| Registry/Git/path materialization | `crates/squish-fetch/src/`, `crates/squish-host/src/lib.rs` |
| XML semantics to relocatable IR | `crates/squish-xml-front/src/`, `docs/dsl.md` |
| Canonical IR, link and debug bundle | `crates/squish-ir/src/`, `crates/squish-link/src/`, `docs/design/ir-model.md` |
| Action planning, cache, scheduling and publication | `crates/squish-build/`, `crates/squish-manager/`, `crates/squish-store/`, `crates/squish-publish/` |
| Semantics-preserving formatter | `crates/squish-format/`, especially `tests/semantic_oracle.rs` |
| Human/short/NDJSON presentation | `crates/squish-presentation/src/lib.rs` |
| Integrated execution status | `docs/design/project-manager-execution-status.md`, `tests/process.rs` |

## References

- [ADR 0009: Microkernel Manager and Reusable IR](../adr/0009-microkernel-manager-and-reusable-ir.md)
- [Core IR model](../design/ir-model.md)
- [Dependency source protocol](../design/dependency-source-protocol.md)
- [Product CLI and terminal experience](cli-experience.md)
- Cargo, [`new`](https://doc.rust-lang.org/cargo/commands/cargo-new.html) and [`init`](https://doc.rust-lang.org/cargo/commands/cargo-init.html)
- npm, [`init`](https://docs.npmjs.com/cli/v11/commands/npm-init/); Go, [`go mod init`](https://go.dev/ref/mod#go-mod-init)
- Cargo, [Configuration](https://doc.rust-lang.org/cargo/reference/config.html) and [External tools](https://doc.rust-lang.org/cargo/reference/external-tools.html)
- Mokhov, Mitchell, and Peyton Jones, [Build Systems à la Carte](https://doi.org/10.1145/3236774), PACMPL/ICFP 2018
