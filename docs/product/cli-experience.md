# Product CLI and Terminal Experience

- Status: Product specification
- Date: 2026-09-14
- Scope: the `xmlsquish` executable, its command grammar, observable terminal behavior,
  automation contract, and recovery experience
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

Every mutating or work-producing command follows the same state machine:

```text
accepted -> locating -> planning -> waiting -> running -> publishing -> finished
    |          |          |          |          |            |
    +----------+----------+----------+----------+-----> failed
    +----------+----------+----------+----------+-----> cancelled
```

Jobs inside an operation use:

```text
planned -> ready -> running -> {succeeded | failed | cancelled}
planned -> {blocked | cancelled}
ready   -> {blocked | cancelled | cached}
```

`blocked` means a job could not run because a prerequisite job failed. It never means “the
manager gave up handling a recoverable filesystem state.” A blocked or cache-hit job does not emit
`job-started`; that event means its executor actually began. Each transition is valid exactly once.
An operation can fail or be cancelled before any job exists (for example during project location
or planning); its final counts are then all zero and its operation-level diagnostics carry the
cause. The final summary is a pure reduction over terminal job states and commit outcomes.

This model gives the scheduler and all renderers the same source of truth:

```text
Result = fold(initial_operation_state, ordered_contract_events)
```

Human output may coalesce replaceable progress events. JSON output may not drop contract events.
Diagnostics, state transitions, recovery actions, artifact publications, and the final result are
never replaceable.

## 3. Command surface

### 3.1 Primary commands

| Command | User goal | Primary result |
| --- | --- | --- |
| `xmlsquish build` | Compile XML source into complete binary IR, link selected entries, and run the `squish` backend | Published `*.prompt` artifacts and build records |
| `xmlsquish fmt` | Format project-owned source without changing DSL semantics | Updated source files, or a check/diff result |
| `xmlsquish add SPEC` | Add or change a typed dependency requirement | Coherent manifest and lock state |
| `xmlsquish remove ALIAS` | Remove a direct dependency by its local alias | Coherent manifest and lock state |

### 3.2 Supporting manager commands

These are part of the coherent product surface, not compiler flags disguised as commands:

| Command | Purpose |
| --- | --- |
| `init [PATH]` / `new PATH` | Create a project in an existing/new directory without an interactive wizard |
| `check` | Run parsing, semantic analysis, IR construction, and linking validation without publishing `*.prompt` |
| `update [SPEC...]` | Re-resolve selected dependency requirements under explicit lock policy |
| `tree` | Explain the resolved package, source, target, or action graph |
| `metadata` | Emit a versioned project/workspace description for tools |
| `inspect` | Inspect IR, linkage, source provenance, cache identity, or a built artifact |
| `clean` | Remove derived project state selected by kind; source and manifests are never candidates |
| `doctor` | Diagnose and, when unambiguous, repair manager-owned state such as an interrupted manifest/lock transaction |
| `config explain KEY` | Show the effective value and every contributing configuration layer |
| `completions SHELL` | Generate shell completions on stdout |
| `help [COMMAND]` | Show built-in documentation without requiring `man` |

Bare `xmlsquish` prints concise help to stdout and exits `0`. An unknown command exits `2`, prints
the closest suggestions, and never executes a path or similarly named command. `-h/--help` and
`-V/--version` work without loading a project.

Pure query commands use a result format rather than the operational event protocol:

```text
xmlsquish metadata --format json          # one versioned JSON document
xmlsquish tree --format human|json
xmlsquish inspect ... --format human|json
xmlsquish completions SHELL               # a raw shell program
```

They reject `--message-format`; otherwise `metadata --format=json` or a completion script would
be indistinguishable from a stream of manager events. Diagnostics for a failed pure query remain
on stderr. `fmt --diff` is not a pure query because it runs a formatter plan and therefore follows
the operational message-format rules below. There is one bootstrap exception: if a pure query is
invoked with an exact `--message-format=json`, the manager has already promised the event protocol,
so it emits a JSON usage diagnostic and `operation-finished` on stdout while rejecting the option.

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
type explicitly: `fmt --path PATH`, `init PATH`, `add --path PATH`, `--manifest-path PATH`, and
`inspect artifact PATH` (where the typed `artifact` subject removes path/identity ambiguity).

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
  diagnostics on stderr. In JSON mode, diffs are structured `format-difference` events rather
  than mixed text.
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
`source` follows provenance without printing source bodies unless the user explicitly requests
them. `cache` explains the declared inputs and result digests for an action key. `artifact`
validates the portable container or published-product digest and follows the committed artifact
manifest: inspecting a prompt can find its `.psdbg` companion, and inspecting `.psdbg` identifies
the exact prompt digest it describes.

Human output is descriptive and may evolve. `--format=json` emits one versioned JSON document on
stdout, not NDJSON events, and includes a stable `kind` discriminator plus typed identities and
digests. A missing, corrupt, wrong-kind, or unsupported-version object is an operation failure
(exit `1`) with a diagnostic on stderr. Inspection never repairs, fetches, recompiles, updates
access-visible semantic state, or treats a stale file as a successful build. Recovery and repair
remain explicit manager lifecycle/`doctor` responsibilities.

### 3.8 Resolution-mode contract

Commands that may resolve packages (`build`, `check`, `add`, `remove`, and `update`) share one
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

The following options are accepted before or after the command name and have the same semantics
for every command that produces operational messages:

```text
-q, --quiet
-v, --verbose                 repeat up to -vv
--color auto|always|never
--progress auto|always|never
--unicode auto|always|never
--hyperlinks auto|always|never
--plain
--no-input
--message-format human|short|json
--diagnostic-format terminal|github
```

Rules:

- `--quiet` removes progress, normal status, and success summaries. It never removes diagnostics
  or requested stdout data.
- `-v` adds cache/scheduling decisions and resolved identities. `-vv` adds manager mechanism
  details suitable for a bug report. Neither writes secrets, source bodies, or complete argument
  values by default.
- `--quiet` and `--verbose` conflict; the parser reports the conflict before project loading.
- `--plain` is an accessibility/portability bundle equivalent to `--color=never
  --progress=never --unicode=never --hyperlinks=never`, but it does not imply `--quiet` and does
  not shorten diagnostics. Combining it with an explicit contradictory presentation value such
  as `--color=always` is a usage error instead of an argv-order precedence trick.
- `--no-input` guarantees that stdin is never read. JSON mode implies `--no-input`.
- `--diagnostic-format=github` emits explicitly requested GitHub Actions annotations in addition
  to plain linear operational output. It is not enabled merely because an environment variable
  exists. It conflicts with `--message-format=json`; JSON remains the richer integration contract.
- `--message-format` applies to operation commands. Pure query commands use their documented
  `--format`; help, version, and completions always produce their direct result.

Configuration precedence, from strongest to weakest, is:

```text
CLI flag
> XMLSQUISH_* environment override
> project/workspace presentation configuration
> user presentation configuration
> capability-derived default
```

`NO_COLOR` is a capability-derived default: any non-empty value disables color unless an
explicit XMLSquish configuration or CLI flag enables it. This follows the `NO_COLOR` convention,
which explicitly allows application configuration and command-line flags to override the
environment hint ([NO_COLOR](https://no-color.org/)). `TERM=dumb` disables color, Unicode
decoration, hyperlinks, and animated progress in `auto` mode.

Semantic build configuration and presentation configuration are distinct. Changing color or
progress never changes an action/cache key. `config explain term.color` shows, for example:

```text
term.color = never
  CLI                     (unset)
  XMLSQUISH_COLOR         (unset)
  project configuration  (unset)
  user configuration     (unset)
  NO_COLOR                set -> never   [effective]
```

## 5. Stream contract

### 5.1 Human and short modes

| Content | Stream | Rationale |
| --- | --- | --- |
| Live status, progress, summaries | `stderr` | Build artifacts are files; status must not contaminate pipelines |
| Diagnostics, warnings, recovery notices | `stderr` | They remain visible when stdout is redirected |
| `metadata`, `tree`, `inspect`, completions | `stdout` | These are the requested result |
| `fmt --diff` unified diff | `stdout` | It is a composable requested result |
| Help and version text | `stdout` | Successful query result |

This follows the general CLI rule that requested/machine-readable results use stdout while
operational messaging uses stderr ([Command Line Interface Guidelines](https://clig.dev/)). The
streams are capability-detected independently. Redirected stdout does not disable color on an
interactive stderr, and vice versa.

Human prose is not a stable parsing API. The following are stable: command grammar, exit status,
diagnostic codes, artifact paths declared in machine events, and the versioned JSON schema.

### 5.2 JSON mode

`--message-format=json` writes UTF-8 newline-delimited JSON (NDJSON), exactly one object per line,
to stdout. It writes no ANSI escapes, carriage-return repaint sequences, terminal hyperlinks, or
human summaries. Except for an unrecoverable failure to write stdout itself, stderr remains empty.

The manager recognizes an exact `--message-format=json` during bootstrap parsing. Therefore even
an invalid command or later option error is returned as a `diagnostic` event followed by
`operation-finished`. If the value itself is malformed, the fallback is a human usage diagnostic
on stderr because no valid machine protocol was selected.

Envelope version 1:

```json
{"schema_version":1,"sequence":0,"reason":"operation-started","command":"build"}
{"schema_version":1,"sequence":1,"reason":"job-started","job_id":"support/support-agent:compile","phase":"compile","package":"support","target":"support-agent"}
{"schema_version":1,"sequence":2,"reason":"artifact-published","job_id":"support/support-agent:publish","kind":"prompt","path":"target/prompts/support-agent.prompt","digest":{"algorithm":"sha256","value":"…"}}
{"schema_version":1,"sequence":3,"reason":"operation-finished","command":"build","success":true,"exit_code":0,"counts":{"succeeded":1,"failed":0,"blocked":0,"cached":1}}
```

Required envelope fields:

| Field | Contract |
| --- | --- |
| `schema_version` | Positive integer; consumers request/support a major schema version |
| `sequence` | Invocation-local, monotonically increasing integer with no gaps |
| `reason` | Stable discriminator; consumers must ignore unknown reasons they do not need |
| `command` | Present on operation events; canonical command name |
| `job_id` | Present on job events; stable within the operation plan |

Core version-1 reasons are:

```text
operation-started
project-resolved
plan-ready
job-started
job-progress
job-cached
diagnostic
recovery-performed
artifact-published
manifest-change
format-difference
job-finished
operation-finished
trace
```

`operation-finished` is emitted exactly once when the output stream remains writable. It is the
authoritative aggregate and contains the process exit code. Concurrent jobs may produce lifecycle
events in real completion order; `sequence` preserves that observation order. The final event's
`jobs` array, when requested with `-v`, is sorted by stable job key. Consumers must not infer
dependency order from adjacent events.

Progress events are rate-limited and optional; absence never means stalled work. All other core
events are lossless. The event sink applies bounded backpressure rather than allocating without
limit. Human-only repaint ticks never enter the contract event queue.

In JSON mode, `-v`/`-vv` selects additional typed fields and `trace` events; it never causes raw
logs or backtraces on stderr. A complete developer trace or backtrace requires the explicit
`--trace-file PATH` option, whose path is acknowledged by an event. This preserves JSON's
stdout-only protocol even for internal failures.

JSON paths use workspace-relative forward-slash logical paths when possible and accompany an
external path with a `file_uri`. They never depend on localized display strings. Durations are
integer milliseconds; byte sizes are integers; hashes always name their algorithm. A source span
uses UTF-8 byte offsets plus one-based display line and Unicode-scalar column values, all labeled
explicitly.

### 5.3 Diagnostic object

```json
{
  "schema_version": 1,
  "sequence": 8,
  "reason": "diagnostic",
  "diagnostic": {
    "code": "XSQ-LINK-0042",
    "severity": "error",
    "message": "export 'main' was not found in dependency 'common'",
    "spans": [{
      "path": "src/agent.xml",
      "byte_start": 184,
      "byte_end": 207,
      "line_start": 9,
      "column_start": 18,
      "line_end": 9,
      "column_end": 41,
      "primary": true,
      "label": "unknown export"
    }],
    "notes": ["dependency 'common' exports: base, chat"],
    "suggestions": [{
      "message": "use the exported chat entry",
      "applicability": "maybe-correct",
      "edits": [{
        "path": "src/agent.xml",
        "byte_start": 184,
        "byte_end": 207,
        "replacement": "pkg:common/chat"
      }]
    }]
  }
}
```

Diagnostic codes are stable identifiers documented by `xmlsquish explain XSQ-LINK-0042` (or
`inspect diagnostic XSQ-LINK-0042` if `explain` is retained under inspection). Suggestions declare
applicability rather than pretending every repair is safe. An edit always carries its own path and
UTF-8 byte range; suggestions may contain several edits, including edits in multiple files, and
consumers apply them as one suggestion or not at all. Rust's diagnostic system provides a
useful precedent for primary spans, notes/help, and explicit suggestion applicability
([Rust compiler diagnostic structures](https://rustc-dev-guide.rust-lang.org/diagnostics/diagnostic-structs.html)).

## 6. Terminal rendering and micro-interactions

### 6.1 Capability matrix

`auto` behavior is evaluated per output stream:

| Environment | Color | Unicode | Hyperlinks | Repainting progress | Output shape |
| --- | --- | --- | --- | --- | --- |
| Interactive terminal | Capability detected | Capability detected | Capability detected | Yes | Compact live view + final lines |
| Redirected/piped stream | No | Conservative | No | No | Append-only lines |
| `TERM=dumb` | No | No decoration | No | No | ASCII append-only lines |
| `CI=true` | Capability detected | Conservative | No by default | No | Append-only grouped phases |
| `--plain` | No | ASCII | No | No | Screen-reader-friendly append-only lines |
| JSON | No | JSON Unicode escapes allowed | No | No | NDJSON only |

GitHub Actions always sets `CI=true`, so it can be detected without relying on runner naming
([GitHub Actions variables](https://docs.github.com/en/actions/reference/workflows-and-actions/variables)).
TTY remains the primary capability test; CI suppresses repainting even if a pseudo-terminal is
allocated.

### 6.2 Live-view behavior

- Auto progress waits approximately 500 ms before its first display so fast operations do not
  flash. Once visible, it is rendered on stderr at no more than 10 frames per second (a 100 ms
  interval). Phase transitions request a refresh but are coalesced until the next allowed frame;
  durable diagnostics are printed immediately after clearing the live region and do not count as
  a progress frame. Explicit `--progress=always` may show immediately but retains the refresh
  limit. Cargo uses the same first-display delay and refresh
  interval to avoid flicker and excessive terminal work
  ([Cargo progress source](https://doc.rust-lang.org/stable/nightly-rustc/src/cargo/util/progress.rs.html#449-462)).
- The live region is bounded by terminal height. It shows overall completed/total jobs, active
  jobs, cache hits, and elapsed time. Additional active jobs collapse into `+N more` rather than
  scrolling.
- Terminal resize is handled on the next tick. Text is truncated at grapheme boundaries, and the
  stable target suffix remains visible.
- Determinate work uses a bar or `completed/total`; indeterminate work uses one spinner next to a
  meaningful verb. No nested spinner forest is allowed.
- A completed phase becomes one durable line. Repaint control sequences are never written after
  a newline-only renderer has been selected.
- Before any durable diagnostic or notice is printed, the renderer clears the complete dynamic
  region, prints the diagnostic atomically, then redraws current progress. Source excerpts can
  therefore never be overwritten by a subsequent progress tick.
- A terminal guard restores cursor visibility and styling on success, error, panic, and first
  cancellation. The program never uses blinking text or terminal bell by default.
- Terminal emulator taskbar/title integration, if later supported, is opt-in and presentation-only.
- Active jobs are selected and labeled by stable action ID, not thread ID or whichever worker most
  recently wrote a message. This keeps the display understandable across scheduler implementations.

### 6.3 Color and hierarchy

The default palette uses standard terminal roles, not hard-coded RGB values:

| Role | Typical styling | Always-present non-color cue |
| --- | --- | --- |
| Success | green/bold | `Finished`, `Published`, or `ok` |
| Error | red/bold | `error[CODE]` |
| Warning | yellow/bold | `warning[CODE]` |
| Note/help | cyan | `note:` / `help:` |
| Cached/skipped | dim | `Cached` / `Skipped` |
| Paths/identifiers | underline or bold | quoting and indentation |

Red versus green is never the only distinction. Background colors are avoided. Styled spans are
short, and reset codes close each span so redirected fragments cannot inherit styling.

Hyperlinks use OSC 8 only in `auto` when the terminal is known to support them. The visible text
is always the usable relative path; an unsupported terminal loses only clickability. A hyperlink
never hides an HTTP URL that is itself the requested output.

### 6.4 Accessible linear rendering

- `--plain` uses ASCII markers and one semantic event per line. It emits no spinner frames,
  box-drawing characters, cursor movement, hyperlinks, or reliance on column alignment.
- Tables have a list fallback when the terminal is narrow or plain mode is active.
- Symbols such as checkmarks are decorative and accompanied by words; they are suppressed in
  plain mode.
- Diagnostics read naturally top-to-bottom: headline, location, excerpt, labels, notes, then a
  copyable help command. Repeated file context is reduced visually but never omitted in plain
  mode.
- Dynamic counts are announced only on meaningful transitions, not every rendering tick. This
  prevents screen readers from receiving an unusable stream of animation updates.
- Width calculations use terminal cell width for display but never alter source columns in the
  diagnostic data model.
- All control characters originating in filenames, arguments, source text, or remote metadata are
  escaped before human rendering.

## 7. CI and GitHub Actions

The repository's cross-platform GitHub Actions jobs should invoke the same binary contract on
Linux, macOS, and Windows:

```yaml
- name: Check formatting
  run: xmlsquish fmt --check --message-format=short --plain

- name: Build prompts
  run: xmlsquish build --workspace --frozen --message-format=json > xmlsquish-events.ndjson
```

Recommended automation behavior:

1. Non-TTY output is append-only and never contains spinner history.
2. `--frozen` makes lock/network policy explicit rather than using CI detection to change
   resolution semantics.
3. NDJSON is saved as an artifact when a failure needs full machine-readable evidence.
4. `--diagnostic-format=github` is an explicit convenience for annotations. The renderer escapes
   GitHub workflow-command delimiters and line breaks according to GitHub's workflow-command
   protocol; it never passes pre-rendered source text through as a raw command. GitHub documents
   `notice`, `warning`, and `error` annotations with file/line metadata
   ([workflow commands](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-commands)).
5. An annotation is a projection of the diagnostic event, not a separate compiler code path.
6. No command silently changes color, resolution, warnings, or failure policy merely because
   `CI=true`; only presentation `auto` choices change.

The Actions matrix must test the product behavior, not merely compilation: help, project
discovery, build and cache reuse, `*.prompt` publication, formatter checks, add/remove dry runs,
plain logs, JSON parsing, and exit statuses on all three operating-system families.

## 8. Errors, cancellation, and recovery

### 8.1 Error-writing pattern

A human diagnostic follows this shape:

```text
error[XSQ-RESOLVE-0017]: dependency alias `common` resolves to two package identities
  --> xmlsquish.toml:24:1
   |
24 | common = { version = "^2", path = "../common" }
   | ^^^^^^ conflicting registry and path sources
   |
help: choose exactly one source kind
      xmlsquish add common --path ../common
```

Every expected failure answers, when known:

1. what operation failed;
2. which project/package/target/source is affected;
3. where the decisive evidence is;
4. what was and was not committed;
5. whether unrelated jobs continued;
6. one concrete next action that does not destroy user data.

An internal invariant failure is visibly different:

```text
error[XSQ-INTERNAL-0001]: the linker produced an unknown source identity
note: no manifest or source file was modified
help: run `xmlsquish doctor --report .temp/xmlsquish-report.json`
```

In human/short mode, backtraces and internal logs require `-vv` or an explicit trace setting and
stay on stderr. In JSON mode, `-vv` produces typed trace data; raw details go only to the explicitly
requested trace file described in section 5.2.

### 8.2 Cancellation

- First Ctrl-C requests cooperative cancellation. The scheduler stops admitting work, active jobs
  reach safe cancellation points, in-flight atomic publication either commits or rolls back, and
  the terminal prints `Cancelling…` once.
- A second Ctrl-C requests immediate termination after the terminal guard restores presentation.
- Cooperative cancellation exits `130` on all supported platforms and emits an
  `operation-finished` event with `success:false`, `cancelled:true` when JSON remains writable.
- Temporary files and partial downloads are manager-owned. Cleanup is best-effort during signal
  handling and deterministic during the next startup recovery scan.
- Cancellation never deletes the last previously committed artifact.

### 8.3 Concurrent operations

The manager coordinates writes to manifests, locks, artifact indexes, and cache metadata:

- Read-only operations share state safely.
- A writer waits with bounded, visible progress rather than failing immediately. `--lock-timeout`
  allows automation to bound the wait.
- The waiting message names the resource and elapsed time, not another user's full command line.
- When the owner disappears, the manager validates the journal and resumes recovery; it does not
  trust a stale lock timestamp alone.
- A change between planning and commit triggers an automatic re-read and re-plan when the user's
  intent remains unambiguous. Otherwise the command reports a concise three-way conflict and
  leaves authoritative files coherent.

### 8.4 Atomic publication and transaction recovery

- Build products are staged in the destination filesystem, synchronized as required by the
  platform, and atomically renamed. A failed target does not overwrite its previous successful
  artifact. The artifact index marks that previous artifact as not produced by the failed
  invocation so stale presence cannot be reported as success.
- `fmt` completes parse/semantic-preservation preflight for the whole selection before the first
  write. Each file replacement is atomic. The operation report truthfully distinguishes
  `committed`, `unchanged`, and `not attempted`; it does not claim impossible cross-filesystem
  atomicity.
- Manifest/lock changes use a small write-ahead transaction journal containing old/new digests
  and staged paths. Startup recovery completes or rolls back an interrupted replacement based on
  those digests. `doctor` exposes the decision and evidence.
- Cache objects are content-addressed and immutable. An interrupted write leaves an unreferenced
  temporary object, never a valid object with wrong bytes. Startup housekeeping may remove it
  after validation.
- Transient Windows sharing violations and rename races receive bounded exponential backoff with
  jitter. The final diagnostic includes attempts and elapsed time at `-v`, while default output
  stays concise.
- A recovered operation emits `recovery-performed` and a durable human notice. Recovery is never
  hidden merely to keep logs pretty.

### 8.5 Broken pipes

For pure query output (`metadata`, `tree`, `inspect`, `completions`, help, or `fmt --diff`), a
downstream closed pipe terminates output quietly without an error-shaped panic. For JSON operation
streams, losing stdout requests cancellation because the promised observable event channel no
longer exists; the process exits `1` unless it had already finished successfully. No diagnostic
is recursively written to the broken stream.

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
(`2`). Warnings do not change the exit code unless an explicit policy such as `--deny-warnings`
promotes them to errors. Multiple job failures still reduce to `1`; their categories remain in the
event/diagnostic data rather than multiplying process codes.

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
7. Cache hits traverse the same job lifecycle and emit the same artifact metadata as executed
   jobs, plus `job-cached`. A cache hit is not silent disappearance.
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
Run `xmlsquish config explain build.jobs` to see why this value was selected.
```

Errors never dump the complete top-level help. They show the invalid fragment, a short correction,
and `For more information, try '--help'.` Examples are copyable in PowerShell, POSIX shells, and
`cmd.exe` unless explicitly labeled for one shell. Paths with whitespace are quoted.

Command and option names remain English and stable. Human messages are localization-ready but
diagnostic codes, JSON keys/reasons, manifest keys, and status verbs in `short` mode are not
localized. Machine interfaces never contain a localized value where an enum is expected.

## 12. Acceptance criteria

### 12.1 Command and stream behavior

- [ ] Bare invocation, every `--help`, and `--version` succeed without filesystem access.
- [ ] All top-level commands pass through the common manager bootstrap and event/status reducer.
- [ ] No worker or domain engine holds stdout/stderr handles.
- [ ] Human build/status output is on stderr; requested query/diff data is on stdout.
- [ ] Redirecting either stream does not change the other stream's capability decision.
- [ ] `--quiet` leaves errors and requested data intact.
- [ ] Pure query commands accept `--format` and reject `--message-format`; operation commands do
  the reverse. Completion output remains a directly executable shell program.
- [ ] Default build keep-going executes an independent target after another fails and marks only
  transitive dependents blocked; `--no-keep-going` stops new admission.
- [ ] Repeated `--emit` values produce the declared union, `--emit=ir` does not invoke squish,
  `--emit=debug` alone fails with code `2`, and comma-separated emit values are rejected.
- [ ] `--plain` output contains no ESC byte, carriage-return repaint, non-ASCII decoration, or
  terminal-width-dependent table.
- [ ] Filenames and argument values containing C0/C1 controls cannot inject lines or styling.

### 12.2 JSON contract

- [ ] Every JSON-mode stdout line parses as exactly one JSON object; stderr is empty.
- [ ] Every object carries `schema_version`, `sequence`, and `reason`; sequences are gapless.
- [ ] All terminal operation paths emit exactly one `operation-finished` while stdout is writable.
- [ ] Invalid CLI syntax emits structured diagnostics when a valid JSON mode was selected.
- [ ] ANSI, OSC, progress repaint bytes, and localized enum values never appear in JSON.
- [ ] A reference consumer that ignores unknown event reasons successfully processes additive
  version-1 fixtures.
- [ ] Event snapshots cover success, cache hit, multiple failures, blocked dependents,
  cancellation, recovery, format difference, and manifest change.
- [ ] Blocked and cache-hit jobs have legal terminal lifecycle events but never emit
  `job-started`; a reference reducer accepts an operation that fails before any job exists.
- [ ] A JSON-mode internal failure, including `-vv`, leaves stderr empty and records typed trace
  data or the explicitly selected trace-file path.
- [ ] Multi-file suggestions name an exact UTF-8 byte range for every edit and can be applied as a
  single all-or-nothing suggestion by a reference consumer.

### 12.3 Terminal and accessibility

- [ ] PTY/ConPTY tests cover color, resize, Unicode width, cursor restoration, and Ctrl-C.
- [ ] Non-TTY tests cover pipes, redirected files, `TERM=dumb`, non-empty `NO_COLOR`, `CI=true`,
  and `--plain`.
- [ ] Color and symbols are redundant with text in every success/warning/error state.
- [ ] A narrow terminal falls back to lists without hiding identifiers or diagnostic locations.
- [ ] Dynamic-region repainting does not exceed 10 frames per second and does not grow with total
  job count; durable diagnostics may interrupt it immediately after the region is cleared.
- [ ] Plain mode produces a useful chronological transcript for a screen reader.

### 12.4 Recovery and concurrency

- [ ] Fault injection at every manifest/lock replacement step proves that the next invocation
  reaches a coherent old or new state without manual deletion.
- [ ] Fault injection before/during artifact rename never exposes partial `*.prompt` bytes.
- [ ] Concurrent `add`, `remove`, `build`, and `fmt` runs wait, re-plan, or report a coherent
  conflict; they never silently lose an edit.
- [ ] First cancellation is cooperative and returns `130`; second cancellation restores terminal
  state before immediate exit.
- [ ] Previously successful artifacts survive a failed build but are not reported as newly valid.
- [ ] Slow JSON consumers cause bounded backpressure without event loss or unbounded memory.

### 12.5 Cross-platform GitHub Actions

- [ ] Windows, Linux, and macOS jobs run command grammar, streams, exits, JSON-schema fixtures,
  build/link/squish publication, `fmt --check`, and add/remove dry-run tests.
- [ ] Platform path separators do not change logical paths, cache keys, JSON snapshots, or
  `*.prompt` bytes.
- [ ] GitHub annotation tests verify escaping, file/line/column mapping, and multiple diagnostics.
- [ ] CI logs contain no animation frames even when the runner provides a pseudo-terminal.
- [ ] Event tests assert gapless sequences, legal per-job partial orders, and stable final job-table
  ordering. Full-stream snapshots run under a controlled deterministic scheduler; unconstrained
  parallel streams are not incorrectly required to have identical cross-job completion order.
- [ ] Artifact snapshots remove only explicitly non-semantic duration fields; tests do not
  normalize away identity or artifact-ordering defects.

### 12.6 Executable end-to-end contract matrix

This matrix is normative. It turns the preceding product statements into process-level tests and
is the cutover evidence for a project manager rather than a compiler-shaped CLI.

**Gate legend:** **K3** means the contract test must pass before the old loose-file CLI, sibling
`*.i.xml`/`*.o.xml` publication, or its temporary adapter may be deleted. **K4** means the same
test must subsequently be part of the required Linux/macOS/Windows gate. A row marked **K3+K4**
blocks deletion and remains a permanent three-platform regression test. ADR 0009 intentionally
does not preserve the old command grammar or old artifact names; the K3 gate proves that the new
manager route is complete, not that both public grammars coexist forever.

#### Harness and fixtures

The test runner executes the built binary directly with an argument array, never through a shell.
For every case it creates a fresh temporary checkout, isolated user configuration directory,
content store, target directory, and fake registry/network endpoint. It captures raw stdout,
stderr, exit status, network calls, and a before/after digest inventory of all authoritative and
published files. Wall-clock durations and platform-native display paths may be ignored only in
fields the schema declares non-semantic; tests must not normalize event order, IDs, logical paths,
digests, or summary counts merely to make a failure disappear.

| Fixture | Required contents |
| --- | --- |
| `basic` | One package `app`, a current lock, one default target `chat`, and golden `chat.prompt` bytes |
| `workspace` | Packages `core`, `app`, and `ops`; default targets `alpha` and `beta`; independently failing target `broken`; `needs-broken` depends on it |
| `format` | Owned clean and dirty XML, mixed content, CDATA/entities/namespaces/comments, one malformed owned file, and an unowned neighboring file |
| `deps` | Two workspace members, a direct alias `common`, a transitive package, current/stale/missing locks, a populated/empty local index, and a request-counting fake registry |
| `artifacts` | Valid prompt, `.psdbg`, `.xsir`, action/build records, their artifact manifest, plus corrupt and unsupported-version copies |
| `recovery` | Old/new manifest-lock generations, format files, prompt/debug generations, and fault injection at every documented durable commit point |

`human` below means the default renderer with stdout and stderr captured as non-TTY streams.
`pty` means a real PTY on Unix and ConPTY-compatible harness on Windows. `json` operation tests
run a reference event reducer; query JSON tests parse exactly one complete document. “No writes”
means byte-identical authoritative files and no newly committed artifact generation; disposable
temporary evidence may exist only where recovery policy explicitly permits it.

#### Dispatch, discovery, selection, and workspace

| ID | Gate | Setup and invocation | Required observations |
| --- | --- | --- | --- |
| `CLI-01` | **K3+K4** | No project; `xmlsquish`, `xmlsquish --help`, `xmlsquish build --help`, `xmlsquish --version` | Each exits `0`, writes its requested text only to stdout, performs no project/config/network access, and does not create state. |
| `CLI-02` | **K3+K4** | Create an existing file named `chat.xml`; run `xmlsquish chat.xml` | Exit `2`; stderr says the token is an unknown command; stdout is empty; the file is never compiled or modified. This is the decisive “no path dispatch” contract. |
| `CLI-03` | **K3+K4** | From a nested directory of `basic`, run `xmlsquish build --message-format=json`; repeat with `--manifest-path` from outside the tree | Both resolve the same logical project and target, emit the selected manifest/root in `project-resolved`, and publish the same bytes. |
| `CLI-04` | **K3+K4** | `xmlsquish --message-format=json unknown` and `xmlsquish build --emit=wat --message-format=json` | Each exits `2`; stdout contains a typed usage diagnostic and exactly one `operation-finished`; stderr is empty and no project is loaded after bootstrap rejection. |
| `SEL-01` | **K3+K4** | Workspace with two targets and no default; `xmlsquish build` | Exit `1` because valid project intent is ambiguous, not `2`; the diagnostic enumerates names and one copyable `-t` command; no job starts. |
| `WS-01` | **K3+K4** | `workspace`; `xmlsquish build --workspace --exclude ops --message-format=json` | Exactly the allowed `core`/`app` default targets are planned once in stable identity order; no `ops` action runs or publishes. |
| `WS-02` | **K3+K4** | `workspace`; run `xmlsquish build -p app -t alpha`, `xmlsquish fmt -p app`, and `xmlsquish add helpers@^2 --registry test -p app --dry-run` | All commands resolve `app` through the same package identity and ownership rules; selector errors use the same diagnostic code family and suggestions. |

#### Build, multiple targets, products, and debug evidence

| ID | Gate | Setup and invocation | Required observations |
| --- | --- | --- | --- |
| `BLD-01` | **K3+K4** | `basic`; `xmlsquish build` in human mode | Exit `0`; stdout empty; stderr names loading/compiling/linking/squishing/publication and final summary; exactly the declared `chat.prompt` generation is committed with golden bytes. No sibling XML intermediate appears. |
| `BLD-02` | **K3+K4** | `workspace`; `xmlsquish build -t alpha -t broken -t needs-broken -t beta` with default keep-going | Exit `1`; `alpha` and `beta` commit; `broken` is failed; `needs-broken` is blocked and never started; final counts distinguish all states and stable target IDs. |
| `BLD-03` | **K3+K4** | Same fixture; `xmlsquish build -t broken -t alpha -t beta --no-keep-going --jobs=1` | After the first failure no new job is admitted; non-started independent jobs are cancelled, not mislabeled blocked unless a prerequisite actually failed. |
| `BLD-04` | **K3+K4** | Give `broken` a previously committed golden artifact, then rebuild it unsuccessfully beside successful `beta` | The prior `broken` bytes and generation pointer survive but are not emitted/reported as produced by this invocation; `beta` commits independently. |
| `BLD-05` | **K3+K4** | `basic`; `xmlsquish build --emit=ir --message-format=json` | Exit `0`; materializes canonical `.xsir` for the selected complete modules and reports their digests; no link/backend/prompt/`.psdbg` job or file exists. |
| `BLD-06` | **K3+K4** | `basic`; `xmlsquish build --emit=prompt --emit=debug --message-format=json` | Exit `0`; one recoverable generation contains `chat.prompt`, `chat.psdbg`, and an artifact manifest naming both digests; `.psdbg` names the exact prompt digest; events identify each publication. |
| `BLD-07` | **K3+K4** | Run duplicate `--emit=prompt` flags, then `--emit=debug`, then `--emit=prompt,debug` | Duplicate values are idempotent and publish once. The latter two invocations exit `2` before project loading, with no writes: debug has no primary output and comma syntax is invalid. |
| `BLD-08` | **K3+K4** | Two selected targets are configured to publish the same destination | Planning exits `1` before executable jobs start; the diagnostic names both owners and the path; neither target changes its previous generation. |
| `BLD-09` | **K3+K4** | Run `basic` cold, warm, then after deleting all disposable derived state | All successful runs produce byte-identical prompt/debug/IR where selected. Warm JSON reports legal cache-hit lifecycle and artifact metadata; the clean rebuild does not depend on SQLite/CAS leftovers. |
| `BLD-10` | **K3+K4** | Repeat the workspace build under `--jobs=1`, `--jobs=2`, and `--jobs=0` with randomized worker completion | Artifacts, diagnostic facts, final job table, counts, and exit code are identical. Cross-job live event interleaving may differ but every per-job partial order and global sequence is legal. |

#### Formatter

| ID | Gate | Setup and invocation | Required observations |
| --- | --- | --- | --- |
| `FMT-01` | **K3+K4** | Clean `format`; `xmlsquish fmt --check` | Exit `0`; no writes; stdout empty; stderr has at most the single success summary. |
| `FMT-02` | **K3+K4** | Dirty valid file; `xmlsquish fmt --check` | Exit `1`; no writes; diagnostic/status identifies the file as different without calling it malformed. |
| `FMT-03` | **K3+K4** | Dirty valid file; `xmlsquish fmt --diff` | Exit `1`; stdout is an applicable unified diff and contains no status/ANSI; stderr contains status/diagnostics only. |
| `FMT-04` | **K3+K4** | Same input; `xmlsquish fmt`, then run it again and compile/inspect before/after semantic IR | First run atomically changes only proven trivia and exits `0`; second run is byte-idempotent; semantic/provenance comparison differs only in permitted spans/trivia and output prompt bytes remain equal. |
| `FMT-05` | **K3+K4** | Select dirty valid and malformed owned files together | Complete preflight exits `1` before the first replacement; neither file changes; parse/semantic failure is distinguished from a format difference. |
| `FMT-06` | **K3+K4** | `xmlsquish fmt --diff --message-format=json` | Exit `1`; stdout is NDJSON containing structured `format-difference` edits and final outcome, not a textual diff; stderr is empty. |
| `FMT-07` | **K3+K4** | Select the unowned neighbor with `fmt --path`; repeat workspace-wide | Explicit selection fails with an ownership diagnostic and no write; workspace traversal never discovers or modifies the file. |

#### Dependency edits and resolution modes

| ID | Gate | Setup and invocation | Required observations |
| --- | --- | --- | --- |
| `DEP-01` | **K3+K4** | `deps`; `xmlsquish add helpers@^2 --registry test -p app --rename helper` | Exit `0`; identity is resolved before editing; manifest and lock commit as one logical transaction; change events distinguish resolve, lock, and commit; unrelated valid lock selections are retained. |
| `DEP-02` | **K3+K4** | Repeat `DEP-01`, then run it with `helpers@^3` | Identical add is a successful no-op with no duplicate key or generation. Changed intent is reported as an update and leaves one typed alias entry. |
| `DEP-03` | **K3+K4** | Run `xmlsquish add helpers@^2 --registry test -p app --dry-run` and `xmlsquish remove common -p app --dry-run`, in human and JSON modes | Resolver/validation result and would-succeed exit match a real run from the same snapshot; manifest, lock, and journal remain byte-identical; JSON has typed `manifest-change` data. |
| `DEP-04` | **K3+K4** | `add common --path ../common --git https://invalid.example/x` | Exit `2` during bootstrap option validation; no project, filesystem mutation, DNS, or network access occurs. |
| `DEP-05` | **K3+K4** | Keep owned references to `common`; `xmlsquish remove common -p app` | Exit `1`; every remaining reference span is reported; manifest and lock are unchanged and no automatic source rewrite occurs. |
| `DEP-06` | **K3+K4** | Remove those references; `xmlsquish remove common -p app` | Exit `0`; manifest removes exactly the alias; lock prunes only newly unreachable nodes; shared/transitively reachable entries and CAS content remain. |
| `MODE-01` | **K3+K4** | Missing or stale lock; run `build --locked` and a state-changing `add --locked` | Exit `1`; no lock/manifest/artifact commit occurs. Build may contact the configured source only when needed to validate existing locked identities, but must never invent or write a selection. |
| `MODE-02` | **K3+K4** | Locally populated index/content but changeable lock; run `build --offline`; repeat with required content absent | Populated case may resolve/update and succeeds with zero network calls. Missing case exits `1`, names all unavailable identities and searched local state, makes zero network calls, and commits no artifact/lock. |
| `MODE-03` | **K3+K4** | Current lock and all remote blobs local; run `build --frozen`, then `build --locked --offline` | Both succeed with identical plan/products, zero network calls, and byte-identical lock. JSON exposes both restrictions as effective policy. |
| `MODE-04` | **K3+K4** | Frozen build with missing blob, and frozen build with edited path/workspace source | Missing blob fails `1` without network or writes. Edited local source is freshly snapshotted and built; frozen never claims local bytes are immutable. |
| `MODE-05` | **K3+K4** | Run idempotently satisfied and state-changing `add`/`remove` requests with `--frozen`, including `--dry-run` | Idempotent request may succeed if all content is local and writes nothing. Any request requiring manifest or lock mutation exits `1`; dry-run makes the same decision; all cases make zero network calls. |

#### Inspection and machine/human presentation

| ID | Gate | Setup and invocation | Required observations |
| --- | --- | --- | --- |
| `INSP-01` | **K3+K4** | `artifacts`; inspect each `ir`, `link`, `source`, `cache`, and `artifact` kind in human mode | Each valid object exits `0`, emits requested data on stdout and diagnostics on stderr only, shows its typed identity/digests, and performs no build, fetch, repair, or authoritative write. |
| `INSP-02` | **K3+K4** | Inspect prompt and `.psdbg` with `--format=json` | Each emits exactly one versioned JSON document with stable `kind`; prompt resolves the committed debug companion when present and debug names the exact prompt digest. No event envelope or human prose is mixed in. |
| `INSP-03` | **K3+K4** | Inspect corrupt, wrong-kind, unsupported-version, missing, and stale-uncommitted objects | Each valid invocation exits `1` with a precise diagnostic on stderr, emits no partial JSON result, never reports stale bytes as a build success, and changes nothing. Unknown inspect subject/options instead exit `2`. |
| `IO-01` | **K3+K4** | Pipe/capture human build, format, add/remove, and inspect output without a TTY | Output is append-only with no ESC/OSC/carriage-return repaint; operation status is stderr; query/diff data is stdout; capability decisions for the two streams are independent. |
| `IO-02` | **K3+K4** | Repeat representative operations with `--plain`, `--quiet`, and contradictory `--plain --color=always` | Plain is ASCII, linear, width-independent, and keeps diagnostics. Quiet preserves diagnostics/requested data. Contradiction exits `2` before project loading. |
| `JSON-01` | **K3+K4** | Run successful, cache-hit, multi-failure, pre-plan failure, format-difference, manifest-change, recovery, and cancellation paths with `--message-format=json` | Every stdout line is one UTF-8 JSON object; stderr is empty; sequences are gapless; legal event lifecycles reduce to the actual exit/final counts; exactly one final event exists while stdout is writable. |
| `JSON-02` | **K3+K4** | Feed version-1 event fixtures with an inserted unknown reason to the reference consumer | Consumer ignores the unknown event and computes the same supported result; removing/changing a required field is rejected rather than guessed. |
| `TTY-01` | **K3+K4** | Run a long build under PTY at wide/narrow widths, resize during work, and finish successfully | Meaningful progress appears only when enabled, repaint stays within one bounded region and 10 Hz, narrow view preserves IDs via list fallback, and cursor/style/line state is restored. |
| `TTY-02` | **K3+K4** | Under PTY trigger domain failure, panic-boundary test failure, first Ctrl-C, and second Ctrl-C | Dynamic region is cleared before durable diagnostics; terminal is restored on every path; cooperative cancellation reports once and exits `130`; immediate termination still restores the guard. |
| `PIPE-01` | **K3+K4** | Close downstream pipe during `inspect`/`fmt --diff`, then during JSON build | Pure query/diff terminates quietly without panic. JSON sink loss requests cancellation, commits no unsafe partial generation, and exits `1` unless success had already completed. |

#### Recovery, concurrency, and exit reduction

| ID | Gate | Setup and invocation | Required observations |
| --- | --- | --- | --- |
| `REC-01` | **K3+K4** | Inject process death at every manifest/lock durable commit point, then run an ordinary manager command | Startup converges automatically to a coherent complete old or new pair from journal/digests; it never asks for deletion; recovery is emitted in human and JSON modes. |
| `REC-02` | **K3+K4** | Inject death at every prompt/debug/artifact-manifest publication point with a prior good generation | No partial prompt/debug is observable as committed; next startup finishes or rolls back deterministically; the prior good generation survives unless the new complete generation commits. |
| `REC-03` | **K3+K4** | Inject death during multi-file formatting after complete preflight | Each reported replacement is whole; next startup reconciles manager-owned transaction evidence and truthfully identifies committed/unchanged/not-attempted files without claiming cross-filesystem atomicity. |
| `REC-04` | **K3+K4** | Corrupt/delete SQLite and one cached blob, preserving sources/manifest/lock; rebuild | Derived state is discarded/recomputed; correct clean bytes result. A corrupt blob is never accepted under its digest. |
| `CONC-01` | **K3+K4** | Race two adds, add/remove, build/add, and fmt/build using barriers at plan and commit | Writers wait or re-plan with visible bounded progress; unambiguous intent converges without lost edits; true conflicts exit `1` with coherent files; no command observes a half transaction. |
| `EXIT-00` | **K3+K4** | Successful build/edit/query, clean format check, and bare help | All exit `0`. |
| `EXIT-01` | **K3+K4** | Compile/resolve/I/O failure, dirty format check, missing inspect object, and lost JSON sink | All valid invocations exit `1`; category remains in structured diagnostics rather than a new process code. |
| `EXIT-02` | **K3+K4** | Unknown command/flag, conflicting source flags, malformed selector/argument, invalid emit combination | All exit `2` before domain execution; no later domain error is reduced to `2`. |
| `EXIT-101` | **K3+K4** | Test-only kernel invariant/panic injection | Exit `101`; user files and last committed artifacts survive; human/JSON output uses the internal-failure diagnostic contract. |
| `EXIT-130` | **K3+K4** | First Ctrl-C in locating, planning, running, and publication-safe-point scenarios | Each cooperatively cancels and exits `130`; priority over concurrent domain failures is stable; final JSON says `cancelled:true` when writable. |

#### K3 deletion decision

Deletion of the old CLI is permitted only when all **K3** rows above pass and the following two
cutover checks are green:

1. Representative ADR 0007 language fixtures are wrapped in explicit temporary projects and
   built through `xmlsquish build`; their final prompt bytes and semantic diagnostic facts match
   the accepted language goldens. Old sibling artifact names and old argv are deliberately not
   compared.
2. A repository search and architecture test show that every public command enters the manager
   kernel/event/status path and no production code can dispatch an operand by filesystem existence
   or publish `*.i.xml`/`*.o.xml`.

If a K3 row is flaky, skipped on a supported platform, or asserted only through an in-process
domain test, the gate is not met. The old route is not retained as a fallback; deletion waits until
the replacement process contract is real.

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
