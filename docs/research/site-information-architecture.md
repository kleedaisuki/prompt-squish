# Site information architecture for the v1.0.4 product story

- **Status:** product and UX specification
- **Research date:** 2026-09-20
- **Scope:** the human-facing product home, DSL manual, release-log index, release stories, and
  the discoverability surfaces that must agree on v1.0.4
- **Implementation boundary:** this document specifies observable behavior and content structure.
  It does not prescribe a component framework migration and does not change the site itself.
- **Evidence labels:** **Observed** means repository or first-party-site evidence; **Inferred** means
  a reasoned diagnosis; **Recommended** means the product decision proposed here.

## Executive decision

Keep one brand shell and add **two purpose-built reading layouts**:

```text
BaseLayout             document metadata, global tokens, skip link
└─ ProductShell        shared product header and footer
   ├─ Home             existing product-story composition
   ├─ ManualLayout     learning and reference reading grammar
   └─ ReleaseLayout    editorial release-story reading grammar

Release index          compact archive/list composition inside ProductShell
```

The two new layouts are not two new brands. They share the logo, product navigation, theme,
language switch, colors, type families, focus treatment, and footer. They differ because the user
jobs differ:

| Surface | Primary user question | Reading grammar |
| --- | --- | --- |
| DSL manual | “How do I perform this task or look up this rule?” | chapter navigation + readable article + local outline |
| Release story | “Should I upgrade, what changed, and what must I do?” | date/version/dek + narrative article + compact facts/actions |

The release index is an archive, not a third long-form layout. It should use a compact blog-style
chronology and route each version to a release story.

For v1.0.4, the public story is **project-owned build state**: generated products, compiler metadata,
and rebuild caches live inside the configured project target, whose default is
`target/xmlsquish`. Product copy must explicitly retire the previous machine-global-cache mental
model. It must not claim that the cache is portable across machines or improves performance unless
those properties are separately verified.

## 1. Problem definition

### 1.1 Explicit requirements

1. Publish v1.0.4 and update the product site accordingly.
2. Explain the new project-local `target/xmlsquish` model, including compiler metadata and caches.
3. Improve the DSL manual and release log by learning from mature documentation and developer-blog
   sites.
4. Add two distinct layouts rather than forcing all reading surfaces through one composition.
5. Preserve coherent Chinese and English behavior and the existing product identity.

### 1.2 Necessary implications

- The manual and release log need different information hierarchies, not merely different colors.
- v1.0.4 must be represented by one release record that drives its routes and version facts; adding
  version strings independently to several surfaces would reproduce the current drift risk.
- Existing historical release stories must remain historically accurate. Do not rewrite v1.0.2 to
  pretend it used project-local caches.
- The build/artifact manual chapter must explain the new state boundary. DSL semantics should not be
  edited unless the language actually changes.
- The public site must distinguish verified facts, trade-offs, and known limitations. A patch-release
  number is not evidence that a storage redesign is operationally trivial.

### 1.3 Out of scope

- adopting Starlight wholesale;
- site-wide search, comments, reactions, analytics, RSS, or pagination for a six-entry archive;
- an interactive compiler/playground;
- one page per DSL directive;
- preserving the old cache implementation or presenting migration compatibility that does not exist;
- inventing benchmark, portability, signing, or reproducibility claims.

## 2. Current-site diagnosis

### 2.1 Structural observations

**Observed:** `site/src/layouts/BaseLayout.astro` owns document metadata and global tokens, while
`site/src/layouts/ProductShell.astro` owns shared product chrome. The actual reading grammars live
inside monolithic page components:

- `site/src/components/NamespacePage.astro` contains the complete manual shell, all route bodies,
  client enhancements, and dense component CSS;
- `site/src/components/ReleaseDetailPage.astro` contains the release article shell and its CSS;
- `site/src/components/ReleaseIndexPage.astro` contains the archive composition and its CSS.

**Inferred:** the product already has the beginnings of two layouts, but they are implicit. Making
those structures explicit will remove duplication and allow content pages to own content rather than
navigation geometry.

### 2.2 Manual pain points

| Observation | User consequence | Required correction |
| --- | --- | --- |
| Every chapter repeats a large two-column masthead and namespace URI panel. | Deep links begin like marketing pages; the first viewport delays the answer. | Use a compact chapter header. Reserve the rich namespace identity block for the manual overview. |
| Desktop has a chapter rail and article but no generated local outline. | Long pages, especially the directive reference, are hard to skim and deep-link. | Add an optional “On this page” rail for `h2`/`h3`, or an inline equivalent when space is insufficient. |
| The chapter rail and a local-page label are conceptually mixed. | Navigation purpose is ambiguous to visual and assistive-technology users. | Name and label “Manual chapters” and “On this page” as separate navigation landmarks. |
| Overview content nests several two-column grids and small card copy. | The page resembles a feature-card catalog and leaves dead space after the short left rail. | Prefer a linear orientation path, three prominent next tasks, and a compact chapter list. |
| The directive reference is about 7,000 px tall and renders eleven cards containing more cards. | Repetition dominates hierarchy; comparison and lookup are slow. | Precede details with a grouped directive matrix; render each directive as a restrained reference section rather than a card stack. |
| Reference content can go from the page `h1` directly to repeated `h3` labels. | The heading outline does not express the visible content hierarchy. | Give every directive an `h2`; use `h3` only for its contract subsections. |
| A generic callout style covers rules, explanations, warnings, and implementation notes. | Readers cannot tell contractual language from teaching material. | Use a small labeled taxonomy: Contract, Note, Example, Warning, Version. |
| Most chapter bodies are conditionals and locale ternaries in one component. | Review, localization parity, and chapter evolution are fragile. | Move long-form chapter bodies into reviewable structured content; keep interface labels in i18n data. |

### 2.3 Release-log pain points

| Observation | User consequence | Required correction |
| --- | --- | --- |
| The archive opens with a display-sized product headline and sparse three-column ledger. | It reads like a landing page or administrative table, not a developer blog archive. | Use a compact title/dek and dense chronological entries. |
| Each archive entry exposes both a linked title and a separate “open” action. | The row has competing destinations and unused horizontal space. | Make one clear linked story title or the entire bounded row the primary action. |
| Release details are forced into exactly Changes / Acquisition / Compatibility. | Every version reads like generated metadata; rationale, migration, evidence, and related docs have no natural home. | Let release stories use a flexible but validated editorial body with mandatory decision blocks. |
| A short metadata rail remains beside a long article. | Wide screens accumulate dead space while the useful article stays narrow. | Put essential facts near the title; use a compact sticky outline/action rail only when it remains useful. |
| Download matrices visually dominate every native release. | Distribution mechanics eclipse why the user should upgrade. | Keep install and verification prominent but secondary to outcome and upgrade impact. |

### 2.4 Cross-surface issues

- The mobile global header currently scrolls horizontally instead of presenting a deliberate compact
  navigation. This is an escape hatch, not a coherent responsive pattern.
- The site globally removes link underlines. Therefore every inline link must retain another
  persistent, non-color cue; color-only prose links are not acceptable.
- The route and content models are typed, but adding v1.0.4 still touches a closed version union,
  localized release records, hand-authored sitemap/`llms.txt`, release JSON, social metadata, and
  browser-test expectations.
- Manual examples and product copy contain version- and storage-sensitive text. They need semantic
  review rather than blind search-and-replace; for example, a manifest schema version is not
  necessarily the product release version.

## 3. Target users and top tasks

| User | Situation | Top tasks | Success signal |
| --- | --- | --- | --- |
| New DSL author | Has XML experience but not xmlsquish’s execution model | bind the namespace, create an entry, build one prompt, diagnose the first failure | obtains a correct `.prompt` and understands `entry` vs `module` |
| Returning author | Knows the language and needs one rule | find a directive, attribute, allowed context, evaluation rule, or error boundary | reaches a stable anchor in at most two navigational choices |
| Project maintainer | Upgrading to v1.0.4 | understand cache ownership, target layout, cleanup, and operational trade-offs | can predict which files are disposable and what a clean rebuild recreates |
| Automation/release consumer | Installs or audits a release | find the correct binary/source command, checksum, protocol, and machine record | all facts identify the same version and published asset set |
| Evaluator | Decides whether xmlsquish fits a workflow | understand the product boundary without reading the whole manual | can explain source → compile → target and the project-owned state model |

## 4. Page hierarchy and route intent

```text
Home
├── product promise
├── real build example
├── v1.0.4 “project-owned state” section
└── links to latest release and relevant manual chapter

Release log
├── /releases/                         chronological archive
├── /releases/1.0.4/                   current release story
├── /releases/<historical version>/    immutable historical story
└── /releases/<version>.json           machine-readable facts where published

DSL manual
├── /ns/                               orientation and namespace identity
├── /ns/getting-started/
├── /ns/source-model/
├── /ns/composition/
├── /ns/control-and-scope/
├── /ns/build-and-artifacts/           v1.0.4 target/cache explanation
├── /ns/reference/                     grouped lookup + directive contracts
└── /ns/limits-and-invariants/

English mirrors the same human route hierarchy under /en/.
Raw specification and historical snapshots remain language-independent resources.
```

Primary navigation remains **Home / Release log / DSL manual**. Product-wide chrome must not expose
layout implementation names. Detail routes mark their parent destination as current.

## 5. Layout A — ManualLayout

### 5.1 Contract

`ManualLayout` serves sequential learning and random lookup. It owns chapter navigation, local
outline, article width, compact chapter identity, previous/next navigation, and the link to the
normative specification. It does not own DSL prose or the rich manual-overview hero.

### 5.2 Wide-screen wireframe

```text
┌──────────────── shared product header ──────────────────────────────────────┐
│ xmlsquish   Home   Release log   DSL manual       Language  Theme  GitHub  │
└─────────────────────────────────────────────────────────────────────────────┘
┌───────────────┬──────────────────────────────────────┬──────────────────────┐
│ MANUAL        │ DSL manual / Composition             │ ON THIS PAGE         │
│               │                                      │ Explicit contracts   │
│ Overview      │ h1 Macros, parameters, and expansion │ String parameters    │
│ 01 Start      │ one-sentence page contract           │ Structured content   │
│ 02 Sources    │                                      │ …                    │
│ 03 Compose ←  │ explanation                          │                      │
│ 04 Control    │ [Contract] [Example + result]         │ Related              │
│ 05 Build      │ explanation                          │ Full specification   │
│ 06 Reference  │                                      │ Report issue         │
│ 07 Limits     │ previous chapter        next chapter │                      │
└───────────────┴──────────────────────────────────────┴──────────────────────┘
```

- The center article should be approximately `42–48rem` or `65–75ch` for prose.
- The left rail groups chapters by user task, indicates the current route with text plus
  `aria-current="page"`, and remains stable across chapters.
- The right rail is generated from meaningful `h2` and `h3` headings, is omitted for short pages,
  and never repeats every tiny label.
- Sticky rails begin below the product header and stop before the footer. Fragment targets use
  `scroll-margin-top` so focused or linked content is not obscured.

### 5.3 Narrow-screen wireframe

```text
┌──────── brand ───────────── Menu  Theme ┐
│ DSL manual / Composition               │
│ h1 Macros, parameters, and expansion   │
│ [Manual chapters ▾] [On this page ▾]   │
├────────────────────────────────────────┤
│ article                                │
│ code block scrolls inside itself       │
│ previous / next                        │
└────────────────────────────────────────┘
```

- Use native, labeled disclosures when practical so navigation remains available without JavaScript.
- Article content follows the compact navigation controls in DOM and focus order.
- Opening one disclosure must not trap focus, push controls outside the viewport, or require
  horizontal page scrolling.

### 5.4 Manual overview variant

Only `/ns/` receives a richer orientation header. Its first viewport answers:

1. What is this DSL?
2. What exact namespace URI do I bind?
3. What is the smallest valid program?
4. Where do I start, compose, or look up a directive?

The namespace URI is selectable and copy-enhanced; the manual still works when copying or JavaScript
is unavailable. Do not repeat the URI panel as a hero on every chapter.

### 5.5 Directive reference variant

Begin with a compact group matrix:

```text
Project/source        Composition             Values/content       Control
entry module import   macro param expand      arg fill slot insert ifr
```

Each item links to a stable fragment. Each directive detail is a normal reference section:

1. `h2` name and one-sentence purpose;
2. syntax;
3. allowed context and children;
4. attribute contract;
5. evaluation effect/order;
6. valid example and expected result;
7. common invalid usage when useful;
8. related concept and adjacent directives.

Avoid a bordered card around every subsection. Borders should express one directive boundary, not
every fact inside it. Filtering is optional enhancement; all directives remain present without
JavaScript.

## 6. Layout B — ReleaseLayout

### 6.1 Contract

`ReleaseLayout` is an editorial article layout. It owns breadcrumb/back navigation, version/date/
status facts, article width, an optional local outline, install/verification actions, related manual
links, and previous/next release navigation. It does not force every story into the same three
numbered sections.

### 6.2 Release-story wireframe

```text
┌──────────────── shared product header ──────────────────────────────────────┐
│ Release log / v1.0.4                                                       │
│ 20 Sep 2026 · Stable · protocol <verified value>                           │
│ h1 Project-owned build state                                               │
│ dek: outcome in one or two sentences                                       │
│ [Install] [Checksums] [GitHub release]                                     │
├───────────────────────────────────────────────────────┬─────────────────────┤
│ Why this changed                                      │ IN THIS RELEASE     │
│ What changed                                          │ Why / What / Upgrade│
│   Target anatomy (real generated tree)                │                     │
│   Local metadata and cache behavior                   │ FACTS               │
│ Upgrade impact                                        │ Version / protocol  │
│ Clean and rebuild                                     │ Support / assets    │
│ Trade-offs and known limitations                      │                     │
│ Install and verify                                    │ RELATED             │
│ Related manual pages                                  │ Build & artifacts   │
├───────────────────────────────────────────────────────┴─────────────────────┤
│ Previous release          All releases                                  →  │
└─────────────────────────────────────────────────────────────────────────────┘
```

- Essential upgrade facts appear before narrative detail.
- The article column should be roughly `45–55rem`; a rail may use the remaining width only if it
  contains continuing orientation or actions. Otherwise place facts in a horizontal block near the
  title and center the article.
- Downloads are a compact reusable module after upgrade impact, not the visual premise of the page.
- Stable change-group IDs feed the local outline. A fixed three-anchor outline is insufficient when
  the article already contains meaningful subsections.

### 6.3 Release-index variant

The index stays inside `ProductShell` and behaves like a blog archive:

```text
Release log
User-visible changes, upgrade notes, and distribution facts.

2026
20 Sep   v1.0.4  Project-owned build state       Stable · migration note  →
17 Sep   v1.0.2  Readable outputs and recovery   Historical               →
16 Sep   v1.0.1  Publication as a product contract …
```

- Use one primary destination per row.
- Group by year only when it reduces scanning cost; do not add filters for six releases.
- Show version, date, outcome headline, and a small compatibility/status signal.
- Use a quiet latest marker, not another product hero.
- The archive entry summary must describe user impact rather than repeat implementation nouns.

## 7. Content architecture

### 7.1 Manual content model

Each chapter needs inspectable fields equivalent to:

```text
slug, locale, title, description, learningOutcome,
normativeSources[], headings[], body, previous, next, updatedForVersion
```

This is a product requirement, not a demand for a particular storage format. Astro content
collections are a good fit because they provide schemas and type-safe entries, but a validated local
module is acceptable. Build-time checks must ensure:

- the same ordered chapter set exists in both locales;
- all eleven directives occur exactly once in the reference index and once as detail anchors;
- previous/next relationships are total and non-cyclic;
- normative claims link to the relevant raw specification section;
- no chapter silently falls back to the other language;
- referenced code examples are complete or explicitly labeled as fragments.

### 7.2 Release content model

Locale-neutral release facts and localized editorial copy must remain distinct:

```text
Facts: version, tag, date, status, protocol, acquisition, asset URLs,
       checksums, metadata path, previous/next
Copy:  title, dek, upgrade impact, rich change sections, trade-offs,
       limitations, related manual links
```

One validated version entry generates both localized detail routes and supplies the archive. The
site may link to repository Markdown as publication source, but it must not infer asset existence
from a filename template alone; the release workflow is the authority for shipped files.

### 7.3 Writing rules

- Lead with observable user outcomes; introduce internal terms only when they help users predict
  behavior.
- Separate **what is true**, **why the design changed**, and **what the user must do**.
- Label destructive or non-compatible changes before install actions.
- Prefer one real generated directory tree over three paragraphs of storage terminology.
- State trade-offs plainly. Project-local caches improve ownership and cleanup clarity but may
  duplicate bytes between projects; do not hide that consequence.
- Do not claim “faster,” “portable,” “atomic,” “reproducible,” or “secure” without direct evidence
  supporting the exact claim.

## 8. v1.0.4 product-page content inventory

### 8.1 Home

- [ ] Latest-version badge and latest-release link identify v1.0.4.
- [ ] The primary product explanation says that the project target is the storage boundary.
- [ ] A real build example shows the actual v1.0.4 `target/xmlsquish` tree, including public product,
      compiler metadata, and project-local cache categories. Generate this evidence from a fixture;
      do not invent internal directory names in translation strings.
- [ ] Cache-hit/miss language refers to project-owned state, not a machine-global Cargo-like cache.
- [ ] Commands, target paths, and Build Explorer evidence are regenerated with the released binary.
- [ ] A concise trade-off note says local state is disposable and may duplicate data across projects.
- [ ] Home metadata and structured data report v1.0.4.

### 8.2 Release index and v1.0.4 story

- [ ] v1.0.4 is the first/current archive entry; prior entries remain unchanged except for current/
      historical status.
- [ ] Headline expresses the outcome: project-owned build state, not merely “cache refactor.”
- [ ] Opening decision block states compatibility/migration impact before downloads.
- [ ] “Why” explains why a small prompt tool does not benefit enough from a machine-global shared
      cache to justify its ownership and cleanup complexity.
- [ ] “What changed” shows the actual target anatomy and explains ownership of outputs, compiler
      metadata, cache entries, and any journals/indexes that remain.
- [ ] “Upgrade impact” states that old global state is not reused, automatic migration is not
      promised, and a cold project-local build recreates derived state—only if implementation tests
      confirm those exact behaviors.
- [ ] “Clean and rebuild” documents precisely which project-local state `clean` removes and what the
      next build recreates.
- [ ] “Trade-offs and limitations” mentions cross-project duplication and avoids unverified
      portability/performance claims.
- [ ] “Install and verify” contains published platform assets, source install, checksum link, and
      signing boundary consistent with the release workflow.
- [ ] Related links point to the Build & artifacts manual chapter and raw release notes.

### 8.3 DSL manual

- [ ] Build & artifacts contains the real v1.0.4 directory anatomy and lifecycle.
- [ ] Getting started ends at the real product path and does not teach a deprecated global cache.
- [ ] Limits/invariants distinguishes authoritative source/lock inputs from disposable derived state.
- [ ] The manual states whether custom target directories move all owned metadata/cache state or only
      products; copy follows verified implementation behavior.
- [ ] No language-semantic or namespace-version claim is changed unless v1.0.4 actually changes it.

### 8.4 Publication and discovery

- [ ] Add both localized `/releases/1.0.4/` detail routes.
- [ ] Publish `/releases/1.0.4.json` if v1.0.4 has a machine-readable release record.
- [ ] Update canonical/hreflang links, sitemap, `llms.txt`, JSON-LD, Open Graph text/art, and any
      `rel="alternate"` release metadata.
- [ ] Update the typed version catalog, localized release copy, route-count expectations, asset
      expectations, and latest-version browser tests from the same release facts.
- [ ] Verify every advertised asset URL only after the release workflow has published it.

## 9. Responsive and accessibility acceptance criteria

### 9.1 Global shell

- [ ] Exactly one shared product header and footer appear on every human route.
- [ ] The current top-level destination uses `aria-current="page"`; child release/manual routes keep
      their parent destination current.
- [ ] At narrow widths, global navigation becomes one deliberate disclosure. The page must not rely
      on horizontal header scrolling to reveal essential destinations.
- [ ] Navigation item wording and order remain consistent across desktop and mobile.
- [ ] Skip navigation targets the unique `main` landmark.

### 9.2 Reflow and responsive behavior

- [ ] At 320, 390, 768, and 1440 CSS px, no document has horizontal overflow.
- [ ] At 400% zoom, content reflows to the equivalent of 320 CSS px without loss of information or
      functionality, except bounded content that genuinely requires two-dimensional presentation.
- [ ] Code and wide tables scroll inside labeled bounded regions and never widen the document.
- [ ] Manual left/right rails collapse before they reduce the article below a readable width.
- [ ] Release facts and download rows stack in label/value order; the article remains the primary
      reading region.
- [ ] Chinese and English layouts are tested independently rather than assuming equal copy length.

### 9.3 Semantics and keyboard behavior

- [ ] Every page has one `h1`; article headings form a logical `h2`/`h3` hierarchy with no visual-only
      ranks.
- [ ] Multiple navigation landmarks have distinct localized labels: Product, Manual chapters,
      On this page, Chapter pagination, and Release pagination as applicable.
- [ ] Keyboard focus follows visual/task order and is never trapped by responsive disclosures.
- [ ] Sticky headers or rails never obscure focused controls or fragment targets.
- [ ] Focus indication remains visible and meets or exceeds the existing 3 px outline treatment.
- [ ] Interactive targets meet WCAG 2.2’s 24 by 24 CSS px minimum; primary controls should target
      approximately 44 px when space permits.
- [ ] Normal text reaches 4.5:1 contrast; large text and meaningful UI boundaries reach 3:1.
- [ ] Because underlines are removed, inline links retain a persistent non-color cue such as weight
      plus marker treatment, an arrow, or a bounded resource treatment, and strengthen on hover/focus.
- [ ] Motion is nonessential and honors `prefers-reduced-motion`.

### 9.4 No-JavaScript behavior

- [ ] All chapters, directive details, release stories, downloads, locale switches, and previous/next
      links remain reachable.
- [ ] Mobile navigation and chapter/outline access use native fallbacks.
- [ ] Copy and filter controls are absent or inertly hidden when they cannot operate; selectable source
      text and the full directive list remain.
- [ ] Theme enhancement may degrade, but content and contrast remain usable.

## 10. Acceptance matrix

| Scenario | Expected observable behavior |
| --- | --- |
| Open a deep manual link on desktop | Compact chapter header, current chapter, readable article, useful local outline, and stable fragment landing are visible without a marketing hero. |
| Open the directive reference | A grouped lookup matrix appears before details; every directive has one `h2` anchor and a consistent contract sequence. |
| Read the manual at 320 px with JavaScript off | Product navigation and chapters remain reachable; article and code do not create page overflow. |
| Browse the release index | Versions scan chronologically with one primary story link and compact compatibility/status context. |
| Open v1.0.4 | Upgrade impact and project-local state model appear before downloads; the article links to the build/artifact chapter. |
| Compare v1.0.4 and v1.0.2 | Each page preserves its own historically accurate cache/storage model and assets. |
| Switch language on any detail route | The equivalent version/chapter opens, canonical and reciprocal alternates remain correct, and no content silently falls back. |
| Inspect release truth surfaces | Home, archive, detail, JSON, structured metadata, sitemap, `llms.txt`, commands, and asset links all identify v1.0.4 consistently. |

## 11. Implementation handoff: file boundaries

The names below are recommended boundaries for the current Astro repository. Equivalent names are
acceptable, but collapsing both reading grammars back into page components is not.

| File or area | Owns | Must not own |
| --- | --- | --- |
| `site/src/layouts/ManualLayout.astro` | `ProductShell` composition; compact chapter heading; desktop chapter rail; optional generated page outline; mobile native disclosures; article-width grid; previous/next/specification footer; manual-only responsive CSS | DSL chapter prose, directive facts, namespace-copy behavior, release UI |
| `site/src/layouts/ReleaseLayout.astro` | `ProductShell` composition; release breadcrumb and editorial title/dek; version/date/status fact block; optional generated outline/action rail; article width; install/verification slot; previous/all/next navigation; release-article responsive CSS | version-specific copy, asset URL construction, archive rows, manual UI |
| `site/src/components/NamespacePage.astro` | manual overview composition and selection/rendering of the requested chapter content through `ManualLayout` | global shell CSS, duplicated chapter navigation, repeated per-chapter namespace hero |
| `site/src/components/ReleaseDetailPage.astro` | map one typed `ProductRelease` and localized story into `ReleaseLayout` slots; emit version-specific structured data | generic article geometry, hard-coded three-section layout, archive rendering |
| `site/src/components/ReleaseIndexPage.astro` | compact archive masthead and chronological semantic list driven by the release catalog | release-article layout or download matrices |
| `site/src/data/dsl.ts` plus structured manual content | ordered chapter/directive identity, source mappings, localized long-form bodies or references to them, and validation | visual layout and global UI labels |
| `site/src/data/releases.ts` | locale-neutral version, protocol, acquisition, asset, metadata, and chronology facts | translated prose or page geometry |
| `site/src/i18n/manual.ts`, `releases.ts`, `messages.ts` | short interface labels and localized editorial copy not moved into content entries | duplicated locale-neutral version/route/asset facts |
| `site/src/layouts/ProductShell.astro` | one product header/footer and responsive product navigation | manual rails, release facts, or page-specific article widths |

Small shared components are justified only when both layouts need the exact semantic behavior. A
generated `PageOutline.astro` may be shared; a generic “three-column content layout” should not be,
because it would erase the deliberate difference between manual lookup and release storytelling.

During parallel implementation, assign one writer to `ProductShell.astro` and shared UI labels;
assign separate writers to the manual and release regions above. The manual worker should not edit
release data/components, and the release worker should not edit DSL content/components.

## 12. Delivery sequence and ownership seams

1. **Freeze content contracts:** release facts/editorial schema, chapter manifest, and the verified
   v1.0.4 target-tree terminology.
2. **Extract ManualLayout:** preserve routes, then introduce compact headers, separate chapter/local
   navigation, semantic callouts, and responsive/no-JS behavior.
3. **Refactor reference content:** grouped matrix, `h2` directive sections, flatter visual hierarchy,
   and optional filtering.
4. **Extract ReleaseLayout:** flexible article body, decision-first facts, useful outline, compact
   acquisition module, and previous/next navigation.
5. **Restyle release index:** compact blog/archive chronology using the same release catalog.
6. **Publish v1.0.4 content:** home evidence, release story, manual updates, machine metadata, discovery
   files, and assets.
7. **Validate:** type/build checks, route/link/metadata contracts, no-JS, keyboard, 320–1440 px reflow,
   both locales, both themes, and screenshots of representative page types.

The shared `BaseLayout` and `ProductShell` should have one owner during extraction. Manual content and
release editorial content can proceed independently once their schemas are fixed.

## 13. External precedents and what to adopt

These are production precedents, not controlled evidence that copying a visual treatment causes
better outcomes.

| First-party source | Observed pattern | Adopt | Do not copy |
| --- | --- | --- | --- |
| [Astro Starlight sidebar](https://starlight.astro.build/guides/sidebar/), [configuration](https://starlight.astro.build/reference/configuration/), and [layout anatomy](https://starlight.astro.build/reference/overrides/) | Grouped global sidebar, `h2`/`h3` page outline, and mobile adaptations are separate concerns. | Separate chapter navigation from local outline; generate both from content structure. | A second framework/design system for seven chapters. |
| [Rust release archive](https://blog.rust-lang.org/releases/) | A compact, year-grouped archive separates finding a release from reading its independent announcement; stable latest/version shortcuts aid retrieval. | Version-addressable stories and a scan-first archive. | Date-based canonical slugs or Rust’s organizational breadth. |
| [Vite blog archive](https://vite.dev/blog) and [Vite 8 announcement](https://vite.dev/blog/announcing-vite8) | Archive entries stay terse; a story explains rationale, user impact, migration, and exits to exhaustive changelogs. | Give v1.0.4 a narrative “why / impact / migration” story. | Long ecosystem celebration when a patch release needs a focused upgrade decision. |
| [GitHub Changelog redesign](https://github.blog/changelog/2025-05-05-improvements-to-changelog-experience/) | GitHub added change-type categories, curated product tags, and month/year grouping to improve differentiation and retrieval. | Use explicit status/compatibility labels and chronological grouping. | Filters and taxonomy machinery for six releases. |
| [Stripe API changelog](https://docs.stripe.com/changelog) | Dense tables expose affected products, breaking-change status, category, and version lineage before detail. | Surface upgrade impact as structured facts. | Stripe’s scale-driven table density and API-family hierarchy. |
| [Astro content collections](https://docs.astro.build/en/guides/content-collections/) | Similar entries can be schema-validated and queried at build time. | Validate release/chapter facts and generate routes/navigation. | Treating the storage API itself as a product requirement. |
| [W3C page regions](https://www.w3.org/WAI/tutorials/page-structure/regions/) and [headings](https://www.w3.org/WAI/tutorials/page-structure/headings/) | Landmarks and logical headings communicate page organization and enable assistive navigation. | Distinct labeled nav regions and a truthful heading hierarchy. | ARIA roles where native HTML already provides the correct semantics. |
| [WCAG 2.2](https://www.w3.org/TR/WCAG22/) | Reflow, target size, consistent navigation, and focus-not-obscured are testable user outcomes. | Make them release gates for the new sticky, multi-rail layouts. | Treating conformance as visual polish rather than behavior. |

## 14. Decision log and falsification conditions

### Decisions

- Use `ManualLayout` and `ReleaseLayout` under the shared product shell.
- Keep the release archive as a compact index, not a third long-form layout.
- Make the rich namespace identity an overview feature, not a repeated chapter hero.
- Make v1.0.4 decision-first: migration and ownership before download mechanics.
- Show target/cache structure from generated evidence rather than manually duplicated diagrams.

### Material trade-offs

- Two layouts add code, but remove page-level duplication and make semantics testable.
- Structured bilingual long-form content increases editorial work, but prevents a single conditional
  component from becoming the de facto CMS.
- Project-local caches simplify ownership and cleanup while giving up cross-project sharing; this is
  a chosen product boundary, not a free optimization.
- A three-column manual helps lookup on wide screens but must collapse early enough to preserve the
  article. The right rail is optional, not an obligation on every page.

### Evidence that would revise this specification

- If heading analytics or user tests show that a right-hand outline is unused or harms reading at
  common laptop widths, keep the generated outline but render it inline near the chapter header.
- If the final v1.0.4 implementation keeps any state outside the configured project target, revise
  “project-owned” copy and the target diagram before publication; do not explain away the mismatch.
- If the release workflow does not publish v1.0.4 native assets or JSON, omit those links rather than
  templating nonexistent resources.
- If the manual’s language semantics change with v1.0.4, treat that as a separate normative DSL
  revision with specification evidence, not as incidental release copy.
