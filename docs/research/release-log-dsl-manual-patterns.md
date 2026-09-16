# Release-log and DSL-manual patterns for xmlsquish

- **Research date:** 2026-09-17
- **Decision target:** a one-row global product header, a versioned release-log index/detail
  system, and a user-oriented DSL manual
- **Evidence basis:** first-party production sites and documentation, official Astro guidance,
  and W3C/MDN accessibility guidance
- **Status:** implementation guidance, not a visual specification

## Executive decision

Use **one brand, three page templates**:

```text
Shared one-row header: [xmlsquish]  [Home] [Release log] [DSL manual]  [Language] [Theme] [GitHub]

Home                 marketing/product-story template
Release log index    chronological archive template
Release detail       editorial change-note template
DSL manual           documentation/manual template
```

“Distinct” should mean a different information hierarchy, density, and reading layout—not three
unrelated visual brands. Keep the logo, color tokens, theme behavior, typography family, focus
language, and footer relationship. Do not reuse the home page's oversized hero, feature-card rhythm,
or alternating promotional bands on Release log and DSL manual pages.

This decision **supersedes** the earlier recommendation in
`docs/research/product-site-home-release-reference-patterns.md` that all three destinations should
use the home page's wide editorial composition. The current user direction is clearer: shared brand
chrome, but purpose-built content shells.

## What the strongest examples establish

| First-party example | Direct observation | Transferable pattern | Limit |
| --- | --- | --- | --- |
| [Rust release archive](https://blog.rust-lang.org/releases/) | A compact year-grouped archive links every dated version to an independent announcement; it also publishes stable `/releases/latest` and `/releases/<version>` shortcuts. | Treat the archive and a release article as different page types; make every version addressable. | Rust's detail canonical URLs are date-based blog URLs behind release shortcuts. xmlsquish should use the simpler version as its canonical path. |
| [Rust 1.98.1 announcement](https://blog.rust-lang.org/2026/09/03/Rust-1.98.1/) | The detail begins with version, date, author, upgrade action, and the exact point-release effect. | Put the user's upgrade decision before project narrative. Patch notes can be short without looking incomplete. | xmlsquish also distributes binaries, so it needs an artifact/checksum section Rust does not show here. |
| [Vite blog archive](https://vite.dev/blog) and [Vite 8 detail](https://vite.dev/blog/announcing-vite8) | The archive is a date/title list; the detail provides a narrative, migration guide, and exhaustive GitHub changelog exit. | Keep the index scannable and let the detail carry context, migration, and technical depth. | Do not mix unrelated news into xmlsquish's release-only archive. |
| [Vite release policy](https://vite.dev/releases) | Cadence, supported versions, pre-release meaning, deprecation, and migration policy live separately from announcements. | Put a small compatibility/support policy on the archive, not repeated in every release story. | xmlsquish does not yet need Vite's multi-branch support matrix. |
| [Bun documentation home](https://bun.com/docs) and [runtime manual](https://bun.com/docs/runtime) | The manual home starts with task/capability entry points; a content page adds a concise purpose statement, runnable examples, hierarchical navigation, and an on-page outline. | A manual should support both learning and lookup: orientation → example → details → reference. | Bun's taxonomy is much larger; xmlsquish should not create empty sections to imitate it. |
| [Rust Reference introduction](https://doc.rust-lang.org/reference/) | It states scope and non-goals, supports table-of-contents browsing and search, treats chapters as independently readable, and publishes versioned historical copies. | State whether a page teaches or specifies; support both browse and search; preserve historical language text. | The formal Rust Reference assumes prior knowledge. The requested xmlsquish surface is a user manual and must begin more gently. |
| [Astro Starlight layout anatomy](https://starlight.astro.build/reference/overrides/) | Its top header contains site identity, search, social, theme, and language controls; a global sidebar supplies manual navigation; a page sidebar supplies the current page outline; both adapt on narrow screens. | This is a production-tested manual anatomy worth borrowing without necessarily adding Starlight. | Installing a full docs framework for a 16-section manual would introduce a second design system and unnecessary machinery. |

These examples are observations, not causal proof that a particular visual treatment is optimal. The
convergence is nevertheless strong: global identity belongs in the top bar; archives optimize
scanning; release details optimize decision-making; manuals optimize orientation and retrieval.

## 1. One-row global navigation

### Recommended desktop composition

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ ✦ xmlsquish   Home   Release log   DSL manual        中文/EN   ◐   GitHub   │
└──────────────────────────────────────────────────────────────────────────────┘
```

Implementation implications for the current `ProductShell.astro`:

1. Move `product-nav` into `header-primary`, between `.product-brand` and `.header-actions`.
2. Remove the second nav strip and its top border. Use one header height (approximately the existing
   home header's 5rem), one bottom border, and one shared vertical center line.
3. Give navigation links, language, theme, and GitHub controls the same minimum control box height.
   Icons may be visually smaller, but their interactive boxes should align.
4. Keep the three destination labels stable: **Home / Release log / DSL manual**. “XML namespace” is
   the language identity inside the manual, not the clearest navigation job label.
5. Mark exactly one destination with `aria-current="page"`. Release details remain current under
   “Release log”; all `/ns/**` manual routes remain current under “DSL manual.” MDN explains that
   [`aria-current`](https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Reference/Attributes/aria-current)
   conveys a visually indicated current item to assistive technology.
6. Keep native `<nav>`, `<ul>`, and `<a>` semantics; do not implement an application `menubar`.
   W3C's [menu-structure guidance](https://www.w3.org/WAI/tutorials/menus/structure/) recommends
   semantic lists and consistent item order, wording, and destinations across responsive variants.

### Responsive behavior

- At widths where the complete row fits, never wrap the three destinations onto a second header row.
- When it does not fit, switch once to a compact disclosure menu. Do not successively hide individual
  destinations or rely on unlabeled icons.
- Keep brand, menu trigger, language, and theme in the visible mobile bar. Put the same three links and
  GitHub destination in the disclosure, in desktop order.
- A tall sticky header can obstruct anchor targets and keyboard focus. W3C notes that persistent
  author-positioned content can violate
  [Focus Not Obscured](https://www.w3.org/WAI/WCAG22/Understanding/focus-not-obscured-minimum).
  Set manual headings' `scroll-margin-top` to the actual header height and test 400% zoom/reflow.

## 2. Release log: directory first, version detail second

### Route contract

```text
/releases/                 localized Chinese archive
/releases/1.0.1/           localized Chinese release detail
/releases/1.0.0/
/releases/0.3.0/
/releases/0.2.0/

/en/releases/              localized English archive
/en/releases/1.0.1/        localized English release detail
...

/releases/latest/          optional redirect to the current locale's latest detail
/releases/<version>.json   retained machine-readable release fact surface
```

Use the version route as the canonical detail URL. It is shorter and more durable than a date/title
slug, and it matches how users identify releases. Every archive item must be an ordinary link, so it
works without JavaScript and appears in the sitemap.

### Archive template

The archive is a log, not a second home page:

```text
Release log                                      [support / version policy]

2026
  16 Sep    v1.0.1  Typed artifact catalog and protocol 3.0       →
  15 Sep    v1.0.0  Project-manager release                        →

2025
  ...
```

Recommended anatomy:

- a compact `h1`, one-sentence scope, latest/stable badge, and optional RSS only when it can be
  generated from the same records;
- a reverse-chronological semantic list, grouped by year only after enough years exist;
- each row shows date, version, short user outcome, stability/compatibility flag, and one clear link;
- latest may receive a quiet accent marker, not a marketing hero;
- a short policy block below the list explains support, protocol changes, signing/checksum boundary,
  and where exhaustive GitHub assets live.

Avoid platform-download cards, feature grids, and the full migration story on the index. They make
historical scanning slower and repeat content that belongs to the detail page.

### Version-detail template

```text
Release log / v1.0.1

v1.0.1                                         16 September 2026
Typed publication boundaries become a product contract
[Stable] [Protocol 3.0] [Migration required]

Summary
Upgrade impact
What changed
  Build and runtime
  Artifacts and inspection
  Reliability
Install and verify
Known limitations
Previous release ←                              → Next release
```

The detail should be a restrained reading layout: roughly 45–55rem for prose, with an optional narrow
facts rail on wide screens. Use ordinary headings, lists, tables, code blocks, and callouts. It can
share coral accents and code colors with Home, but not Home's enormous serif display hierarchy,
alternating promotional bands, or card-everything composition.

The detail's order should answer:

1. **What shipped?** Version, date, outcome summary.
2. **Does it affect me?** Protocol, compatibility, migration, limitations.
3. **What exactly changed?** User-visible changes grouped by task.
4. **How do I get and verify it?** GitHub Release, six native assets, source install, SHA-256.
5. **Where next?** Previous/next release and relevant manual page.

GitHub remains the binary authority. GitHub defines releases as tagged deployable iterations that can
carry notes and binary assets in its official
[release documentation](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases).
The product site should not proxy binaries or claim stronger signing provenance than exists.

### Release content model

The current site has typed facts only for `latestRelease` in `site/src/data/releases.ts`; older
releases are split between translated strings, ad-hoc URLs, `docs/releases/*.md`, and JSON files.
Index/detail routing will amplify that drift unless each version becomes one validated entry.

Use an Astro build-time content collection (or an equivalently strict local data module) with one
entry per version and schema fields such as:

```text
version, date, status, protocol, headline, summary, upgradeImpact,
changeGroups[], platforms[], githubRelease, checksums, sourceInstall,
limitations[], previousVersion, nextVersion
```

Astro recommends content collections for structurally similar groups and provides schema validation,
type safety, and editor IntelliSense
([Astro content collections](https://docs.astro.build/en/guides/content-collections/)). Generate the
static detail routes from those entries rather than hand-creating one component per version. Keep
`docs/releases/<version>.md` as the GitHub workflow's publication source unless the release pipeline is
deliberately changed; do not silently make the website's paraphrase the GitHub release authority.

## 3. XML namespace as a DSL user manual

### Product boundary

The manual answers **how to write and reason about xmlsquish programs**. The raw `docs/dsl.md` remains
the normative language specification. The manual may teach with friendlier sequencing and examples,
but must link normative claims back to stable specification sections. This separation mirrors the
Rust project's explicit distinction between an introductory book and its formal reference
([Rust Reference: “What The Reference is not”](https://doc.rust-lang.org/reference/#what-the-reference-is-not)).

Keep the namespace identity exactly:

```text
https://xmlsquish.moesegfault.dev/ns
```

The `/ns/` dereferenceable page becomes the manual home. English `/en/ns/` describes the same
namespace; it must not imply a second URI.

### Recommended manual information architecture

The existing `docs/dsl.md` is about 603 lines with 16 top-level sections. That is already large enough
for a real manual shell but still small enough to avoid a documentation framework migration.

```text
DSL manual /ns/
├── Start
│   ├── Overview and first build
│   ├── Namespace and QName
│   └── Entry vs module
├── Compose
│   ├── Import and expand
│   ├── Macros, params, and args
│   ├── Slots and fills
│   └── Insert and conditional matching
├── Understand execution
│   ├── Source identity and loading
│   ├── Frames, environments, and scope
│   ├── Recursion and resource limits
│   └── Determinism and errors
├── Build
│   ├── Project build and link
│   └── .prompt, .xsir, and .psdbg artifacts
└── Reference
    ├── Directive index (all 11 xs:* elements)
    ├── Language invariants and exclusions
    ├── Full normative specification
    └── Historical snapshots
```

Do not organize the user manual as 11 equal “directive cards.” Users arrive with tasks—compose a
document, define a macro, pass values, fill structure, diagnose a build—not with prior knowledge of
which element solves the task. Retain a directive index for lookup, but teach in execution order.

### Manual page layout

```text
┌───────────────┬──────────────────────────────────────┬─────────────────┐
│ Manual tree   │ Breadcrumb                           │ On this page    │
│               │ h1 + one-sentence contract           │                 │
│ Start         │                                      │ headings        │
│ Compose       │ explanation                          │                 │
│ Execution     │ example / output / warning           │                 │
│ Build         │                                      │                 │
│ Reference     │ previous page  ·  next page          │                 │
└───────────────┴──────────────────────────────────────┴─────────────────┘
```

- **Left rail:** persistent chapter navigation on wide screens; a labeled “Manual” disclosure on
  narrow screens. Group by learning task. Show the current page in text and with `aria-current`.
- **Article:** 42–48rem readable line length; one `h1`; a short purpose/contract; runnable XML example
  early; then explanation, observable output, errors, and related concepts.
- **Right rail:** “On this page” links generated only from `h2`/`h3`; omit it for short pages. W3C
  describes a table of contents in a navigation landmark as a useful skimming mechanism
  ([in-page navigation](https://www.w3.org/WAI/tutorials/page-structure/in-page-navigation/)).
- **Footer navigation:** explicit previous/next pages supports sequential learning while sidebar and
  search support lookup. W3C's
  [Multiple Ways guidance](https://www.w3.org/WAI/WCAG22/Understanding/multiple-ways) explains why
  users benefit from search, tables of contents, and sequential paths.
- **Examples:** copyable XML with an adjacent expected result or diagnostic. Copy is enhancement; raw
  text remains selectable with JavaScript disabled.
- **Callouts:** use a small closed set—`Note`, `Contract`, `Error`, `Version`—rather than decorative
  marketing cards.
- **Search:** index titles, headings, aliases, directive names, and prose. Eleven directive names do
  not justify search alone; 16 conceptual chapters do. If full-text search cannot ship reliably in
  this iteration, provide a complete manual index first rather than a fake title-only input.

W3C notes that headings communicate content organization and enable in-page navigation
([Headings tutorial](https://www.w3.org/WAI/tutorials/page-structure/headings/)). Keep heading ranks
logical, give every page a unique title, and make chapter links deep-linkable.

### Visual direction

Use a documentation voice rather than the home page's marketing voice:

| Surface | Home | Release log | DSL manual |
| --- | --- | --- | --- |
| Primary rhythm | Large narrative sections | Compact chronological rows / article | Persistent navigation + readable chapters |
| Heading scale | Display / promotional | Editorial | Documentation |
| Accent use | Broad highlights and diagrams | Sparse status/date accents | Current-page, code, callouts |
| Content width | Wide, often two-column | Narrow article / medium archive | 3-column shell with narrow article |
| Cards | Feature storytelling | Rare; not every release row | Only examples/callouts, not every concept |

Shared tokens preserve recognition. Template-specific spacing, grids, and heading scales prevent the
Release log and manual from looking like copied home sections.

## 4. Accessibility and interaction acceptance criteria

1. **Landmarks:** one banner/header, one main, a labeled global navigation, labeled manual and
   on-page navigations, and a footer. W3C recommends semantic regions and distinct labels when a page
   has multiple navigation landmarks
   ([Page Regions](https://www.w3.org/WAI/tutorials/page-structure/regions/)).
2. **Keyboard:** every header, release, sidebar, TOC, previous/next, copy, and search control is
   reachable in visual order. No focus trap in the mobile menu.
3. **Focus:** retain the existing strong outline; do not let sticky header/rails cover focused or
   targeted content. Test anchor navigation as well as tabbing.
4. **No underline requirement:** an inline link still needs a persistent non-color cue or at least
   3:1 contrast with surrounding text plus an additional hover/focus cue, per
   [W3C Technique G183](https://www.w3.org/WAI/WCAG22/Techniques/general/G183). Use weight, a subtle
   marker background, an external-arrow glyph, or a bounded resource treatment—never color alone.
5. **Reflow:** at 320 CSS px / 400% zoom, collapse sidebars into disclosures, keep code horizontally
   scrollable inside its own region, and avoid two-dimensional page scrolling. W3C's
   [Reflow guidance](https://www.w3.org/WAI/WCAG22/Understanding/reflow.html) specifically warns that
   sticky content can consume or obscure narrow viewports.
6. **No-JavaScript baseline:** archive/detail navigation, full manual content, sidebar links, code,
   and namespace URI all work. Search, clipboard, and mobile disclosure may enhance that baseline.
7. **Localization:** language switching preserves the exact surface and version/manual chapter.
   Missing translations must be a build error or an explicit fallback with correct `lang`, never a
   silent route back to Home.

## 5. Adopt / avoid matrix

| Adopt now | Avoid now |
| --- | --- |
| One desktop header row with brand, three destinations, and equal-height controls | The current two-level header with product destinations on a second strip |
| One shared brand system with three purpose-built templates | Reusing Home's hero, section bands, and card language everywhere |
| `/releases/` archive plus canonical `/releases/<version>/` details | A single giant latest-release page with old versions as external links |
| Reverse-chronological date/version rows | Accordions as the only access to release content |
| Typed release entries feeding index, details, metadata, and navigation | Latest facts in TypeScript while history remains positional translated copy |
| Task-led DSL chapters plus a directive index | A flat catalog of directives presented as the whole manual |
| Manual sidebar, optional on-page TOC, search/index, previous/next | A marketing landing page disguised as documentation |
| Normative specification and immutable snapshots as explicit exits | Rewriting the normative specification into friendly prose without traceability |
| Progressive enhancement and semantic links | Client-only content, fake search, or click-only cards |

## 6. Concrete implications for this repository

Current evidence:

- `site/src/layouts/ProductShell.astro` explicitly renders `header-primary` and `product-nav` as two
  stacked rows. The requested alignment is a structural move, not a CSS nudge.
- `site/src/components/ReleasePage.astro` combines latest release, downloads, changes, migration, and
  history in one marketing-composition page. Split it into an archive component and a detail layout.
- Only `/releases.astro` and `/en/releases.astro` exist; add locale-preserving dynamic version routes.
- `site/src/data/releases.ts` models only one `ProductRelease`. History must become first-class typed
  data before generating detail routes.
- `site/src/components/NamespacePage.astro` is a single visual explorer. Replace it with a manual
  landing/content shell; reuse the exact URI, copy behavior, and complete directive facts inside the
  manual rather than preserving the current home-like composition.
- `docs/dsl.md` already supplies a coherent 16-section normative source. Use its conceptual structure
  to design the manual, but do not render all 603 lines as one undifferentiated page.
- Keep `/ns/dsl.md` and immutable `/ns/<version>/dsl.md` snapshots for machines and formal review.

Suggested minimum route set for the first implementation:

```text
/releases/                       archive
/releases/[version]/             release detail
/en/releases/                    archive
/en/releases/[version]/          release detail

/ns/                             manual overview / first build
/ns/language/                    namespace, QName, entry/module
/ns/composition/                 imports, macros, args, slots/fills
/ns/execution/                   identity, frames, scope, recursion
/ns/build/                       linking and artifacts
/ns/reference/                   directive and invariant index

/en/ns/...                       exact English counterparts
```

If implementation time forces one compromise, ship release detail routes fully and initially keep
the DSL manual as one structured page with the same sidebar/TOC anatomy. Do **not** compromise by
restyling the current marketing sections and calling them a log or manual; the information
architecture is the requested change.

## Evidence strength and remaining uncertainty

- **Strong:** archive/detail separation, task-led manual navigation, semantic current-state and
  landmark requirements, and Astro's support for typed static content are directly evidenced by
  first-party implementations and standards.
- **Moderate:** exact three-column proportions, breakpoint, type scale, and the choice between one
  manual page and several chapters are design judgments. Validate them with screenshots at 1440px,
  768px, 390px, 320px/400% zoom, and keyboard-only navigation.
- **Open question for later evidence, not a blocker:** full-text search is justified when the manual
  is split into chapters, but the best engine depends on built output size and Chinese tokenization.
  Measure Pagefind or an equivalent against the real bilingual corpus before adding a dependency.
