# Product CLI and Terminal Experience

- Status: Implemented production contract
- Date: 2026-09-14
- Scope: the `xmlsquish` executable, its command grammar, observable terminal behavior,
  automation contract, and recovery experience
- Evidence rule: current Rust types and executable tests override examples or exploratory features
  that are not present in the composed binary
- Related: [ADR 0009](../adr/0009-microkernel-manager-and-reusable-ir.md),
  [core IR model](../design/ir-model.md), and the [refactor map](../design/refactor-map.md)

## 1. Product decision

`xmlsquish` is the one public executable and the project manager, not a compiler binary with a
collection of unrelated modes. Its interface is command-first:

```text
xmlsquish <command> [command options] [selectors]
```

There is no legacy path-dispatch grammar and no heuristic that treats an unknown command as a
file. `build`, `fmt`, `add`, and `remove` are reserved commands. All commands enter the same
manager kernel, which provides project discovery, configuration, scheduling, cancellation,
structured events, presentation, and exit-status reduction. A command domain supplies typed
intent and domain operations; it does not own the terminal.

The interface is designed as a whole rather than as a sequence of minimally viable releases.
Implementation may proceed in dependency order, but every public behavior below is the target
contract. Temporary implementation staging must not create a second public grammar or a second
event protocol.

### 1.1 Product principles

1. **One entrance, several bounded contexts.** The executable is a microkernel-style host. Build,
   formatting, dependency management, inspection, and maintenance are separate domain contexts
   behind the same orchestration contract.
2. **Human first, automation first-class.** The default is calm, legible terminal output. A
   versioned event stream exposes the same operation without scraping prose.
3. **The output explains the model.** A build visibly distinguishes source loading, IR
   compilation, linking, the `squish` backend, cache reuse, and publication of `*.prompt`.
4. **No theatrical activity.** A spinner is not evidence. Show determinate progress only when a
   meaningful denominator exists; otherwise show the current phase and elapsed time.
5. **No color-dependent meaning.** Words, diagnostic codes, symbols, indentation, and exit status
   carry semantics. Color only increases scan speed.
6. **No terminal I/O from workers.** Concurrent completion order cannot corrupt output or become
   an accidental API.
7. **Recovery is a manager responsibility.** Interrupted publication, concurrent manifest edits,
   partial downloads, and stale temporary files are detected and reconciled where the manager has
   enough information. The normal answer is not “delete this directory and try again.”
8. **Silence is composability.** `stdout` is reserved for requested data. Status, progress, and
   diagnostics use `stderr` in human mode.

Cargo provides useful production precedent for separately configurable color, Unicode,
hyperlinks, progress, quietness, and verbosity, including `auto|always|never` policies
([Cargo configuration](https://doc.rust-lang.org/cargo/reference/config.html)). Cargo also exposes
stable, versioned metadata and one-JSON-object-per-line build messages for external tools
([Cargo external tools](https://doc.rust-lang.org/cargo/reference/external-tools.html)). We adopt
the separation, not Cargo's exact schemas or prose.

## 2. Product architecture visible at the CLI

```text
argv + environment + terminal capabilities
                  |
                  v
        +---------------------+
        | Manager microkernel |
        |---------------------|
        | bootstrap parser    |
        | project locator     |
        | config resolver     |
        | scheduler/cancel    |
        | event sequencer     |
        | status reducer      |
        +----------+----------+
                   |
        typed request / event protocol
                   |
     +-------------+-------------+----------------+
     |                           |                |
 build domain                fmt domain     dependency domain
 XML -> complete IR          lossless CST    add/remove/update
 IR -> linked prompt         validation      resolver/lock state
     |                           |                |
     +-------------+-------------+----------------+
                   |
      human | short | json renderer
                   |
             stdout / stderr
```

The manager kernel is deliberately small. It owns mechanisms shared by every command, but no XML
semantics, formatting policy, resolver policy, or backend behavior. Command registration is a
table of typed command descriptors rather than a growing nested conditional. Each descriptor
provides:

```text
CommandDescriptor {
    name
    help
    parse(argv) -> CommandIntent
    plan(ProjectSnapshot, CommandIntent) -> OperationPlan
    execute(OperationPlan, EventSink, Cancellation) -> OperationResult
}
```

The public event vocabulary is smaller than internal tracing. Internal events may be arbitrarily
detailed; contract events are intentionally curated and versioned.

### 2.1 Observable state model

The observable lifecycle is the accepted ADR 0009/v2 algebra, not a renderer-specific list of
status verbs:

```text
PlanningStarted
  -> PlanningStepStarted -> {PlanningStepSucceeded | PlanningStepFailed | PlanningStepCancelled} ...
  -> {PlanReady | PlanningFailed | PlanningCancelled}

PlanReady
  -> complete ordered ActionDeclared set
  -> each action: ActionStarted -> {ActionSucceeded | ActionFailed | ActionCancelled | ActionSuperseded}
                 or {CacheHit | ActionBlocked | ActionCancelled | ActionSuperseded} without a start
  -> PlanClosed { executed | reported | superseded }

final build plan, when catalog persistence is required
  -> FinalizationStarted
  -> {FinalizationSucceeded | FinalizationFailed}

OperationCompleted -> JobFinished
```

A revision race may close one immutable plan as superseded and start a new planning attempt; it
never mutates a plan after `PlanReady`. A pre-plan failure legitimately has no action vertices.
`blocked` means failed/blocked prerequisites, not an unrecovered filesystem special case. A cache
hit never emits `action_started`. Finalization is job-scoped terminal work, not a fake DAG action.

The kernel is the single reducer for this state machine. Human output may coalesce replaceable
progress, but native NDJSON does not drop protocol events. The final `JobFinished` summary is a
pure reduction over the final non-superseded plan, finalization outcome, and root failures.

## 3. Command surface

### 3.1 Primary commands

| Command | User goal | Primary result |
| --- | --- | --- |
| `xmlsquish build` | Compile XML source into complete binary IR, link selected entries, and run the `squish` backend | Published `*.prompt` artifacts and build records |
| `xmlsquish fmt` | Format project-owned source without changing DSL semantics | Updated source files, or a check/diff result |
| `xmlsquish add SPEC` | Add or change a typed dependency requirement | Coherent manifest and lock state |
| `xmlsquish remove ALIAS` | Remove a direct dependency by its local alias | Coherent manifest and lock state |

### 3.2 Direct manager commands only

The production command set is exactly `build`, `fmt`, `add`, `remove`, and `inspect`. Bare
`xmlsquish` prints concise help to stdout and exits `0`; an unknown command exits `2` and is never
reinterpreted as a path. `-h`/`--help` and `-V`/`--version` do not load a project.

`inspect` is the only pure-query command. It uses `--format=human|json` and rejects the operational
`--message-format`. `fmt --diff` remains an operation: human/short modes place the requested
unified diff on stdout, while JSON mode carries diff artifacts in the typed operation result.

Earlier drafts listed `init`, `new`, `check`, `update`, `tree`, `metadata`, `clean`, `doctor`,
`config explain`, and `completions`. They are not commands in the production grammar and are not
release gates. Adding one requires a separate product decision, a typed protocol request, and
process-level acceptance evidence; this document does not reserve an unimplemented second
surface.

### 3.3 Consistent selection grammar

Selection uses named concepts, never positional path guessing:

```text
--manifest-path PATH       locate an explicit project/workspace manifest
-p, --package PACKAGE      select a package; repeatable
-t, --target TARGET        select a declared target; repeatable
--workspace                select all workspace packages allowed by the command
--exclude PACKAGE          subtract from --workspace; repeatable
--profile PROFILE          select a named build profile
```

When no package or target is specified, the manifest's declared defaults apply. If no default can
be chosen unambiguously, the diagnostic names the available selectors and gives a copyable
command. Selection is fully resolved before work begins.

Selectors are names, not filesystem paths. Commands that legitimately accept paths label their
type explicitly: `fmt --path PATH`, `add --path PATH`, `--manifest-path PATH`, and `inspect
artifact PATH` (where the typed `artifact` subject removes path/identity ambiguity).

### 3.4 Build

```text
xmlsquish build [selection]
    [-j, --jobs N]
    [--keep-going | --no-keep-going]
    [--emit prompt|ir|debug]...
    [--arg TARGET.NAME=VALUE]...
    [--locked] [--offline] [--frozen]
    [--message-format human|short|json]
```

- `-j 0` means manager-selected bounded concurrency; positive `N` is a hard upper bound on active
  jobs, not necessarily OS threads.
- Keep-going is enabled by default: failure blocks only dependent jobs while independent selected
  targets continue. `--no-keep-going` changes admission after the first failure.
- The normal pipeline is `XML source -> complete reusable binary IR -> link entry -> squish ->
  *.prompt`. “Complete” means IR is not pruned merely because the current entry does not use some
  source information. Source maps, provenance, and debug identities remain available through the
  IR/debug artifacts and build record.
- `--emit` is repeatable with exactly one enum value per occurrence. `prompt` publishes linked,
  squished end products; `ir` publishes reusable complete IR; `debug` publishes source-map and
  provenance companions for the selected outputs. Duplicate values are idempotent. With no flag,
  the manifest setting applies and otherwise defaults to `prompt`. `--emit=ir` alone never invokes
  the squish backend. `--emit=debug` alone is a usage error because it has no selected primary
  output to describe. Comma-separated values are rejected; repeat the flag instead.
- `--keep-going` runs jobs whose prerequisite closure remains valid. `--no-keep-going` stops
  admitting new jobs after the first failure while allowing already publishing jobs to finish
  safely.
- `--frozen` is the compositional shorthand for immutable lock state plus no network access. It
  does not disable reads from a verified content store.
- A successful build names every published `*.prompt`, cache reuse, and total elapsed time. A
  failed build names failed and blocked targets separately; it never reports them as one count.

Example interactive transcript:

```text
$ xmlsquish build -t support-agent
  Resolving  14 packages (locked)
  Loading    38 sources
  Compiling  support-core -> ir:sha256:8c36…
  Compiling  support-agent -> ir:sha256:07b1…
  Linking    support-agent
  Squishing  support-agent
  Published  target/prompts/support-agent.prompt
  Finished   1 target, 2 IR units, 1 cache hit in 1.84s
```

Status verbs have stable meanings. They are not log levels and are never abbreviated in plain
mode.

### 3.5 Format

```text
xmlsquish fmt [selection] [--path PATH]...
    [--check] [--diff]
    [--style-edition EDITION]
    [--message-format human|short|json]
```

- With neither `--check` nor `--diff`, `fmt` atomically rewrites files after a complete semantic
  preflight.
- `--check` writes nothing. Exit `1` means at least one valid selected file would change.
- `--diff` implies `--check`, writes unified diffs to stdout in human/short mode, and keeps
  diagnostics on stderr. In JSON mode, diff artifacts are named in the typed format result carried
  by `operation_completed`; no textual diff or invented event variant is mixed into the stream.
- A parse or semantic-preservation failure is not treated as a formatting difference. It produces
  a diagnostic and exit `1`.
- An unchanged run is one quiet success line, not one line per file. Changed files are named.
- When `--path` selects a file outside project ownership, the command explains the ownership
  boundary rather than formatting it opportunistically.

### 3.6 Add and remove

```text
xmlsquish add SPEC
    [-p PACKAGE]
    [--rename ALIAS]
    [--path PATH | --git URL [--rev REV | --tag TAG] | --registry NAME]
    [--features LIST] [--optional]
    [--locked] [--offline] [--frozen]
    [--dry-run]

xmlsquish remove ALIAS
    [-p PACKAGE]
    [--locked] [--offline] [--frozen]
    [--dry-run]
```

- The source kind is typed. Conflicting source flags are usage errors, not precedence rules.
- `SPEC` is never executed as a shell fragment and never inferred from a nearby directory.
- `add` resolves package identity before editing. If the selected package already has the alias,
  it performs an explicit, reported update rather than duplicating a TOML key.
- `remove` operates on the direct dependency alias. If references remain, it reports every
  source span and does not commit an incoherent project unless an explicitly designed rewrite
  operation can preserve semantics.
- `--dry-run` performs all reads, resolution, validation, and planning, prints the exact logical
  changes, writes nothing, and reports whether the real operation would succeed.
- A normal, unambiguous command does not ask “Are you sure?”. An ambiguous package choice is
  resolved with `-p`; non-interactive invocation never hangs awaiting input.
- Manifest, lock, and transaction journal behavior is one manager operation. Human output says
  `Resolved`, `Updated`, and `Committed` separately so a resolution failure cannot look like an
  edit failure.

Example:

```text
$ xmlsquish add prompt-common@^2 -p support --rename common
  Resolving  prompt-common ^2
  Adding     common -> prompt-common 2.4.1
  Locking    3 packages
  Committed  xmlsquish.toml, xmlsquish.lock
```

### 3.7 Inspect

`inspect` is the supported way to understand manager state and products. It is not a raw database
or Rust-structure dump:

```text
xmlsquish inspect ir IDENTIFIER [--format human|json]
xmlsquish inspect link IDENTIFIER [--format human|json]
xmlsquish inspect source IDENTIFIER [--format human|json]
xmlsquish inspect cache ACTION-KEY [--format human|json]
xmlsquish inspect artifact PATH [--format human|json]
```

The subject word gives `IDENTIFIER` its type, so a module digest cannot be silently interpreted as
a path or target name. `ir` reports schema/dialect, module identity, imports, exports, semantic and
provenance digests, and source identities. `link` reports resolved bindings and the selected entry.
`source` follows provenance metadata without printing source bodies. `cache` explains the declared
inputs and result digests for an action key. `artifact`
validates the portable container or published-product digest and follows the committed artifact
manifest: inspecting a prompt can find its `.psdbg` companion, and inspecting `.psdbg` identifies
the exact prompt digest it describes.

Human output is descriptive and may evolve. `--format=json` emits one versioned JSON document on
stdout, not NDJSON events, and includes a stable `kind` discriminator plus typed identities and
digests. A missing, corrupt, wrong-kind, or unsupported-version object is an operation failure
(exit `1`) with a diagnostic on stderr. Inspection never repairs, fetches, recompiles, updates
access-visible semantic state, or treats a stale file as a successful build. Recovery remains an
explicit manager lifecycle responsibility during ordinary startup/planning, not a side effect of
inspection.

### 3.8 Resolution-mode contract

Commands that may resolve packages (`build`, `add`, and `remove`) share one
mode algebra. `fmt` never fetches dependencies, and `inspect` observes an already named object, so
neither accepts resolution flags.

| Mode | Network | Lock mutation | Required behavior |
| --- | --- | --- | --- |
| default | Allowed | Allowed when intent requires it | Prefer still-valid locked versions and report every lock change |
| `--locked` | Allowed | Forbidden | Require a present lock current for the resolution-relevant manifest projection |
| `--offline` | Forbidden | Allowed | Resolve only from locally available index and content data; identify every unavailable item |
| `--frozen` | Forbidden | Forbidden | Exactly `--locked --offline`; verified local content-store reads remain allowed |

The flags compose by restriction. Supplying `--frozen` together with either implied flag is
idempotent, not a conflict. No mode makes workspace or path-source bytes immutable: each operation
still snapshots their current contents. `--offline` must be enforced at the network port, not by
waiting for a connection failure, and presentation/configuration lookup must not create a hidden
network path.

For `add` and `remove`, `--locked`/`--frozen` may succeed only when the requested operation is
already idempotently satisfied and therefore changes neither manifest intent nor exact lock state.
Otherwise they fail with exit `1` before committing either file. `--dry-run` uses the same mode and
would-succeed decision as the real operation; it merely removes the commit step.

## 4. Global interaction controls

The composed parser accepts the following controls before or after a command:

```text
-q, --quiet
-v, --verbose                 repeat up to -vv
--color auto|always|never
--progress auto|always|never
--plain
--message-format human|short|json
--config KEY=VALUE            repeatable TOML-typed override
```

Rules:

- `--quiet` removes progress, ordinary status, and successful summaries; diagnostics and requested
  stdout data remain.
- `-v` and `-vv` add typed lifecycle and cache detail. They do not switch to an unstructured log
  stream or expose registry credentials.
- `--quiet` and `--verbose` conflict before project loading.
- `--plain` selects color-free, repaint-free, append-only output. An explicit contradictory
  `--color=always` or `--progress=always` is a usage error, not an argv-order precedence rule.
- `--message-format` selects the operational renderer. `inspect` instead uses its own
  `--format=human|json` result format.
- Repeated `--config` values are applied in argv order and are parsed as typed TOML values.

Configuration precedence is:

```text
defaults < user config < workspace config < environment < CLI/--config
```

`NO_COLOR`, `TERM`, `CI`, and independently probed stdout/stderr terminal capabilities influence
only `auto` presentation decisions. They do not change resolution, action keys, artifacts, or exit
meaning. `--unicode`, `--hyperlinks`, `--no-input`, `--diagnostic-format`, `--trace-file`, and
`--lock-timeout` appeared in an earlier exploration but are not accepted options and are not
implicit release requirements. The current parser contract is executable in
`crates/squish-cli/src/lib.rs` and `crates/squish-cli/tests/cli_contract.rs`; layered precedence is
exercised in `crates/squish-config/tests/loading.rs` and composed in `src/main.rs`.

## 5. Stream contract

### 5.1 Human and short modes

| Content | Stream | Rationale |
| --- | --- | --- |
| Live status, progress, summaries | `stderr` | Artifact bytes are files; status must not contaminate pipelines |
| Diagnostics, warnings, recovery notices | `stderr` | They remain visible when stdout is redirected |
| `inspect` result | `stdout` | It is the requested pure-query value |
| `fmt --diff` unified diff | `stdout` | It is requested data in human/short mode |
| Help and version text | `stdout` | Successful direct result |

Human prose is not the parsing API. Command grammar, exit status, diagnostic codes, typed
identities, artifact digests, and the versioned JSON schemas are machine contracts. stdout and
stderr capability decisions are independent.

### 5.2 JSON operation events: native protocol v2

`--message-format=json` writes UTF-8 newline-delimited JSON (NDJSON), one native event object per
line on stdout. It writes no ANSI or carriage-return repaint sequences; after successful CLI,
project, configuration, and host bootstrap, stderr remains empty.

The native envelope is `squish_protocol::Event` at protocol version `2.0`:

```json
{"version":{"major":2,"minor":0},"invocation":"cli-123","sequence":0,"payload":{"type":"planning_started","data":{"job":"build-cli-123","attempt":"attempt-1"}}}
{"version":{"major":2,"minor":0},"invocation":"cli-123","sequence":1,"payload":{"type":"planning_step_started","data":{"job":"build-cli-123","attempt":"attempt-1","step":"locate","kind":"locate"}}}
{"version":{"major":2,"minor":0},"invocation":"cli-123","sequence":2,"payload":{"type":"planning_step_succeeded","data":{"job":"build-cli-123","attempt":"attempt-1","step":"locate","timing":{"elapsed_ms":0}}}}
```

Required envelope fields are:

| Field | Contract |
| --- | --- |
| `version` | Object `{major, minor}`; native emission currently uses `{2, 0}` |
| `invocation` | Stable identifier shared by all events in one dispatched invocation |
| `sequence` | Invocation-local, gapless observation order starting at zero |
| `payload.type` | Snake-case typed discriminator owned by `EventPayload` |
| `payload.data` | Variant-specific typed data; there is no flattened `reason` field |

The v2 vocabulary is the algebra in `crates/squish-protocol/src/lib.rs`: planning attempts and
steps, immutable plan declaration/closure, action lifecycle and cache hits, post-plan
finalization, diagnostics, one typed `operation_completed` result, and the terminal
`job_finished` summary. `job_finished` is the final native event while stdout remains writable;
its `sequence` equals the number of preceding events. Unknown additive `payload.type` values may
be skipped by consumers within a supported major. Major version 1 is accepted only by the
compatibility decoder as an inspection projection; native emission is v2.

The old flattened example using `schema_version`, `reason`, `operation-started`, and
`operation-finished` never describes the implemented protocol and is superseded. In particular,
there is no separate `artifact-published` event: artifacts are typed outputs of
`action_succeeded`, `cache_hit`, and the domain result carried by `operation_completed`.

Bootstrap failures are deliberately a separate, smaller contract because no kernel invocation ID
or lifecycle exists yet. When an exact `--message-format=json` was recognized, parse, discovery,
configuration, or host-composition failures emit two `xmlsquish-bootstrap-v1` records (`diagnostic`
then `finished`) on stdout. They must not pretend to be v2 lifecycle events.

Executable evidence is exact rather than aspirational:

| Contract | Executable evidence |
| --- | --- |
| v2 envelope, current version, typed discriminator, unknown-event policy | `crates/squish-protocol/src/lib.rs`: `CURRENT_VERSION`, `Event`, `EventPayload`, `unknown_additive_event_is_skipped`, `known_event_round_trips` |
| Legal sequencing and terminal reduction | `crates/squish-kernel/src/lib.rs` lifecycle tests, including `finalization_is_sequential_terminal_work_after_the_final_plan` |
| One canonical object and immediate flush per line | `crates/squish-presentation/src/lib.rs`: `NdjsonRenderer::render`, `ndjson_is_one_canonical_object_per_line` |
| Composed stdout-only operation stream | `tests/process.rs::json_build_is_canonical_ndjson_on_stdout` |
| Pre-kernel JSON exception | `src/main.rs::BootstrapRecord`, `tests/process.rs::json_parse_failures_are_diagnostic_plus_one_terminal_record`, `json_pre_dispatch_failures_are_diagnostic_plus_terminal_on_stdout` |

The renderer writes and flushes each event synchronously. There is no separate unbounded JSON
queue and therefore no independent “slow-consumer backpressure subsystem” to accept. A write
failure is handled as output loss at the root process boundary.

### 5.3 Diagnostic payload

A native structured diagnostic is a normal v2 payload:

```json
{
  "version": {"major": 2, "minor": 0},
  "invocation": "cli-123",
  "sequence": 8,
  "payload": {
    "type": "diagnostic",
    "data": {
      "id": "link-missing-export",
      "code": "LNK004",
      "severity": "error",
      "phase": "link",
      "message": "required export was not found",
      "primary": {"source": "project:app/src/agent.xml", "start": 184, "end": 207},
      "related": [],
      "help": "inspect the dependency exports"
    }
  }
}
```

`Diagnostic` currently carries an instance ID, stable code, severity, typed phase, optional
primary UTF-8 byte span, zero or more related labeled spans, and optional help. Line/column
projections and rich multi-edit suggestions from earlier sketches are not wire fields and cannot
be claimed by consumers. `crates/squish-protocol/src/lib.rs::Diagnostic` is authoritative; human
rendering remains free to derive readable presentation without changing the payload.

## 6. Terminal rendering and micro-interactions

### 6.1 Capability decisions

Color and dynamic progress are separate policies. In `auto`, color requires a terminal with ANSI
support and is disabled by non-empty `NO_COLOR` or `TERM=dumb`. Dynamic progress additionally
requires a known live width, dynamic-terminal support, normal/verbose human mode, and no `CI` or
`TERM=dumb`. `always` may override environment defaults but cannot manufacture a missing terminal
capability; `never`, `--plain`, JSON, quiet output, and query rendering disable repaint.

| Environment | Color in `auto` | Dynamic progress in `auto` | Output shape |
| --- | --- | --- | --- |
| Capable interactive terminal | Yes | Yes when width is known | Bounded live region plus durable lines |
| Redirected/piped stderr | No | No | Append-only lines |
| `TERM=dumb` | No | No | Append-only lines |
| `CI` | Capability-dependent | No | Append-only lines |
| `--plain` | No | No | Accessible append-only lines |
| JSON operation stream | No | No | NDJSON on stdout |

The root probes stdout and stderr independently. Human operation rendering owns stderr; an
`inspect` result or human diff on stdout cannot inherit stderr's style decision.

### 6.2 Live-view behavior

- Auto progress waits 500 ms before first display; visible progress is refreshed no faster than
  once per 100 ms. `--progress=always` removes the initial delay but retains the refresh bound.
- Progress derives from declared action counts and active stable action IDs. It does not invent a
  spinner or expose thread IDs.
- The current terminal width is re-probed on ticks and clear operations. Text is sanitized,
  truncated only at grapheme boundaries, and kept within the reported cell width.
- A durable diagnostic or terminal event clears the dynamic region before printing. Completion,
  failure, cancellation, broken output, and the second-interrupt emergency path restore terminal
  styling/line state.
- Non-TTY and plain output are chronological and append-only. Semantic words and diagnostic codes
  carry meaning; color is redundant.

These are executable properties in `crates/squish-presentation/src/lib.rs` (the delayed-progress,
resize, sanitizer, color-policy, and terminal-event tests) and `tests/pty.rs` (real PTY/ConPTY
resize plus first/second interrupt process tests). There is no separate hyperlink protocol or
user-selectable Unicode mode in the production renderer.

## 7. CI and GitHub Actions

`.github/workflows/ci.yml` is the only cross-platform release workflow for this contract. Its
Ubuntu, Windows, and macOS jobs build and test the workspace and run the composed manager smoke;
quality jobs also enforce formatting, the declared Rust 1.88 minimum, strict Clippy, and the site
build. `.github/scripts/ci_smoke.py` checks the five-command help surface, machine-readable usage
failure, human unified diff, default `.prompt`, explicit `.xsir`, and JSON inspection.

CI receives append-only output and may archive NDJSON as ordinary job evidence. The program does
not emit GitHub workflow commands or annotations, and `CI` affects only automatic progress
repainting. A workflow definition is not proof that supported desktops pass: release acceptance
requires a completed run for the committed revision, recorded in
`docs/design/project-manager-execution-status.md`.

## 8. Errors, cancellation, and recovery

### 8.1 Diagnostics

Expected failures identify the stable diagnostic code and, when known, the affected operation,
package/target/source, decisive span, commit outcome, and a non-destructive recovery action. Human
formatting is not a second diagnostic model: it renders the same `Diagnostic` facts described in
Section 5.3. Internal composition failures remain distinguishable by exit `101`; verbosity adds
lifecycle detail on the selected renderer rather than a hidden stderr channel in JSON mode.

### 8.2 Cancellation

- First Ctrl-C requests cooperative cancellation and stops new admission. Active work observes the
  shared cancellation token at supported boundaries; durable publication/repository decisions are
  completed or recovered rather than left ambiguous.
- On a dynamic interactive terminal, publication of that state clears transient progress and emits
  exactly `Cancelling; Ctrl-C again to force` on `stderr`. Later progress remains suppressed, making
  the notice both actionable UI and a stable ordering boundary. Quiet, short, plain, non-TTY, and
  NDJSON modes remain unchanged.
- A second Ctrl-C performs the bounded emergency terminal reset and exits `130` immediately.
- The native JSON lifecycle closes with typed cancellation/action facts and terminal
  `job_finished { status: "cancelled" }` while stdout remains writable.
- Cancellation never deletes the previous committed generation. Manager-owned temporary state is
  reconciled by the next ordinary invocation.

`src/interrupt.rs`, `tests/pty.rs`, the kernel cancellation tests, and
`tests/recovery_process.rs` are the executable evidence.

### 8.3 Concurrent operations

Manifest/lock edits use the repository transaction port and bounded automatic revision retry.
Build publication and catalog finalization use separate durability ports. A change between plan
and commit either produces a new immutable plan when intent remains valid or reports a coherent
conflict; workers cannot silently overwrite another accepted revision. The production CLI has no
`--lock-timeout` option. Contention/replan behavior is covered in
`crates/squish-manager/tests/mutation.rs` and the repository transaction tests.

### 8.4 Atomic publication and startup recovery

- A target generation stages complete prompt/debug/manifest bytes and advances one generation
  pointer only after its durable decision. A failed target preserves the previous committed
  generation and cannot report it as newly produced.
- `fmt` performs complete preflight before replacement and records each file's truthful outcome.
- Manifest and lock candidates are journaled and recovered as one logical transaction.
- CAS objects are immutable and verified by digest before exposure; build catalogs are namespaced
  by project identity.
- Recovery is normal manager planning work plus typed diagnostics. There is no `doctor` command or
  obsolete `recovery-performed` event required to make recovery real.

Real child-process termination before and after each supported commit decision is exercised by
`tests/recovery_process.rs`; component transaction tests cover the underlying ports.

### 8.5 Broken output

A downstream close during the requested `inspect` result or human `fmt --diff` terminates quietly
without an error-shaped panic. Losing the operational NDJSON sink is an output failure at the root
and requests cancellation where execution is still active. `tests/process.rs` covers closed diff
and inspect pipes; `src/main.rs` owns the renderer-failure reduction. Help/version use the same
quiet direct-output helper rather than recursing through diagnostics.

## 9. Exit-status contract

Keep the public set small and cross-platform:

| Code | Meaning | Examples |
| ---: | --- | --- |
| `0` | Requested operation succeeded | clean build, successful edit, no `fmt --check` differences |
| `1` | Valid invocation completed unsuccessfully | compile/link error, resolution failure, I/O failure, format differences, JSON sink lost |
| `2` | Invocation could not be formed | unknown command/flag, conflicting flags, malformed selector or argument |
| `101` | XMLSquish internal invariant failure | panic boundary or corrupted internal state not attributable to project input |
| `130` | User cancellation | Ctrl-C during planning, execution, or publication |

Manifest schema and project configuration errors are domain failures (`1`), not CLI syntax errors
(`2`). Warnings do not independently change the exit code. Multiple job failures still reduce to
`1`; their categories remain in the event/diagnostic data rather than multiplying process codes.

Exit-code priority is:

```text
cancelled (130)
> internal invariant failure (101)
> malformed invocation before execution (2)
> operation failure (1)
> success (0)
```

Once domain execution begins, later domain errors cannot become code `2`.

## 10. Scheduling as product behavior

Scheduling policy is internal, but its observable promises are public:

1. A frozen `OperationPlan` assigns stable job IDs and prerequisite edges before execution.
2. Ready jobs are admitted through a bounded queue. `--jobs` bounds active jobs; it does not alter
   results, artifact bytes, diagnostic identity, or final ordering.
3. Worker completion is communicated as typed data. Workers cannot print, repaint, or choose an
   exit code.
4. Human live output follows meaningful completion time. The durable final summary and JSON final
   job table use stable `(workspace package, target, phase, source)` ordering.
5. Failure blocks only transitive dependents. Independent work follows the selected keep-going
   policy.
6. Diagnostic groups are never interleaved at the character or source-excerpt level.
7. Cache hits remain explicit `cache_hit` payloads with action key, digest, and complete output
   metadata. They do not emit `action_started`, because no executor began.
8. Re-running with `--jobs=1` is the supported diagnostic simplification and must require no
   separate “safe mode.”

The deeper build-system design should keep scheduling (when a job may run) separate from
rebuilding (whether its declared result is already valid), following the orthogonal model in
Mokhov, Mitchell, and Peyton Jones,
[“Build Systems à la Carte”](https://doi.org/10.1017/S0956796820000088). The CLI therefore reports
`waiting`, `running`, `cached`, and `blocked` as distinct states.

## 11. Help, discoverability, and writing style

Every command help page contains, in this order:

1. one-sentence purpose;
2. usage line;
3. two dominant examples;
4. selectors and command options;
5. output/exit behavior;
6. related commands.

Help uses the user's vocabulary (“build a prompt”, “add a dependency”) before implementation
vocabulary (“lower an IR unit”). Advanced concepts link back to inspection commands:

```text
Run `xmlsquish inspect ir --help` to inspect reusable IR.
Use repeated `--config KEY=VALUE` overrides for one-invocation configuration changes.
```

Errors never dump the complete top-level help. They show the invalid fragment, a short correction,
and `For more information, try '--help'.` Examples are copyable in PowerShell, POSIX shells, and
`cmd.exe` unless explicitly labeled for one shell. Paths with whitespace are quoted.

Command and option names remain English and stable. Human messages are localization-ready but
diagnostic codes, JSON keys/discriminators, manifest keys, and status verbs in `short` mode are not
localized. Machine interfaces never contain a localized value where an enum is expected.

## 12. Acceptance evidence

This section replaces the unchecked aspirational K3/K4 checklist that preceded the manager
cutover. A checkbox without a named executable witness is not evidence. The production release
contract is now the five-command scope in `docs/product/project-manager-scope.md`, ADR 0009, the
current protocol types, and the tests below.

Status terms are intentionally narrow:

- **Implemented and exercised** means the production path and a focused executable regression both
  exist in the repository.
- **Workflow gate** means acceptance comes from a completed GitHub Actions run of the committed
  workflow, not from the YAML file alone; the current run evidence is recorded in
  `docs/design/project-manager-execution-status.md`.
- **Superseded** means an earlier exploration was never added to the production grammar/protocol
  and is not an open mandatory gate.

### 12.1 Current product matrix

| Product contract | Status | Authoritative executable evidence |
| --- | --- | --- |
| Bare help/version, unknown-command rejection, five typed commands | **Implemented and exercised** | `crates/squish-cli/tests/cli_contract.rs`; `tests/process.rs::{version_reports_the_installed_root_package_version,parse_failure_is_stderr_with_usage_exit_status}` |
| One CLI -> kernel -> manager route; domains do not render | **Implemented and exercised** | `src/main.rs`, `crates/squish-kernel/src/lib.rs`, `crates/squish-manager/src/lib.rs`; kernel lifecycle tests and root process suite |
| Human/short stream separation, quiet and append-only non-TTY output | **Implemented and exercised** | `tests/process.rs::{format_check_and_write_preserve_stream_contract,quiet_is_silent_and_non_tty_human_output_is_linear}`; presentation renderer tests |
| Native v2 NDJSON envelope and stdout-only operation stream | **Implemented and exercised** | Section 5.2 evidence table; `Event { version, invocation, sequence, payload }` and `payload.type` are the schema |
| Typed bootstrap failure stream before kernel dispatch | **Implemented and exercised** | `src/main.rs::BootstrapRecord`; the two JSON bootstrap process tests named in Section 5.2 |
| Default keep-going, dependency blocking, cancellation and truthful terminal reduction | **Implemented and exercised** | `crates/squish-build/src/tests.rs`; `crates/squish-kernel/src/lib.rs` lifecycle tests |
| XML -> canonical `.xsir` -> link/instantiate -> squish -> `.prompt`, with `.psdbg` provenance | **Implemented and exercised** | `crates/squish-manager/tests/build.rs::{complete_build_publishes_prompt_debug_and_ir_from_cas,semantic_example_publishes_fully_traceable_debug_bundle}`; `tests/process.rs::build_warms_cache_and_publishes_all_selected_artifact_kinds` |
| Semantic formatter, no-write check, human unified diff and JSON diff artifact | **Implemented and exercised** | `crates/squish-format/tests/semantic_oracle.rs`; `crates/squish-manager/tests/fmt.rs`; `tests/process.rs::{format_check_and_write_preserve_stream_contract,fmt_diff_is_unified_stdout_for_humans_and_artifact_based_json}` |
| Typed path/Git/registry/workspace add/remove with coherent manifest/lock transaction | **Implemented and exercised** | `crates/squish-manager/tests/mutation.rs`, resolver/fetch suites, and `tests/process.rs::add_dry_run_then_add_and_remove_have_truthful_file_effects` |
| Typed `inspect ir|link|source|cache|artifact`, single JSON document and quiet broken pipe | **Implemented and exercised** | `crates/squish-manager/tests/inspect.rs`; `tests/process.rs::{inspect_json_is_one_stdout_document_after_build,inspect_ir_link_source_and_cache_have_typed_human_and_single_json_views}` |
| TTY width/resize, bounded progress, two-stage interruption and restoration | **Implemented and exercised** | `crates/squish-presentation/src/lib.rs` progress/resize tests; `tests/pty.rs` real PTY/ConPTY process tests |
| Repository, artifact-generation, and build-catalog process-death recovery | **Implemented and exercised** | `tests/recovery_process.rs`; `docs/design/process-recovery-testing.md` |
| Ubuntu, Windows, and macOS acceptance of the same composed binary | **Workflow gate** | `.github/workflows/ci.yml`; only a completed run recorded in the execution-status ledger satisfies this row |

These rows preserve the required observable behavior without asserting that every old test sketch
was implemented verbatim. The repository's focused tests may reorganize; when they do, this table
must be updated to another exact executable witness rather than reverted to anonymous boxes.

### 12.2 Explicitly superseded acceptance sketches

The following items from the pre-cutover checklist are not unresolved defects and do not block the
current production scope:

| Superseded sketch | Current decision |
| --- | --- |
| GitHub workflow-command/annotation renderer | Not in the CLI. CI consumes ordinary plain/NDJSON output; environment detection must not create a second protocol. |
| A separate bounded queue for slow JSON consumers | `NdjsonRenderer` writes and flushes synchronously. There is no unbounded internal JSON queue; root output-loss behavior is the relevant contract. |
| `metadata`, `tree`, `completions`, `doctor`, `config explain`, `init/new`, `check`, `update`, `clean` | Not production commands. The current pure query is typed `inspect`; automatic startup recovery replaces deletion-oriented repair instructions. |
| `--unicode`, `--hyperlinks`, `--no-input`, `--diagnostic-format`, `--trace-file`, `--lock-timeout`, `--deny-warnings` | Not accepted flags. Color, progress, plain mode, verbosity, message format, and typed `--config` are the shipped controls. |
| Rich multi-edit suggestion objects and stored display line/column fields | Not fields of the v2 `Diagnostic` wire type. Current spans are typed source identities plus UTF-8 byte ranges; human projection is presentation-only. |
| Full-stream snapshot equality under concurrent completion | Rejected because it would encode scheduler timing. Per-invocation sequence and legal per-plan/action partial order are required instead. |

These removals narrow neither ADR 0009 nor the user-visible five-command product. They remove
unimplemented inventions that had become accidental “mandatory” gates. Reintroducing any item is
a new product change requiring protocol/CLI ownership and executable acceptance evidence, not a
box to tick in this historical document.

## 13. Non-goals and rejected patterns

| Rejected pattern | Reason |
| --- | --- |
| `xmlsquish PATH` as an implicit build | Conflicts with a command-first manager and hides project/target selection |
| Filesystem-existence command dispatch | Identical argv would mean different things on different machines |
| One renderer embedded in each command | Duplicates TTY/CI behavior and lets domains create incompatible APIs |
| Spinner/progress from worker threads | Interleaves output and makes scheduling nondeterminism user-visible |
| Colorized JSON or a `rendered`-only diagnostic | Machines need typed fields; accessibility cannot depend on ANSI prose |
| Automatic prompts for ordinary edits | Breaks scripts and replaces precise intent with a modal interaction |
| Automatic GitHub workflow commands | Environment detection must not silently change the output protocol |
| One exit code per diagnostic category | Commands already identify context; many codes are brittle for scripts |
| Deleting caches/locks as standard recovery | Discards evidence and pushes manager consistency bugs onto users |
| Reporting an old artifact as success after a failed rebuild | File existence is not build validity |
| Treating complete IR as a user-facing XML intermediate | IR is machine-oriented, cacheable, inspectable data; the linked product is `*.prompt` |

## 14. Decision summary

The product contract is one manager executable with typed commands, a shared operation state
machine, structured events, and replaceable renderers. Interactive use receives restrained color,
clear phase verbs, bounded progress, source-rich diagnostics, and copyable recovery guidance.
Pipes and CI receive append-only output; tools receive a versioned NDJSON stream. Accessibility is
a first-class rendering mode rather than a side effect of disabling color.

Most importantly, the CLI teaches the actual architecture:

```text
project sources -> complete reusable IR -> link an entry -> squish backend -> *.prompt
```

That vocabulary lets future frontends, backends, caches, databases, and inspectors join the same
manager without turning `xmlsquish` into a bag of special cases.
