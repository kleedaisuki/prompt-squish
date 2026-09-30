# Opt-in persistent invocation tracing (v1.2)

## Contract

- Global `--trace[=events]`, `--trace=summary`, and `--trace=off` are presentation-independent
  bootstrap settings, excluded from `OperationRequest` and all semantic cache keys.
- `XMLSQUISH_TRACE=summary|events|1|true` is a captured-process environment fallback. Explicit
  `--trace=off` overrides it. Unknown environment values leave tracing disabled; unknown CLI
  values are usage errors. The default is disabled.
- Summary records contain operation identity, version, monotonic timings, and actual exit status.
  Events mode additionally preserves the existing versioned kernel event stream, including
  planning, compile, link, instantiate, backend, publication, cache outcomes, and diagnostics.
  Events can contain sensitive paths and diagnostic/query payloads. No network exporter exists.
- Each completed run gets `<workspace.target-dir>/metadata/traces/<invocation>.jsonl` within the
  canonical owning project. The default target directory is `target/xmlsquish`; custom manifest
  target directories are respected. Existing records remain readable across subsequent runs.
- Nonproject invocations and discovery failures use the current directory's default layout;
  this fallback is explicit and telemetry failure never hides the original command failure.
- Schema `xmlsquish.trace.v1` fields are `schema`, `invocation`, `kind`, `elapsed_us`, and `data`.
  Start/end records identify wall-clock start (`unix_ms`) and exit code; nested bootstrap phases
  carry monotonic duration microseconds. Protocol event invocation identity matches outer records.

## Lifecycle and safety

The host lazily migrates incompatible project-owned cache layouts. Opening a trace directly in
metadata before migration could make the host unlink an active file. Therefore the invocation
spools to `<project>/.temp/xmlsquish-traces/<invocation>.jsonl`, and after command completion copies
it using `create_new` into the effective metadata trace namespace, then removes the spool. This
also avoids large in-memory event buffers and works across filesystem volumes. Failed persistence
keeps the spool recoverable. Abrupt termination may leave a partial buffered spool; no claim of
crash-atomic telemetry durability is made. Clean can remove previous metadata traces, while its own
completed trace is published after clean finishes.

ProjectBuildLayout validates target-root aliasing. Directory creation independently rejects each
metadata/traces and spool component that is a symbolic link or a Windows reparse point. This is the
same non-hostile-concurrent-filesystem trust model as current project storage, not a guarantee
against adversarial path swaps. File creation never overwrites existing records.

Writes are serialized through one buffered file per invocation; processes never contend over a
shared JSONL append file. The first recorder error stops further recording and causes one final
`TRACE001` warning, not a different product or exit code. Renderer errors retain their established
failure semantics. Tracing does not change compiler input, artifact bytes, or cache identity.

## Startup and disabled-mode cost

Argument parsing now precedes environment snapshot allocation. Bare help, `--help`, `--version`,
and usage errors do not enumerate the entire environment or install runtime services. This is a
low-risk startup simplification, not a measured performance claim. Disabled tracing opens no file,
allocates no recorder, performs no trace clock reads, and retains the original event sink rather
than allocating a tee. Phase spans are allocation-free empty guards in disabled mode.

Accepted commands are traced from post-parse command setup through discovery, configuration,
service composition, kernel dispatch, and actual process outcome. Invalid CLI usage/help/version
are intentionally outside persisted tracing. The existing typed action events provide compiler
stage spans; frontend suboperations within a single compile action are not independently timed.
No startup speedup or tracing-overhead number is claimed without CI measurements.

## Verification

- `cargo test -p squish-cli --test cli_contract tracing_is_global_opt_in_and_not_part_of_the_operation --offline -j 1`
  passed locally on Windows (1 test, 17 filtered out; 2026-10-01). The complete CLI contract
  suite subsequently passed (18 tests); additional help coverage is included for CI.
- Added root telemetry tests for disabled zero-storage behavior, multiple-run preservation,
  failed exit status, phase completion, selective event recording, identity correlation,
  custom target directories, active-trace survival during simulated layout deletion, and
  Unix metadata alias rejection. Full root compilation/execution is delegated to CI per the
  user's local-resource constraint, not represented as a local pass.
- `rustfmt --edition 2024` completed on the owned source/test files.

## External grounding

OpenTelemetry separates trace identity, timed spans, and span events. This implementation adopts
those useful concepts while avoiding a heavyweight network-export SDK for a short-lived local CLI:
https://opentelemetry.io/docs/specs/otel/trace/api/

Sampling research highlights the uncertainty induced when selectively sampled traces are used to
estimate global performance. These explicit per-invocation modes are not random sampling; reports
must not infer workload-wide percentiles from users' manually selected traces:
https://www.usenix.org/conference/mad12/workshop-program/presentation/coehlo

TraceMesh explores streaming trace selection for cloud-scale high-dimensional workloads. That
problem is materially different from this project-local CLI; it is not justification for adding
collector infrastructure or probabilistic selection in v1.2:
https://arxiv.org/abs/2406.06975
