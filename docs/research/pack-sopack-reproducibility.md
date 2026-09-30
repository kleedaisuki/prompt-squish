# Pack/SOPack: portable compilation and reproducible packaging

Date: 2026-10-01. Status: engineering research and acceptance constraints.
Related: [IR model](../design/ir-model.md), [compiler prior art](compiler-manager-ir-prior-art.md).

## Scope and evidence

The new distribution units must not turn a compiler object into a rendered prompt,
nor turn an immutable library into a mutable extracted source checkout. Existing
relocatable unit IR and content-addressed project storage are the starting point.
This document separates upstream evidence from project-specific inference.

| Evidence | What it supports | Project implication |
| --- | --- | --- |
| [Reproducible Builds archive metadata guidance](https://reproducible-builds.org/docs/archives/) | Archive order, timestamps, permissions, owner metadata and ZIP extra fields are environmental inputs. | Sort portable member names; fix timestamps and modes; omit variable extra fields/comments. Deterministic contents alone are insufficient. |
| [LLVM LTO architecture](https://llvm.org/docs/LinkTimeOptimization.html) | Reusable compiler representations can remain available until cross-unit optimization. | Do not render library bodies during packaging; preserve unresolved symbols and macro operations. |
| [ThinLTO documentation](https://clang.llvm.org/docs/ThinLTO.html) | Summary-based linking avoids loading all bodies in the serial coordination stage. | Prefer source-neutral unit summaries and lazy immutable payload access over a second SOPack-only linker. Do not copy distributed LTO machinery without a workload. |
| [Build Systems a la Carte, ICFP 2018](https://www.microsoft.com/en-us/research/publication/build-systems-la-carte/) | Scheduling and rebuilding are distinct decisions with explicit dependency models. | Asset bytes and archive identities must be declared cache inputs, not undeclared reads during emission. |
| [An Empirical Study on Reproducible Packaging in Open-Source Ecosystems, ICSE 2025](https://www.cs.cmu.edu/~ckaestne/pdf/icse25_rb.pdf) | Reusable component packaging has reproducibility problems across six ecosystems. | Verify complete archives byte-for-byte after relocation and fresh builds, rather than trusting a deterministic serializer alone. |
| [OpenTelemetry trace API](https://opentelemetry.io/docs/specs/otel/trace/api/) | No-op tracers and explicit spans let observability be suppressed without changing domain work. | Disabled telemetry must avoid opening trace files, collecting full event payloads or influencing action identities. Persistent traces belong to metadata, never to product ZIPs. |

The 2025 empirical work is peer-reviewed; the upstream manuals establish production
contracts. None proves that a particular xmlsquish optimization is fast. Performance
claims require measurements on the actual compiler and CLI.

## Four-layer risk analysis

1. **Data structures:** one provider-neutral compiled-unit value, a portable source
   identity, and assets identified relative to the defining source. An archive
   contains immutable objects and the exact referenced bytes, not physical paths.
2. **Special cases:** resolve local files and SOPack members before the linker;
   both become the same typed closure. The caller's working directory must never
   determine a macro definition's asset lookup.
3. **Complexity:** keep archive writing narrow and deterministic. No clocks,
   filesystem enumeration order, host usernames, absolute checkout paths or
   telemetry IDs may enter the wire representation. Avoid eagerly evaluating
   parameter-dependent macro bodies merely to advertise optimization.
4. **Destructive analysis:** invalid ZIP member paths, duplicate/case-colliding
   output names, unsupported IR versions, missing members, digest drift and
   malformed source maps fail before publication. A failed build preserves the
   previous published generation. The authorized compatibility break covers old
   internal cache schemas, not unrelated CLI commands or skill installation.

## Discriminating checks

| Test | Distinguishes |
| --- | --- |
| Build a pack twice from fresh action caches and compare all ZIP bytes. | Reproducibility versus merely a warm cache returning identical data. |
| Copy a SOPack to another root, delete its authoring source, then compile a consumer. | Portable library versus an archive retaining hidden checkout dependencies. |
| Define a macro with an asset, call it from a different directory containing a conflicting file. | Definition-relative identity versus caller-relative lookup. |
| Mutate only asset bytes without touching XML. | Correct cache invalidation versus stale product reuse. |
| Import a module through an include; include within SOPack. | Typed unit restrictions versus accidental generic XML concatenation. |
| Malform member names, source identities or payload digests. | Validated immutable object boundary versus unsafe archive extraction. |
| Enable and disable telemetry around the same build. | Observability side data versus changes in product semantics. |
| Compare small cold/warm CLI runs separately from large asset/macro cases. | Startup constants versus asymptotic or long-tail improvement. |

## Performance interpretation

Measure process startup, planning/source acquisition, frontend, middle-end, linking,
instantiation and backend/publication separately where instrumented. Record compiler
commit, binary identity, runner environment, repetitions and raw samples. Use the
same benchmark corpus for baseline and candidate. New archive functionality has no
old-version equivalent: report absolute cost and scaling, not a fabricated speedup.
Do not extrapolate a macro microbenchmark win into end-to-end startup improvement.
Archive compression is a policy choice: stored ZIP members trade size for stable,
low-constant-factor writing; compression would require pinned deterministic encoder
semantics and separate measurement.
