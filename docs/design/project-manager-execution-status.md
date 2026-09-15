# Project Manager Rework: Execution Status

**Last updated:** 2026-09-15
**Decision purpose:** Preserve the final, evidence-backed acceptance record for the `xmlsquish` compiler-to-project-manager rework.
**Normative sources:** The linked ADR and design documents remain authoritative; this file is the durable coordination index and cutover ledger.

## 1. Product Direction

`xmlsquish` has been rebuilt as a Cargo-like project manager with direct `fmt`, `build`, `add`, `remove`, and `inspect` commands. The architecture is intentionally top-down rather than an adapter around the legacy compiler:

```text
CLI bootstrap
    -> protocol request
    -> microkernel lifecycle and event stream
    -> manager capability
    -> repository / resolver / fetch / CAS / publisher ports
    -> XML frontend -> reusable XSIR -> link root -> .prompt + .psdbg
```

The completed cutover did not preserve the old internal architecture. It preserved the XML DSL, made the external command and artifact contracts explicit, and deleted the legacy paths.

## 2. Durable Knowledge Map

| Question | Durable artifact | Status |
|---|---|---|
| Why a microkernel manager and reusable IR? | [`../adr/0009-microkernel-manager-and-reusable-ir.md`](../adr/0009-microkernel-manager-and-reusable-ir.md) | Accepted, including the observable planning lifecycle amendment |
| What is the complete IR, linking, provenance, and debug model? | [`ir-model.md`](ir-model.md) | Implemented through canonical IR, link trace, and self-contained debug bundle |
| How did the old tree map to the current architecture? | [`refactor-map.md`](refactor-map.md) | Historical/completed cutover map with verified current owners |
| What does each CLI command feel like, and which proposals were accepted or superseded? | [`../product/cli-experience.md`](../product/cli-experience.md) | Current command contract plus evidence and supersession tables |
| What is in and out of the project-manager product? | [`../product/project-manager-scope.md`](../product/project-manager-scope.md) | Product scope defined |
| How do registry and Git dependencies work? | [`dependency-source-protocol.md`](dependency-source-protocol.md) | Protocol specified; fetch implementation and review fixes committed |
| Which industry and academic systems informed the architecture? | [`../research/compiler-manager-ir-prior-art.md`](../research/compiler-manager-ir-prior-art.md) | Compiler/manager/IR synthesis complete |
| Which mature project managers informed the product boundary? | [`../research/project-manager-prior-art.md`](../research/project-manager-prior-art.md) | Prior-art review complete |

The research is not an isolated literature dump. Each research artifact is connected to an ADR, a code boundary, or an executable acceptance gate.

## 3. Implemented and Committed Foundations

The remotely accepted production snapshot is `b870c84`. Its first documentation-only descendants were `bf3394f`, a two-line correction in `docs/product/cli-experience.md`, and `f2a75f5`, this acceptance ledger. This reconciliation remains confined to the ledger and `docs/design/refactor-map.md`; executable sources, tests, workflow, lockfiles, and `site/` inputs remain byte-for-byte those exercised at `b870c84`. Earlier foundation commits remain part of the evidence chain; this table emphasizes the final cutover and the defects closed after `c616fef`.

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
| Unix CAS portability | `a3d6d23` | The Unix rename callback is expressed with the higher-ranked lifetime contract required by Rust 1.88, closing the Linux/macOS compilation failure exposed by the first remote run |
| Unix fetch-test hygiene | `911304b` | Platform-specific fixture cleanup removes the Windows-only retry loop from Unix compilation, closing strict Clippy warnings and Unix cleanup failures without weakening Windows cleanup |
| Dynamic terminal rendering | `a8084d6`, `be625af`, `b870c84` | Quiet work pumps the renderer; cancellation emits one structured notice only before a terminal event; the terminal ordering test waits on an explicit renderer barrier rather than timing |
| Link scalar completeness | `543b271` | A non-tail scalar composition witness guards the linker against an order-sensitive false positive that tail-only fixtures could miss |
| Contract reconciliation | `4fff0dc`, `bf3394f` | IR, CLI, and completeness documents describe implemented behavior; the stale `init` path example is removed rather than retained as an unimplemented product promise |
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

## 5. Remote Acceptance History

Remote execution was diagnostic evidence, not a ceremonial rerun. Two failing runs exposed platform-specific defects that local Windows validation could not reveal; the third run accepted their fixes and the complete product snapshot.

| Run | Snapshot | Result | Diagnostic or acceptance evidence |
|---|---|---|---|
| [34947807569](https://github.com/kleedaisuki/prompt-squish/actions/runs/34947807569) | `270feb2` | **Failed** | Ubuntu quality and Linux/macOS builds rejected the Unix CAS rename callback because its `FnMut`/`FnOnce` lifetime implementation was not general enough on Rust 1.88. Windows Rust and Site passed. This led to `a3d6d23`. |
| [34948934092](https://github.com/kleedaisuki/prompt-squish/actions/runs/34948934092) | `a3d6d23` | **Failed** | The CAS build defect was closed. Ubuntu strict Clippy then exposed a Unix-unused Windows retry variable/never-loop in the Git fixture cleanup, and Linux/macOS PTY tests exposed renderer/interrupt behavior that depended on timing. This led to `911304b`, `a8084d6`, `be625af`, and `b870c84`. |
| [34955324426](https://github.com/kleedaisuki/prompt-squish/actions/runs/34955324426) | `b870c84` | **Passed** | All five jobs completed successfully: Rust quality on Ubuntu, Rust tests on Linux/macOS/Windows, and Site. This is the supported-desktop acceptance run. |

The remote gate is satisfied. No known milestone evidence gap remains. New work should be opened only when a failing test, observed production behavior, or explicit product decision provides concrete evidence; this statement does not claim that future requirements or defects are impossible.

## 6. Cutover Gates

| Gate | State | Evidence |
|---|---|---|
| All five commands use the new typed CLI/kernel/manager route; no legacy fallback remains | **Satisfied** | `377fd08`, `95c4368`; `src/main.rs` and `tests/process.rs` are the only tracked root Rust entry/process files |
| Expensive and mutating phases are scheduled work with truthful lifecycle and failure attribution | **Satisfied** | Protocol/kernel/build/manager tests at the milestone snapshot |
| A warm invocation restores cacheable transform outputs from the action index and CAS | **Satisfied** | Manager restoration tests and root warm-cache process regression |
| Selected `.prompt`, `.xsir`, `.psdbg`, and build-record products publish through recoverable generations | **Satisfied** | `d5a7ea0`, manager build tests, publish crash-point tests |
| Registry, Git, path, and workspace sources obey typed online/offline/locked/frozen boundaries; configured registry credentials are usable | **Satisfied at component/composition level** | Fetch/resolver/host suites; `df5a8d8`, `a177c78` |
| Root stream, exit, format-diff, typed inspection, storage-namespace, exact Git package, and example provenance contracts pass | **Satisfied** | 24 root process regressions passed locally; the full workspace suite and composed smoke subsequently passed on all three hosted operating systems in run 34955324426 |
| Real PTY/ConPTY interaction covers resize and two-stage interrupt restoration | **Satisfied** | `1137239`, `a8084d6`, `be625af`, `b870c84`; the PTY suite passed locally and in all three remote Rust-test jobs |
| Real process death at the supported durable boundaries converges without manual repair | **Satisfied** | `17a08b8`; 6/6 composed recovery-process tests passed locally and the all-feature workspace suite passed in all three remote Rust-test jobs |
| Strict MSRV Clippy passes | **Satisfied** | `5f52c25`, `911304b`; the strict Rust 1.88 job passed remotely in run 34955324426 |
| The committed workflow passes on supported GitHub-hosted desktops | **Satisfied** | [Run 34955324426](https://github.com/kleedaisuki/prompt-squish/actions/runs/34955324426) passed all five jobs at production snapshot `b870c84` |

## 7. Evidence Discipline

Status terms in this ledger have precise meanings:

- **Specified:** a durable contract exists, but production code may not yet implement it.
- **Implemented:** code exists and its local focused tests passed.
- **Reviewed:** an independent agent inspected the affected execution paths; any concrete defects were resolved.
- **Cut over:** the root binary and process tests use only the replacement path.
- **Complete:** cross-platform workspace validation passes and no required legacy fallback remains.

This distinction prevents a design document, a green unit test, or an agent completion message from being mistaken for a finished product.

## 8. Acceptance Evidence

**Audit date:** 2026-09-15

**Remotely accepted production snapshot:** `b870c84`

**Documentation snapshot before this ledger update:** `bf3394f`

**Scope:** the Cargo-like manager cutover: direct commands, scheduling and state management, XML to reusable binary IR to linked prompt, complete debug evidence, modern output behavior, dependency acquisition, and supported-desktop automation.

### 8.1 Evidence classes

- **Committed evidence:** independently inspectable code, tests, documentation, or workflow in the named snapshot.
- **Local Windows execution:** a command was executed successfully in this workspace on Windows; it is evidence for this host only.
- **Remote cross-platform execution:** the named GitHub-hosted workflow completed successfully; its individual jobs prove only the commands they ran.

### 8.2 Product acceptance matrix

| Product dimension | Current judgment | Evidence |
|---|---|---|
| Architecture and cutover | **Committed and cut over** | Separate protocol/kernel/manager/service crates; direct root dispatch; legacy compiler entry removed |
| `fmt` | **Accepted on supported desktops** | Deterministic semantic formatting, batch preflight, stable unified diff, non-UTF-8 diagnostic, stdout/stderr and dirty-check exits (`d5a7ea0`, `95c4368`); workspace tests and composed smoke passed in run 34955324426 |
| `build` and reusable products | **Accepted on supported desktops** | Compile/link/instantiate/backend/publish/catalogue actions; default prompt; persistent cache; representative semantic `.psdbg` traceability (`c616fef`, `d5a7ea0`); workspace tests and composed smoke passed in run 34955324426 |
| `add` / `remove` | **Accepted on supported desktops** | Coherent manifest/lock transaction, dry run, removal references, contention/replan, and root process round trip; the full workspace suite passed on all three hosted operating systems |
| `inspect` | **Accepted on supported desktops** | Typed `ir`, `link`, `source`, `cache`, and `artifact` views; protocol-level artifact selectors; CAS/provenance validation; Windows path equivalence; quiet closed-pipe handling; workspace tests and composed JSON-link smoke passed in run 34955324426 |
| Git packages | **Accepted on supported desktops** | Exact locked package-root return, owned materialization handle, documented child-process lifecycle, and platform-specific fixture cleanup (`8342d91`, `8455269`, `911304b`); workspace tests passed on Linux/macOS/Windows |
| Storage and recovery mechanisms | **Accepted on supported desktops** | Verified CAS/action catalogues, EFS and long-path handling, per-project namespace, recoverable journals, Unix Rust 1.88 CAS portability, and 6/6 real child-death recovery cases (`3b77843`, `126e27d`, `628ee9a`, `17a08b8`, `a3d6d23`); the all-feature workspace suite passed remotely on all three operating systems |
| Authenticated registries | **Accepted at component/composition scope** | Scoped environment lookup, redirect re-scoping, redaction, distinct missing/rejected outcomes, actionable variable names (`df5a8d8`, `a177c78`); their committed suites passed remotely on all three operating systems (the workflow does not contact a live authenticated registry) |
| Terminal behavior | **Accepted on supported desktops** | Live width and resize, two-stage interrupt/restoration, quiet-work pumping, structured cancellation ordering, and explicit terminal barrier (`6d8df1a`, `2f7f46e`, `f6b9f1e`, `1137239`, `a8084d6`, `be625af`, `b870c84`); the PTY suite passed in all three remote Rust-test jobs |
| MSRV quality | **Accepted remotely** | Rust 1.88 check, formatting, and strict all-target/all-feature Clippy passed in the Ubuntu quality job of run 34955324426 |
| Documentation and site | **Accepted remotely** | Current manager workflow and artifacts in `8b856f6`, `a9d29f7`, and `4fff0dc`; Astro/TypeScript checks and the site build passed in run 34955324426 |
| Supported desktops | **Complete for the milestone** | Run 34955324426 passed the full workspace build/test and composed manager smoke on hosted Linux, macOS, and Windows; Linux additionally passed source installation and installed-binary smoke |

### 8.3 Local Windows validation record

At snapshot `17a08b8`, before the remote diagnostic cycle, the following validation completed successfully on the local Windows workspace:

```text
cargo test --workspace --all-features --locked
python .github/scripts/ci_smoke.py target/debug/xmlsquish.exe .temp/status-smoke
cargo test --test pty --locked                 # five consecutive runs, two tests per run
cargo test --features fault-injection --test recovery_process --locked  # 6/6
cargo +1.88.0 clippy --workspace --all-targets --all-features --locked -- -D warnings
```

The workspace run included 24 root process tests, typed inspect-selector coverage, the representative semantic provenance regression, formatter/inspect/storage/credential/terminal tests, the two real-PTY process tests, and all crate doc tests. The composed smoke checked the five-command help grammar, machine-readable usage failure, human unified diff, default `.prompt`, explicit `.xsir`, and JSON link inspection. The recovery suite independently killed real children at supported pre/post-commit boundaries and all six cases passed.

This local record is retained to distinguish what was known before remote execution. It did **not** predict the Unix defects found by runs 34947807569 and 34948934092. The later remote acceptance claims come exclusively from run 34955324426, not from extrapolating this Windows record.

### 8.4 Remote job evidence at `b870c84`

[Run 34955324426](https://github.com/kleedaisuki/prompt-squish/actions/runs/34955324426) completed successfully with exactly five jobs:

| Job | Commands represented by the successful job | What the result proves |
|---|---|---|
| Rust quality (Ubuntu, MSRV 1.88) | Rust 1.88 toolchain verification, `cargo check`, formatting check, strict Clippy | The workspace type-checks at the declared MSRV on Ubuntu, is formatted, and has no warnings under the committed strict Clippy scope |
| Rust test (Linux, MSRV 1.88) | All-target/all-feature build, full workspace tests, composed root CLI smoke, source install, installed-binary smoke | Linux compilation and tests pass; the five-command manager surface works as a composed binary; a fresh locked source install produces a working binary |
| Rust test (macOS, MSRV 1.88) | All-target/all-feature build, full workspace tests, composed root CLI smoke | macOS compilation, tests, PTY behavior, and composed manager CLI smoke pass |
| Rust test (Windows, MSRV 1.88) | All-target/all-feature build, full workspace tests, composed root CLI smoke | Windows compilation, tests, ConPTY-compatible behavior, and composed manager CLI smoke pass |
| Site | Dependency installation, Astro/TypeScript check, production build | The committed site sources type-check and produce the production static build |

The source-install steps are intentionally Linux-only in the workflow and were skipped, not failed, on macOS and Windows. The run therefore does not claim source-install coverage on those two hosts.

### 8.5 Closed findings trace

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
| Unix CAS rename callback failed Rust 1.88 lifetime generalization | Run 34947807569 diagnosed it; `a3d6d23` fixed it; run 34955324426 passed Ubuntu check and Linux/macOS builds |
| Unix compilation exposed a Windows-only fixture retry loop to strict Clippy | Run 34948934092 diagnosed it; `911304b` split cleanup by platform; run 34955324426 passed strict Clippy and all three test jobs |
| Quiet work could starve dynamic rendering; terminal/cancellation output had a timing-sensitive order | Run 34948934092 exposed the Unix PTY failures; `a8084d6`, `be625af`, and `b870c84` added pumping, one structured pre-terminal notice, and an explicit ordering barrier; all three PTY suites passed in run 34955324426 |
| Link completeness evidence used a tail scalar that could hide order sensitivity | `543b271` added a non-tail scalar witness; the full three-platform workspace suite passed in run 34955324426 |
| Product/IR documents retained contracts not matching the implemented manager | `4fff0dc` reconciled the durable contracts; `bf3394f` removed the last stale `init` example |

### 8.6 Milestone conclusion

The Cargo-like manager cutover is **complete for its stated milestone scope**. The remotely accepted production snapshot is `b870c84`, and all five jobs in run 34955324426 passed.

`bf3394f` is a documentation-only descendant: `git diff --name-status b870c84..bf3394f` reports only `docs/product/cli-experience.md`, and the diff changes two documentation lines. `f2a75f5` updates only this ledger. The present reconciliation changes only `docs/design/refactor-map.md` and this ledger; it does not modify Rust sources, manifests, lockfiles, tests, `.github/`, or `site/`. Consequently, these documentation changes do not invalidate the executable, workflow, or site evidence from the accepted snapshot.
