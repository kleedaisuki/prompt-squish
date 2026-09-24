# Compatibility-led refactor and governance review (2026-09-24)

## Decision boundary

The object of this review is the current v1.0.4 workspace, not the historical
compiler-to-manager migration. The product contract is held fixed: XML DSL
semantics, command-line grammar and exit behavior, NDJSON and terminal output,
XSIR/debug/product bytes, publication layout, and site UI. Internal code may
change only where a focused regression test and the existing full CI matrix can
exercise the claimed equivalence. This is a maintenance milestone, not a new
feature or a cache-format revision.

The dominant use case is a manifest-backed project build. Our tactical design
rule is to express one domain rule in one place, but not to merge independent
trust boundaries just because their checks look similar. This follows the
accepted [manager/IR architecture](../adr/0009-microkernel-manager-and-reusable-ir.md)
and the [project-local storage decision](../architecture/target-xmlsquish-layout.md).

## Findings and decisions

| Area | Observation | Decision and compatibility argument |
| --- | --- | --- |
| Scheduling | Worker completion and cancellation repeated flight retirement and resource return. | One retirement method now owns that transition; queued cancellation is shared. A repeated completion must not release resources twice. Action ordering and events stay unchanged. |
| Dependency edits | `add`/`remove` reparsed the original manifest after `CandidateManifest::parse` had validated it. | Retain that semantic snapshot for the single edit, while validating the edited candidate anew. Later transaction/resolution checks have different trust boundaries and remain. |
| IR and linking | Consumers repeatedly matched `Entry`/`Module` merely to read common fields. | Expose read-only common views on `RelocatableUnitIr`; preserve actual kind-specific logic and the unchanged persistent encoding. |
| Resolution | Explicit paths and workspace members had duplicate local edge insertion. | Normalize both to a manifest-relative local path before one resolution/edge path; preserve source identity and error distinctions. |
| CAS and publication | Byte and stream CAS writes had separate publication/event policy; Windows exclusive rename had a second unsafe implementation; portable-path alias logic was duplicated. | Share the staged-blob commit path and existing platform rename primitive, and centralize component-boundary-aware alias comparison. Keep digest verification, no-clobber semantics, and recovery checks. |
| Protocol and backend | One ID manually duplicated the established ID generator; whitespace metrics rescanned all output gaps. | Reuse the ID generator without changing JSON string encoding; count gaps in the emitting pass without changing product bytes or provenance. |
| Host, source, kernel, presentation | Similar-looking checks mostly guard distinct lifecycle, identity, or error boundaries. | Do not abstract them merely to reduce line count; the compatibility risk exceeds the demonstrated benefit. |
| Site CI | Browser contracts and checked-in demo consistency existed locally but were absent from CI; demo passed a removed configuration key. | Test the real generated site in Chromium and regenerate the demo with the current CLI. No UI or published route changes. |

No broad removal of security checks was justified. In particular, digest and
size verification, path confinement and alias rejection, atomic no-replace
publication, and publication recovery serve explicit corruption, concurrency,
or workspace-escape models. The removed duplication was *implementation*, not
an independent validation boundary. The Windows EFS fallback has injected
coverage, but a real EFS host remains an untested environment for this review.

## External calibration

Production precedent: [Cargo's workspace contract](https://doc.rust-lang.org/cargo/reference/workspaces.html)
shares one lockfile and target directory, and [`cargo test`](https://doc.rust-lang.org/cargo/commands/cargo-test.html)
does not implicitly select every member of a non-virtual workspace; our CI
therefore names `--workspace --all-targets --all-features --locked` explicitly.
[GitHub Actions' workflow syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax)
supports the OS matrix used to catch platform-specific filesystem behavior.
These are mature operational contracts, not proof that our particular code is
correct.

Research perspective: [*Build Systems à la Carte* (Mokhov et al., ICFP 2018)](https://doi.org/10.1145/3236774)
separates scheduling, rebuilding, and task description; our changes preserve
those boundaries rather than introducing a second build route. Recent work on
[correct-by-construction incrementalization (DeCo, PACMPL 2026)](https://doi.org/10.1145/3798264)
shows stronger formal guarantees are possible when a typed change calculus is
available, but that result does not automatically apply to a filesystem-backed
CLI and is not a reason to add such machinery here. Our practical equivalence
evidence is regression tests plus the cross-platform composed binary and site
checks, not a formal behavior-preservation proof.

## Verification and follow-up

The local sequence is format, architectural-edge check, Rust quality and full
workspace tests at MSRV 1.88, release metadata checks, site type/build/browser
tests, and demo regeneration. GitHub Actions runs the Rust matrix on Linux,
Windows, and macOS and the site job on Linux. Record the exact run and result
alongside the final milestone rather than treating local tests as cross-platform
evidence.

One separate observation is intentionally deferred: executable bootstrap
captures an environment snapshot for credentials/fault controls, while config
override loading reads the process environment. Changing that timing requires a
dedicated contract test and was not part of this behavior-preserving refactor.
