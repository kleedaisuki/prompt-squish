# Prior Art for Evolving `xmlsquish` into a Project Manager

Status: historical research synthesis; its earlier convergence is superseded by [ADR 0009](../adr/0009-microkernel-manager-and-reusable-ir.md)
Date: 2026-09-14

## 1. Question and scope

This note evaluates production package/project managers and relevant research for evolving
`xmlsquish` from a compiler-style CLI into a Cargo-like project manager with at least
`fmt`, `build`, `add`, and `remove`.

The decision question is not merely which command names to copy. It is:

> What shared project state, dependency model, build plan, scheduling policy, and compatibility
> contract must make those commands coherent?

The review emphasizes Cargo, Go modules, npm, pnpm, uv, reproducible builds, dependency
resolution, and formatter stability. It intentionally does not design a registry protocol or
perform a security audit.

### Historical convergence (superseded)

This document records the earlier design space; it is not the implementation contract. The
following convergence was replaced by [ADR 0009](../adr/0009-microkernel-manager-and-reusable-ir.md)
after the project adopted a top-down microkernel, full resolver/lock/cache architecture, and direct
command interface. It is retained only to explain the rejected intermediate design:

- the MVP has one root package and exact editable local path dependencies, with no user-visible
  workspace, remote source, resolver, or dependency lockfile;
- compatibility-period commands enter manager mode through the previously invalid `--project`
  option (`xmlsquish --project build|fmt|add|remove`), not positional command words;
- `fmt` preflights every selected project file before any write, then uses individually atomic
  replacements; invalid input changes no selected file, while a later commit failure is not
  misrepresented as a project-wide filesystem transaction;
- immutable Git/registry distribution, real lock state, workspace membership, top-level short
  commands, and parallel scheduling require later decisions and evidence.

Where a later section says “v1” or “Milestone 1”, ADR 0009 takes precedence.

### Evidence labels

| Label | Meaning |
| --- | --- |
| **Official behavior** | Current first-party documentation or upstream implementation of a production tool. Strong evidence of an implemented contract, but not proof that the contract is universally best. |
| **Peer-reviewed model** | Published research with a precise model or empirical study. Stronger for the claim studied, but its assumptions may not match this project. |
| **Frontier preprint** | Recent, inspectable research that is not yet mature enough to dictate production architecture. |
| **Project inference** | A recommendation derived from the evidence and the current `xmlsquish` language/implementation. It is not an externally observed fact. |

## 2. Current repository facts that constrain the design

These are observations from the repository, especially
[`docs/dsl.md`](../dsl.md) and
[`ADR 0004`](../adr/0004-single-package.md):

1. `xs:import src="..."` is the only source-loading operation, and every `src` is a static URI
   literal. The compiler can therefore discover and freeze the full source closure before
   execution.
2. Pure import cycles are legal. An import graph is not necessarily a DAG and must not be fed
   directly to a conventional topological build scheduler.
3. A source identity is a canonical logical URI. It is deliberately not collapsed by physical
   filesystem identity or by content hash.
4. Inputs are never overwritten by the current compiler path. Failures are isolated between
   independent inputs, and successfully persisted outputs determine reported success.
5. The core compiler does not read the environment or clock and does not execute subprocesses.
   `Compiler::prepare` freezes and links a reusable program; each expansion receives fresh
   arguments and budget state.
6. The existing naked-path CLI (`xmlsquish file.xml`, directories, and globs) is an established
   user contract.

These properties are unusually favorable for a deterministic project build: imports are static
and compilation is already close to a pure transformation. They also rule out treating every
XML import edge as a package or task dependency.

## 3. The central model: three graphs, not one

The most important architectural separation is:

| Graph | Nodes and edges | Cycles | Owner |
| --- | --- | --- | --- |
| **Package resolution graph** | Package identities, exact versions/sources, transitive package requirements | Policy-dependent; normally resolved before building | Resolver and lockfile |
| **XML import graph** | Canonical `SourceId` values and static `xs:import` edges | Legal in the current language | Existing compiler/loader |
| **Build target/action graph** | Entry targets, preparation/expansion/output actions, and workspace task dependencies | Scheduler graph should be acyclic; import SCCs can be condensed into nodes | Project build planner and scheduler |

**Project inference:** collapsing these graphs would create false special cases. In particular,
rejecting or topologically sorting the XML import graph would break the language. The package
resolver should map logical package sources to immutable content; the compiler should continue
to resolve and intern canonical source URIs; the scheduler should operate on an immutable build
plan above both.

## 4. What the production systems actually teach

### 4.1 Comparative summary

| System | Strong production lesson | Important limitation for `xmlsquish` |
| --- | --- | --- |
| Cargo | One project model backs dependency editing, locking, workspace selection, builds, and machine-readable metadata. Manifest intent and exact lock state are distinct. | Cargo's mature resolver and crate feature semantics are more machinery than a new prompt-package ecosystem initially needs. |
| Go modules | Minimal Version Selection (MVS) is deterministic without a traditional lockfile because requirements are lower bounds and module identity changes at incompatible major versions. `go.sum` authenticates content rather than selecting versions. | This depends on a strong compatibility convention that a new XML package ecosystem does not yet possess. |
| npm | Normal development installs may reconcile manifest and lock state; `npm ci` is an explicitly frozen, non-mutating mode for automation. | The mutable `node_modules` deployment model and lifecycle script surface are not appropriate defaults for deterministic prompt compilation. |
| pnpm | A content-addressable store avoids duplicate package content; workspace scheduling uses a ready queue rather than unrelated topological barriers. | Its symlink/hard-link deployment exists to satisfy Node resolution and should not be copied when the XML loader can read a store directly. Some pipeline caching facilities remain experimental. |
| uv | Project commands automatically keep manifest, lock, and environment coherent; `--locked`, `--frozen`, and `--no-sync` expose strict alternatives. It uses a cross-platform lockfile and a production PubGrub resolver. | Python markers, wheel variants, and environment materialization are far more complex than platform-neutral XML source packages. |
| Bazel/Nix | Declared inputs, immutable/content-addressed artifacts, and hermetic action identities enable safe reuse and reproducibility. | Full sandboxing, remote execution, and general build languages would be premature for this small, internally pure compiler. |

### 4.2 Cargo: the closest command and state model

**Official behavior.** Cargo describes `Cargo.toml` as author-maintained broad dependency intent
and `Cargo.lock` as exact, Cargo-maintained resolution state. Cargo recommends checking the lock
file into version control in most cases:
[Cargo.toml vs Cargo.lock](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html).

`cargo add` and `cargo remove` are domain operations over a manifest, with package/source
selection and dry-run support; they are not generic text insertion commands:
[cargo add](https://doc.rust-lang.org/cargo/commands/cargo-add.html),
[cargo remove](https://doc.rust-lang.org/cargo/commands/cargo-remove.html).
Cargo searches for the nearest manifest in the current directory or a parent.

`cargo build` separates package/workspace selection, target selection, feature/profile selection,
and job scheduling:
[cargo build](https://doc.rust-lang.org/cargo/commands/cargo-build.html).
Cargo workspaces share a root lockfile and output directory:
[Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html).

Cargo exposes a stable, versioned JSON metadata surface and JSON build messages rather than
requiring tools to link unstable internals:
[Cargo external tools](https://doc.rust-lang.org/cargo/reference/external-tools.html),
[cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html).

**Project inference.** Copy the separation and compatibility surface, not all of Cargo's feature
and resolver semantics. Project discovery, target selection, lock policy, and output policy
should be common infrastructure used by every new subcommand.

### 4.3 Go modules: an instructive counterexample to “every project needs a lockfile”

**Official behavior.** Go computes a deterministic build list using Minimal Version Selection:
each requirement states a minimum module version, and the selected version for a module is the
highest minimum encountered in the graph. Newer releases do not change the result unless a
requirement changes. Go therefore does not save its build list in a traditional lockfile:
[Go Modules Reference: MVS](https://go.dev/ref/mod#minimal-version-selection).

`go.sum` contains cryptographic hashes for downloaded module content and metadata. It may contain
several versions and does not itself select the build list:
[Go Modules Reference: go.sum](https://go.dev/ref/mod#go-sum-files).
Go makes incompatible v2+ releases use a new module path, encoding compatibility into identity:
[Go Modules: v2 and Beyond](https://go.dev/blog/v2-go-modules).

Go also makes reconciliation explicit through `go mod tidy`, including a diff-only automation
mode, while `go fmt` is a source mutation command distinct from build:
[go command reference](https://pkg.go.dev/cmd/go).

**Project inference.** A lockfile is appropriate if `xmlsquish` exposes ordinary SemVer ranges
and highest-compatible/backtracking behavior. Go MVS should not be copied until package identity
and backward-compatibility rules are credible. Its useful lesson is to keep the resolver policy
small, explicit, and explainable, and to distinguish version selection from content verification.

### 4.4 npm and uv: ergonomic development mode versus frozen automation

**Official behavior.** `package-lock.json` records an exact dependency tree, resolved locations,
and integrity values and is intended for version control:
[npm package-lock.json](https://docs.npmjs.com/cli/v11/configuring-npm/package-lock-json/).
`npm ci` requires an existing lockfile, rejects manifest/lock disagreement, removes the existing
installation, and never writes the manifest or lockfile:
[npm ci](https://docs.npmjs.com/cli/v11/commands/npm-ci/).

uv automatically locks and synchronizes project state before project commands. `--locked`
checks that the lock is current without modifying it; `--frozen` uses the lock without checking
it against the manifest; and `--no-sync` skips environment synchronization:
[uv locking and syncing](https://docs.astral.sh/uv/concepts/projects/sync/).
uv preserves locked versions as preferences unless constraints exclude them, while updates are
explicit. Its project interface provides native `add`, `remove`, `sync`, `lock`, `build`,
`format`, and `check` operations:
[uv CLI](https://docs.astral.sh/uv/reference/cli/).

uv's lockfile is cross-platform, exact, human-readable but machine-managed, and intended for
version control:
[uv project layout](https://docs.astral.sh/uv/concepts/projects/layout/).
Workspaces have per-package manifests and one shared lockfile:
[uv workspaces](https://docs.astral.sh/uv/concepts/projects/workspaces/).

The upstream `uv add` implementation snapshots manifest/lock state and restores it if resolution
or synchronization fails. This is implementation evidence, not a documented universal guarantee:
[`add.rs`](https://github.com/astral-sh/uv/blob/c0df400a4cf4aad88f7f34bb2ac3ebb5a8f3839e/crates/uv/src/commands/project/add.rs#L558-L818).

**Project inference.** Local interactive `build` may create or refresh an absent/stale lock to
avoid needless user friction, while CI should use `build --locked` (or `--frozen` for locked plus
offline). A failed `add` or `remove` resolution must leave the prior manifest and lock intact.
Dependency materialization is derived state and can be completed or repaired after the durable
manifest/lock transaction.

### 4.5 pnpm and uv: content reuse without semantic leakage

**Official behavior.** pnpm stores package files in a content-addressable store and hard-links
them into project installations; its linked layout then recreates Node's dependency resolution:
[pnpm symlinked node_modules](https://pnpm.io/symlinked-node-modules-structure).
This yields cross-project reuse, but the layout and symlinks are Node compatibility mechanisms,
not general package-manager requirements.

uv's cache is append-only and safe for concurrent readers/writers; it uses a file lock for
mutable project environments. Cache buckets are independently versioned. It falls back to
copying when cache and environment are on different filesystems:
[uv caching](https://docs.astral.sh/uv/concepts/cache/).

**Project inference.** Start with an immutable package-content store keyed by an archive/source
digest and a normal copy/read path that works on every supported platform. Hard links, reflinks,
and shared extraction are optional optimizations with copy fallback. Cache paths must never enter
logical `SourceId`, the lockfile, diagnostics, or generated prompt output. Dependency CAS and
compiled-artifact cache are separate concerns and should have separate namespaces and invalidation
rules.

### 4.6 pnpm: a practical workspace scheduler model

**Official behavior.** pnpm's workspace task orchestrator represents a task as
`<project>#<script>`. A task becomes ready after all dependencies succeed; ready tasks run under
global and per-task concurrency limits, and independent work does not wait at unrelated
topological-level barriers. It supports stable dry-run graph output, JSON inspection, cycle
detection, continuing independent tasks after failure, and prefixed or aggregated concurrent
output:
[pnpm workspace task orchestration](https://pnpm.io/workspace-task-orchestration).

**Project inference.** This ready-queue policy is a good future workspace scheduler. It need not
be implemented in the first project-manager release. The first version can execute independent
entry targets sequentially behind the same immutable `BuildPlan` interface; later bounded
parallelism should preserve the existing policy that one target's failure does not suppress
unrelated targets. Deterministic reports require stable target IDs and tie-breaking even when
actual completion order is nondeterministic.

## 5. Recommended state model and command contracts

### 5.1 Three states

```text
Manifest                 Lockfile                    Materialized state
(human intent)           (exact resolution)          (derived, disposable)
------------------       ------------------------    -------------------------
broad constraints        exact name/version/source  downloaded/extracted XML
declared targets         content digest             compiler intermediates
workspace membership     transitive package edges   final build artifacts
language/style edition   resolver/lock schema       caches and indexes
```

The manifest is authoritative intent. The lockfile is a public, versioned format managed by the
tool. Materialized state is never authoritative and can be deleted and reconstructed.

### 5.2 Command boundaries

| Command | Reads | May write | Must not do |
| --- | --- | --- | --- |
| `fmt` | Manifest-selected source files and format policy | Source files, unless `--check`/`--diff` | Resolve/fetch dependencies, compile outputs, or invoke the existing lexical `squish` pass |
| `build` | Manifest, lock, selected targets, resolved source closure | Lock in interactive mode if policy permits; build artifacts and disposable cache | Rewrite source files or silently change the formatting style |
| `build --locked` | Manifest, existing current lock, sources | Artifacts/cache only | Create or change the lock |
| `add` | Manifest, lock, source/index metadata | Manifest and lock as one logical transaction; fetched immutable blobs | Rewrite XML imports or opportunistically update unrelated packages |
| `remove` | Manifest and lock | Manifest and re-resolved/pruned lock | Delete shared cache as part of dependency removal or scan/rewrite user XML |
| future `update` | Manifest and lock | Selected lock entries | Conflate a requested package update with broad unrelated updates |
| future `tidy` | Manifest, static source/package usage | A reviewed manifest/lock reconciliation | Guess across dynamic behavior (the current static imports make this more tractable) |

`--dry-run` should compute and display the same plan without persistence. A versioned JSON form
should exist before third-party tooling starts scraping human output.

### 5.3 Logical transaction for `add` and `remove`

Recommended sequence:

1. Discover and lock the project root for mutation.
2. Parse the manifest and lockfile into typed models; record their original content hashes.
3. Apply the requested edit in memory.
4. Resolve the complete resulting package graph, retaining locked versions when still valid.
5. Produce `manifest_after`, `lock_after`, and an inspectable change plan.
6. Recheck original hashes, stage both files in their destination directories, and atomically
   replace them in a documented order.
7. Fetch/materialize derived package content. A materialization failure may leave the coherent
   manifest and lock committed because the cache is recoverable; a resolution failure must leave
   both durable files unchanged.

No portable filesystem primitive atomically replaces two files. Recovery therefore matters more
than pretending the operation is physically atomic. Every command should detect manifest/lock
disagreement and either reconcile it in interactive mode or report it under `--locked`.

## 6. Package and source identity

Before `add` can be stable, the project must define what is being added.

Recommended distinct types:

```text
PackageId   = (normalized package name, canonical source identity, exact version when resolution exists)
SourceSpec  = Registry(name) | Git(url, rev) | Path(path) | Workspace(member)
SourceId    = existing canonical URI of one XML source unit
TargetId    = (workspace package, target name, profile/parameter identity)
```

A distribution package is not an XML namespace. One package may export several macro namespaces,
and different packages can still collide at the compiler's expanded-name layer. Keep package
identity, source URI identity, and macro identity separate.

A package-aware reference fits the existing static-loader architecture without changing macro
semantics, for example `pkg:common/main`, where `common` is a direct dependency alias and `main`
is a declared export. The project resolver maps this access request to a resolved source. The
compiler can use a distinct stable identity URI such as `xmlsquish://package/path.xml`, not a
machine-specific locator. Import references and canonical identities must be different types:
an alias may differ from the actual package name, and a source path is not an export name.
Consequently, `file.uri` stays portable and generated output does not leak `%LOCALAPPDATA%`, drive
letters, or home directories. A future immutable source resolver maps through the lockfile; the
path-only MVP maps directly through its manifest.

For local workspace packages, prefer an explicit typed source or `workspace:`-style protocol.
pnpm refuses to fall back to a registry when the `workspace:` protocol is requested:
[pnpm workspaces](https://pnpm.io/workspaces). Silent local/registry fallback makes identical
manifests mean different things on different machines.

## 7. Lockfile and resolver policy

### 7.1 Minimum lockfile contents

The public schema should be versioned and include, for every resolved package:

- normalized name and exact version;
- canonical source identity and source kind;
- immutable Git commit or registry record;
- content digest and digest algorithm;
- resolved transitive package edges;
- resolver version and lockfile schema version;
- any future conditional-resolution markers that affect the selected graph.

It should not include absolute cache paths, access timestamps, temporary extraction paths, or
host-native path separators. Portable logical paths use URI `/` separators and are converted to
native paths only at the I/O boundary.

**Peer-reviewed empirical evidence.** A 2026 study of Cargo, npm, pnpm, Poetry, Pipenv, Gradle,
and Go lock behavior recommends generating lockfiles by default, respecting locked selections,
avoiding silent updates, keeping the file human-inspectable, and retaining resolution-relevant
versions, sources, and checksums:
[“The Design Space of Lockfiles Across Package Managers”](https://doi.org/10.1007/s10664-025-10789-w)
([open manuscript](https://arxiv.org/html/2505.04834)). This is cross-ecosystem evidence, not a
substitute for defining `xmlsquish` package semantics.

The lock should carry a fingerprint of the **resolution-relevant projection** of the manifest,
not necessarily the entire manifest byte string. Editing a description or reformatting TOML must
not invalidate dependency resolution, while changing a dependency/source/resolver setting must.

### 7.2 Resolver complexity and scope

**Peer-reviewed research.** Dependency resolution becomes NP-complete in non-trivial models that
allow version choice plus non-co-installability/conflicts. Ad-hoc greedy solvers can be incomplete;
the research recommendation is to separate package policy from a specialized solver:
Abate et al., [“Dependency Solving Is Still Hard, but We Are Getting Better at It”](https://www.dicosmo.org/Articles/2020-SANER.pdf), SANER 2020.

**Official production implementation.** Dart's PubGrub adapts conflict-driven clause learning,
records causes while backtracking, and uses those causes for comprehensible failure explanations:
[Dart Pub solver design](https://github.com/dart-lang/pub/blob/master/doc/solver.md).
uv uses the Rust PubGrub implementation and emphasizes that metadata loading and decision
prioritization dominate many real resolutions:
[uv resolver internals](https://docs.astral.sh/uv/reference/internals/resolver/).

**Frontier preprint.** Gibb et al. formalize a common package calculus with root inclusion,
dependency closure, and version uniqueness, prove NP-completeness of the expressive core, and
model conflicts, concurrent versions, peers, features, formulae, variables, and virtual packages
as extensions. The work is Lean-mechanized but, as a 2026 preprint, should guide boundaries rather
than mandate an implementation:
[“Package Managers à la Carte”](https://arxiv.org/abs/2602.18602).

**Project inference.** Version the resolver boundary from day one, but do not build a generic SAT
solver before the package ecosystem exists. A defensible staged policy is:

1. MVP: one root package plus exact editable local path packages, without a resolver lock;
2. next: exact Git revisions and immutable registry packages, with content digests in a real lock;
3. later: SemVer ranges backed by a mature solver such as PubGrub, with locked-version preference
   and causal error explanations;
4. defer platform markers, peer dependencies, virtual packages, features, and multiple concurrent
   versions until a demonstrated prompt-package use case requires them.

## 8. Reproducibility is stronger than locking

The Reproducible Builds project defines a reproducible build as producing bit-for-bit identical
artifacts from the same source, build environment, and build instructions:
[definition](https://reproducible-builds.org/docs/definition/).
A lockfile fixes dependency selection; it does not by itself fix the compiler version, build
options, environment, input order, timestamps, randomness, absolute paths, or line-ending policy.

Relevant production guidance includes stable, locale-independent input ordering
([stable inputs](https://reproducible-builds.org/docs/stable-inputs/)) and a standardized source
timestamp when a timestamp is truly necessary
([SOURCE_DATE_EPOCH specification](https://reproducible-builds.org/specs/source-date-epoch/)).

**Peer-reviewed empirical evidence.** A 2025 MSR distinguished paper rebuilt 709,816 historical
Nix packages. Bitwise reproducibility ranged from 69% to 91% despite rebuildability over 99%; about
15% of observed unreproducibility failures involved embedded build dates. This demonstrates that
functional dependency/environment management greatly helps but does not make every task
deterministic automatically:
Malka et al., [MSR 2025 paper](https://doi.org/10.1109/MSR66628.2025.00115).

For `xmlsquish`, a complete build identity should cover at least:

```text
source closure contents
+ exact package graph and content digests
+ xmlsquish/compiler version
+ DSL/language edition
+ resolver version
+ formatter style edition (for source checks, not build semantics)
+ target/profile/output mode
+ entry arguments and resource-budget options that affect success/output
+ workspace member and explicit local overrides
```

Build discovery and reporting must use a stable order independent of filesystem enumeration and
locale. No current-time field should enter `.i.xml` or `.o.xml`; if packaging later needs dates,
use a source-derived value. Local path dependencies are live development inputs: a lock that only
records their path is not sufficient for bitwise reproducibility, so either hash their source
closure in the build identity or require a vendored/registry snapshot for releases.

It is useful to distinguish four claims in user-facing documentation:

| Claim | What it establishes |
| --- | --- |
| Rebuildability | The build can still complete later. |
| Environment reproducibility | Tool and dependency environment can be reconstructed. |
| Bitwise reproducibility | Declared artifacts compare byte-for-byte equal. |
| Verifiable provenance | Evidence links an artifact to claimed inputs and steps. |

A build record can preserve input/output digests, tool version, target, and options as a sidecar.
Wall-clock duration, invocation IDs, and host paths belong only in diagnostic/build records, not in
the artifact identity. A future record can align fields with
[SLSA build provenance](https://slsa.dev/spec/v1.2/build-provenance), but a locally self-asserted
record is useful for debugging and repetition, not independent attestation.

## 9. Build-system research and the appropriate scheduler

**Peer-reviewed model.** Mokhov, Mitchell, and Peyton Jones separate two orthogonal build-system
choices: the order in which tasks are run (scheduler) and whether a task should rebuild
(rebuilder). They compare static/dynamic dependencies, restarting/suspending schedulers, early
cutoff, and persistent build information across Make, Shake, Bazel, Nix, and others:
[“Build Systems à la Carte: Theory and Practice”](https://doi.org/10.1017/S0956796820000088)
([author PDF](https://www.microsoft.com/en-us/research/wp-content/uploads/2020/04/build-systems-jfp.pdf)).

**Project inference.** Current static `xs:import` edges remove the need for a dynamic dependency
scheduler. The simplest correct starting point is:

1. discover the project and selected targets;
2. resolve packages and freeze an immutable plan;
3. condense each XML import strongly connected component for planning purposes without changing
   the compiler's logical `SourceId` semantics;
4. detect output-path collisions before any target publishes an artifact;
5. execute independent target jobs sequentially initially, then by a bounded ready queue;
6. a failed job blocks only its dependents; unrelated jobs continue;
7. publish each target's artifacts atomically and aggregate a non-zero final status;
8. emit stable target-keyed diagnostics or versioned NDJSON so concurrency does not turn logs into
   an undocumented API.

Do not start with a daemon, remote execution, general script runner, or compiled-artifact cache.
When measurements justify incremental caching, key an action by declared inputs, tool/format
versions, arguments, and dependency digests. Bazel's official remote-cache model makes the same
distinction between an action cache and a content-addressable output store:
[Bazel remote caching](https://bazel.build/remote/caching).

## 10. Formatter correctness and evolution

`fmt` is the highest-risk of the four proposed commands because XML whitespace, CDATA, namespace
bindings, QName lexical context, and scalar macro bodies can be semantically observable. The
existing `squish` operation is a final-product lexical transform and must not be reused as a source
formatter.

W3C XML rules make an ordinary parse/serialize loop an insufficient foundation. XML processors
must pass non-markup character data to applications, and `xml:space` may explicitly request
preservation ([XML 1.0, section 2.10](https://www.w3.org/TR/xml/#sec-white-space)). The XML
Information Set does not retain attribute order, quote style, character-reference spelling, or
CDATA boundaries ([XML Infoset](https://www.w3.org/TR/xml-infoset/)). Canonical XML deliberately
normalizes several of those forms for identity/signature use, so it is not a human source
formatter ([Canonical XML 1.1](https://www.w3.org/TR/xml-c14n/)). In this prompt DSL, scalar and
mixed-content whitespace can also affect generated text and tokenization.

Recommended formatter laws:

```text
Idempotence:          fmt(fmt(source, style), style) == fmt(source, style)
Parse preservation:  parse(fmt(source, style)) succeeds whenever parse(source) succeeds
Semantic preservation:
  erase_spans(prepare(fmt(source, style))) == erase_spans(prepare(source))
```

Where a full prepared-program comparison is impractical, compare a semantic AST/IR that retains
all whitespace-sensitive scalar text and namespace/QName meaning while erasing only source spans
and structurally irrelevant layout.

The formatter should therefore use a lossless concrete syntax tree (CST) or token/trivia edits,
preserving original bytes for character data, CDATA, entity spellings, attribute values, comments,
processing-instruction bodies, and namespace/QName-sensitive regions. Initially normalize only
layout proven to be compiler-irrelevant, such as selected inter-declaration trivia and tag-internal
separator whitespace. Do not reorder imports, macros, attributes, or namespace declarations.

**Official production practice.** Rustfmt provides `--check` with a CI-friendly exit code and
Cargo-wide traversal:
[rustfmt](https://github.com/rust-lang/rustfmt).
Rust's formatter stability RFC recognizes that formatting drift breaks CI and pollutes diffs;
style evolution is explicit and edition-based rather than silently following every tool upgrade:
[RFC 2437](https://rust-lang.github.io/rfcs/2437-rustfmt-stability.html),
[RFC 3338](https://rust-lang.github.io/rfcs/3338-style-evolution.html).

Black checks AST equivalence by default, while documenting narrowly scoped exceptions:
[Black AST checks](https://black.readthedocs.io/en/stable/the_black_code_style/current_style.html#ast-before-and-after-formatting).
Prettier's production experience argues for very few style options because option proliferation
recreates formatting debates and multiplies behavior combinations:
[Prettier option philosophy](https://prettier.io/docs/option-philosophy).

**Empirical caution.** The Scalafmt thesis reports that idempotence bugs remained after enabling
idempotence tests across a 1.2-million-line corpus. Corpus tests alone therefore do not establish
the law; property tests and adversarial fixtures are needed:
[Geirsson, *Scalafmt: Automated Code Formatting for Scala*](https://geirsson.com/assets/olafur.geirsson-scalafmt-thesis.pdf), section 4.2.3.

Minimum `fmt` test matrix:

- apply the formatter twice to every fixture and randomized valid source;
- compare prepared semantic IR before/after, excluding spans only;
- cover mixed content, whitespace-sensitive `xs:arg` bodies, CDATA, entities, comments,
  processing instructions, namespace rebinding, prefixed QNames, BOM, LF/CRLF, deep nesting,
  and incomplete/invalid files;
- verify `fmt --check` performs no writes and uses a stable exit contract;
- preflight all selected files so invalid input causes no writes; after preflight, report every
  individually atomic commit result without claiming cross-file transactional replacement;
- use temporary-file plus atomic-replace persistence, preserving the repository's established
  file-write invariants.

The initial style should be opinionated and nearly configuration-free. Store a `style-edition`
in the project manifest so future style changes are explicit migrations rather than surprise diffs.

## 11. Compatibility and observability

The existing direct compiler CLI must survive. A compatible routing strategy is:

```text
xmlsquish file.xml                 # legacy direct invocation, unchanged behavior
xmlsquish directory-or-glob        # legacy batch invocation, unchanged behavior
xmlsquish --project build [target/selectors] # explicit project mode
xmlsquish --project fmt [paths/selectors]
xmlsquish --project add <package-spec>
xmlsquish --project remove <package-name>
```

A file or directory literally named `build`, `fmt`, `add`, `remove`, or `project` creates an
unavoidable ambiguity for positional command grammars. The selected MVP therefore uses the
previously invalid `--project` option; preserve `xmlsquish -- <path>` for option-looking paths and
add regression tests.
The new `build` command should call the same compiler core rather than creating a second
compilation implementation.

Before ecosystem integrations appear, expose:

- `metadata --format-version 1` for project, targets, selected workspace, package graph, and
  logical source roots;
- `build --message-format human|json` with versioned, one-record-per-line events;
- `--dry-run --json` for dependency edits and future workspace task graphs;
- a configuration explanation showing the discovered project/workspace root and effective values.

Stable machine interfaces protect observability: concurrent scheduling should never require
hiding or discarding diagnostics.

## 12. Recommended delivery order

### Milestone 1: coherent local project manager

1. Typed, versioned manifest and upward project discovery.
2. Named entry targets and project-local artifact directory.
3. `build` as an adapter over the existing compiler, with the old CLI unchanged.
4. Syntax-aware, idempotent, semantics-preserving `fmt` with `--check` and `--diff`.
5. Local path dependency identity plus a package-aware loader URI.
6. `add`/`remove` plan-and-commit over the manifest; no lock exists while every dependency is an
   exact editable local path.
7. Stable metadata/build-event JSON contracts.

### Milestone 2: immutable distribution

1. Exact Git revisions, then an immutable registry source.
2. Content digests in the lockfile and an immutable dependency-content store.
3. `--locked`, `--offline`, `--frozen`, and explicit `update`.
4. Reproducibility tests across clean directories, Windows/Linux path forms, line endings, and
   randomized filesystem enumeration order.

### Milestone 3: scale only after measurement

1. SemVer ranges and a mature causal resolver such as PubGrub.
2. Workspace-wide shared lock and package selection.
3. Bounded ready-queue parallel target execution and `--keep-going` behavior.
4. Content-keyed prepared-program or build-artifact cache if profiling shows material benefit.
5. `tidy`, `tree`, `why`, and richer graph inspection.

Remote execution, plugin systems, arbitrary lifecycle scripts, peer dependencies, platform marker
forking, and general-purpose task languages are deliberately outside the initial design.

## 13. Decision summary

The evidence supports a Cargo/uv-like **state architecture**, a Go-like bias toward narrow and
explainable policy, a pnpm-like future ready-queue scheduler, and Bazel/Nix-like declared-input
discipline. It does not support copying any one tool wholesale.

The smallest durable core is:

```text
CLI (legacy adapter + project commands)
  -> Project discovery and typed manifest
  -> Local package resolver (exact path graph; no MVP lock)
  -> Immutable BuildPlan (targets/actions)
  -> Scheduler and artifact manager
  -> Existing compiler (one resolved, frozen program)
```

The first release should prove this shared model with local packages, exact state, deterministic
builds, and a safe formatter. A registry, general solver, parallelism, and caching become normal
extensions of those boundaries rather than special-case rewrites.
