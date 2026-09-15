# Project Manager Rework: Execution Status

**Last updated:** 2026-09-15
**Decision purpose:** Keep the `xmlsquish` compiler-to-project-manager rework inspectable while implementation proceeds in parallel.
**Normative sources:** The linked ADR and design documents remain authoritative; this file is the durable coordination index and cutover ledger.

## 1. Product Direction

`xmlsquish` is being rebuilt as a Cargo-like project manager with direct `fmt`, `build`, `add`, `remove`, and `inspect` commands. The architecture is intentionally top-down rather than an adapter around the legacy compiler:

```text
CLI bootstrap
    -> protocol request
    -> microkernel lifecycle and event stream
    -> manager capability
    -> repository / resolver / fetch / CAS / publisher ports
    -> XML frontend -> reusable XSIR -> link root -> .prompt + .psdbg
```

The cutover does not preserve the old internal architecture. It does preserve the current XML DSL and makes the external command and artifact contracts explicit before deleting legacy paths.

## 2. Durable Knowledge Map

| Question | Durable artifact | Status |
|---|---|---|
| Why a microkernel manager and reusable IR? | [`../adr/0009-microkernel-manager-and-reusable-ir.md`](../adr/0009-microkernel-manager-and-reusable-ir.md) | Accepted, including the observable planning lifecycle amendment |
| What is the complete IR, linking, provenance, and debug model? | [`ir-model.md`](ir-model.md) | Implemented through canonical IR, link trace, and self-contained debug bundle |
| How does the old tree map to the new architecture? | [`refactor-map.md`](refactor-map.md) | Active cutover map |
| What should each CLI command feel like and how is cutover accepted? | [`../product/cli-experience.md`](../product/cli-experience.md) | Command contract and executable K3/K4 gates defined |
| What is in and out of the project-manager product? | [`../product/project-manager-scope.md`](../product/project-manager-scope.md) | Product scope defined |
| How do registry and Git dependencies work? | [`dependency-source-protocol.md`](dependency-source-protocol.md) | Protocol specified; fetch implementation and review fixes committed |
| Which industry and academic systems informed the architecture? | [`../research/compiler-manager-ir-prior-art.md`](../research/compiler-manager-ir-prior-art.md) | Compiler/manager/IR synthesis complete |
| Which mature project managers informed the product boundary? | [`../research/project-manager-prior-art.md`](../research/project-manager-prior-art.md) | Prior-art review complete |

The research is not an isolated literature dump. Each research artifact is connected to an ADR, a code boundary, or an executable acceptance gate.

## 3. Implemented and Committed Foundations

The milestone snapshot is `17a08b8`. Earlier foundation commits remain part of the evidence chain; this table emphasizes the final cutover and the defects closed after `c616fef`.

| Area | Committed evidence | Verified contract |
|---|---|---|
| Protocol, kernel, and scheduler | `crates/squish-protocol`, `crates/squish-kernel`, `crates/squish-build` | Observable planning, immutable plans, truthful terminal reduction, post-plan catalogue finalization, bounded scheduling, persistent action identities, cancellation, and cache restoration |
| Root command, output, and inspection contracts | `377fd08`, `c616fef`, `95c4368`, `7e619af` | The legacy entry path is deleted; all five direct commands use the typed manager route; bare build emits `.prompt`; JSON bootstrap failures, human unified diff, broken pipes, exit classes, and typed artifact selectors have 24 root process regressions |
| Manager products and provenance | `d5a7ea0` | Public unified diff text; Windows ordinary/verbatim path equivalence; multi-module semantic builds retain the complete source-origin graph and publish traceable `.prompt`, `.xsir`, and `.psdbg` products |
| Repository, resolution, fetch, and publication | repository/resolver/fetch/publish crates plus their focused tests | Frozen project observations, deterministic resolution, registry/Git/path/workspace sources, lock-first materialization, and recoverable whole-generation publication |
| Exact Git package materialization | `8342d91`, `8455269` | Fetch returns the locked package root rather than a repository root, keeps the materialized handle alive for its consumer, and does not intentionally leave detached Git maintenance processes |
| Content and action storage | `3b77843`, `126e27d` | Verified CAS and action records; Windows EFS cross-volume publication and long-path handling preserve atomic, content-addressed semantics |
| Project catalogue isolation | `628ee9a` and [`project-storage-namespaces.md`](project-storage-namespaces.md) | A shared manager storage root derives a stable namespace per canonical project identity; two projects cannot consume each other's build records |
| Registry credentials | `df5a8d8`, `a177c78` and [`dependency-source-protocol.md`](dependency-source-protocol.md) | The shipped host resolves scoped environment credentials, re-scopes redirects by origin, never persists secrets, distinguishes missing from rejected credentials, and names non-secret recovery inputs |
| Terminal capability and interrupts | `6d8df1a`, `2f7f46e`, `f6b9f1e` | Live terminal width probing and resize observation; one three-state interrupt coordinator; first delivery requests cooperative cancellation and the second performs bounded emergency restoration before exit `130` |
| Interactive terminal process contract | `1137239` | A real PTY/ConPTY-compatible harness exercises progress, resize, cooperative first interrupt, destructive second interrupt, restoration bytes, and exit `130`; it passed five repeated local Windows runs and the workspace suite |
| Process-death recovery | `17a08b8` and [`process-recovery-testing.md`](process-recovery-testing.md) | Feature-gated real child-process death covers both sides of repository, artifact-generation, and build-catalogue commit decisions; ordinary follow-up commands recover automatically (6/6 process tests) |
| MSRV quality portability | `5f52c25` | Strict Clippy remains clean on Rust 1.88 after removing newer-lint assumptions from production code |
| Cross-platform CI definition | `454b4af` | Locked MSRV 1.88 quality checks, full workspace tests on Ubuntu/Windows/macOS, composed-manager smoke, and Linux source-install smoke are encoded in GitHub Actions |
| Product documentation and examples | `8b856f6` | README, changelog, DSL guide, product scope, and shipped example manifests describe the manager workflow and current artifact names |
| Live website | `a9d29f7` | The current landing page, reference content, and build explorer present the project-manager workflow without rewriting historical namespace snapshots |

## 4. Integrated Product State

There is now one production path rather than parallel legacy and manager implementations:

```text
xmlsquish argv
  -> squish-cli typed request + layered squish-config
  -> root bootstrap output/interrupt contracts
  -> squish-kernel lifecycle
  -> squish-manager operation
  -> squish-host ports
  -> repository/resolver/fetch/store/publish
  -> selected generation + typed catalogue
  -> human/short/NDJSON or typed inspect rendering
```

The following previously open integration findings are closed in committed code and are not carried forward as risks:

- transform-cache hydration, action identity, build-catalogue finalization, and interrupted-catalogue recovery;
- hidden production storage and transport dependencies;
- root legacy dispatch, missing default prompt output, JSON bootstrap stream violations, private format-diff output, inspect broken-pipe failure, and Windows absolute-path mismatch;
- stringly typed inspect artifact paths and partial root coverage of the inspect subjects;
- flattened `.psdbg` provenance in the representative multi-module/macro build;
- Git materialization returning the repository rather than the exact locked package and retaining ambiguous process ownership;
- shared global-storage catalogue collisions between projects;
- Windows EFS/cross-volume CAS publication and long-path handling;
- a configured authentication scope backed by `NoCredentials` in the shipped executable;
- fixed-width terminal composition and the missing second-interrupt state machine.
- the absence of real PTY signal/resize coverage and composed process-death recovery evidence.

The architecture and product material are also synchronized: shipped examples (`8b856f6`), the live site (`a9d29f7`), the workflow (`454b4af`), and root process tests use the direct manager commands and `.prompt`/`.xsir`/`.psdbg` vocabulary.

## 5. Remaining Evidence Gaps

This is an acceptance gap, not a known P1 implementation defect.

| Gap | What is already known | Evidence still required |
|---|---|---|
| Remote three-platform execution | The Ubuntu/Windows/macOS jobs and composed CLI smoke are committed in `454b4af`. Local Windows workspace, PTY, recovery, and MSRV Clippy runs cannot predict hosted runner images or Unix filesystem behavior. | Observe a green GitHub Actions run of the committed workflow at `17a08b8` or a descendant on all three hosted operating systems. |

No old MVP checklist is reopened here. New work should be added only when a failing test, an observed production behavior, or an explicit product decision supplies concrete evidence.

## 6. Cutover Gates

| Gate | State | Evidence |
|---|---|---|
| All five commands use the new typed CLI/kernel/manager route; no legacy fallback remains | **Satisfied** | `377fd08`, `95c4368`; `src/main.rs` and `tests/process.rs` are the only tracked root Rust entry/process files |
| Expensive and mutating phases are scheduled work with truthful lifecycle and failure attribution | **Satisfied** | Protocol/kernel/build/manager tests at the milestone snapshot |
| A warm invocation restores cacheable transform outputs from the action index and CAS | **Satisfied** | Manager restoration tests and root warm-cache process regression |
| Selected `.prompt`, `.xsir`, `.psdbg`, and build-record products publish through recoverable generations | **Satisfied** | `d5a7ea0`, manager build tests, publish crash-point tests |
| Registry, Git, path, and workspace sources obey typed online/offline/locked/frozen boundaries; configured registry credentials are usable | **Satisfied at component/composition level** | Fetch/resolver/host suites; `df5a8d8`, `a177c78` |
| Root stream, exit, format-diff, typed inspection, storage-namespace, exact Git package, and example provenance contracts pass on the local Windows host | **Satisfied locally** | 24 root process tests and local validation recorded in Section 8.3 |
| Real PTY/ConPTY interaction covers resize and two-stage interrupt restoration | **Satisfied locally** | `1137239`; the PTY suite passed five consecutive local Windows runs and the workspace run |
| Real process death at the supported durable boundaries converges without manual repair | **Satisfied locally** | `17a08b8`; `tests/recovery_process.rs` passed 6/6 against the composed binary |
| Strict MSRV Clippy passes | **Satisfied locally** | `5f52c25`; local Rust 1.88 Clippy validation recorded in Section 8.3 |
| The committed workflow passes on supported GitHub-hosted desktops | **Pending remote run** | Workflow exists in `454b4af`; configuration alone is not execution evidence |

## 7. Evidence Discipline

Status terms in this ledger have precise meanings:

- **Specified:** a durable contract exists, but production code may not yet implement it.
- **Implemented:** code exists and its local focused tests passed.
- **Reviewed:** an independent agent inspected the affected execution paths; any concrete defects were resolved.
- **Cut over:** the root binary and process tests use only the replacement path.
- **Complete:** cross-platform workspace validation passes and no required legacy fallback remains.

This distinction prevents a design document, a green unit test, or an agent completion message from being mistaken for a finished product.

## 8. Acceptance Evidence and Remaining Gates

**Audit date:** 2026-09-15

**Committed snapshot:** `17a08b8`

**Scope:** the Cargo-like manager cutover: direct commands, scheduling and state management, XML to reusable binary IR to linked prompt, complete debug evidence, modern output behavior, dependency acquisition, and supported-desktop automation.

### 8.1 Evidence classes

- **Committed evidence:** independently inspectable code, tests, documentation, or workflow in `HEAD`.
- **Local Windows execution:** a command was executed successfully in this workspace on Windows; it is evidence for this host only.
- **Remote cross-platform execution:** a GitHub-hosted Ubuntu, Windows, and macOS workflow completed successfully. This evidence is still pending.

### 8.2 Product acceptance matrix

| Product dimension | Current judgment | Evidence |
|---|---|---|
| Architecture and cutover | **Committed and cut over** | Separate protocol/kernel/manager/service crates; direct root dispatch; legacy compiler entry removed |
| `fmt` | **Committed; locally exercised** | Deterministic semantic formatting, batch preflight, stable unified diff, non-UTF-8 diagnostic, stdout/stderr and dirty-check exits (`d5a7ea0`, `95c4368`) |
| `build` and reusable products | **Committed; locally exercised** | Compile/link/instantiate/backend/publish/catalogue actions; default prompt; persistent cache; representative semantic `.psdbg` traceability (`c616fef`, `d5a7ea0`) |
| `add` / `remove` | **Committed; locally exercised for path workflow** | Coherent manifest/lock transaction, dry run, removal references, contention/replan, and root process round trip |
| `inspect` | **Committed; locally exercised** | Typed `ir`, `link`, `source`, `cache`, and `artifact` views; protocol-level artifact selectors; CAS/provenance validation; Windows path equivalence; quiet closed-pipe handling; 24 root process tests (`d5a7ea0`, `95c4368`, `7e619af`) |
| Git packages | **Committed; locally exercised** | Exact locked package-root return, owned materialization handle, and documented Git child-process lifecycle (`8342d91`, `8455269`) |
| Storage and recovery mechanisms | **Committed; locally process-tested** | Verified CAS/action catalogues, EFS and long-path handling, per-project namespace, recoverable journals, and 6/6 real child-death recovery cases (`3b77843`, `126e27d`, `628ee9a`, `17a08b8`) |
| Authenticated registries | **Committed; component/composition-tested** | Scoped environment lookup, redirect re-scoping, redaction, distinct missing/rejected outcomes, actionable variable names (`df5a8d8`, `a177c78`) |
| Terminal behavior | **Committed; locally process-tested** | Live width probe, progress resize, first/second interrupt, emergency restoration, and exit `130` passed through the real PTY/ConPTY-compatible harness for five consecutive local Windows runs (`6d8df1a`, `2f7f46e`, `f6b9f1e`, `1137239`) |
| MSRV quality | **Committed; locally exercised** | Strict workspace Clippy passed on Rust 1.88 after `5f52c25` |
| Documentation and site | **Committed** | Current manager workflow and artifacts in `8b856f6` and `a9d29f7` |
| Supported desktops | **Configured, not remotely accepted** | Three-OS MSRV workflow and manager smoke in `454b4af`; no remote run result is recorded in this snapshot |

### 8.3 Local Windows validation record

At snapshot `17a08b8`, the following validation completed successfully on the local Windows workspace:

```text
cargo test --workspace --all-features --locked
python .github/scripts/ci_smoke.py target/debug/xmlsquish.exe .temp/status-smoke
cargo test --test pty --locked                 # five consecutive runs, two tests per run
cargo test --features fault-injection --test recovery_process --locked  # 6/6
cargo +1.88.0 clippy --workspace --all-targets --all-features --locked -- -D warnings
```

The workspace run included 24 root process tests, typed inspect-selector coverage, the representative semantic provenance regression, formatter/inspect/storage/credential/terminal tests, the two real-PTY process tests, and all crate doc tests. The composed smoke checked the five-command help grammar, machine-readable usage failure, human unified diff, default `.prompt`, explicit `.xsir`, and JSON link inspection. The recovery suite independently killed real children at supported pre/post-commit boundaries and all six cases passed.

This record does **not** imply that the GitHub-hosted Linux, macOS, or Windows jobs ran. The PTY evidence is real on this Windows host, but it is not a substitute for observing the committed harness on each remote runner. Site checks and `cargo +1.88.0 check` are likewise not claimed by this local record; strict Rust 1.88 Clippy is claimed because it was run explicitly.

### 8.4 Closed findings trace

| Former blocker | Closing evidence |
|---|---|
| Root output/exit contracts, unified-diff delivery, JSON bootstrap records, and query broken pipes | `95c4368` |
| Representative provenance, public diff artifact, and Windows inspect-path equivalence | `d5a7ea0` |
| Manager-oriented CI and source-install smoke | `454b4af` |
| Current website still described the deleted compiler workflow | `a9d29f7` |
| Windows EFS/cross-device CAS publication and long paths | `3b77843`, `126e27d` |
| Shared storage exposed another project's build catalogue | `628ee9a` |
| Root terminal width was fixed/unknown and could not observe resize | `6d8df1a` |
| Production root always supplied no registry credentials; missing input was not actionable | `df5a8d8`, `a177c78` |
| Second interrupt had no bounded emergency-restoration path | `2f7f46e`, `f6b9f1e` |
| README/examples/product scope still centered the old compiler workflow | `8b856f6` |
| Inspect artifact selection was a string/path convention rather than a protocol type | `7e619af` |
| Git fetch returned repository scope rather than the exact locked package and had an undocumented handle lifetime | `8342d91`, `8455269` |
| PTY/ConPTY resize, first/second interrupt, and restoration existed only below the process boundary | `1137239`; five consecutive local Windows PTY runs plus the workspace suite |
| Recovery claims stopped at component journals rather than real process termination | `17a08b8`; 6/6 composed recovery-process tests |
| Strict Clippy used assumptions unavailable on the declared MSRV | `5f52c25`; local Rust 1.88 strict Clippy pass |

### 8.5 Remaining acceptance sequence

Only one evidence step remains in this milestone ledger:

1. **Run the committed GitHub Actions workflow remotely.** Record the run that passes MSRV quality checks, the full workspace suite, composed CLI smoke on Ubuntu/Windows/macOS, the Linux source install, and the site job.

Until that run has evidence, the accurate judgment is: **the manager architecture and its process-boundary contracts are implemented and locally validated on Windows, while full supported-desktop acceptance remains pending only on the remote GitHub Actions result.**
