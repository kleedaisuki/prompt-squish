# xmlsquish Site: Release Log and DSL Manual, Iteration 2

- **Status:** implementation-ready product specification
- **Decision date:** 2026-09-17
- **Scope:** human-facing website information architecture and interaction
- **Locales:** Simplified Chinese (`zh-CN`) and English (`en`)
- **Production origin:** `https://xmlsquish.moesegfault.dev`
- **Supersedes:** the release and namespace page composition in
  `docs/design/v1.0.1-site-product-plan.md`

## 1. Product decision

The second iteration keeps one coherent xmlsquish brand while assigning each destination the layout
that matches its user job:

1. **Home is the product story.** Its existing editorial landing-page composition remains intact.
2. **Release log is a chronological directory.** `/releases/` lists versions; selecting one opens a
   first-party, version-specific release detail page.
3. **XML namespace is the DSL user manual.** `/ns` remains the exact namespace identity and becomes
   the entry to a task-oriented, multi-chapter manual with persistent manual navigation.
4. **The global header is one row.** Home, Release log, and XML namespace sit on the same horizontal
   axis as the xmlsquish brand, language control, theme control, and GitHub action. The current second
   navigation row is removed.

“Release and XML Namespace must not use the home-page style” is interpreted as **do not reuse the
home-page layout or storytelling composition**, not “invent unrelated brands.” All surfaces keep the
same logo, color tokens, typography family, link language, theme behavior, controls, border treatment,
and footer. They differ in information architecture:

| Surface | Layout grammar | Must not become |
| --- | --- | --- |
| Home | Wide editorial story, large hero, demonstrations, alternating bands | A changelog or manual |
| Release index | Compact reverse-chronological log | A second marketing landing page |
| Release detail | Version masthead followed by factual release sections | A copy of Home or a raw commit dump |
| DSL manual | Application-like manual shell: chapter nav, readable article, in-page outline | A decorative landing page or raw Markdown wall |

The implementation may replace the current internal components and content model. Stable public URLs,
the XML namespace identity, release metadata resources, accessible behavior, and localized routes are
external contracts and must remain valid.

## 2. Requirements and necessary implications

| Classification | Requirement or implication | Product response |
| --- | --- | --- |
| Explicit | Put all three destinations on the same height as brand, language, theme, and GitHub. | Replace the two-level header with one 64 px row on wide screens; use a same-row menu trigger when the full set cannot fit. |
| Explicit | Release and namespace must not use the home layout. | Give Releases a log/detail composition and Namespace a manual reading composition; reuse brand primitives, not Home sections. |
| Explicit | Releases need a log directory and version-specific pages. | `/releases/` becomes an index. Every listed version links to `/releases/<version>/`; English mirrors under `/en/`. |
| Explicit | XML Namespace becomes a designed DSL user manual. | Split the 850-line normative specification into task-oriented chapters, with manual navigation, code examples, directive reference, and version policy. |
| Necessary | A release index is useful only if every visible version resolves locally. | Ship detail pages for `1.0.1`, `1.0.0`, `0.3.0`, and `0.2.0` in both locales in the same change. |
| Necessary | `/ns` is both a dereferenceable namespace and a human manual route. | Keep `https://xmlsquish.moesegfault.dev/ns` byte-for-byte unchanged and make it the manual overview; localized English documentation describes that URI but does not define another namespace. |
| Necessary | A single-row header cannot fit all labels at every width. | Preserve a single physical header bar and collapse content navigation into a keyboard-operable disclosure below the fit threshold rather than adding a permanent second row. |
| Existing contract | Human links use no underline while retaining visible affordance. | Keep accent, weight, boundary/shape, arrow where useful, hover wash, and a strong `:focus-visible` ring; never rely on color alone for ambiguous body links. |
| Existing contract | The static site remains useful without JavaScript. | Navigation, all release content, every manual chapter, anchors, and raw resources work without client scripting. Copy, filtering, and auto-close behaviors are optional enhancements. |

## 3. Users and jobs

| User | Primary question | Minimum successful flow |
| --- | --- | --- |
| Evaluating developer | “What is xmlsquish?” | Home → understand product → Release log or DSL manual |
| Returning user | “What versions exist and what changed?” | Release log → choose version → read migration impact and changes |
| Installer | “Where is the binary for this exact version?” | Release detail → platform/architecture download → checksum instructions |
| Automation owner | “Did the machine protocol change?” | Release detail → protocol and required-action callout |
| First-time DSL author | “How do I create and build a working file?” | Manual overview → Getting started → copy example → build |
| Experienced DSL author | “What does `xs:fill` permit?” | Manual → Directive reference anchor → related composition chapter |
| Tool author | “What is stable and machine-readable?” | Exact namespace URI, raw current spec, immutable snapshots, versioned release JSON |

The primary success measure is completion of these tasks, not time on page or decorative consistency.

## 4. Route and resource architecture

### 4.1 Human-facing routes

All human routes use trailing slashes except the canonical namespace URI, whose exact identity remains
the slashless `/ns`. The server may serve `/ns/` as the same document, but canonical metadata and every
XML example must use the slashless URI.

```text
Simplified Chinese                         English
────────────────────────────────────────  ───────────────────────────────────────
/                                         /en/

/releases/                                /en/releases/
/releases/1.0.1/                          /en/releases/1.0.1/
/releases/1.0.0/                          /en/releases/1.0.0/
/releases/0.3.0/                          /en/releases/0.3.0/
/releases/0.2.0/                          /en/releases/0.2.0/

/ns                                       /en/ns/
/ns/getting-started/                      /en/ns/getting-started/
/ns/source-model/                         /en/ns/source-model/
/ns/composition/                          /en/ns/composition/
/ns/control-and-scope/                    /en/ns/control-and-scope/
/ns/build-and-artifacts/                  /en/ns/build-and-artifacts/
/ns/reference/                            /en/ns/reference/
/ns/limits-and-invariants/                /en/ns/limits-and-invariants/
```

There is deliberately no separate `/docs/` root. The DSL manual is the useful human representation
obtained by dereferencing the namespace. There is also no `/blog/`; the release log is a product
history, not a publication system.

### 4.2 Stable machine and source resources

These URLs do not move, localize, or acquire the human page shell:

| URL | Contract |
| --- | --- |
| `/releases/1.0.1.json` | Current versioned machine-readable release record |
| `/releases/1.0.0.json` | Existing versioned historical release record |
| `/ns/dsl.md` | Current raw normative DSL specification generated from `docs/dsl.md` |
| `/ns/0.3.0/dsl.md` | Immutable 0.3.0 DSL snapshot |
| `/ns/0.2.0/dsl.md` | Immutable 0.2.0 DSL snapshot |
| `https://xmlsquish.moesegfault.dev/ns` | Exact XML built-in namespace identity and Chinese manual overview |

Do not rewrite JSON or Markdown requests to HTML detail pages. Do not make `/en/ns/` a second XML
namespace. The manual must explicitly explain the distinction among **namespace identity**, **current
manual**, **product version**, and **immutable language snapshot**.

### 4.3 Canonical, alternate, and discovery metadata

- Every localized human page has a self-canonical URL and reciprocal `hreflang="zh-CN"`,
  `hreflang="en"`, and `x-default` links.
- Language switching preserves the exact page and version: for example,
  `/releases/1.0.0/` ↔ `/en/releases/1.0.0/` and `/ns/composition/` ↔
  `/en/ns/composition/`.
- Sitemap generation includes every human route, but excludes raw JSON and Markdown resources.
- Release detail structured data identifies its own version, date, release URL, and downloads. It must
  not inherit the latest release's version when rendering an older entry.
- The home version cue links to `/releases/1.0.1/`, while the header's Release log item links to the
  index `/releases/`.

## 5. One-row global header

### 5.1 Wide-screen anatomy

At widths where the labels fit, the header is exactly one row:

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ [mark] xmlsquish   Home   Release log   XML namespace   English  Theme  GitHub │
└──────────────────────────────────────────────────────────────────────────────┘
```

- Use one inner container with three logical groups: brand, content navigation, utilities.
- The bar is 64 CSS pixels high (`min-height` and block size), sticky, and vertically centers every
  direct control on the same cross-axis.
- Remove the current `.product-nav` second row and its top border entirely.
- Brand, primary links, language, theme, and GitHub all use at least a 40 by 40 CSS pixel interactive
  box; icon-only controls have accessible names.
- The primary navigation occupies the flexible middle region. It never wraps.
- Current destination uses `aria-current="page"` and the existing accent-wash/pill language. Release
  detail pages count as Release log; every manual chapter counts as XML namespace.
- The brand goes to locale Home. The product tagline may remain beside the mark only while it does not
  force navigation into a second row.

Suggested breakpoint behavior is based on actual content fit, not a device label:

| Available width | Header behavior |
| --- | --- |
| `>= 960px` | Full brand, three inline destinations, language, theme, GitHub in one 64 px row |
| `720–959px` | Hide brand tagline; keep wordmark; replace three inline destinations with a same-row “Menu” disclosure |
| `< 720px` | Keep mark/short wordmark and the same-row Menu, language, theme, and GitHub controls; abbreviate the visible language label if necessary |
| `< 360px` | The wordmark may visually collapse to the mark, but its accessible name remains “xmlsquish” |

The final breakpoint must be chosen by overflow tests in both languages. Chinese labels and English
labels must both fit without collision at one zoom level equivalent to 200% desktop zoom.

### 5.2 Collapsed navigation

- The collapsed control remains in the same 64 px bar. Opening it reveals a positioned panel below
  the bar; it does not create a permanent second header row or shift page content.
- Prefer a native disclosure (`details`/`summary`) so all links remain operable without JavaScript.
- The panel contains Home, Release log, and XML namespace, each with the same current-page semantics.
- Escape-to-close and close-after-selection may be enhanced with JavaScript, but are not prerequisites
  for access. If custom dialog behavior is introduced, implement focus management and return focus.
- A click/tap outside may close an enhanced menu; losing enhancement must not strand or hide links.
- The panel must not be clipped by the sticky header or page shells, and must remain above manual
  sidebars and release content.

## 6. Release product

### 6.1 Release index: `/releases/`

The index is a directory, not a hero-led landing page. Its information hierarchy is:

1. **Compact masthead** — “Release log,” a one-paragraph explanation, and a small current stable
   version indicator. Avoid the Home page's split hero, decorative build graph, and long editorial
   headline.
2. **Latest release entry** — first item in the same chronology, given a “Latest” label but not a
   different mini landing page. It contains version, date, status, protocol version, headline,
   two-sentence outcome summary, and migration severity.
3. **Chronological log** — all releases newest first, optionally grouped by year when more than one
   year exists. Initial entries are `1.0.1`, `1.0.0`, `0.3.0`, and `0.2.0`.
4. **Distribution note** — a compact footer note linking to GitHub Releases and explaining that each
   local detail page provides assets and verification instructions.

Each entry uses a consistent row/card, not a marketing feature card. The version heading is the
primary local link to `/releases/<version>/`; an explicit “View release” affordance repeats the target
for clarity. Do not make a nested-interactive whole card. GitHub is a secondary external link, never
the main click target.

Required visible fields per index item:

```text
version, date, stability, machine protocol (when applicable),
headline, summary, migration level, local detail URL
```

Migration levels use text and icon/shape, never color alone:

- **No action** — behavior-compatible maintenance.
- **Review** — some users should inspect changes.
- **Action required** — a consumer or source migration is required.

Do not add search, tag filtering, subscriptions, pagination, or infinite scroll for four entries.
Introduce pagination only after the chronological list becomes materially difficult to scan.

### 6.2 Release detail: `/releases/<version>/`

Every release detail uses this order, omitting only sections genuinely absent from that release:

1. **Back link and version masthead** — “All releases,” `vX.Y.Z`, date, stability, protocol, and a
   short outcome-led summary.
2. **Migration/upgrade callout** — the most consequential action appears before downloads. For 1.0.1,
   this must plainly state that machine protocol 3.0 is non-additive despite the patch version.
3. **Install and downloads** — tagged source command plus platform/architecture archives available for
   that version. Never construct old asset URLs with the latest version.
4. **Verification and platform trust** — `SHA256SUMS`, OS-specific command examples, unsigned and
   Apple-notarization boundary, and archive legal notices.
5. **Highlights** — outcome-led changes grouped by user-facing concern.
6. **Detailed changes** — factual product and protocol behavior, not a commit-title transcript.
7. **Compatibility, limitations, and known issues** — concise, version-specific boundaries.
8. **Release provenance** — immutable GitHub Release, tag, machine JSON when one exists, and source
   release notes.
9. **Previous/next navigation** — chronologically adjacent local release pages plus “All releases.”

Historical releases have different distribution facts. A detail page must render only assets and
commands that actually existed for that version. In particular, do not retroactively imply that 0.2.0
had the six native archives if its release used tagged-source installation.

### 6.3 Release content model

Use one typed, ordered catalog as the page-generation source. A record needs at least:

```text
version, tag, date, stability, protocol?, headline[locale], summary[locale],
migration: { level, title[locale], body[locale], actions[locale][] },
distribution: { sourceInstall?, platforms[], checksums?, trustBoundary[locale] },
highlightGroups[locale][], compatibility[locale][], limitations[locale][],
githubRelease, machineMetadataPath?, sourceNotesPath?, previous?, next?
```

This replaces the current single `latestRelease` assumption. The catalog is sorted semantically by a
validated version key, and exactly one record is marked current. The index, localized detail routes,
Home's current-version cue, download URLs, JSON-LD, and previous/next links derive from this catalog.

Build-time validation must reject:

- duplicate versions or routes;
- an index entry without two localized detail pages;
- more or fewer than one current release;
- mismatched asset filenames/tags/versions;
- a current version different from root `Cargo.toml` and current release JSON;
- a machine JSON path that does not exist;
- a locale missing its summary, migration text, or change content.

The bilingual Markdown files in `docs/releases/` remain release-authoring evidence. The visible pages
must not parse one mixed-language block at runtime or display both languages together. Implementation
may migrate their facts into the typed catalog or introduce a validated content collection, provided
there is one authoritative record per version and drift checks remain inspectable.

### 6.4 Release visual composition

Reuse brand colors, typography, buttons, focus rings, light/dark tokens, maximum page width, and footer.
Do not reuse the Home page's giant serif hero, two-column build graphic, alternating journey bands,
capability feature grid, or closing marketing CTA.

The Release index should feel like a maintained ledger: compact title, clear chronology axis, quiet
metadata, strong version headings, and consistent rows. A detail page may use cards for downloads and
warnings, but the main text follows a simple top-to-bottom document rhythm. Target prose width is
approximately 65–75 characters; asset tables may extend wider or reflow into cards on narrow screens.

## 7. DSL user manual

### 7.1 Manual shell

The manual uses the shared one-row global header and footer, then a dedicated reading shell:

```text
┌───────────────┬──────────────────────────────────────┬────────────────┐
│ Chapter nav   │ Article                              │ On this page   │
│               │ h1, lead, examples, prose, tables    │ h2 anchors     │
│               │ previous / next chapter              │                │
└───────────────┴──────────────────────────────────────┴────────────────┘
```

- Desktop (`>= 1120px`): sticky chapter rail, readable article column, and optional in-page outline.
- Medium (`720–1119px`): chapter rail remains or becomes a compact left rail; hide the in-page outline.
- Narrow (`< 720px`): no persistent rail. A visible “Manual chapters” native disclosure appears at the
  top of the article, followed by the current chapter label. It works without JavaScript.
- Sticky elements begin below the 64 px global header and never overlap headings reached by fragments.
- Every page ends with previous/next chapter links and a path back to the manual overview.

The shell retains xmlsquish colors, code styling, border radii, and type families, but not Home's hero,
product demonstration bands, metric graphics, or marketing CTA rhythm. It is a deliberate manual
layout: stable navigation and readable explanations take precedence over decoration.

### 7.2 Chapter hierarchy and content ownership

The manual reorganizes the normative `docs/dsl.md` by user task. It does not weaken or silently change
normative semantics.

| Route | Chapter title | Required content from the current specification | User outcome |
| --- | --- | --- | --- |
| `/ns` | DSL manual overview | Exact namespace URI, two-layer model, `module` vs `entry`, smallest complete example, manual learning paths, version policy | Understand what the DSL is and choose a path |
| `/ns/getting-started/` | Getting started | Create the smallest project, bind `xmlns:xs`, author module and entry files, build, inspect output, common first errors | Produce one working `.prompt` |
| `/ns/source-model/` | Sources, names, and imports | QName/expanded names; symbol uniqueness; `SourceUnit`, `SourceId`, frozen runs; module/entry restrictions; `xs:import`; definition location | Predict identity and loading |
| `/ns/composition/` | Macros and structural composition | `xs:macro`, `xs:param`, `xs:expand`, `xs:arg`, return values, `xs:slot`, `xs:fill`, evaluation order, isolated call frames | Reuse definitions without hidden context |
| `/ns/control-and-scope/` | Values, scope, and control | Scalar environments; `arg.*`, `file.*`, `match.*`; `xs:insert`; `xs:ifr`; captures; string construction/destruction; recursion and computation | Write conditional and recursive expansions safely |
| `/ns/build-and-artifacts/` | Build, link, and artifacts | Project build; source snapshot/frontend; binary XSIR reuse; link; `.prompt`, `.xsir`, `.psdbg`; determinism; diagnostics and error model | Understand the observable build boundary |
| `/ns/reference/` | Directive reference | All eleven built-ins (`module`, `entry`, `import`, `macro`, `param`, `expand`, `arg`, `fill`, `slot`, `insert`, `ifr`), allowed parents/children, attributes, required/optional values, stable fragment IDs | Answer syntax questions quickly |
| `/ns/limits-and-invariants/` | Limits, exclusions, and invariants | Resource budgets; cycles and recursion; abstract/core model as needed; explicitly excluded features; complete language invariants; normative resource links | Know hard boundaries and non-goals |

English routes mirror the same chapter order and semantics under `/en/ns/`. English is not a summary
edition. Code, formulas, tables, normative modal strength, and warnings must convey equivalent meaning.

### 7.3 Manual overview: `/ns`

The overview is not a repeat of Home's product story. Its first viewport answers four questions:

1. What is the DSL?
2. What exact namespace URI must I bind?
3. What is the smallest valid entry?
4. Where should I go next?

Required opening content:

- “DSL user manual” title and current language/specification status;
- exact selectable `https://xmlsquish.moesegfault.dev/ns` URI;
- a progressively enhanced Copy URI action;
- a concise explanation that the URI identifies built-ins and is not fetched during builds;
- one complete, valid `xs:entry` example and matching build command;
- three learning-path links: **First project**, **Compose macros**, and **Look up a directive**;
- a compact language map showing XML structure values versus Unicode scalar values;
- current product version, current raw specification, and historical snapshot links.

### 7.4 Directive reference behavior

The reference is a manual chapter, not eleven marketing cards. It begins with a compact table/index and
then provides one stable anchored section per directive:

```text
#module, #entry, #import, #macro, #param, #expand,
#arg, #fill, #slot, #insert, #ifr
```

Each directive section contains, in this order:

1. signature/syntax example;
2. purpose in one sentence;
3. allowed context and children;
4. attribute table with required/optional status;
5. result/effect and evaluation order;
6. one valid example;
7. one common invalid usage or diagnostic where useful;
8. links to the conceptual chapter and adjacent directives.

Optional client-side filtering may narrow the index. Without JavaScript, all entries and anchors remain
present. A search result must not be the only way to reach content.

### 7.5 Normative and explanatory content

Manual pages must distinguish:

- **Normative rule** — language behavior the implementation must obey;
- **Explanation** — rationale or mental model;
- **Example** — illustrative, runnable input and expected result;
- **Warning** — invalid use, migration issue, or operational boundary;
- **Implementation note** — non-contractual behavior, used sparingly.

Use visually consistent callouts with text labels and icons; do not encode category only by color.
Normative “must,” “must not,” and “may” from `docs/dsl.md` cannot be softened by marketing copy. The raw
specification remains the final normative resource and is linked from every chapter footer.

Code examples must be syntactically complete where the user is expected to copy them. Every tutorial
command declares its working directory and expected output. Do not present fragments as runnable
documents without labeling them as fragments.

### 7.6 Manual content source and drift prevention

`docs/dsl.md` is the normative source. The designed manual is a structured explanation of it. Avoid
copying all 850 lines into locale message objects, where semantic changes would drift invisibly.

The implementation should establish an inspectable content pipeline with:

- one ordered chapter manifest containing slug, localized title, description, source mapping, and
  previous/next relationship;
- localized authored chapter bodies with explicit links to the normative section(s) they explain;
- a directive manifest shared by the reference index and detail sections;
- build checks that all eleven directives exist exactly once and that every chapter route exists in
  both locales;
- a review checklist requiring DSL semantic changes to update the raw spec and affected manual pages
  together.

The product requirement is synchronization and inspectability, not a particular Astro content API.

## 8. Shared interaction and accessibility contract

### 8.1 Link language without underlines

Human pages keep the requested underline-free interaction at rest, hover, focus, active, and visited
states. Removing an underline does not remove affordance:

- navigation links have pill/selection boundaries;
- release versions use heading weight, a directional arrow, and row boundary;
- manual body links use accent color plus weight and a subtle inline highlight/marker treatment;
- resource links use cards or labeled buttons;
- `:focus-visible` uses a high-contrast outline with offset on every link and control;
- hover strengthens the same highlight language rather than moving text or changing layout.

Do not globally suppress `outline`. Do not rely on coral versus body text alone for inline links.

### 8.2 Semantic and keyboard behavior

- One `h1` per page; headings descend without skipped levels in the main article.
- Global navigation and manual navigation use named `<nav>` landmarks.
- The release index uses a list/ordered chronology. A version title is a real link.
- Tables have headers and captions when their purpose is not obvious from a preceding heading.
- Fragment navigation lands below sticky chrome via `scroll-margin-top`.
- Current chapter and current product destination expose `aria-current="page"`.
- Copy actions announce success/failure in a polite live region, but the selectable source text remains.
- Decorative symbols are hidden from assistive technology; icon-only controls have meaningful names.
- Theme selection remains operable by keyboard and reflects its selected state.
- Motion respects `prefers-reduced-motion`; no essential meaning depends on animation.

### 8.3 No-JavaScript behavior

With JavaScript disabled:

- all human routes render complete localized content;
- the responsive header menu and mobile manual chapters are accessible through native disclosures;
- release downloads and previous/next navigation work;
- code and namespace URI remain selectable;
- search/filter controls that cannot work are absent rather than dead;
- copy buttons that cannot work are absent rather than dead;
- theme falls back to the initial server/system-resolved presentation without obscuring content.

### 8.4 Responsive behavior

- No page has horizontal document overflow at 320 CSS px width.
- Release metadata rows wrap as label/value blocks; download grids become one column.
- Wide code samples scroll inside their own region and never widen the document.
- Manual sidebars collapse according to Section 7.1; article remains first in reading order after the
  mobile chapter disclosure.
- Tables either use a labeled horizontal scroll container or transform into semantic stacked records;
  do not shrink text below the base readable size.
- Tap targets meet at least 24 by 24 CSS pixels, with 40 by 40 preferred for header controls.

## 9. Migration from the current site

| Current state | Required migration | Compatibility treatment |
| --- | --- | --- |
| `ProductShell.astro` has `header-primary` plus a second `.product-nav` row. | Replace it with a one-row header primitive and responsive native disclosure. | Header internals may be rewritten; routes and actions remain. |
| `/releases/` renders all 1.0.1 content and embeds older release cards at fragments. | Make it the chronological index. Move 1.0.1 into `/releases/1.0.1/`; create detail pages for all four versions. | Keep old `#v1-0-0`, `#v0-3-0`, and `#v0-2-0` IDs on corresponding index entries so existing fragment URLs still land usefully. |
| `ReleasePage.astro` assumes one latest release. | Split index and detail compositions driven by an ordered release catalog. | No internal component compatibility required. |
| `releases.ts` models only the current version. | Model the complete release catalog and version-specific distribution facts. | Preserve existing JSON and GitHub asset URLs. |
| `/ns` is a single visual namespace explorer. | Make it the manual overview and add seven chapter routes in each locale. | Preserve exact namespace URI, raw spec route, snapshots, and all eleven directive anchors on `/ns/reference/`. |
| `NamespacePage.astro` holds identity, search cards, pipeline, and resources in one page. | Decompose into manual shell, overview, chapters, and directive reference. | No internal component compatibility required; visible facts must survive in the appropriate chapter. |
| Reference strings live in a large locale object. | Move long-form content into reviewable localized manual sources and retain short UI labels in i18n messages. | Build fails on missing locale content rather than falling back silently. |
| Tests assert only the two top-level reference routes. | Test index/detail matrices, chapter matrices, one-row header, resource stability, and manual semantics. | Replace layout-specific assertions with user-visible contracts. |

No redirect is needed from `/releases/` because it remains valid and intentionally changes to the
directory. Home and any latest-version CTA must be changed to `/releases/1.0.1/` when the intent is to
open that release rather than browse the directory.

## 10. Acceptance criteria

### 10.1 Global header

- [ ] At 1440 px in both locales, brand, Home, Release log, XML namespace, language, theme, and GitHub
      are vertically centered within the same 64 px header row.
- [ ] There is no persistent second navigation row or border beneath a primary header row.
- [ ] At 960, 720, 390, 360, and 320 px, controls do not collide or overflow.
- [ ] Where inline navigation does not fit, the same-row Menu disclosure exposes all three destinations
      with JavaScript disabled and by keyboard.
- [ ] Detail/chapter routes correctly mark their parent destination current.

### 10.2 Release log and detail

- [ ] `/releases/` and `/en/releases/` list exactly `1.0.1`, `1.0.0`, `0.3.0`, and `0.2.0` newest first.
- [ ] Selecting each version opens its local localized detail route, not GitHub.
- [ ] All eight localized detail pages build and have self-canonical and reciprocal locale links.
- [ ] A detail page's version, date, protocol, commands, assets, checksums, structured metadata, and
      previous/next links all describe that same version.
- [ ] The 1.0.1 detail calls out the non-additive protocol 3.0 migration before downloads.
- [ ] Historical pages do not fabricate assets or capabilities absent from their release.
- [ ] Existing release JSON resources return unchanged content type and remain directly addressable.
- [ ] Release pages do not contain Home's build graph, journey, capabilities, metrics, or closing CTA.

### 10.3 DSL manual

- [ ] `/ns` presents the exact slashless namespace URI, smallest valid example, learning paths, and
      version/snapshot distinction.
- [ ] All seven chapter routes exist in both locales and preserve page identity when switching language.
- [ ] Desktop exposes chapter navigation and a readable article column; mobile exposes an operable
      chapter disclosure without horizontal overflow.
- [ ] The Directive reference contains all eleven built-ins exactly once with stable fragment IDs.
- [ ] Each directive provides syntax, context, attributes, effect, a valid example, and related links.
- [ ] Previous/next navigation follows the manifest order and every chapter links to the raw normative
      `/ns/dsl.md` resource.
- [ ] `/ns/dsl.md`, `/ns/0.3.0/dsl.md`, and `/ns/0.2.0/dsl.md` remain raw resources, not HTML manual
      redirects.
- [ ] The manual makes no semantic claim inconsistent with `docs/dsl.md`; normative rules retain their
      force in both locales.
- [ ] Manual pages do not reuse Home's wide hero, product journey bands, metrics, or marketing CTA.

### 10.4 Cross-cutting quality

- [ ] Every page is complete and navigable without JavaScript.
- [ ] No authored human-page link has an underline in any interaction state, while all ambiguous links
      retain a non-color cue and visible focus ring.
- [ ] Keyboard traversal reaches header, disclosures, chapter links, article links, and footer in a
      coherent order.
- [ ] Light, dark, and system themes remain readable with sufficient contrast.
- [ ] No document overflow occurs at 320 px; no sticky region obscures fragment headings.
- [ ] Static build, type checks, browser tests, link checks, and sitemap checks pass.
- [ ] Visual regression captures cover Home, release index, one current and one historical release
      detail, manual overview, reference chapter, both locales, desktop/mobile, and light/dark themes.

## 11. Implementation sequencing and ownership boundaries

The following slices minimize shared-file conflicts while keeping each commit coherent:

1. **Content schemas and route manifests** — complete release catalog, manual chapter manifest, locale
   mapping, and build-time validation. No visible redesign yet.
2. **One-row global header** — shared shell, responsive disclosure, accessibility, and header tests.
   Validate Home visual preservation before proceeding.
3. **Release directory and detail routes** — index, version route generation, localized content,
   metadata, and release browser tests.
4. **Manual shell and chapters** — manual navigation, overview, chapter rendering, directive reference,
   and semantic drift checks.
5. **Cross-surface validation and visual QA** — no-JS, mobile, keyboard, dark/light, localized links,
   raw resources, and live deployment checks.

Only one task should own the shared route types and global i18n UI labels at a time. Release content and
manual content can proceed independently once their manifests are fixed. Avoid having multiple tasks
edit the same monolithic message file; the redesign is an opportunity to separate short interface
strings from long-form content.

## 12. Decisions, trade-offs, and deferred scope

### Decisions

- A single row plus a responsive disclosure is preferred over squeezing labels or restoring a second
  navigation row.
- Releases use local first-party detail pages; GitHub remains the immutable asset/provenance host.
- The manual is multi-page because the current specification is too large for a usable single-page
  layout and users need both learning and lookup paths.
- Visual coherence comes from shared primitives and tokens, not copying Home's composition.
- Stable external resources outweigh freedom to discard old internal implementation details.

### Trade-offs

- Multi-page manuals require localized content maintenance, but give stable deep links, smaller reading
  units, clearer progress, and better lookup behavior.
- Native disclosure menus are less visually programmable than custom dialogs, but preserve no-JS and
  keyboard access with substantially less failure surface.
- Typed release data duplicates some prose from Markdown release notes unless an authoring pipeline is
  introduced; explicit validation is required to prevent drift.

### Deferred, not part of this iteration

- site-wide search or third-party search service;
- release subscriptions, RSS, reactions, or analytics dashboards;
- pagination/filtering for four releases;
- one page per directive;
- interactive code playground or browser compiler;
- automatic translation;
- version selector that pretends old snapshots share current manual semantics;
- changing the namespace URI or retrofitting missing historical release artifacts.

## 13. Evidence basis and falsification checks

This plan is grounded in the inspected repository state on 2026-09-17:

- `site/src/layouts/ProductShell.astro` currently creates a 4.15 rem primary row plus a 2.7 rem
  navigation row, directly explaining the user's first complaint.
- `site/src/components/ReleasePage.astro` currently combines latest release detail and history on one
  route, so it cannot satisfy the requested directory-to-detail flow.
- `site/src/data/releases.ts` currently models only one `latestRelease`, making historical local detail
  pages unsafe without a catalog.
- `site/src/components/NamespacePage.astro` is a compact vocabulary explorer, while `docs/dsl.md`
  contains sixteen substantial normative sections; a user manual needs a different reading model.
- Existing raw JSON and versioned DSL files prove that machine/resource routes are already public and
  must not be accidentally absorbed into the new human routing.

The strongest competing design is a single long release page and a single long manual page. It has
lower implementation cost, but fails the explicit “directory then click a version” behavior and makes
an 850-line bilingual semantic specification hard to learn, scan, and deep-link. Revisit the multi-page
manual only if observed user research shows that cross-chapter navigation is more costly than long-page
search, or if the specification becomes small enough that distinct learning and lookup paths no longer
matter. Revisit the header breakpoint only when browser measurements in both locales demonstrate a
different fit threshold; never preserve the numeric breakpoint at the cost of collision or a second row.
