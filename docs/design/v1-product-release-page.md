# xmlsquish 1.0 Product Release Page

- **Status:** implementation contract
- **Date:** 2026-09-15
- **Audience:** product, site, release, and validation owners
- **Applies to:** `/releases/` and `/en/releases/`
- **Product authority:** [project-manager scope](../product/project-manager-scope.md),
  [CLI experience](../product/cli-experience.md), [ADR 0009](../adr/0009-microkernel-manager-and-reusable-ir.md),
  [ADR 0010](../adr/0010-transactional-new-project-creation.md), and the
  [execution ledger](project-manager-execution-status.md)

## 1. Decision

The 1.0 page is a **product launch page**, not a reformatted manual and not a long release-note
document. It must let a new visitor answer five questions, in this order:

1. What is xmlsquish now?
2. What can I accomplish with it?
3. Why should I trust the 1.0 claim?
4. How do I install and start?
5. What changes if I used an older release?

The one-sentence product model is:

> **xmlsquish is the project manager for XML prompts: create, format, resolve, build, and inspect
> reproducible prompt artifacts with one command-line tool.**

The narrative pivot is **compiler to project manager**. Compilation remains an important engine,
but it is no longer the product boundary. The visual proof is the six-command lifecycle:

```text
new → fmt → add/remove → build → inspect
```

The page must name all six direct commands—`new`, `fmt`, `build`, `add`, `remove`, and `inspect`—and
show how they belong to one workflow. It must not present six unrelated feature tiles or lead with
the XML macro language.

## 2. Audience and desired actions

| Audience | Question | Page answer | Primary action |
| --- | --- | --- | --- |
| New prompt engineer | “Can this replace my scripts and loose XML files?” | One manifest, one manager, named targets, dependencies, deterministic artifacts | Install, then run `xmlsquish new` |
| Existing 0.x user | “Will my prompt sources still work?” | The 0.3 XML vocabulary remains; the invocation and artifact model moves to projects | Read the compact migration boundary |
| CI/tool author | “Can automation consume this without terminal scraping?” | Stable exits plus Human, Short, and versioned NDJSON presentation | Open the CLI contract or GitHub source |
| Evaluator | “Is 1.0 merely a new label?” | Exact platform matrix, checksums, tagged source, release workflow, and stated limitations | Inspect release evidence |
| Search crawler or robot agent | “What entity, version, capabilities, and downloads does this page describe?” | Static semantic HTML and matching structured facts | Follow canonical release/download/source links |

The primary conversion is a successful local `new → fmt --check → build --offline` run. Download
count is not the product outcome.

## 3. Information architecture

Replace the current 0.3 compiler-oriented body with the following sequence. The first screen must
communicate value before history or internals.

### 3.1 Global header

Retain the xmlsquish brand, explicit language switch, and three-state theme control. Rename
navigation wording if needed so the page reads as a product surface rather than a reference manual.
Do not auto-redirect based on language or theme.

### 3.2 Hero: the product change

Required content:

- eyebrow: `xmlsquish 1.0.0` plus the release date sourced from release metadata;
- headline: “Your prompts are projects now.” / “从现在起，把提示词当作项目。”;
- supporting line that explicitly identifies xmlsquish as an XML prompt project manager;
- primary CTA to platform downloads;
- secondary CTA to the six-command quick start;
- compact facts: free/open source, GPL-3.0-or-later, Windows/Linux/macOS, x64/ARM64;
- a visual command orbit or pipeline that makes the manager model legible without JavaScript.

The hero must not say only “new language”, “macros”, “minification”, or `XML → XML`. Those describe
the internal compiler-era product and are no longer an adequate 1.0 promise.

### 3.3 Six-command workflow: show one successful journey

Use one numbered, copyable terminal sequence rather than six isolated examples:

```bash
xmlsquish new support-agent --vcs=git
cd support-agent
xmlsquish fmt --check
xmlsquish add company-policy
xmlsquish remove company-policy
xmlsquish build --offline
xmlsquish inspect artifact prompt
```

Explain each command in one short phrase. `add` and `remove` belong to one dependency-management
stage so the lifecycle stays visually simple. A visible output card shows the canonical scaffold
(`xmlsquish.toml`, `src/prompt.xml`) and the published `.prompt`, with `.xsir` and `.psdbg` identified
as optional inspectable companions.

The example demonstrates the interface; it must not imply that a newly added remote dependency is
available to an offline build unless the example establishes that cache state. For an executable
zero-network quick start, make `new → fmt --check → build --offline` the copyable command and
demonstrate `add`/`remove` separately in the same workflow section.

### 3.4 Product pillars: why a manager matters

Use three high-signal cards:

1. **Project lifecycle** — canonical `new`, upward project discovery, workspaces, and formatting.
2. **Reproducible builds** — exact dependency state, immutable source observation, binary XSIR,
   linking, and atomic artifact publication.
3. **Built for humans and tools** — Human/Short/NDJSON outputs, stable exit meanings, typed
   inspection, cancellation, and recovery.

Each card connects a feature to a user outcome. Internal crate names, scheduler phase names, and
detailed budget tables belong in linked technical material, not the launch narrative.

### 3.5 Trust strip: make 1.0 falsifiable

Present a compact, visible evidence block—not vague badges—with these facts:

- the tagged commit and GitHub release page;
- release binaries are built and natively smoke-tested for six targets: Windows, Linux, and macOS,
  each on x64 and ARM64;
- release automation verifies the six-command help surface and a real
  `new → fmt → build → inspect` journey before packaging;
- archives have a release-local `SHA256SUMS` file;
- minimum source-build toolchain is Rust 1.88;
- binaries are unsigned and macOS artifacts are not notarized; checksums establish byte integrity,
  not publisher identity.

Only display a CI run as “passed” when it is the exact tagged 1.0.0 source and the run is terminal
and successful. Pre-release milestone run 34968311247 is useful engineering evidence but must not be
misrepresented as the tagged 1.0 release run.

### 3.6 Installation: route by operating system

Retain six direct asset links using release automation's stable names:

```text
xmlsquish-1.0.0-x86_64-pc-windows-msvc.zip
xmlsquish-1.0.0-aarch64-pc-windows-msvc.zip
xmlsquish-1.0.0-x86_64-unknown-linux-gnu.tar.gz
xmlsquish-1.0.0-aarch64-unknown-linux-gnu.tar.gz
xmlsquish-1.0.0-x86_64-apple-darwin.tar.gz
xmlsquish-1.0.0-aarch64-apple-darwin.tar.gz
SHA256SUMS
```

State the existing runtime boundaries beside the choices: Linux glibc 2.35+, macOS 11+, and
Windows MSVC desktop targets. Also offer the pinned source install:

```bash
cargo install --git https://github.com/kleedaisuki/prompt-squish --tag v1.0.0 --locked
```

The verification result shown below it is `xmlsquish 1.0.0`. State that source install requires
Rust 1.88+ and that xmlsquish is not currently published on crates.io.

Download buttons must not be represented as working until the tag and corresponding release assets
exist. Publishing the page and release is therefore one coordinated release gate; a candidate
deployment may label itself “1.0.0 candidate” and route to GitHub Releases, but cannot fabricate
successful asset URLs.

### 3.7 Migration and compatibility boundary

This section is short and visible; do not hide the core answer in a collapsed manual.

| From 0.3 compiler workflow | 1.0 project-manager workflow |
| --- | --- |
| loose input operand | manifest target in `xmlsquish.toml` |
| sibling `.o.xml` output | target-directory `.prompt` product |
| printable intermediate XML | reusable binary `.xsir` |
| optional provenance output | self-contained `.psdbg` |
| direct compiler invocation | `xmlsquish build -t NAME` |

The page says both sides of the compatibility decision:

- **Preserved:** the 0.3 XML DSL—entries, modules, imports, macros, arguments, slots, and expansion
  semantics—remains the source-language contract.
- **Changed intentionally:** 1.0 is command-first and project-first. It does not guess that an
  unknown command is a source path. Existing loose-source users create a manifest and target.

Also state that `new` requires a destination that does not exist; adopting an existing populated
directory (`init`) is not part of 1.0. Do not promise universal power-loss durability: tested
process-death recovery on supported desktops does not prove sudden-power-loss behavior on every
filesystem or storage controller.

### 3.8 Closing CTA

End with one outcome, not another feature inventory:

> Create a real prompt project, build it offline, and inspect what you ship.

Buttons: **Download 1.0.0** and **View source on GitHub**. Technical details link to the tagged
changelog/release notes, product CLI contract, namespace page, and license; they do not occupy the
main narrative.

## 4. Bilingual content contract

Chinese and English are two complete, monolingual product pages at stable URLs. They expose the
same sections, facts, commands, asset choices, limitations, and CTA destinations. Translation does
not require identical sentence length, but neither locale may contain a material claim absent from
the other.

| Message intent | English direction | Simplified Chinese direction |
| --- | --- | --- |
| SEO title | `xmlsquish 1.0 — Project manager for XML prompts` | `xmlsquish 1.0 — XML 提示词项目管理器` |
| Meta description | `Create, format, resolve, build, and inspect reproducible XML prompt projects with xmlsquish 1.0.` | `使用 xmlsquish 1.0 创建、格式化、解析、构建并检查可复现的 XML 提示词项目。` |
| Hero | `Your prompts are projects now.` | `从现在起，把提示词当作项目。` |
| Value statement | `One manager from a new project to an inspectable artifact.` | `从新建项目到可检查产物，只用一个管理器。` |
| Workflow heading | `Six commands. One project lifecycle.` | `六个命令，一条项目生命周期。` |
| Evidence heading | `A 1.0 claim you can verify.` | `可以验证的 1.0。` |
| Migration heading | `Coming from 0.3? Keep the language; adopt the project.` | `从 0.3 升级？保留语言，迁移到项目。` |
| Closing | `Create a real prompt project, build it offline, and inspect what you ship.` | `创建真正的提示词项目，离线构建，并检查你交付的内容。` |

`reference.ts` keeps one structural type for both locale objects so missing or shape-mismatched
translations fail TypeScript checking. Tests additionally compare material arrays (commands,
platforms, migration rows, and evidence facts) by key or length; TypeScript alone cannot detect a
semantically omitted translation represented by an empty string.

## 5. Visual system and interaction

Use the pinned **MoeSegfault Style** stylesheet already loaded by `BaseLayout.astro`;
release-specific CSS composes its public tokens (`--moe-*`) and existing xmlsquish aliases
(`--xs-*`). It must not fork a second palette or hard-code a light-only surface.

Visual direction:

- warm cream/orange/brown MoeSegfault palette, editorial typography, rounded but restrained cards;
- a large typographic hero with a dark terminal/product panel as the visual counterweight;
- monospace version numbers, commands, artifact names, and evidence identifiers;
- one warm accent for focus and action, not rainbow feature coloring;
- roomy product storytelling above dense details;
- real text and CSS for the command lifecycle, not a text-bearing raster image;
- decorative anime/moe energy may appear in small accents but must not compete with command or
  installation tasks.

Theme behavior:

- `auto`, `light`, and `dark` remain explicit choices and persist through the existing control;
- render the initial theme before paint using the existing inline bootstrap;
- every release surface derives foreground/background/border/focus colors from theme tokens;
- `prefers-reduced-motion` removes nonessential movement;
- the page remains complete without JavaScript; JavaScript only enhances copy buttons and controls.

At 390, 768, and 1280 CSS pixels, both locales and both themes have no document-level horizontal
overflow. Download targets remain at least as clear on touch as on desktop, and long commands or
asset names wrap or scroll within their own bounded container.

## 6. Search and robot-agent contract

Optimize the same page for humans, crawlers, and agents rather than maintaining an agent-only
parallel story:

1. Generate static HTML with one descriptive `h1`, ordered `h2` sections, actual lists/tables, and
   visible text for every material fact. Never require hydration to discover product meaning,
   compatibility, commands, or download links.
2. Keep locale-specific canonical URLs, reciprocal absolute `hreflang="zh-CN"` and `hreflang="en"`,
   and `x-default` pointing to the Chinese release page. Each localized page canonicals to itself.
3. Give 1.0 its own localized title and description. Add a release-appropriate Open Graph image and
   `summary_large_image`; do not reuse an image whose visible text claims the old compiler model.
4. Embed truthful JSON-LD with a `WebPage` whose `mainEntity` is a `SoftwareApplication`. Include
   `name`, `softwareVersion: "1.0.0"`, localized `description`, `applicationCategory:
   "DeveloperApplication"`, operating systems, license URL, code repository, release-notes URL,
   download URL, and a zero-price `Offer`. Do not invent ratings, reviews, usage counts, awards, or
   compatibility. Structured facts also appear visibly on the page.
5. Publish crawlable `robots.txt` and `sitemap.xml` containing both release URLs. Do not block
   ordinary search or agent crawlers. A future `llms.txt` may summarize stable navigation, but it is
   optional and must not become the only source for a fact.
6. Link directly to the immutable `v1.0.0` tag, release, checksums, changelog, license, namespace,
   and repository with descriptive anchor text. Avoid “click here”.
7. Keep the language switch as an ordinary anchor. Do not locale-redirect crawlers or users based
   on `Accept-Language`.

This follows Google's guidance to use separate language URLs with reciprocal `hreflang` annotations
and visible monolingual content. `SoftwareApplication` markup is an entity description, not a
promise of a rich result; without genuine reviews or ratings the page must not manufacture them to
satisfy a rich-result eligibility rule.

## 7. Non-goals

- Reproducing the full CLI reference, XML DSL specification, internal architecture, or test ledger
  inline.
- Retelling the 0.2-to-0.3 `mount`/`call` migration as the headline of the 1.0 page.
- Claiming that xmlsquish is a hosted service, registry, template marketplace, IDE, prompt test
  framework, deployment platform, or general task runner.
- Adding telemetry, sign-up gates, account creation, newsletter forms, pricing tiers, or fabricated
  social proof.
- Claiming code signing, notarization, crates.io availability, hermetic editable path dependencies,
  universal filesystem support, or universal sudden-power-loss safety.
- Maintaining materially different Chinese and English claims.
- Building an agent-only shadow page that can drift from the human-visible facts.

## 8. Verifiable acceptance checklist

### Product truth

- [ ] Cargo package, lockfile, binary `--version`, install command, tag links, download paths, visible
      copy, and JSON-LD all say exactly `1.0.0` / `v1.0.0`.
- [ ] The hero calls xmlsquish an XML prompt **project manager**, not only a compiler.
- [ ] All six commands are visible and participate in one project lifecycle.
- [ ] The zero-network quick start successfully executes with the tagged release binary.
- [ ] The canonical scaffold and `.prompt` product are accurate; `.xsir` and `.psdbg` are optional
      artifacts rather than XML text files.
- [ ] Compatibility and limitations match the tagged product contract.

### Release evidence

- [ ] All six expected archives and `SHA256SUMS` exist at the linked GitHub release.
- [ ] Every download link returns the intended asset rather than a GitHub error page.
- [ ] `SHA256SUMS` matches the published bytes.
- [ ] The displayed successful workflow is for the exact immutable `v1.0.0` commit.
- [ ] Source-install and native archive smoke produce `xmlsquish 1.0.0` and exercise the six-command
      surface required by release automation.

### Internationalization and discovery

- [ ] `/releases/` renders complete Simplified Chinese and `/en/releases/` complete English, with no
      mixed-locale fallback text.
- [ ] Commands, platforms, facts, migration rows, limitations, and CTA links are materially equal in
      both locales.
- [ ] Each page has correct `html[lang]`, self-canonical, reciprocal absolute `hreflang`, and
      `x-default` metadata.
- [ ] Localized title, description, Open Graph fields, and JSON-LD validate and agree with visible
      content.
- [ ] `robots.txt` permits crawling and the sitemap contains both localized release URLs.
- [ ] Production HTML contains product identity, version, commands, install path, compatibility, and
      source/release links without executing JavaScript.

### Experience and accessibility

- [ ] Product narrative appears before migration/reference detail; the page does not read like a
      technical document.
- [ ] Light, dark, and system modes work without a wrong-theme flash that obscures content.
- [ ] Both locales pass browser checks at 390, 768, and 1280 pixels in light and dark modes with no
      page overflow.
- [ ] Keyboard focus, skip link, language switch, theme control, `<details>` if retained, and copy
      feedback are operable and labelled; color is not the sole information channel.
- [ ] Without JavaScript, all claims, commands, downloads, migration facts, and links remain usable.
- [ ] Reduced-motion preference removes nonessential animation.

## 9. Evidence and references

Repository evidence inspected for this decision:

- `site/src/components/ReleasePage.astro` — current 0.3 product-like layout but compiler-era story;
- `site/src/i18n/reference.ts` — current paired translation shape and 0.3 migration content;
- `site/src/layouts/BaseLayout.astro` and `ReferenceLayout.astro` — canonical/hreflang, theme
  bootstrap, MoeSegfault stylesheet, and product-width shell;
- `.github/workflows/release.yml` and `.github/scripts/release.py` — exact native matrix, asset
  naming, package smoke, checksum, and publication contract;
- `docs/design/project-manager-execution-status.md` — accepted six-command milestone evidence and
  its explicit durability boundary.

External implementation guidance:

- Google Search Central, [localized versions](https://developers.google.com/search/docs/specialty/international/localized-versions)
  and [multilingual sites](https://developers.google.com/search/docs/specialty/international/managing-multi-regional-sites):
  separate language URLs, reciprocal alternates, visible monolingual content, and no forced locale redirect.
- Google Search Central, [`SoftwareApplication` structured data](https://developers.google.com/search/docs/appearance/structured-data/software-app):
  visible, truthful application facts and validation; search presentation is not guaranteed.
- Schema.org, [`SoftwareApplication`](https://schema.org/SoftwareApplication): the vocabulary for the
  software entity described by the release page.
