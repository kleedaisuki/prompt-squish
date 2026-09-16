# Site v2 Refactor Map

- **Inspected revision:** `84a00f1` (`test(site): enforce product experience contracts`)
- **Inspection date:** 2026-09-17
- **Scope:** exact local changes needed for a one-row product header, a release-log index with
  per-version pages, and an XML namespace route redesigned as a DSL user manual
- **Constraint:** preserve the public machine/resource contracts while allowing internal Astro
  components, layouts, data models, and tests to change without compatibility shims

## 1. Requested outcome translated into route structure

The requested change is structural rather than a request for three copies of the home page.
The smallest coherent information architecture is:

```text
shared one-row product header and shared legal footer
├── / and /en/                         existing product home composition (unchanged)
├── /releases/ and /en/releases/       release-log index
│   └── /{version}/                    version detail, statically generated
└── /ns and /en/ns/                    DSL user manual
    └── /ns/dsl.md                     raw, machine-friendly design specification (preserved)
```

“Release log” and “DSL manual” should remain visually coherent with the brand tokens, theme
control, and global chrome, but must not reuse the home page's long hero/journey/card composition.
This calls for **one shared header**, then separate release-index/detail and manual content layouts.
It does not require a second color system or a second implementation of language/theme controls.

## 2. Current implementation and the exact mismatch

### 2.1 Header

`site/src/layouts/ProductShell.astro` currently has two stacked header rows:

```text
.header-primary: brand ───────────────── language / theme / GitHub
.product-nav:               Home / Release log / XML namespace
```

The second row is not accidental page markup: it is defined entirely by `ProductShell`. All six
current human routes use this shell, either through their page entry point (Home) or inside the
page component (Release and Namespace). The minimal fix is to put `.product-nav` between the brand
and `.header-actions` inside a single `.header-row` and remove the separate top border/min-height.

Important responsive constraint: three localized navigation labels, the brand, a language action,
the three-button theme group, and GitHub cannot all fit at 320 CSS px with their current desktop
padding. A correct one-row implementation therefore needs an explicit narrow-width policy rather
than accidental wrapping. The least machinery is:

1. keep one semantic row and one `nav` at every width;
2. hide the brand tagline first, then the `xmlsquish` wordmark at the narrowest width while keeping
   the logo and its accessible name;
3. reduce pill padding and gaps but keep each interactive target at least 24 by 24 CSS px;
4. allow the middle navigation region to scroll horizontally if its localized labels still do not
   fit, while keeping brand and actions fixed; do not hide destinations;
5. retain the current `aria-current="page"`, persistent non-color active cue, and 3 px focus ring.

The theme control is a 3-option React island (`ThemeControl.tsx`) and already has compact icon-only
visuals plus screen-reader labels. It does not need redesign or state duplication.

### 2.2 Releases

`site/src/components/ReleasePage.astro` currently combines three jobs in one component:

- a latest-release landing page for `latestRelease` only;
- installation/download, migration, and change details for 1.0.1;
- an embedded historical list whose old versions link only to GitHub (plus 1.0.0 JSON).

Both `/releases/` and `/en/releases/` instantiate this same latest-release component. There is no
version route or route parameter. `site/src/data/releases.ts` models only the current release and
derives archive URLs from module-level `version` and `downloadRoot` constants. Historical summaries
live separately in localized `reference.ts`, so they are not routable records.

The minimal clean split is:

| New responsibility | Recommended module | Notes |
| --- | --- | --- |
| ordered release identity/catalog | `site/src/data/releases.ts` | One typed record per 1.0.1, 1.0.0, 0.3.0, 0.2.0; `latestRelease = releases[0]` |
| localized release content | `site/src/i18n/releases.ts` | Keyed by version, never positional; title, summary, sections, migration/limitations |
| log index renderer | new `ReleaseIndexPage.astro` | Reverse chronology; every card links to a local version route |
| version renderer | repurpose `ReleasePage.astro` or new `ReleaseDetailPage.astro` | Receives `locale` and a release record; no embedded history section |
| localized index routes | existing `pages/releases.astro`, `pages/en/releases.astro` | Render index only |
| localized detail routes | new `pages/releases/[version].astro`, `pages/en/releases/[version].astro` | `getStaticPaths()` over the catalog |

Astro route generation should be deterministic and reject unknown versions at build time:

```ts
export function getStaticPaths() {
  return releases.map((release) => ({
    params: { version: release.version },
    props: { release },
  }));
}
```

The localized route files can share a small helper returning the same version records, but keeping
two thin page entry points is simpler than inventing locale parameters or runtime routing for a
four-version static site. Generated human URLs should be:

```text
/releases/1.0.1/       /en/releases/1.0.1/
/releases/1.0.0/       /en/releases/1.0.0/
/releases/0.3.0/       /en/releases/0.3.0/
/releases/0.2.0/       /en/releases/0.2.0/
```

The trailing-slash directories coexist safely with `/releases/1.0.1.json` and
`/releases/1.0.0.json` because the static files have distinct names.

Do not blindly generalize the current binary-download function to every release. Repository release
notes establish different acquisition contracts: 1.0.1, 1.0.0, and 0.3.0 have six native archives,
whereas 0.2.0 documents tagged source installation rather than that native matrix. Express this as
a discriminated acquisition type (`native` versus `source`) so impossible download cards are not
manufactured for old versions.

### 2.3 Namespace / DSL manual

`NamespacePage.astro` currently presents a large product hero, identity card, broad section bands,
directive cards, a pipeline visualization, and promotional resource cards. It has good progressive
enhancement for URI copy and directive search, but its composition deliberately mirrors the home
page. The user now wants a manual.

The manual can preserve the useful behavior while changing the reading model:

```text
compact manual masthead: “xmlsquish DSL” + stable namespace URI/copy
┌──────────────────────┬──────────────────────────────────────────────┐
│ sticky section TOC   │ user task chapters                           │
│ Getting started      │ 1. Quick start and exact namespace binding  │
│ Project model        │ 2. entry vs module, source/import identity   │
│ Composition          │ 3. macro/param/expand and evaluation order   │
│ Values and content   │ 4. arg/fill/slot/insert                      │
│ Control              │ 5. ifr, recursion, limits                    │
│ Build and artifacts  │ 6. project build, XSIR, .prompt/.psdbg       │
│ Reference            │ 7. directive reference and version policy    │
└──────────────────────┴──────────────────────────────────────────────┘
```

This is a new manual composition, not a re-skinned Home hero. A sticky table of contents can collapse
to a `details`/jump control at narrow widths. The existing 11 directives, stable fragment IDs,
filter/search enhancement, copy fallback, raw specification resource, historical snapshots, and
pipeline facts remain useful within the manual.

`docs/dsl.md` is **not** directly suitable as the human manual source: it is a 603-line Chinese
design specification with formal semantics, a two-counter-machine argument, internal boundaries,
and no complete English counterpart. Rendering it wholesale would violate the bilingual route
contract and turn the manual into an implementation design document. Keep it byte-for-byte exposed
through `/ns/dsl.md`; curate localized, user-oriented chapters in a keyed module such as
`site/src/i18n/manual.ts`. Reuse technical facts from the specification, but do not parse headings
or split bilingual content at build time.

The current parallel arrays are already partly improved (group names plus keyed element copy), but
the group records still live in the component. Move directive identity/group/order into a locale-
neutral DSL data module and keep only human prose in translations. That makes the 11-element
invariant inspectable without coupling it to presentation.

## 3. Layout boundary recommendation

Keep `ProductShell.astro` responsible only for:

- `BaseLayout` metadata handoff;
- the one-row global header and common footer;
- active primary destination, locale switch, theme, and GitHub;
- global underline-free/focus contracts.

It should not impose a home, release, or manual content grid. Page-specific structure should be:

| Surface | Structural owner | Visual grammar |
| --- | --- | --- |
| Home | existing `LandingPage.astro` | current editorial hero, demonstrations, wide bands, closing CTA |
| Release index | `ReleaseIndexPage.astro` | compact log heading, reverse-chronological list/cards, dates/status/version |
| Release detail | `ReleaseDetailPage.astro` | article/change-log reading width, metadata rail, downloads only when applicable |
| DSL manual | `DslManualPage.astro` (or repurposed `NamespacePage`) | documentation TOC + reading pane + reference tables/code |

`ReferenceLayout.astro` is currently dead code: no page imports it after the product-shell redesign.
Delete it once searches and the build confirm no consumers; do not revive its generic global
`.reference-content` descendants, which would create style coupling across both new surfaces.

Brand coherence can still come from the existing `--moe-*`/`--xs-*` tokens, typefaces, link state,
and header. “Do not use the homepage layout” does not require a forced new theme, duplicated chrome,
or bypassing the current dark/light preference.

## 4. Routing, canonical, language, and metadata coupling

The current route abstraction cannot correctly represent version details:

- `Page` is only `"home" | "namespace" | "releases"`;
- `ProductShell` computes the language link as `paths[other][page]`;
- `BaseLayout` computes canonical and all `hreflang` links from the same index path;
- `BaseLayout` emits the latest release JSON alternate for every page whose `page` is `releases`.

If a detail page simply passes `page="releases"`, it would canonicalize
`/releases/1.0.0/` as `/releases/`, switch language to `/en/releases/`, and advertise 1.0.1 JSON.
That is a silent metadata bug.

The minimal robust API is to separate **active navigation section** from **concrete localized
location**. For example:

```ts
type LocalizedLocation = {
  canonicalPath: string;
  alternates: Record<Locale, string>;
};

// ProductShell props
page: Page;                 // active nav section
location?: LocalizedLocation; // defaults to paths[*][page] for the six existing routes
releaseMetadataPath?: string; // detail-specific; absent when no JSON exists
```

`ProductShell` uses `location.alternates[other]` for the language link. `BaseLayout` uses the same
object for canonical and hreflang. The release index may advertise the latest JSON as discovery if
desired; a version detail must advertise only its own JSON, and 0.3.0/0.2.0 should omit the alternate
unless corresponding immutable JSON artifacts are actually created. Keep `page="releases"` for all
release details so primary navigation remains correctly active.

Structured metadata also needs separation:

- Home keeps its one `SoftwareApplication` entity sourced from `latestRelease`.
- Release index should use a collection/list entity or no JSON-LD rather than pretending the index
  is one version.
- Each detail owns at most one version-correct release entity. Do not let `BaseLayout` duplicate it.
- The release social card may remain shared, but its alt text must use the detail version rather than
  unconditionally `latestRelease.version`.

## 5. Current coupling by file

| File | Current coupling | Required v2 change |
| --- | --- | --- |
| `layouts/ProductShell.astro` | two-row header; language route derived only from page kind; shared product content classes | one-row header; accept concrete localized location; keep only chrome/global interaction styles |
| `layouts/BaseLayout.astro` | canonical/hreflang and release JSON fixed to index/latest; social alt fixed to latest | accept concrete location and optional release discovery/social version data |
| `components/LandingPage.astro` | consumes release index path and latest version only | preserve section order/layout; no release/manual composition copied here |
| `components/ReleasePage.astro` | latest detail + acquisition + history in one surface | make version detail only, driven by record/locale |
| `components/NamespacePage.astro` | product hero/explorer/bands/resources | replace composition with task-oriented manual shell; retain proven copy/search fallbacks |
| `data/releases.ts` | latest-only identity; module-global URL derivation | ordered typed catalog; functions derive URLs from the passed record |
| `i18n/reference.ts` | nav + namespace + latest detail + historical summaries in one large object | split release index/detail and manual translations into keyed modules; retain nav labels centrally |
| `i18n/routes.ts` | only three fixed route kinds | preserve primary paths; add pure helpers for localized version detail URLs rather than enumerating versions in `Page` |
| `pages/releases*.astro` | latest detail entry points | release index entry points |
| new `[version].astro` pages | absent | static localized detail generation |
| `LandingPage.test.mjs` | one six-route matrix; latest details assumed at index; history only anchors | extend route model for 8 detail routes; separate index/detail/manual behavior contracts |
| `public/sitemap.xml` | manually lists only six human routes | add eight localized version-detail URLs and reciprocal alternates |
| `public/llms.txt` | latest human page points at index | distinguish release index from latest detail URL |

`messages.ts` merges `referenceMessages` into the home message object. It need not become the home
page's authority for manual/release prose. Separate exports (`navMessages`, `releaseMessages`,
`manualMessages`) reduce the high-conflict `reference.ts` file and make page ownership explicit.

## 6. Tests that must change versus tests that must remain

The current single browser file has good behavioral evidence but is coupled to the six-route shape.
Refactor tests around route categories:

### Preserve unchanged guarantees

- Home section order and BuildExplorer behavior.
- One `h1`, skip link, locale correctness, reciprocal hreflang, active primary destination.
- language switch preserves the exact destination (including release version).
- all visible links remain underline-free at rest, hover, and focus, with the 3 px focus outline.
- no document overflow at 320, 390, 768, and 1440 px in light and dark themes.
- blocked-storage theme behavior and reduced motion.
- `/ns/dsl.md` equals `docs/dsl.md`; immutable 0.3.0 and 0.2.0 snapshot digests stay exact.
- `/releases/1.0.1.json` and `/releases/1.0.0.json` remain correct and reachable.

### Replace layout-specific assertions

- Replace “all six routes” with base-route, release-index, release-detail, and manual matrices.
- Replace the current `/releases/` assertion for downloads/migration with the equivalent
  `/releases/1.0.1/` and English detail assertions.
- Replace `#v1-0-0` embedded-history assertions with index card href assertions for every version.
- Assert each index card opens a 200 response whose `h1`, canonical, locale alternate, date, and
  version agree with the record.
- Assert old releases do not accidentally use latest metadata/downloads; specifically, 0.2.0 must
  not render the six-archive matrix.
- Replace namespace home-like selectors (`.namespace-hero`, `.pipeline-band`) with manual landmarks:
  TOC, chapter anchors, exact namespace URI, all 11 directives, resource links, and code examples.

### Keep progressive-enhancement tests

- no JavaScript: manual content and all directive contracts remain readable; copy/search controls
  remain absent rather than dead;
- clipboard denial selects the namespace URI for manual copy;
- search/filter, if retained in the reference chapter, returns `xs:ifr` for `ifr` and Escape resets;
- fragment navigation accounts for the now shorter one-row sticky header.

The header contract should explicitly compare the vertical centers (or bounding-box row overlap) of
brand, `.product-nav`, `.language-link`, and `.theme-control` at desktop width. Merely asserting that
all elements are visible would allow the two-row regression to return.

## 7. Public contracts and preservation constraints

These are established external surfaces and must remain:

| Contract | Preservation rule |
| --- | --- |
| `/`, `/en/` | same home content order and behavior |
| `/releases/`, `/en/releases/` | remain valid human routes; semantics change to log index |
| `/ns`, `/en/ns/` | remain valid localized descriptions of the same namespace identity |
| namespace identity | exactly `https://xmlsquish.moesegfault.dev/ns`, no suffix or trailing slash |
| `/ns/dsl.md` | byte-for-byte current `docs/dsl.md` response |
| `/ns/0.3.0/dsl.md`, `/ns/0.2.0/dsl.md` | immutable existing snapshot bytes/digests |
| `/releases/1.0.1.json`, `/releases/1.0.0.json` | immutable machine endpoints; do not replace them with HTML or redirects |
| release asset URLs | existing GitHub release/tag/download URLs and archive names |
| canonical/hreflang | self-canonical details and reciprocal same-version locale links |
| home/release JSON-LD | one truthful entity, correct version/date/assets, no duplicate microdata |
| `sitemap.xml` / `llms.txt` | include new human detail routes without deleting raw machine/resource routes |

Adding `/releases/{version}/` is additive and does not conflict with the JSON files. Historical
release Markdown under `docs/releases/*.md` remains release-source evidence and should not be
rewritten merely to serve the site.

## 8. Recommended file ownership boundaries

To avoid concurrent edits to the same hotspots:

1. **Chrome/metadata owner**
   - `layouts/ProductShell.astro`, `layouts/BaseLayout.astro`, optional deletion of
     `layouts/ReferenceLayout.astro`, and header-focused tests.
   - Provides the new location props before page owners integrate.
2. **Release owner**
   - `data/releases.ts`, new release i18n module, release index/detail components, both fixed index
     pages, and both dynamic route files.
   - Owns release behavior tests; does not edit header CSS.
3. **Manual owner**
   - DSL data/i18n module and `NamespacePage.astro` (or `DslManualPage.astro`).
   - Owns manual behavior tests; does not change raw snapshot files or namespace URI.
4. **Discovery/QA owner (after integrations land)**
   - `LandingPage.test.mjs` consolidation, `public/sitemap.xml`, `public/llms.txt`, complete visual
     matrix and metadata validation.

`i18n/reference.ts` and `LandingPage.test.mjs` are current merge hotspots. Either assign a single
integration owner or split their content first; do not give independent workers overlapping edits.

## 9. Minimal implementation order

1. Extend `BaseLayout`/`ProductShell` with concrete localized locations and convert the header to a
   tested one-row structure. Keep existing routes compiling through defaults.
2. Expand the release data model, add static detail routes, and change existing release routes into
   the index. Then update metadata and discovery to the concrete paths.
3. Replace Namespace's composition with the manual layout while preserving namespace identity,
   directive/search/copy behavior, and raw resources.
4. Update the browser route matrices and sitemap/llms contracts, build, then visually inspect
   Chinese and English at 320/390/768/1440 px in light/dark modes.

This sequence normalizes route identity first, so page work does not accumulate special-case
canonical, locale, or header fixes. It also keeps the established home composition outside the main
refactor, which is the safest interpretation of the user's request.
