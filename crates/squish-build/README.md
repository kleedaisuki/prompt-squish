# `squish-build` scheduling model

`squish-build` is the pure planning and synchronous scheduling domain. It does
not run processes, create threads, print, or depend on a concrete CAS/database.

## Core invariants

- `BuildPlan` is immutable after validating unique action IDs, complete edges,
  acyclicity, action-local output-name uniqueness, and typed input references.
- Dependency edges express readiness. They may be **order-only** and therefore
  never automatically affect an action key.
- `KeyRecipe` contains an ordered list of `InputRef::Blob` and
  `InputRef::Output` values. A final `ActionKey` is materialized only after every
  referenced named output has a content digest. Predecessor action keys and
  unreferenced outputs are excluded, enabling early cutoff.
- Output names and kinds form pure action output schema. User-visible
  publication destinations are deliberately outside the action key.
- The ready queue is ordered by `ActionId`. CPU slots, I/O slots, and memory
  bytes are reserved together and never oversubscribed.
- Equal materialized keys use single-flight. Every logical action still receives
  its own `ResultAvailable` event and output view.
- A failure blocks descendants through explicit direct-dependency links. With
  keep-going enabled, independent actions remain runnable; without it,
  not-yet-running independent actions are cancelled while already-running work
  is allowed to report its real outcome.
- Cooperative cancellation stops dispatch immediately, terminalizes queued
  actions, and lets the host acknowledge stopped leaders before their reserved
  resources are released and their single-flight members become cancelled.

## Host/store boundary

The `BlobStore`, `ActionIndex`, and `ArtifactPublisher` traits are ports, not
implementations. A host can query `ActionIndex` with `Dispatch::key` and finish
through `Scheduler::complete_cached`, or execute a worker and call
`Scheduler::complete`. Workers return only `ActionResult` and structured events.

Concrete CAS, SQLite, filesystem publication, terminal color, and event protocol
rendering belong to their respective domains rather than this crate.
