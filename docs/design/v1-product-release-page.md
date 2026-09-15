# xmlsquish 1.0 Product Release Surface: Implementation and Evidence Record

- **Status:** release preparation complete; publication gate pending
- **Evidence date:** 2026-09-15
- **Audience:** product, site, release, and validation owners
- **Product routes:** `/releases/` (Simplified Chinese) and `/en/releases/` (English)
- **Machine route:** `/releases/1.0.0.json`
- **Prepared version/tag:** `1.0.0` / `v1.0.0`
- **Exact validated site commit:** `8a2be527beb4091beba6477d3b7a870436ffc691`

This file records the implemented release surface and the evidence supporting it. It is not the
release page itself. The public routes are deliberately **product launch pages**, not documentation
pages: they lead with the user outcome, present one six-command lifecycle, route users to an
installation choice, and keep migration detail subordinate to the product story.

Product and technical authority remain in the
[project-manager scope](../product/project-manager-scope.md),
[CLI experience](../product/cli-experience.md),
[ADR 0009](../adr/0009-microkernel-manager-and-reusable-ir.md),
[ADR 0010](../adr/0010-transactional-new-project-creation.md), and the
[execution ledger](project-manager-execution-status.md).

## 1. Product decision and implemented journey

The launch claim is:

> **xmlsquish is the project manager for XML prompts: create, format, resolve, build, and inspect
> reproducible prompt artifacts with one command-line tool.**

The narrative pivot is from compiler to project manager. Both localized pages identify xmlsquish
as an XML prompt project manager and expose the same lifecycle:

```text
new → fmt → add/remove → build → inspect
```

The page presents all six direct commands—`new`, `fmt`, `build`, `add`, `remove`, and `inspect`—as
one journey rather than unrelated feature tiles. The copyable, zero-network path is intentionally
separate from the dependency-management demonstration:

```bash
xmlsquish new support --vcs=none
cd support
xmlsquish fmt --check
xmlsquish build --offline
xmlsquish inspect artifact target/xmlsquish/prompt.prompt --format=json
```

This avoids the false implication that a newly added remote dependency is available to an offline
build. The page also shows the canonical scaffold (`xmlsquish.toml`, `src/prompt.xml`), the
published `.prompt`, and the inspectable artifact model. Migration copy preserves the 0.3 XML DSL
while making the intentional command-first/project-first boundary explicit. `new` requires a
nonexistent destination; adopting an existing populated directory (`init`) is not promised for
1.0.

## 2. Release state model

Two states must not be collapsed:

| State | Meaning | Current status |
| --- | --- | --- |
| **Release preparation complete** | Versioned product UI, localization, machine discovery, candidate install, tests, release automation, and intended asset metadata are ready. | **Complete** |
| **Publication gate** | Immutable `v1.0.0` tag exists and the GitHub Release contains all six native archives plus a verified `SHA256SUMS`. | **Pending** |

The machine metadata therefore says `releaseStatus: "prepared"` and describes archive locations as
`intendedUrl`. A prepared URL is not proof that an asset exists. The product page renders six
disabled, non-link controls and explicitly labels them pending. It does not expose a working
download affordance before publication.

The current `SoftwareApplication` JSON-LD is also deliberately candidate-safe:

- exactly one release-page entity;
- `softwareVersion: "1.0.0"`;
- repository identity through `sameAs`;
- `creativeWorkStatus: "release candidate"`;
- no `downloadUrl`, `Offer`/`offers`, rating, review, usage-count, or fabricated social proof;
- facts match visible localized content.

Google documents `offers.price` as required for eligibility for its software-app rich result. We do
not invent an offer merely to chase eligibility: truthful entity description is more important than
a rich-result badge. After the tag and assets exist, publication must update availability only from
verified release facts.

## 3. Product-page information architecture

The implemented page order is designed for product evaluation rather than reference lookup:

1. **Hero:** “Your prompts are projects now.” / “从现在起，把提示词当作项目。”
2. **Product map:** project → dependency graph → build → inspectable artifact.
3. **Six-command journey:** the complete manager command surface in one ordered flow.
4. **Runnable quick start:** the zero-network command sequence above.
5. **Manager outcomes:** lifecycle, reproducibility, and human/tool-facing output.
6. **Installation:** six platform candidates, honest availability, checksums, and source install.
7. **Migration boundary:** preserved XML language and intentional project-model changes.
8. **Closing action:** create, build offline, and inspect what is shipped.

The page does not reproduce the CLI manual, internal crate graph, scheduler phases, or complete test
ledger. Those remain linked technical material. This separation is a product requirement, not just
a visual choice.

## 4. Internationalization and visual system

Chinese and English are complete monolingual pages at stable URLs. They share one typed content
shape and the same command, platform, evidence, migration, limitation, and CTA structures.
Locale-specific self-canonicals, reciprocal absolute `hreflang="zh-CN"`/`hreflang="en"`, and
Chinese `x-default` are emitted. The language switch is an ordinary link; neither crawlers nor
users are redirected from `Accept-Language`.

The page uses **MoeSegfault Style 0.1.2** from its immutable versioned URL. `BaseLayout.astro` pins
`https://style.moesegfault.dev/v0.1.2/css/all.css` with the exact Subresource Integrity (SRI) value
`sha384-Tsj7ndpNmGrJTPrbAcVgLnpddldDQeHjlbEZRTpJmfvqM6xgXNrrqmsYK61doSFn`. The upstream
[0.1.2 manifest](https://style.moesegfault.dev/v0.1.2/manifest.json) identifies the package/version
and provides file hashes. Release-specific CSS composes the public `--moe-*` tokens and local
`--xs-*` aliases instead of forking a light-only palette.

Implemented interaction properties include:

- explicit `auto`, `light`, and `dark` modes with a pre-paint theme bootstrap;
- readable responsive layouts with no document-level horizontal overflow in tested viewports;
- bounded wrapping for long immutable install commands and asset names;
- visible keyboard focus, skip navigation, labelled locale/theme controls, and non-color status
  cues;
- a complete no-JavaScript content path; JavaScript only enhances controls;
- `prefers-reduced-motion` handling for nonessential movement.

## 5. Search and robot-agent discovery

The human-visible product page remains the primary truth. Important identity, version, commands,
installation state, compatibility, and source links are emitted as static semantic HTML rather than
hidden behind hydration. Localized titles/descriptions, Open Graph/Twitter cards, canonical links,
and the single truthful JSON-LD entity agree with that visible content.

Four machine-discovery surfaces are implemented:

| Surface | Authority and purpose |
| --- | --- |
| `/robots.txt` | Crawler access policy under the Robots Exclusion Protocol; permits crawling and points to the sitemap. |
| `/sitemap.xml` | Indexable URL discovery, including both localized release routes and reciprocal alternates. |
| `/releases/1.0.0.json` | Product-owned, structured release state: prepared version, locale routes, intended tag/assets, commands, platform matrix, and integrity limitations. |
| `/llms.txt` | Additive navigation for robot agents under a community proposal; it links to authoritative pages/metadata but is not crawler control, authorization, an SEO requirement, or the sole source of any claim. |

The `llms.txt` surface is implemented, but its status is intentionally narrow. The Answer.AI-origin
proposal describes a convention for helping agents locate concise material; it is not an IETF or
search-engine standard. RFC 9309 governs `robots.txt`. Google explicitly says that ordinary SEO
fundamentals remain applicable to its AI search features and that no special AI text file or schema
is required. Consequently the durable optimization is truthful static content, crawlability,
internal discovery, locale correctness, structured data matching visible text, and good page
experience—not a parallel agent-only narrative.

## 6. Installation and publication contract

The prepared native matrix has stable intended names:

```text
xmlsquish-1.0.0-x86_64-pc-windows-msvc.zip
xmlsquish-1.0.0-aarch64-pc-windows-msvc.zip
xmlsquish-1.0.0-x86_64-unknown-linux-gnu.tar.gz
xmlsquish-1.0.0-aarch64-unknown-linux-gnu.tar.gz
xmlsquish-1.0.0-x86_64-apple-darwin.tar.gz
xmlsquish-1.0.0-aarch64-apple-darwin.tar.gz
SHA256SUMS
```

Runtime boundaries are Linux glibc 2.35+, macOS 11+, and Windows MSVC desktop targets. Binaries
are unsigned and macOS archives are not notarized; SHA-256 checksums establish byte integrity, not
publisher identity. The source build requires Rust 1.88+ and xmlsquish is not published on
crates.io.

Until the tag exists, the page separates the proven immutable candidate command from the future
tag command:

```bash
cargo install --git https://github.com/kleedaisuki/prompt-squish \
  --rev 2eb5834b15d47717a7b45092a3b72bfa475f4c79 --locked

# Valid only after publication:
cargo install --git https://github.com/kleedaisuki/prompt-squish --tag v1.0.0 --locked
```

The exact remote `--rev` installation was executed against GitHub with Rust 1.88, installed
`xmlsquish 1.0.0`, and successfully completed `new → fmt --check → build --offline → inspect`.
This is candidate evidence, not evidence that the tag or release assets exist.

## 7. Verification evidence

### 7.1 Exact remote CI

[GitHub Actions run 34975527521](https://github.com/kleedaisuki/prompt-squish/actions/runs/34975527521)
completed successfully for exact commit `8a2be527beb4091beba6477d3b7a870436ffc691`. All five jobs
were terminal and green:

| Job | Result |
| --- | --- |
| Site | success |
| Rust quality, Ubuntu, MSRV 1.88 | success |
| Rust tests, Linux, MSRV 1.88 | success |
| Rust tests, macOS, MSRV 1.88 | success |
| Rust tests, Windows, MSRV 1.88 | success |

This is exact candidate-CI evidence. It is not described as the tagged-release workflow because
`v1.0.0` has not yet passed the publication gate.

### 7.2 Site and visual validation

- Astro/TypeScript build and the browser/static product contract: **38/38 passed**.
- Initial visual matrix: Chinese/English × mobile/desktop × light/dark = **8 full-page + 8 hero
  screenshots**, all inspected; horizontal overflow was exactly zero in all eight cases.
- Incremental recheck after the runnable quick start and split install commands: **4 additional
  full-page screenshots** plus focused quick-start/source captures; all four documents again had
  zero horizontal overflow.
- Six pending platform controls were non-links in every visual case; no fabricated
  `/releases/download/v1.0.0/` anchor was present.

The visual evidence used Chromium/Playwright on Windows. Safari/WebKit and Firefox were not part of
this check; representative contrast was measured rather than exhaustively proving every token
combination.

### 7.3 Rust and packaging validation

- Rust 1.88 format, locked workspace check, strict Clippy, release build, version, and root CLI
  smoke: passed.
- Isolated all-feature workspace suite: **518/518 passed** (505 unit/integration tests plus 13
  doctests).
- A repeated native Windows candidate package was byte-identical on the validation host.
- Release automation defines six native build/package jobs and a final publish step that verifies
  the complete matrix before creating assets.

The repeated-package evidence covers one Windows host. It does not independently prove
cross-architecture execution, future GitHub asset upload, signing/notarization, or sudden-power-loss
durability on every filesystem.

## 8. Acceptance ledger

### Release preparation complete

- [x] Cargo package, lockfile, CLI `--version`, visible copy, machine metadata, and intended paths
      consistently use `1.0.0` / `v1.0.0`.
- [x] The public release routes are product pages and lead with the project-manager model.
- [x] All six commands appear in one lifecycle; the zero-network quick start is executable.
- [x] Simplified Chinese and English pages have materially equal product facts and stable locale
      metadata.
- [x] Light/dark/auto behavior, responsive presentation, no-script content, focus, reduced motion,
      and bounded long commands are implemented and covered by the stated visual/browser evidence.
- [x] MoeSegfault Style 0.1.2 is version-pinned with SRI and used through shared tokens.
- [x] `robots.txt`, localized `sitemap.xml`, candidate-safe release JSON, and additive `llms.txt`
      are published by the static site.
- [x] Exactly one candidate-safe `SoftwareApplication` JSON-LD entity is emitted, with `sameAs`
      and `creativeWorkStatus`, and without downloads, offers, ratings, or reviews.
- [x] Exact remote candidate revision `2eb5834b15d47717a7b45092a3b72bfa475f4c79` installs and runs
      the documented journey.
- [x] Browser/static tests pass 38/38, visual checks pass the stated matrices, Rust tests pass
      518/518, and exact remote CI run 34975527521 is green for all five jobs.

### Publication gate pending

- [ ] Create immutable Git tag `v1.0.0` at the approved release commit.
- [ ] Run the release workflow against that exact tag and require all six native build/package jobs
      plus the final publisher to succeed.
- [ ] Publish all six archives and `SHA256SUMS` on the GitHub Release.
- [ ] Verify every release asset URL resolves to the intended file.
- [ ] Verify `SHA256SUMS` against the bytes downloaded from GitHub.
- [ ] Run native archive smoke tests and the tag-based source-install command; confirm exact output
      `xmlsquish 1.0.0` and the required command surface.
- [ ] Replace pending controls/metadata only after those assets are observed; then verify the
      production pages and JSON-LD remain truthful.

Absent tag/assets are pending publication work, **not failures of the prepared candidate page**.
Conversely, preparation evidence must never be promoted into a claim that the release has already
been published.

## 9. External rationale and sources

- Google Search Central,
  [Software app structured data](https://developers.google.com/search/docs/appearance/structured-data/software-app):
  supported `SoftwareApplication` fields, visible-data consistency, and rich-result eligibility.
- Google Search Central,
  [Localized versions of pages](https://developers.google.com/search/docs/specialty/international/localized-versions):
  separate locale URLs and reciprocal `hreflang` relationships.
- Google Search Central,
  [Build and submit a sitemap](https://developers.google.com/search/docs/crawling-indexing/sitemaps/build-sitemap):
  XML sitemap and localized-page discovery guidance.
- Google Search Central,
  [AI features and your website](https://developers.google.com/search/docs/appearance/ai-features):
  foundational SEO remains applicable; no special AI file or schema is required.
- IETF, [RFC 9309: Robots Exclusion Protocol](https://www.rfc-editor.org/rfc/rfc9309): the
  normative crawler-access protocol implemented by `/robots.txt`.
- Answer.AI-origin community proposal,
  [`llms.txt`](https://llmstxt.org/): an additive agent-navigation convention, not crawler control
  or a standards-track SEO requirement.
- MoeSegfault Style,
  [version 0.1.2 manifest](https://style.moesegfault.dev/v0.1.2/manifest.json): exact package,
  version, file inventory, and integrity metadata used to audit the pinned stylesheet.
