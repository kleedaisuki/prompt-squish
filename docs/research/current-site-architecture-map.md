# Current Astro Site Architecture Map

- **Inspected revision:** `a4a0ef6c804ebaa82b5f66c4a2d1238c0475d52b`
- **Inspection date:** 2026-09-16
- **Scope:** current `site/` implementation, generated route surface, presentation primitives,
  release/namespace coupling, tests, and deployment path
- **Purpose:** implementation map for the v1.0.1 release and product-site redesign; this is not a
  redesign specification

## 1. Runtime and component hierarchy

The site is a statically generated Astro 7 application with one small React island. There is no
server runtime and no content collection.

```text
Astro route
└─ BaseLayout.astro                         global metadata, theme bootstrap, MoeSegfault CSS
   ├─ LandingPage.astro                    product home (direct child on / and /en/)
   │  ├─ ThemeControl.tsx                  client:load React island
   │  └─ BuildExplorer.astro               Astro markup + custom element enhancement
   └─ ReferenceLayout.astro                namespace/release shell
      ├─ ThemeControl.tsx                  client:load React island
      ├─ NamespacePage.astro               namespace identity and vocabulary table
      └─ ReleasePage.astro                 v1 product release page
```

Concrete entry points:

| Route source | Component root | Output route |
| --- | --- | --- |
| `site/src/pages/index.astro` | `BaseLayout(zh-CN) > LandingPage` | `/` |
| `site/src/pages/en/index.astro` | `BaseLayout(en) > LandingPage` | `/en/` |
| `site/src/pages/ns.astro` | `NamespacePage > ReferenceLayout > BaseLayout` | `/ns` |
| `site/src/pages/en/ns.astro` | `NamespacePage > ReferenceLayout > BaseLayout` | `/en/ns/` |
| `site/src/pages/releases.astro` | `ReleasePage > ReferenceLayout > BaseLayout` | `/releases/` |
| `site/src/pages/en/releases.astro` | `ReleasePage > ReferenceLayout > BaseLayout` | `/en/releases/` |
| `site/src/pages/ns/dsl.md.ts` | endpoint importing `docs/dsl.md?raw` | `/ns/dsl.md` |

`BaseLayout.astro` owns canonical/alternate URLs, Open Graph and Twitter metadata, optional social
cards, the landing-page `SoftwareApplication` JSON-LD, the global skip link, the pre-paint theme
bootstrap, and the pinned external MoeSegfault stylesheet. `ReferenceLayout.astro` adds a separate
header/footer and a generic reading-column treatment. `ReleasePage.astro` opts into a wider
`product-release` variant but still inherits the reference shell. This shell boundary is the main
reason the release and namespace surfaces currently feel like documents rather than extensions of
the home product interface.

## 2. Routing and internationalization

`site/src/i18n/routes.ts` is the typed route authority:

```ts
type Page = "home" | "namespace" | "releases";
paths["zh-CN"] = { home: "/", namespace: "/ns", releases: "/releases/" };
paths.en       = { home: "/en/", namespace: "/en/ns/", releases: "/en/releases/" };
```

It also owns the nonlocalized vocabulary identity
`https://xmlsquish.moesegfault.dev/ns`. `BaseLayout` uses `Page` and `paths` to derive self-canonical,
reciprocal `hreflang`, and Chinese `x-default` links. There is no locale negotiation or redirect;
the language selector is a normal anchor.

Localization is compiled TypeScript rather than content files:

- `site/src/i18n/messages.ts` defines the `Locale`, home-page `Messages` shape, Chinese and English
  objects, and merges in `referenceMessages`.
- `site/src/i18n/reference.ts` defines one complete English object, constrains Chinese to the same
  inferred `ReferenceMessages` shape, and contains both namespace and release copy.
- Components select `messages[locale]`; markup and layout are shared, so locale drift is primarily
  a copy-length/layout risk rather than duplicated-template risk.

The manually maintained `site/public/sitemap.xml` repeats the same six localized human routes.
`site/public/llms.txt` separately repeats the release and namespace route/version story. Neither is
generated from `paths`, so both can drift.

## 3. Home page composition and reusable visual primitives

`LandingPage.astro` is a monolithic 1,187-line component: roughly 155 lines of product markup and
over 1,000 lines of scoped responsive CSS. Its information architecture is:

1. sticky product header (`site-header`, `header-inner`, `brand`, `header-nav`, `header-actions`);
2. two-column hero with headline/actions and a build-graph figure;
3. three-step `journey-band`;
4. real build explorer (`BuildExplorer`);
5. project/frame inheritance model;
6. capability cards;
7. CLI terminal and structured diagnostic;
8. artifact metrics;
9. explicit boundary/details area;
10. closing CTA and product footer.

The strongest primitives to reuse for the integrated home/release/namespace product surface are:

| Primitive | Current selector/source | Reuse value |
| --- | --- | --- |
| width container | `.page-width` | Stable `min(100% - 3rem, 76rem)` product grid |
| product chrome | `.site-header`, `.header-inner`, `.brand`, `.header-nav`, `.header-actions` | Existing home identity the redesign is required to preserve |
| section rhythm | `.section`, `.section-heading` | Consistent generous editorial spacing and heading scale |
| semantic kicker | `.eyebrow` | Mono, accent-colored wayfinding without looking like docs navigation |
| highlighted actions | `.action.primary`, `.action.secondary` | Existing non-underlined high-salience link treatment |
| quiet navigation | `.header-nav a`, `.language-link`, `.footer-inner a` | Non-underlined text links with accent hover |
| product diagrams | `.build-graph`, `.frame-node`, `.inheritance-map` | Visual explanation language for release and namespace concepts |
| card/surface vocabulary | `.capability`, `.metric-stage`, `.engine-details` | Border/surface patterns already coherent with the home page |
| dark code surface | `.command-terminal`, `.product-map`, `--xs-code` | Product demo/evidence without generic documentation prose |
| responsive rules | 1050/800/520 px media queries | Existing tested collapse strategy |
| focus treatment | global `:focus-visible` | 3 px accent outline; preserve for every new link treatment |
| theme control | `ThemeControl.tsx` + `.css` | Shared auto/light/dark control and stored preference |

The release page already partially reimplements several of these under different names (`.button`,
`.section`, `.hero`, `.eyebrow`, `.grid`, `.closing`). Consolidating these into shared product-shell
components/styles would remove the special case instead of synchronizing two near-duplicates.

## 4. CSS and theme tokens

`BaseLayout.astro` loads MoeSegfault Style 0.1.2 from
`https://style.moesegfault.dev/v0.1.2/css/all.css`, pinned with SRI. Local styling consumes its
public tokens, especially:

- color: `--moe-background`, `--moe-heading`, `--moe-text`, `--moe-text-soft`,
  `--moe-accent`, `--moe-accent-strong`, `--moe-accent-wash`, `--moe-on-accent`,
  `--moe-surface`, `--moe-surface-strong`, `--moe-surface-muted`, `--moe-border`, `--moe-danger`;
- type: `--moe-font-family-mono`;
- dimensions: `--moe-space-*`, `--moe-radius-sm/md/lg`.

Four site aliases are defined globally in `BaseLayout.astro`:

| Token | Meaning |
| --- | --- |
| `--xs-line` | translucent border derived from `--moe-border` |
| `--xs-glow` | translucent accent glow |
| `--xs-code` | dark brown code/product-diagram surface, with a dark-theme override |
| `--xs-code-text` | cream foreground for that surface |
| `--xs-max` | 73.75 rem width (used by release/reference width) |

Styles otherwise live inside components and are Astro-scoped. `ReferenceLayout` alone uses
`<style is:global>`, so its `.reference-content ...` rules intentionally reach slotted child
markup. That global descendant coupling makes a visual integration risky if new product sections
remain under `ReferenceLayout`.

## 5. Existing link behavior

The home page already provides the requested direction:

- brand, header navigation, language switch, CTA buttons, and footer links explicitly use
  `text-decoration: none`;
- header/language links highlight through `color: var(--moe-accent-strong)` on hover;
- `.action` links are filled or bordered controls with a small upward hover translation;
- all keyboard-focusable links retain a clear accent outline.

The inconsistent case is `.text-link`: it is accent colored and bold but retains the user-agent or
upstream stylesheet underline, with only `text-underline-offset` customized. Generic anchors in
`ReferenceLayout`, `NamespacePage`, and non-button links in `ReleasePage` also inherit the upstream
link decoration. Therefore the underlines are not one isolated selector; the redesign needs one
explicit shared inline-link/highlight primitive and a deliberate exception, if any, for prose.

Avoid a blanket `a { text-decoration: none }`: it would remove affordance from dense namespace
resource lists without adding a replacement. Prefer a product-style highlighted state (accent
color/background/border or icon movement) applied consistently with `:hover`, `:focus-visible`, and
visited-state decisions.

## 6. Release and namespace surfaces

### ReleasePage

`site/src/components/ReleasePage.astro` is a static latest-release implementation, not a release
log model. It hardcodes `version = "1.0.0"`, derives the tag/download URLs, constructs six native
asset links from a local platform matrix, emits its own JSON-LD entity, and renders a launch page:
hero/product map, six-command journey, quickstart, feature/output cards, downloads, source install,
migration, durability boundary, and closing CTA.

All prose is in `reference.ts`, but release identity/date/asset structure is split across the
component, translations, `BaseLayout`, public JSON, `llms.txt`, tests, images, and design evidence.
There is no list of releases, version selector, structured changelog data, or per-version route.
Consequently, changing the page to v1.0.1 currently overwrites the v1.0.0 story rather than adding
an actual release log.

### NamespacePage

`site/src/components/NamespacePage.astro` is a compact reference renderer with:

- stable namespace identity and an XML example;
- current/tagged/historical specification links;
- a hardcoded ordered array of 11 element names zipped by index against translated descriptions;
- validation/versioning prose and three standards links.

It links the current working spec through `/ns/dsl.md`, while immutable snapshots live as copied
files under `site/public/ns/0.2.0/` and `0.3.0/`. The current endpoint imports repository
`docs/dsl.md` directly, so the web build depends on a file outside `site/`. The element-name array
and translated `elements[]` are positionally coupled; missing/reordered entries are type-correct
but semantically wrong.

## 7. Data and version authorities

There are three distinct data classes:

1. **Product demo evidence:** `site/src/data/build-demo.json`, generated by
   `site/scripts/build-demo.mjs` from the real `examples/site-demo` project by running the workspace
   CLI, building both argument scenarios, and inspecting artifacts through public typed locators.
   `LandingPage` and `BuildExplorer` consume this file. `npm run demo:check` detects drift, but CI's
   site job does not currently run it.
2. **Localized product copy:** `messages.ts` and `reference.ts`, manually maintained.
3. **Published release machine contract:** `site/public/releases/1.0.0.json`, manually maintained
   and copied byte-for-byte to `dist` by Astro.

The v1.0.0 identity is duplicated in at least these production/test surfaces:

- `ReleasePage.astro` version, tag, date, asset URLs, install command, and JSON-LD;
- `reference.ts` titles and prose in both locales;
- `BaseLayout.astro` home JSON-LD version, release metadata link/title, and release social image;
- `public/releases/1.0.0.json`;
- `public/llms.txt`;
- `public/release-v1.{svg,png}`;
- `LandingPage.test.mjs` expected links, metadata, JSON-LD, and assertions.

`build-demo.json` also contains package version 1.0.0, but this is the demo fixture's package
version, not necessarily the product release authority; changing it blindly would conflate domains.

## 8. Tests and visual-QA capability

`site/src/components/LandingPage.test.mjs` is a Node test file that launches pinned Playwright
Chromium against a minimal local static server serving `site/dist`. Its present 38-test suite covers:

- BuildExplorer tabs, keyboard navigation, exact copied source, modes, and anchor integrity;
- visible project-creation/new-vs-init messaging;
- home pages at widths 320/390/768/1440 in light and dark with no document overflow;
- no-JavaScript BuildExplorer fallback;
- release and namespace locale/canonical/hreflang identity at 390 and 1440 px;
- release command, asset, source-install, quickstart, social image, and JSON-LD contracts;
- release light/dark responsiveness;
- release JSON, `llms.txt`, robots, and sitemap discovery contracts;
- current DSL and byte-exact 0.2.0/0.3.0 snapshots;
- theme behavior under blocked local storage and reduced motion.

If `UI_SCREENSHOT_DIR` is set, the suite emits full-page and selected hero screenshots. Historic,
inspectable artifacts under `.temp/v1-visual-validation/` additionally demonstrate a custom matrix
capture workflow for zh/en × mobile/desktop × light/dark, focused crops, overflow measurements,
and manual image review. The repository does not expose that custom capture script as a maintained
site command; it is temporary evidence rather than production tooling.

Important gate gap: `npm run test` only runs `astro check` and `astro build`. CI and Pages run check
and build but do **not** run `npm run test:browser` or `npm run demo:check`. Browser correctness and
the real-CLI-backed demo can therefore regress while required CI remains green.

## 9. Build, preview, and deployment

The package contract (`site/package.json`) is:

| Command | Behavior |
| --- | --- |
| `npm run dev` | `astro dev` |
| `npm run check` | Astro/TypeScript checking |
| `npm run build` | static build into `site/dist` |
| `npm run preview` | `astro preview` |
| `npm run test` | check + build only |
| `npm run test:browser` | Node/Playwright contract suite; requires a current build |
| `npm run demo:generate` | regenerate real CLI demo evidence |
| `npm run demo:check` | verify real CLI demo evidence has not drifted |

`astro.config.ts` fixes the origin to `https://xmlsquish.moesegfault.dev`, static output, and React
integration. `site/public/CNAME` binds the custom domain. `.github/workflows/pages.yml` runs on every
push to `main`, installs via the tracked npm lockfile on Node 24, runs `npm run check`, lets the
pinned official Astro action build/upload, then deploys through GitHub Pages. `.github/workflows/ci.yml`
independently installs, checks, and builds the site. Both use `package-lock.json`; the untracked
`pnpm-lock.yaml` and `pnpm-workspace.yaml` are not authoritative.

## 10. Coupling and redesign risks

| Risk | Evidence | Consequence |
| --- | --- | --- |
| release version fan-out | version repeated across component, copy, layout, public metadata, discovery, imagery, and tests | partial v1.0.1 update can ship contradictory pages or broken asset URLs |
| release log has no data model | one hardcoded latest release page | adding history by conditionals will increase special cases and make per-version linking difficult |
| two visual shells | home uses `LandingPage` chrome; release/namespace use `ReferenceLayout` | integration can preserve content but still fail the requested product-interface feel |
| monolithic home CSS | 1,000+ scoped CSS lines; useful primitives are not exportable | copying styles creates drift; broad editing causes avoidable merge conflicts |
| global reference descendants | `ReferenceLayout` uses global `.reference-content ...` rules | product cards/tables/code under the shell inherit document styling unexpectedly |
| positional namespace vocabulary | `names[]` zipped with translated `elements[]` | reorder or missing translation silently attaches the wrong contract |
| manual route/discovery copies | sitemap and llms are not derived from typed routes/releases | stale pages and metadata remain syntactically valid |
| external stylesheet dependency | remotely loaded pinned CSS provides core tokens and body typography | offline rendering differs and upstream retirement would remove the base style despite SRI safety |
| weak required gate | browser tests and demo drift check absent from CI/Pages | visual/product-contract regression can deploy |
| stale visual evidence | temporary reports reflect earlier candidate states in parts | historic PASS must not be treated as proof for the redesigned site |

## 11. Recommended single-writer boundaries

To support parallel work without a repository-wide writer lock:

1. **Release/data owner:** new structured release data module plus `ReleasePage.astro`, release copy,
   release JSON/discovery, and version-specific tests. These files form one consistency boundary.
2. **Product-shell owner:** shared header/footer/nav/link/action primitives and style extraction from
   `LandingPage.astro`, plus `BaseLayout`/`ReferenceLayout` integration. This owner should land the
   extraction before page owners rebase onto it.
3. **Namespace owner:** `NamespacePage.astro` and namespace-specific localized data/tests only;
   consume, do not duplicate, the product shell.
4. **QA/deployment owner:** browser-test harness, screenshot tooling, CI/Pages gates, sitemap
   validation, and visual matrix evidence. Avoid concurrent edits to product markup.
5. **Asset owner:** social/release artwork under `site/public`; coordinate filenames/metadata with
   the release/data owner but keep binary changes isolated.

The high-conflict files are `LandingPage.astro`, `reference.ts`, `BaseLayout.astro`, and
`LandingPage.test.mjs`. Do not assign independent page redesigns that all edit these four files.

## 12. Suggested architectural direction

Preserve the home layout by promoting its existing visual language rather than replacing it:

- extract one `ProductHeader`, `ProductFooter`, section-heading, action, and highlighted-link system;
- make home, release log, and namespace three product routes in that shared shell;
- represent releases as typed data with one latest pointer and an ordered history, then derive page
  content, metadata, downloads, discovery, and tests from it;
- represent namespace vocabulary as keyed records, never parallel positional arrays;
- retain static/no-JavaScript first rendering and the current minimal islands;
- add browser contracts and demo drift checks to the required CI path;
- rerun a fresh bilingual mobile/desktop light/dark screenshot matrix after integration, because
  old release-page evidence cannot validate the new shell.

This direction eliminates special cases while preserving the tested, recognizable home page—the
dominant real-world user entry point.
