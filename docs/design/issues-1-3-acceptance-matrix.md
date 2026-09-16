# Issues #1–#3: Acceptance and Test-Evidence Matrix

## Purpose and decision boundary

This document turns the checklists in [issue #1](https://github.com/kleedaisuki/prompt-squish/issues/1),
[issue #2](https://github.com/kleedaisuki/prompt-squish/issues/2), and
[issue #3](https://github.com/kleedaisuki/prompt-squish/issues/3) into observable behavior and
executable evidence. It is an acceptance ledger, not another implementation plan: API names such
as `BuildRuntimePorts` are illustrative, and the implementation may use one aggregate port or a
few cohesive typed ports. The implementation investigation for #1 lives separately in
[`../research/issue-1-runtime-port-map.md`](../research/issue-1-runtime-port-map.md).

The three issues are related but have different success conditions:

- **#1** is accepted when the manager orchestrates an injected build runtime rather than composing
  production adapters.
- **#2** is accepted when logical artifact identity is the caller contract and only the
  publisher/catalog adapter understands physical generations.
- **#3** is accepted when CI deterministically rejects an unreviewed internal direct dependency.

A green end-to-end build alone cannot prove #1 or #3, and a source grep alone cannot prove #2.
Each row below therefore names the lowest-cost evidence that can falsify its claim.

## Evidence levels and notation

| Level | What it proves | Typical command or location |
| --- | --- | --- |
| **S — structural** | Ownership and dependency direction | `cargo metadata`, manifest/source boundary assertions |
| **U — focused** | One port contract or invariant under deterministic inputs/faults | Crate unit/integration test |
| **I — composed integration** | Production adapters are wired through the intended seam | `squish-host` integration test |
| **P — process** | User-visible CLI behavior and real filesystem/process recovery | Root `tests/process.rs` and `tests/recovery_process.rs` |
| **D — documentation** | The reviewed prose points to the executable source of truth | Link and stale-text checks during review |

`Required` below means required to close the issue. `Regression` means existing evidence that must
remain green; it is not, by itself, proof of the new boundary.

## User-visible artifact locator contract

Issue #2 becomes testable only after “stable way to locate” is defined. The product contract is:

1. Every successfully committed, user-declared artifact has exactly one **logical locator**. The
   locator is normalized, project-relative, independent of the target hash and generation hash,
   and unique within the build result. A typical locator is
   `target/xmlsquish/chat.prompt`.
2. The machine build result carries that locator as a typed field; it is not reconstructed from an
   artifact URI. The immutable content digest and any adapter-readable location are separate
   facts. Because compatibility with the old representation is not required for this change, a
   protocol/schema correction is preferable to retaining an ambiguous `uri` field.
3. Normal and short human output display the logical locator and never display or parse
   `.squish-publish`, target hashes, generation hashes, journals, or `current` manifest paths.
4. The displayed locator can be copied verbatim into
   `xmlsquish inspect artifact <LOCATOR>`. Inspection resolves the current committed artifact
   through the typed catalog query and verifies the returned bytes/digest. It does not join or
   parse a publisher-owned path.
5. Rebuilding the same target with changed content preserves the logical locator while changing
   the digest and generation as appropriate. The locator subsequently resolves to the new current
   generation; an immutable artifact ID still resolves to the exact historical identity wherever
   that identity-based query is supported.
6. Two selected artifacts may not silently claim the same logical locator. Planning must either
   assign distinct locators or reject the collision before publication. Last-writer-wins is not an
   acceptable resolution.
7. A logical locator is not necessarily a directly readable operating-system path. If the product
   promises a materialized file, the publisher must create that stable path atomically. Otherwise
   the CLI must call it an artifact locator/selector and offer the inspection/read path above; the
   renderer must not print a nonexistent path as though it were a file.

This follows the useful separation seen in production build tools: Cargo exposes typed JSON
`compiler-artifact` messages to tools rather than asking them to scrape terminal output, while
`cargo metadata --format-version 1` supplies a versioned graph representation. See the official
[Cargo external-tools contract](https://doc.rust-lang.org/cargo/reference/external-tools.html) and
[`cargo metadata` reference](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html).

### Executable locator scenarios

| ID | Scenario | Required assertions | Best evidence |
| --- | --- | --- | --- |
| **L1** | First successful build emits prompt, IR, and debug artifacts | Every declared product has a non-empty normalized logical locator; locators are unique; the machine result associates locator, kind, artifact ID, digest, and size | U + P |
| **L2** | Rebuild after source content changes | Locator set is unchanged; relevant digest changes; ordinary output contains no generation path/hash; inspection by the old displayed locator returns the newly committed artifact | P |
| **L3** | Warm cached rebuild | Locator set and inspection behavior equal the cold build; cache reuse does not substitute a CAS URI for the logical locator | U + P |
| **L4** | Two targets would produce the same locator | Failure occurs during planning, before either generation becomes current, and diagnostic identifies both conflicting domain owners and the locator | U |
| **L5** | Catalog adapter returns opaque non-filesystem locations | Manager and presentation still report the supplied logical locator and can inspect it; no code assumes separators, hexadecimal IDs, or a `.squish-publish` prefix | U + I |
| **L6** | Crash before and after publication commit decision | Before-decision death preserves the previous locator-to-generation mapping; after-decision recovery selects exactly one complete, digest-verified generation; the locator never resolves to a mixed set | P |
| **L7** | Missing/corrupt current catalog entry | Absence is reported as not found; corruption is a typed operation failure; neither case falls back to scanning or guessing generation directories | U + P |

## Issue #1 — inject build runtime ports

| Original checklist item | Executable behavior / invariant | Required evidence and assertion | Regression evidence that remains green |
| --- | --- | --- | --- |
| Manager no longer opens CAS, action index, or publishers | Opening a build obtains an invocation-scoped runtime through an injected service. No manager path creates production storage handles, chooses storage roots, or installs publication observers. | **S1:** the final internal-edge policy has no `squish-manager -> squish-store` or `squish-manager -> squish-publish` normal edge. **S2:** a boundary check/review finds no construction/open call for those adapters in `squish-manager`, and no direct invocation of the production frontend, linker, instantiator, or backend implementation. Dependencies used only for neutral semantic request/result types are not a composition violation. | Workspace build and manager build tests |
| `BuildExecutor` does not name concrete `Cas`, `VerifiedActionIndex`, or `FileArtifactPublisher` | Executor state contains prepared domain data, invocation-local intermediate state, and typed runtime port(s) only. | **S3:** compile/source boundary assertion rejects those concrete symbols in the executor module. Prefer absence of the concrete crate dependencies over a grep as the durable proof. | Existing plan/execution tests |
| Production implementations are assembled by host/root | One production runtime owns mutually consistent blob store, action index, target publisher, catalog publisher, and selected compile/link/instantiate/backend implementations. It is created by `squish-host` or the executable composition root and handed inward. | **I1:** `squish-host` composition test obtains a runtime and executes representative blob, cache, toolchain, and both publication-space operations without manager-side reopening. **S4:** dependency direction is adapter → port owner, never manager → adapter. | Root CLI smoke tests |
| Tests inject deterministic in-memory/faulting implementations through the same ports | Manager tests use the public runtime seam used by production. The fake records calls and can fault blob read/write, action lookup/record, each toolchain stage, target publication, and catalog publication independently. | **U1:** a complete build with an in-memory runtime proves call order and published membership. **U2:** one representative fault in each capability family becomes the expected typed action/finalization failure. No test-only executor bypass is accepted as proof. | Existing semantic fixtures may delegate to real pure compilers |
| Determinism, cache verification, cancellation, event ordering, and crash-safe publication remain covered | Port inversion does not move action-key policy, cache eligibility, digest/schema validation, cancellation decisions, kernel event emission, or publication finalization into adapters. Runtime adapters emit no kernel events. | **U3:** identical frozen input + runtime descriptor gives identical plan/action keys; changing the relevant descriptor invalidates the relevant keys. **U4:** corrupt/mismatched cache bytes are rejected. **U5:** blocking/faulting runtime preserves cancellation and ordered terminal events. **P1:** real process-death recovery suite remains green. | `crates/squish-manager/tests/core.rs`, build/cache tests, `tests/process.rs`, `tests/recovery_process.rs` |
| Dependency contract/ADR updated if ownership differs | Normative prose names the actual port owner and composition root and links to executable graph policy. Historical notes no longer claim manager-owned publisher layout knowledge is an accepted current limitation. | **D1:** review links in ADR-0009, manager architecture, and `refactor-map.md`; every “current owner” path exists and agrees with manifests/policy. | Documentation link check if available |

### Focused #1 test matrix

| ID | Injected condition | Observable assertion |
| --- | --- | --- |
| **R1** | Deterministic runtime, cold build | Each planned capability is called with the sealed inputs; target publication occurs only after required transforms; catalog finalization records the terminal facts. |
| **R2** | Same runtime reused for warm build | Verified transform results are restored, effectful publication policy is unchanged, and every restored byte/digest/schema is revalidated. |
| **R3** | Runtime returns bytes that do not match requested digest | Build fails with the cache/storage integrity diagnostic; downstream work is blocked and no target generation is committed. |
| **R4** | Runtime compile/link/instantiate/render fault | Exactly the owning action fails, dependency blocking and keep-going follow the existing plan, and finalization still receives truthful terminal facts. |
| **R5** | Cancellation arrives while a runtime call is blocked | Orchestrator remains the sole lifecycle authority; emitted events form a kernel-valid sequence and committed publication is not retroactively reported cancelled. |
| **R6** | Target publication commits but catalog publication faults | The target generation is recoverable/adoptable on the next planning pass; it is never half visible. |
| **R7** | Runtime descriptor changes while source bytes do not | Only recipes that semantically depend on the changed toolchain identity miss; unrelated inputs do not acquire accidental nondeterminism. |

## Issue #2 — typed catalog queries and hidden publisher layout

| Original checklist item | Executable behavior / invariant | Required evidence and assertion | Regression evidence that remains green |
| --- | --- | --- | --- |
| Manager does not construct, parse, or validate publisher-internal stable/generation URI layouts | Manager exchanges typed target identities, logical locators, generation descriptors, and bytes. Physical URI validation/recovery stays inside the production publisher/catalog adapter. Presentation also consumes the logical locator directly instead of reverse-parsing an artifact URI. | **S5:** no `.squish-publish`, `generations`, target-key hashing, current-manifest path construction, or generation URI parser outside publisher/catalog adapter code. **U6/L5:** fake returns opaque URIs and non-hex IDs successfully. | Manager domain validation of target/artifact membership and digests must remain |
| Inspection accepts logical identity and resolves through a typed port | A caller gives project + logical locator (and, internally, a typed target/artifact identity where needed). The catalog answers absent, unique current artifact, or typed ambiguity/corruption; the manager never scans directories. | **U7:** fake catalog records the typed query and returns an artifact whose immutable ID differs from the locator. **P2:** build output locator copied verbatim to `inspect artifact` succeeds. | Existing selector/result-shape tests in protocol and manager inspect suites |
| Recovery stays crash-safe and digest-verified | Catalog recovery yields no mixed generation, verifies manifest membership plus bytes/digests, and distinguishes absence from corruption. | **U8:** corrupt manifest/blob cases fail without fallback. **P3/L6:** process kill at meaningful durable boundaries either retains the previous complete generation or recovers the new complete generation. | Publisher unit recovery tests and `tests/recovery_process.rs` |
| Content-derived target/generation hashes need not be known by callers | Hashes may exist in adapter state but do not appear in query input, normal/short output, or locator construction. Changing them does not change lookup behavior. | **U9:** use two 64-hex target/generation IDs behind one logical locator and assert the caller supplies neither. **P4:** normal/short output excludes internal layout; trace output may expose evidence only as explicitly documented. | Human disclosure tests |
| Successful build exposes an unambiguous stable locator for every declared artifact | The locator contract and L1–L4 above hold for cold, warm, and changed-content builds. | **P5:** composed process test parses the machine result, checks one-to-one locator coverage, copies each locator into inspection, and verifies the returned artifact ID/digest. **U10:** duplicate locator is rejected before commit. | Existing build result/published membership tests |
| Coupling notes and manager docs are resolved/updated | Documentation describes the typed boundary, not a known manager-layout exception. | **D2:** `refactor-map.md` and manager architecture contain no unresolved exception and point to the typed port plus tests. | N/A |

### What #2 does *not* require

- It does not require exposing generation directories as a public ABI.
- It does not require a symlink or copied “pretty path” on every platform if inspection/read by a
  stable logical locator is the documented product behavior.
- It does not require the manager to trust adapter metadata. Digest, size, schema, target
  membership, and logical-locator uniqueness remain manager/domain validation.
- It does not permit presentation code to become a second layout parser merely because the issue
  checklist names manager code. The publisher/catalog boundary owns all layout interpretation.

## Issue #3 — executable crate-edge policy

| Original checklist item | Executable behavior / invariant | Required evidence and assertion |
| --- | --- | --- |
| CI extracts workspace internal direct-dependency graph | Checker invokes `cargo metadata --format-version 1` and derives edges only when both endpoints are workspace members. Dependency package/path identity, not the local rename, identifies the destination. | **A1:** run `python scripts/check_architecture.py` at repository root; it succeeds against current locked metadata. Focused parser tests include renamed and external dependencies. |
| Every internal edge is checked against committed policy | Normal, development, and build kinds are explicit; target-specific declarations cannot disappear merely because the CI host does not activate that target. Third-party dependencies are ignored. Unexpected and stale allowlisted edges both fail, keeping prose and policy honest. | **A2:** checker compares actual and allowed sets in both directions; the committed policy is a reviewed repository file. A stale-edge fixture is a useful strengthening, but the issue's required negative fixture is the forbidden-edge case in A3/A5. |
| Forbidden acyclic edge fails with endpoints | A syntactically valid `core -> adapter` edge exits non-zero and diagnostic contains source, dependency kind, and destination. The diagnostic does not require rendering a graph. | **A3:** black-box fixture test asserts exit `1` and the complete forbidden edge text. |
| Current intended graph passes on Linux without graph tools | Python standard library + Cargo are sufficient; no Graphviz/network/runtime installation is used. | **A4:** Ubuntu CI runs focused checker tests and the real policy check before ordinary Rust tests. |
| Focused test/fixture demonstrates rejection | Fixture represents an acyclic forbidden edge, preferably including rename/optional/target data so resolution is not name-based. | **A5:** `python -m unittest discover -s scripts/tests -p "test_*.py"` passes and separately verifies a rejecting subprocess. |
| ADR-0009 links executable policy | The ADR says the checked-in edge file/checker is normative; a conceptual diagram remains explanatory only. | **D3:** links resolve to the policy and checker, and CI invokes the same default policy rather than a copied list. |

### Required #3 edge cases

| Case | Expected result |
| --- | --- |
| Internal normal edge present in policy | Pass |
| Internal dev/build edge listed under the wrong kind | Fail with actual forbidden edge and stale expected edge |
| Renamed internal dependency | Resolve to the workspace package name and check normally |
| Target-specific internal dependency inactive on Linux | Still included and checked as a declared edge |
| Third-party dependency with the same package name as a workspace crate | Ignore unless its resolved path/package identity is the workspace member |
| Policy duplicate, malformed line, unknown kind, or unsupported metadata version | Exit as checker/configuration error, distinct from policy mismatch |
| Policy contains an edge removed from manifests | Fail as stale allowlist entry |

## Cross-issue release gate

The issues should be closed only when all of the following commands pass from a clean checkout:

```text
python -m unittest discover -s scripts/tests -p "test_*.py"
python scripts/check_architecture.py
cargo test -p squish-manager --tests --locked
cargo test -p squish-host --tests --locked
cargo test -p squish-publish --locked
cargo test --workspace --all-features --locked
```

In addition, the root process tests must explicitly exercise L1–L7 where the scenario requires a
real process. A final structural review must confirm that the *post-refactor* edge allowlist is
committed. Running #3 against the pre-refactor graph and blessing manager-to-adapter edges would
make #3 green while #1 remains false.

### Traceability summary

| Issue | Structural proof | Focused proof | Composed/user proof | Documentation proof |
| --- | --- | --- | --- | --- |
| #1 | S1–S4 | U1–U5, R1–R7 | I1, P1 | D1 |
| #2 | S5 | U6–U10 | P2–P5, L1–L7 | D2 |
| #3 | A1–A5 | Fixture/self-tests | Ubuntu CI real-graph run | D3 |

## Over-design and false-confidence risks

| Risk | Why it is harmful | Acceptance guardrail |
| --- | --- | --- |
| One trait object per trivial operation or runtime-loaded plugins | Adds lifetime/error/composition machinery without serving any issue criterion | Start with one cohesive, statically composed runtime or a small number of capability groups; split only on demonstrated independent lifetimes |
| A high-level `execute(BuildWork)` host callback | Moves orchestration into the adapter and creates a second executor | Manager retains planning, validation, cache policy, lifecycle, and finalization decisions |
| Making generation URI the logical locator | Re-exports the physical layout and repeats the original defect | Logical locator is an explicit typed fact and survives generation change |
| Teaching the renderer to parse physical URIs | Hides the leak from normal output but preserves architectural coupling | L5 uses an opaque URI; presentation receives the locator directly |
| Creating symlink/copy mirrors solely to make printed paths “real” | Adds cross-platform and crash-consistency surface without proving user need | Use the typed locator + inspect/read contract unless direct files are an explicit product requirement |
| Only grepping for forbidden concrete type names | Aliases or wrapper constructors evade the check | Dependency edge removal plus host composition test is the durable proof; grep is only a diagnostic aid |
| Allowlisting every new edge automatically | Converts the architecture checker into a graph snapshot with no policy | Changes to policy require review against ADR ownership rules; stale edges fail too |
| Checking only the resolved Linux graph | Misses declared target-specific coupling | Read package dependency declarations from metadata and test a target-specific fixture |
| Exhaustive syscall fault combinations | Large brittle suite with little additional semantic coverage | Test distinct commit states and capability families, then keep real process-death recovery for the production adapter |
| Snapshotting entire human transcripts | Cosmetic wording changes obscure locator and leak regressions | Assert semantic locator coverage, copyability, uniqueness, and absence of internal layout tokens |

## Acceptance judgment rule

Treat a criterion as complete only when its required evidence exists and passes. If a behavior is
covered only by a mock, label it **inferred for production** until host/process evidence exists. If
an end-to-end test passes but the structural check still shows manager-to-adapter ownership, #1 is
not complete. If logical lookup works but output still exposes or reverse-parses generation paths,
#2 is not complete. If the real graph passes but no rejecting fixture exists, #3 is not complete.
