# Pack / SOPack v1.2 independent acceptance validation

Date: 2026-10-01. Owner: independent validation agent.

Current candidate status: complete hosted CI passed at `1870f3b`; see the final
candidate checkpoint below. Earlier failure records are retained as historical
evidence, not the current verdict. Release publication requires a later exact-tag
source gate and the complete native asset matrix.

Final delivery status: **v1.2.0 published** at tagged source
`85b68c04c88b6a6b8bce12f1b4e3e10a5a218366`. Exact main
[CI 36778366614](https://github.com/kleedaisuki/prompt-squish/actions/runs/36778366614)
and six-target [release 36779433911](https://github.com/kleedaisuki/prompt-squish/actions/runs/36779433911)
both passed. The [public release](https://github.com/kleedaisuki/prompt-squish/releases/tag/v1.2.0)
is neither draft nor prerelease and contains six native archives, `SKILL.md`
and `SHA256SUMS`. All seven manifest hashes were compared to GitHub's computed
asset SHA-256 digests; the downloaded small guide also matches tagged source
byte-for-byte. Native archives were not downloaded locally to save disk.

Pages [deployment 36778361865](https://github.com/kleedaisuki/prompt-squish/actions/runs/36778361865)
passed, and the public `/releases/1.2.0/` page returned HTTP 200 with the new
version and SOPack content. Final main raw performance evidence and honest
residual-overhead limits are recorded in `docs/performance/v1.2-final-main.json`
and `docs/performance/v1.2-pipeline.md`. The post-release documentation commit
does not retag or change the immutable shipped source.

## Basis and scope

Expected behavior comes from the requested v1.2 contract, the pre-existing
definition-site semantics in `docs/dsl.md`, and the compiler / manager design
agreement. Tests do not derive expected asset bytes from compiler internals.
All fixtures live under repository `.temp/pack-tests`; production code is not
modified by this validation work. Heavy Rust builds and tests must run in
GitHub Actions, not on the resource-constrained development machine.

Confirmed syntax: `xs:pack`, `xs:sopack`, `xs:asset path="..." name="..."`,
and `xs:include path="..." name="..."`. Assets default to their relative path;
includes default to the source stem plus `.prompt`. ZIP members use sorted
UTF-8 safe relative paths, STORED compression, and a fixed DOS date of
1980-01-01. A SOPack may declare macros and import modules, but never include
an entry or store a finished prompt.

## Acceptance matrix

| Contract | Independent process evidence in `tests/pack_process.rs` | Expected outcome |
| --- | --- | --- |
| Existing entry workflow | Build unchanged entry fixture | Exact existing minified prompt bytes |
| Pack binary assets | Include invalid UTF-8 and NUL bytes | Byte-for-byte archive asset preservation |
| Include entry | Include whitespace-heavy XML entry | Minified `.prompt` member, not XML source |
| Reproducible ZIP | Build equal trees in different absolute roots and creation orders, then rebuild | Identical complete archive bytes; sorted safe members and fixed timestamps |
| Include module forbidden | Pack attempts to include a legal module | Nonzero exit; no published pack |
| Include forbidden in SOPack | Direct and macro-body include | Nonzero exit; no published SOPack |
| Portable compiled library | Move SOPack into new project, delete producer source | Import and macro expansion still work |
| Definition-site asset ownership | Caller contains same relative asset name with different bytes | Macro expansion packs provider bytes, never caller bytes |
| SOPack contains no finished product | Inspect archive directory | No `.prompt` or `.pack` product members |
| Immutable dependencies | Add, mutate archive, build, remove | Drift is rejected without changing locked checksum; remove detaches alias |
| Failed dependency addition | Add invalid ZIP | Failure leaves manifest and lock unchanged |
| Opt-in telemetry | Default build, two traced builds, explicitly disabled build | Default creates no telemetry; traced records persist across invocations; disabled build preserves log |
| Asset cache input | Change only asset bytes after successful build | New bytes in next pack; no stale warm-cache product |
| Included-entry cache input | Change only included entry body after successful build | New prompt member in next pack; no stale warm-cache product |
| Include program isolation | Two included entries import different modules defining the same QName | Each entry uses its own standalone module closure |
| Distinct package source identity | Two dependencies both own `src/lib.xml` and `src/owned.bin` | Relocated SOPack retains both symbol/asset providers without path collision |
| Names are not package identity | Root and two aliased dependencies all named `library`, version `1.0.0`, with different source paths | Both dependency providers survive relocation despite identical name/version/path basenames |
| Formatted asset macro body | Module macro contains indentation, XML comment and processing instruction around asset | Only exact asset member emitted; authoring trivia does not reject packaging |
| Portable output paths | Traversal, rooted, and duplicate member names | Rejection before artifact publication |
| SOPack-root macro export | `[exports] main="src/main.xml"` points to SOPack root declaring a macro | Relocated consumer imports `pkg:library/main` and expands the root macro with provider-owned bytes |

The ZIP reader in the tests independently verifies local/central signatures,
STORED lengths, dates, time, absence of extra fields/comments, lexicographic
member order, local/central filename agreement, safe paths, central-directory
size and offsets, and bitwise-computed CRC32. Complete archive equality is
tested independently of those structural checks.

## Execution status

Source and prior verification documents inspected locally. Rust syntax formatting
and whitespace hygiene passed with:

```powershell
rustfmt --edition 2024 tests/pack_process.rs
git diff --check -- tests/pack_process.rs docs/verification/pack-sopack-v1.2.md
```

No Rust compilation or process acceptance test has been executed locally. The
requested hosted command is `cargo test --locked --test pack_process` (the
ordinary full root-package test suite also discovers this file).

Hosted [CI run 36770694172](https://github.com/kleedaisuki/prompt-squish/actions/runs/36770694172)
executed 13 process fixtures on Linux: **5 passed and 8 failed**. The inspectable
local log is `.temp/v1.2-ci/ninth-linux.log`; this run is not a passing acceptance
verdict. Three include-related failures report backend document ABI rejection;
the compiler owner is repairing that mismatch. Five remaining tests fail to
find expected products after successful commands. Source inspection establishes
the latter as a publication defect, not an expected-path test error:

- `Target::output_path` derives `.pack` / `.sopack`, and repository target
  resolution joins that name to the expected artifacts directory.
- The manager publishes those products as `ArtifactKind::Other("pack")` /
  `Other("sopack")`.
- `squish-publish::is_user_artifact` initially accepts only `Prompt`, `BinaryIr`
  and `DebugInfo`; `materialize_generation` filters using that predicate, so it
  skips materializing the newly supported products while returning success.

Manager and root owners were notified with these exact code paths. The test
harness now retains each project's last CLI stdout, stderr, arguments and exit
status in thread-local memory; a missing-product assertion reports those plus
actual artifact / metadata directory listings. This improves diagnosis without
adding fixture inputs or relaxing the public publication contract. A 14th
formatted-asset-macro fixture was subsequently added. All fixes and additions
require a new hosted run before acceptance can be approved.

## Settled root-export contract

The compiler owner confirmed that a SOPack root is import-compatible with a
module and owns macro definitions. A producer may map a public export directly
to the SOPack-root source in `[exports]`; callers import it via
`pkg:ALIAS/EXPORT`. `sopack_root_macros_are_publicly_importable_after_relocation`
tests that requested capability. The initial archive implementation checked
strict module-only exports; production owners were informed of the discrepancy
and must make archive validation agree with linker import compatibility.

## Same-name dependency contract basis

`sopack_retains_same_name_version_dependency_packages_and_root_without_identity_collision`
uses `add library --rename left --path left` and the equivalent right alias.
This is valid under existing resolution contracts: the CLI stores the requested
package name independently of its alias, `resolve_local` verifies that requested
name, and `install_candidate` distinguishes candidates by their `LockedSource`
identity and package name before assigning a package ID. Distinct local paths
must therefore not collapse merely because names and versions are equal.
The SOPack archive-local representation must preserve that distinction without
requiring producer checkout paths at consumption time.

## Hosted integration findings

The hosted checkpoint at commit `f01cf61` ran the complete native workspace
suite with `--no-fail-fast`: Linux reported 41 test binaries, 676 passing
tests and eight failing tests, all in the new pack process suite. The MSRV,
strict Clippy, architecture, release metadata and documentation gates passed.
Evidence: [Actions run 36769251446](https://github.com/kleedaisuki/prompt-squish/actions/runs/36769251446).
This is a diagnostic checkpoint, **not** a release approval.

The process tests revealed three integration assumptions that must not return:

1. XML single-document-element validation belongs to entry/prompt semantics,
   not to directive-only archive assembly. Archive evaluation still needs full
   arena and provenance validation, even when it produces no document element.
2. A SOPack's executable root needs its own source origin even when its body is
   empty or follows macro definition regions. Definition origins cannot stand
   in for the actual packaging root.
3. A resolved package's display name is not its source-provider identity.
   Disambiguated same-name providers need an explicit exact source identity in
   the frontend context; accepting string prefixes is not an ownership check.

An earlier hosted checkpoint also exposed duplicated frontend ABI strings in
production and test runtime descriptors. The frontend now exports the ABI
authority rather than requiring adapters to copy its spelling. These fixes
must be exercised by a later full hosted suite before the final verdict.

## Final candidate checkpoint

[CI run 36777315965](https://github.com/kleedaisuki/prompt-squish/actions/runs/36777315965)
passed for `1870f3b81024735c7e8482403bb217f87a51e904` with the actual shipping
ThinLTO profile and prompt-only cache-output simplification. MSRV 1.88, strict
Clippy, formatting, architecture, release metadata, doctests, all three native
platform jobs, composed/source-installed CLI smoke, checked-in real-CLI site demo,
site/browser checks and complete standard performance measurement passed.

Linux reported 41 test binaries, **698 passed and zero failed**, including all
14 independent pack process fixtures. Windows and macOS full native jobs also
passed, including archive/SOPack smoke. The old prompt process and skills/recovery
suites remained passing. The site demo was refreshed from hosted CLI output,
with unchanged finished prompt bytes and action counts; hashes were not invented.

Raw final candidate measurement is retained in
`docs/performance/v1.2-shipping-hosted.json`; its methods and limitations are in
`docs/performance/v1.2-pipeline.md`. This checkpoint proves candidate workflows,
not that a GitHub release has already been published. Final main/tag verification
and native release publishing are tracked in the repository Actions history.

## Remaining coverage limits

Cross-device portability is approximated by separate absolute project roots
and removal of the producer tree. Actual operating-system portability needs
the CI platform matrix. Concurrency, crash publication, cache-schema migration,
and archive resource-exhaustion limits need separate focused evidence; a
successful ordinary process workflow is not proof of those properties.
