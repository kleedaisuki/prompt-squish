# xmlsquish 1.0 Product Release Surface: Publication and Evidence Record

- **Status:** published; release and live-site gates complete
- **Release date:** 2026-09-15
- **Audience:** product, site, release, and validation owners
- **Product routes:** `/releases/` (Simplified Chinese) and `/en/releases/` (English)
- **Machine route:** `/releases/1.0.0.json`
- **Version/tag:** `1.0.0` / `v1.0.0`
- **Release source commit:** `8eeb856a38fac255e51337e7c0e2f27c6255ccde`
- **Public release:** [xmlsquish 1.0.0 on GitHub](https://github.com/kleedaisuki/prompt-squish/releases/tag/v1.0.0)

This file records the implemented release surface, publication history, and supporting evidence. It
is not the release page itself. The public routes are deliberately **product launch pages**, not
documentation pages: they lead with the user outcome, present one six-command lifecycle, route
users to native downloads or a source installation, and keep migration detail subordinate to the
product story.

Product and technical authority remain in the
[project-manager scope](../product/project-manager-scope.md),
[CLI experience](../product/cli-experience.md),
[ADR 0009](../adr/0009-microkernel-manager-and-reusable-ir.md),
[ADR 0010](../adr/0010-transactional-new-project-creation.md), and the
[execution ledger](project-manager-execution-status.md).

## 1. Product decision and published journey

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

This avoids implying that a newly added remote dependency is already available to an offline build.
The page also shows the canonical scaffold (`xmlsquish.toml`, `src/prompt.xml`), the published
`.prompt`, and the inspectable artifact model. Migration copy preserves the 0.3 XML DSL while making
the intentional command-first/project-first boundary explicit. `new` requires a nonexistent
destination; adopting an existing populated directory (`init`) is not promised for 1.0.

## 2. Published state and source identity

The preparation/publication distinction is now closed with observed evidence:

| State | Meaning | Result |
| --- | --- | --- |
| **Release preparation** | Versioned product UI, localization, machine discovery, tests, candidate install, and release automation were ready. | **Complete** |
| **Publication gate** | Immutable tag, six native archives, `SHA256SUMS`, public Release, and live site agree. | **Complete** |
| **Operational boundaries** | Signing/notarization and universal power-loss durability are not claimed. | **Explicitly retained** |

`v1.0.0` is an annotated Git tag. Tag object
`51004bb88a784f0944317454a578863b8e346e72` peels to immutable source commit
`8eeb856a38fac255e51337e7c0e2f27c6255ccde`. The annotated tag is unsigned; GitHub Release API's
mutable `targetCommitish: main` is therefore not used as source-identity evidence. The peeled tag
commit is authoritative.

The GitHub Release is public, non-draft, and non-prerelease. It was published at
`2026-09-15T15:24:48Z` and exposes seven uploaded assets: six native archives and
`SHA256SUMS`.

The published `SoftwareApplication` JSON-LD is truthful and release-aware:

- exactly one release-page entity;
- `softwareVersion: "1.0.0"`;
- repository identity through `sameAs`;
- `creativeWorkStatus: "Published"`;
- six verified `downloadUrl` values derived from the same platform matrix as the visible buttons;
- no `Offer`/`offers`, rating, review, usage-count, or fabricated social proof;
- facts match the visible localized content.

Google documents `offers.price` as required for eligibility for its software-app rich result. We do
not invent an offer merely to chase eligibility: truthful entity description is more important than
a rich-result badge.

## 3. Product-page information architecture

The published page order is designed for product evaluation rather than reference lookup:

1. **Hero:** “Your prompts are projects now.” / “从现在起，把提示词当作项目。”
2. **Product map:** project → dependency graph → build → inspectable artifact.
3. **Six-command journey:** the complete manager command surface in one ordered flow.
4. **Runnable quick start:** the zero-network command sequence above.
5. **Manager outcomes:** lifecycle, reproducibility, and human/tool-facing output.
6. **Installation:** six direct platform downloads, checksums, and tagged source install.
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
installation, compatibility, and source links are emitted as static semantic HTML rather than
hidden behind hydration. Localized titles/descriptions, Open Graph/Twitter cards, canonical links,
and the single truthful JSON-LD entity agree with that visible content.

Four machine-discovery surfaces are live:

| Surface | Authority and purpose |
| --- | --- |
| `/robots.txt` | Crawler access policy under the Robots Exclusion Protocol; permits crawling and points to the sitemap. |
| `/sitemap.xml` | Indexable URL discovery, including both localized release routes and reciprocal alternates. |
| `/releases/1.0.0.json` | Product-owned structured release state: published version, locale routes, release/assets, commands, platform matrix, and integrity limitations. |
| `/llms.txt` | Additive navigation for robot agents under a community proposal; it links to authoritative pages/metadata but is not crawler control, authorization, an SEO requirement, or the sole source of any claim. |

The `llms.txt` surface is useful but intentionally narrow. The Answer.AI-origin proposal describes
a convention for helping agents locate concise material; it is not an IETF or search-engine
standard. RFC 9309 governs `robots.txt`. Google explicitly says that ordinary SEO fundamentals
remain applicable to its AI search features and that no special AI text file or schema is required.
The durable optimization is truthful static content, crawlability, internal discovery, locale
correctness, structured data matching visible text, and good page experience—not a parallel
agent-only narrative.

## 6. Published installation and integrity contract

The public Release contains these six native archives:

```text
xmlsquish-1.0.0-x86_64-pc-windows-msvc.zip
xmlsquish-1.0.0-aarch64-pc-windows-msvc.zip
xmlsquish-1.0.0-x86_64-unknown-linux-gnu.tar.gz
xmlsquish-1.0.0-aarch64-unknown-linux-gnu.tar.gz
xmlsquish-1.0.0-x86_64-apple-darwin.tar.gz
xmlsquish-1.0.0-aarch64-apple-darwin.tar.gz
SHA256SUMS
```

All seven public links returned HTTP 200 after following GitHub's asset redirects. The downloaded
`SHA256SUMS`, live site release JSON, and GitHub Release API contain the same six archive names.
Every manifest digest equals GitHub's independently reported per-asset SHA-256 digest:

| Native archive | SHA-256 |
| --- | --- |
| `xmlsquish-1.0.0-aarch64-apple-darwin.tar.gz` | `f5cb515b9d094860cc93bbc769ff860204b78098997b189c00d6535456e02b59` |
| `xmlsquish-1.0.0-aarch64-pc-windows-msvc.zip` | `02033ff3d7cfb640b1d1bba2422d276369bfc8a1543cb8db8b8ec510b0d39209` |
| `xmlsquish-1.0.0-aarch64-unknown-linux-gnu.tar.gz` | `bd5160983dbab25def457d88053101efccd40b9c32803eb3de066488aa8155f2` |
| `xmlsquish-1.0.0-x86_64-apple-darwin.tar.gz` | `e4d140ecf52a6da694ba74e9e8eda7ff4aca7be7f1a7bcdbdaaaa20d71572448` |
| `xmlsquish-1.0.0-x86_64-pc-windows-msvc.zip` | `09969760de7d4f56b767e72c0534d49a40bc51e16ca546652388ece33c3a1acd` |
| `xmlsquish-1.0.0-x86_64-unknown-linux-gnu.tar.gz` | `c09e07f086c9096e859885198b0bccffce34d7a80bc18290c18a5cbf3817694d` |

The 667-byte `SHA256SUMS` asset itself has SHA-256
`214099529e16dc7c9cb3d73220f83e50b996a4ac7e9180e5e8b605e9ffca5251`, matching the GitHub API.

Runtime boundaries are Linux glibc 2.35+, macOS 11+, and Windows MSVC desktop targets. Binaries
are unsigned and macOS archives are not notarized; SHA-256 establishes byte integrity, not
publisher identity. The source build requires Rust 1.88+ and xmlsquish is not published on
crates.io. The published source-install command is:

```bash
cargo install --git https://github.com/kleedaisuki/prompt-squish --tag v1.0.0 --locked
```

Before the tag was cut, exact remote candidate revision
`2eb5834b15d47717a7b45092a3b72bfa475f4c79` installed with Rust 1.88, reported
`xmlsquish 1.0.0`, and completed `new → fmt --check → build --offline → inspect`. This remains useful
pre-publication integration evidence; release identity now comes from the annotated tag.

## 7. Verification and publication history

### 7.1 Source, main, and Pages gates

All three gates used exact release source commit
`8eeb856a38fac255e51337e7c0e2f27c6255ccde`:

| Gate | Workflow run | Result and scope |
| --- | --- | --- |
| Pre-tag candidate CI | [34980263497](https://github.com/kleedaisuki/prompt-squish/actions/runs/34980263497) | Five jobs passed: Site, Ubuntu quality, and Linux/macOS/Windows tests. |
| `main` CI | [34980902926](https://github.com/kleedaisuki/prompt-squish/actions/runs/34980902926) | Five jobs passed after the release source reached `main`; Linux also passed source-install smoke. |
| GitHub Pages | [34980902735](https://github.com/kleedaisuki/prompt-squish/actions/runs/34980902735) | Build and deploy succeeded; live routes were subsequently fetched and parsed. |

Cache-bypassed live verification returned HTTP 200 for `/releases/`, `/en/releases/`,
`/releases/1.0.0.json`, `/llms.txt`, `/robots.txt`, and `/sitemap.xml`. The localized HTML, canonical
and alternate metadata, six direct downloads, published release JSON, and machine-discovery files
were parsed from live responses rather than inferred from repository build output.

### 7.2 Adverse evidence and repairs

Failed attempts are retained because they explain why the final automation is trustworthy:

| Run/change | Observation | Resolution |
| --- | --- | --- |
| Release run [34980935644](https://github.com/kleedaisuki/prompt-squish/actions/runs/34980935644) | Both Apple jobs failed packaging because dependency legal notices were missing; the first concrete failure named `block2-0.6.2`. Publish did not run. | `d70c25b874688f8f4e7b39c31b1e7ca7787b9833` added reviewed license/source records for `block2`, `dispatch2`, `objc2`, and `objc2-encode`. |
| Rerun [34982186340](https://github.com/kleedaisuki/prompt-squish/actions/runs/34982186340) | Both Apple jobs passed, but Windows x64 release tests hung in the two PTY tests until the 35-minute timeout; publish was skipped. | The old Windows Server 2022 ConPTY shutdown behavior was reproduced and analyzed instead of deleting tests. |
| `5c8bf1f3dc65ffff3255e78232d1d8cd638b5869` | PTY reader ownership changed from strong to weak during cursor replies, and the immutable tag's release job moved to the Windows 2025 runner. | Mechanism, controlled experiments, uncertainty, and sources are preserved in [`docs/research/windows-conpty-release-test-hang.md`](../research/windows-conpty-release-test-hang.md). |

The ConPTY mitigation retained the complete native test suite. It did not turn an infrastructure
failure into a skipped test or silently weaken the publication gate.

### 7.3 Final release workflow

[Release run 34987421053](https://github.com/kleedaisuki/prompt-squish/actions/runs/34987421053)
used automation commit `5c8bf1f3dc65ffff3255e78232d1d8cd638b5869`, resolved the annotated tag to
the immutable `8eeb856a...` source, and completed **8/8 jobs successfully**:

| Job | Result |
| --- | --- |
| Resolve immutable tag source | success |
| Build/test/package `x86_64-unknown-linux-gnu` | success |
| Build/test/package `aarch64-unknown-linux-gnu` | success |
| Build/test/package `x86_64-pc-windows-msvc` | success |
| Build/test/package `aarch64-pc-windows-msvc` | success |
| Build/test/package `x86_64-apple-darwin` | success |
| Build/test/package `aarch64-apple-darwin` | success |
| Verify complete matrix and publish | success |

The publisher created the non-draft, non-prerelease
[public GitHub Release](https://github.com/kleedaisuki/prompt-squish/releases/tag/v1.0.0) only after
the complete native matrix succeeded.

### 7.4 Pre-publication site, visual, and Rust evidence

- Browser/static product contract: **38/38 passed**.
- Initial visual matrix: Chinese/English × mobile/desktop × light/dark = **8 full-page + 8 hero
  screenshots**; horizontal overflow was zero in every case.
- Incremental recheck: **4 additional full-page screenshots** plus focused quick-start/source
  captures, again with zero horizontal overflow.
- Isolated Rust 1.88 all-feature workspace suite: **518/518 passed** (505 unit/integration tests plus
  13 doctests).
- Rust 1.88 formatting, locked workspace check, strict Clippy, release build, CLI version, and root
  smoke passed.

These candidate checks complement but do not replace the final six-target Release workflow and live
HTTP verification.

## 8. Acceptance ledger

### Product and release preparation

- [x] Cargo package, lockfile, CLI `--version`, visible copy, machine metadata, tag, and download
      paths consistently use `1.0.0` / `v1.0.0`.
- [x] The public release routes are product pages and lead with the project-manager model.
- [x] All six commands appear in one lifecycle; the zero-network quick start is executable.
- [x] Simplified Chinese and English pages have materially equal product facts and stable locale
      metadata.
- [x] Light/dark/auto behavior, responsive presentation, no-script content, focus, reduced motion,
      and bounded long commands are implemented and covered by the stated visual/browser evidence.
- [x] MoeSegfault Style 0.1.2 is version-pinned with SRI and used through shared tokens.
- [x] `robots.txt`, localized `sitemap.xml`, published release JSON, and additive `llms.txt` are live.
- [x] Exactly one published `SoftwareApplication` JSON-LD entity is emitted, with `sameAs`,
      `creativeWorkStatus`, and six verified download URLs, but without offers, ratings, or reviews.
- [x] Browser/static tests pass 38/38, visual checks pass the stated matrices, Rust tests pass
      518/518, and the exact pre-tag/main/Pages gates are green.

### Publication gate

- [x] Annotated Git tag `v1.0.0` exists and peels to exact source commit `8eeb856a...`.
- [x] The corrected release workflow resolved the tag and completed all six native build/package
      jobs plus resolve and publish: 8/8 successful.
- [x] The GitHub Release is public, non-draft, and non-prerelease.
- [x] All six archives and `SHA256SUMS` are uploaded; no asset is zero bytes.
- [x] Every site/API release asset URL resolves with HTTP 200 to the intended GitHub asset.
- [x] The downloaded `SHA256SUMS` filename set equals the site JSON and Release API asset set.
- [x] All six downloaded manifest digests equal GitHub's per-asset digests.
- [x] The six-target release automation ran native tests and CLI smoke before packaging and
      published only after the complete matrix passed.
- [x] Live Chinese/English pages and release JSON expose `published` state and direct downloads;
      no stale prepared/candidate wording remains in the verified responses.

## 9. Explicitly retained boundaries

- Binaries are unsigned and macOS archives are not Apple-notarized. Operating-system warnings may
  appear; users should not disable system-wide protections.
- The annotated Git tag itself is unsigned. Its peel target proves source identity within Git, not
  cryptographic publisher identity.
- Checksums prove byte integrity relative to the release manifest/API, not publisher identity.
- Controlled process-kill recovery is tested. The release does not claim proof against sudden
  Windows power loss, storage-controller cache loss, or arbitrary remote filesystems.
- Exclusive atomic rename depends on host/filesystem support; unsupported environments are rejected
  rather than given a race-prone emulation.
- xmlsquish is not published on crates.io; the supported source-install path is the immutable Git
  tag.

## 10. External rationale and sources

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
