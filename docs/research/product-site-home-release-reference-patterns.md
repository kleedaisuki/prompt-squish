# Product-site patterns for home, releases, and namespace/reference surfaces

- **Research date:** 2026-09-16
- **Decision target:** redesign `xmlsquish.moesegfault.dev` without turning it into a documentation site
- **Scope:** information architecture, release navigation, XML namespace/reference presentation,
  link interaction, and accessibility
- **Method:** first-party site inspection at a 1440×1000 desktop viewport; first-party standards and
  product documentation review; repository inspection; a small amount of peer-reviewed HCI evidence
- **Evidence status:** observations describe the live sites on the research date. They are examples,
  not universal proof that their designs cause better outcomes.

## Executive decision

Keep xmlsquish as **one product site with three first-class destinations**:

```text
Home                  /                  /en/
Releases              /releases/         /en/releases/
XML Namespace         /ns/               /en/ns/
Raw, stable material  /ns/dsl.md and /ns/<version>/dsl.md
```

Use one product header, one footer, one color/type/token system, and the existing home page's wide,
editorial layout across all three destinations. Do **not** introduce a generic “Docs” section, a
documentation sidebar, or a separate documentation-looking shell. A user should perceive Releases
and Namespace as product surfaces, not as a manual attached to the product.

The highest-value design choices are:

1. Make `Home`, `Releases`, and `Namespace` the only content destinations in the primary navigation.
2. Put the current version in a compact status chip near the brand or hero. The chip leads to the
   latest release entry.
3. Make `/releases/` a chronological product update surface. Lead with the current release and its
   user outcomes; keep full engineering details and binary assets linked to the GitHub Release.
4. Make `/ns/` a visually rich reference **landing page**: canonical URI, copy control, compatibility
   policy, vocabulary cards, one short real example, and links to the raw/versioned specifications.
5. Remove text underlines throughout the authored UI, as requested, but replace them with persistent
   non-color affordances: a marker/highlight band, a pill or card boundary, weight, and/or an arrow.
   Hover and focus should strengthen the same highlight rather than introduce a different visual
   language.
6. Keep the home page's present composition and content order. Integrate a small “latest release”
   rail and update primary navigation; do not rebuild it as a docs portal.

## Current repository evidence

The current implementation already has the right raw ingredients but presents two different product
identities:

| Observation | Repository evidence | Design implication |
| --- | --- | --- |
| Home is a wide product page with a branded header and sections such as build, model, and CLI. | `site/src/components/LandingPage.astro` | Preserve its layout, content sequence, and visual hierarchy. |
| Home navigation mixes in-page anchors (`#build`, `#model`, `#cli`) with `Releases` and `Namespace`. | `LandingPage.astro:49-55` | Promote the three site destinations; keep section jumps local or secondary. |
| Releases and Namespace use a separate narrow “reference” shell. | `site/src/layouts/ReferenceLayout.astro` | Replace the separate identity with the home shell and product-width compositions. |
| The release route is currently one launch page, not a chronological release log. | `site/src/components/ReleasePage.astro` | Turn `/releases/` into a current-release-led history while retaining the strong 1.0 launch material. |
| Namespace is mostly prose, tables, and lists. | `site/src/components/NamespacePage.astro` | Retain the semantics, but present identity and vocabulary as product UI components. |
| Global focus treatment is already strong: a 3px outline with a 4px offset. | `site/src/layouts/BaseLayout.astro:151-154` | Preserve this behavior while unifying all link states. |
| Home `.text-link` still uses the default underline. | `LandingPage.astro:498-503` | Replace it with the shared marker/highlight link treatment. |

## What mature developer-product sites do

### Comparison matrix

| Site | Observed information architecture | Release treatment | Reference treatment | What xmlsquish should take | What xmlsquish should not take |
| --- | --- | --- | --- | --- | --- |
| [Rust](https://rust-lang.org/) | Product home leads to Install, Learn, Playground, Tools, Governance, Community, and Blog. | The current version is a prominent link beside the primary Get Started CTA; official release announcements have a dedicated [release index](https://blog.rust-lang.org/releases/). | Reference and learning material live outside the marketing narrative. | A visible current-version route and an explicit chronological release index. | Its many organizational destinations and fragmented domains are disproportionate for xmlsquish. Its default underlined navigation also conflicts with the requested visual language. |
| [Astro](https://astro.build/) | Marketing-first home; Documentation and Blog are stable global destinations. The hero remains a product pitch. | A compact “Astro 7.3 / Available now” chip links directly into a release story. Release articles such as [Astro 7.3](https://astro.build/blog/astro-730/) use a strong visual cover, date, concise outcome summary, upgrade command, and detailed changes. | Reference is separated from guides; individual pages state purpose before enumerating options, as in the [Configuration Reference](https://docs.astro.build/en/reference/configuration-reference/). | A latest-release chip on home; outcome-first release detail; reference pages that open with orientation and a real example. | Mixing product releases, monthly roundups, partner news, and case studies in one blog feed. xmlsquish needs a smaller, more predictable release log. |
| [Bun](https://bun.sh/) | Home, Docs, Guides, Reference, and Blog share one strong global header; the home page is unapologetically product-led. | A release announcement sits above the hero and a versioned install CTA is visible immediately. | [Bun Docs](https://bun.sh/docs) presents its four product capabilities as large cards before the long taxonomy. | A stable shared shell, product capability cards, current-version announcement, and direct copyable action. | Bun's two navigation rows and deep documentation tree. xmlsquish has only one compact namespace vocabulary and should not simulate a large docs estate. |
| [Vite](https://vite.dev/) | Home, Guide, Config, Plugins, Resources, version, and search share one site shell. The home page keeps a marketing hero, proof, and runnable install command. | [Releases](https://vite.dev/releases) explains cadence, support, and migration policy; [the blog](https://vite.dev/blog) contains major/minor release stories; GitHub holds the exhaustive changelog. | Config and API are top-level task destinations rather than one undifferentiated docs bucket. | Separate “what changed and why” from exhaustive commit-level notes; keep the installed version visible. | Vite's three-way split between policy, announcement, and GitHub would be too much navigation for a small project unless volume eventually justifies it. |
| [Cloudflare Developer Docs](https://developers.cloudflare.com/) | The landing page is a product directory: broad categories lead with user tasks and runnable actions. Changelog is a top-level destination. | The [Changelog](https://developers.cloudflare.com/changelog/) is chronological, filterable, tagged by product, and offers RSS. Entries state the user-visible change before linking to instructions. | Reference/API and task guides are distinct destinations. | Date rail + compact category chips + outcome-first release summaries. Add RSS only if it is generated from the same release data and has a maintenance owner. | A permanent left docs sidebar, product mega-directory, or filters before there are enough releases to need them. |
| [Tailwind CSS](https://tailwindcss.com/) | Home, Docs, Play, Blog, Showcase, Partners, and version share one minimal product header. The hero is a live demonstration, not a documentation table of contents. | Version is a small badge adjacent to the brand; Blog remains separate. | Reference is discoverable through search and topical navigation, while the home page continues to sell the mental model. | Compact version badge, product-level global header, and a concrete visual demonstration. | Exposing implementation syntax as decoration when it does not teach a real xmlsquish workflow. |

### Cross-site synthesis

The sites differ in style, but five structural regularities recur:

1. **The home page answers “why this product?” before “where is every page?”** Astro, Bun, Vite,
   and Tailwind all keep a marketing hero and an immediate runnable action.
2. **The current release is visible without dominating global navigation.** Rust uses a version link;
   Astro uses a release chip; Bun uses an announcement rail; Tailwind and Vite use version badges.
3. **Release stories and exhaustive change data have different jobs.** Astro and Vite narrate
   meaningful outcomes in authored pages while GitHub or package changelogs remain the exhaustive
   engineering record.
4. **Reference starts with orientation.** Bun's docs landing uses capability cards, while Astro's
   reference pages explain who and what the reference is for before listing symbols or options.
5. **Large documentation machinery follows content scale rather than preceding it.** Cloudflare's
   sidebar and filters make sense for hundreds of products and daily changes; copying them for a
   small CLI would create empty complexity.

This supports a compact architecture for xmlsquish: three global destinations, no generic Docs tab,
and progressively disclosed technical depth.

## Proposed page models

### 1. Shared product shell

Desktop header:

```text
[xmlsquish mark + name]     Home   Releases   Namespace      v1.0.1   中文/EN   theme   GitHub
```

Mobile header:

```text
[mark + name]                                                [Menu]
```

The opened menu contains the same destinations, version, locale, theme, and GitHub links. Do not
hide the primary destinations at small widths without providing a replacement menu; the current
home implementation hides `.header-nav` below 800px.

Use an active-state highlight behind the current destination. The page hierarchy should not depend
on a breadcrumb in this small site. Keep the header static unless testing shows sticky navigation
helps; a sticky header consumes a large fraction of short mobile viewports.

### 2. Home: preserve the current product story

Keep the current home page's layout and major sections. Make only integrative changes:

- replace section-anchor-heavy global navigation with the shared three-destination header;
- add a compact latest-release chip above or adjacent to the hero headline:
  `v1.0.1 · runtime ports, stable artifact locators →`;
- let the chip lead to the v1.0.1 section/detail on `/releases/`;
- retain the copyable workflow and product diagrams;
- add a small closing trio of destination cards: “Start a project,” “See what's new,” and
  “Use the XML namespace”; and
- keep section anchors available through local contextual links, not as the site's main IA.

The release chip is a status/navigation element, not a second hero. It should occupy one line and
must wrap cleanly on narrow screens.

### 3. Releases: a product update surface, not a changelog dump

Recommended `/releases/` composition:

1. **Hero:** “Ship with confidence” or similarly user-centered heading; current stable version,
   date, supported platforms, and install/download CTA.
2. **Current release feature:** v1.0.1 outcome headline, a short summary, three to five grouped
   changes, and a direct GitHub Release/download action.
3. **Chronological release rail:** v1.0.0, v0.3.0, v0.2.0. Each entry includes date, stability badge,
   one-sentence outcome, 2–4 change bullets, migration/compatibility status, and a “Read release”
   card action.
4. **Release contract:** semantic versioning/support statement, checksum/integrity boundary, and a
   link to the exhaustive GitHub release history.

Release records should be typed data rendered into both the visible page and machine surfaces. Do
not hand-maintain separate visible and JSON claims. GitHub defines releases as deployable iterations
based on Git tags and designed to package notes and binary assets; use GitHub Release as the artifact
authority and the product site as the narrative/discovery surface ([GitHub, “About releases”](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases)).

When a release has enough content for a detail URL, prefer `/releases/1.0.1/`; the index still shows
its meaningful summary. For the current small history, detail sections with stable fragment IDs are
also acceptable, provided each version remains directly linkable. Avoid accordions as the only path
to release content because expanded state is less robust for linking, search, and no-JavaScript use.

### 4. Namespace: a reference landing, not a docs article

The W3C requires/recommends that a namespace URI be dereferenceable and that its namespace document
describe the relationship between the URI and the defining specification. It also recommends a
clear change/persistence policy and material for human and ideally machine consumption
([W3C namespace URI policy](https://www.w3.org/guide/editor/namespaces.html)). The XML Namespaces
specification also makes clear that namespace names are compared as literal strings, so seemingly
small URI variants are not interchangeable
([Namespaces in XML 1.0 §2](https://www.w3.org/TR/xml-names/#ns-decl)).

The `/ns/` page therefore has an identity job before it has a teaching job. Recommended composition:

1. **Identity hero**
   - “The xmlsquish XML namespace”
   - canonical namespace URI in a copyable field;
   - status: stable/current;
   - current language version and last updated date.
2. **What it enables**
   - 2–3 outcome cards: composition, arguments, and deterministic project builds.
3. **Vocabulary explorer**
   - cards grouped by conceptual role rather than a long undifferentiated table;
   - each card exposes the local name, kind, short semantics, and a linkable anchor;
   - an optional search/filter appears only if the vocabulary becomes large enough.
4. **Real minimal example**
   - one compact XML sample with Copy;
   - annotated outcome below it, not a tutorial sequence.
5. **Stability and versioning contract**
   - whether compatible additions reuse the namespace;
   - what kind of change would require a new URI;
   - exact links to current raw Markdown and frozen versioned copies.
6. **Technical exits**
   - “Read raw specification,” “View examples,” “Version history,” and “Report an issue.”

Keep `/ns/dsl.md` machine-readable and stable. The human landing and raw material can coexist; W3C
explicitly frames a namespace document as something that may provide information directly **or by
reference**, for humans and ideally machines. Do not redirect the namespace URI straight to GitHub,
because doing so gives up the product-owned identity and persistence statement.

## Release data and content ownership

Use one source of truth per release, with fields similar to:

```text
version, tag, date, status, headline, summary, highlights[], compatibility,
migration, platforms[], githubReleaseUrl, downloadBaseUrl, checksumsUrl
```

Derive these surfaces from the same record:

- release index and detail UI;
- current-version badge/chip on home;
- structured release JSON;
- sitemap entries;
- Open Graph title/description/image metadata; and
- optional RSS.

The product site's copy should be curated, not automatically generated from commits. GitHub's own
generated release notes are framed as an overview of merged pull requests and contributors plus a
full-changelog link, and GitHub explicitly recommends reviewing the generated set
([GitHub, “Automatically generated release notes”](https://docs.github.com/en/repositories/releasing-projects-on-github/automatically-generated-release-notes)).
That is useful release engineering input, not finished product copy.

## Link language without underlines

### Constraint and evidence

The requested direction—no underlines—is viable, but “remove underline and change only the text
color” is not a safe implementation.

- WCAG 2.2 SC 1.4.1 requires that color not be the only visual means of identifying an action
  ([W3C Understanding 1.4.1](https://www.w3.org/WAI/WCAG22/Understanding/use-of-color)).
- W3C Technique G183 permits color-only identification when the link differs from surrounding text
  by at least 3:1 in luminance and gains an additional hover/focus cue, but W3C explicitly says this
  is not the preferred approach for inline prose
  ([W3C G183](https://www.w3.org/WAI/WCAG22/Techniques/general/G183)).
- A controlled visual-search study found that link formatting affected search speed and participants
  preferred bold-underlined links, although not accuracy. It predates current product UI conventions
  and should be treated as evidence that affordance matters, not as a mandate to underline every link
  (Ling & Van Schaik, *Ergonomics* 47(8), 2004,
  [doi:10.1080/00140130410001670417](https://doi.org/10.1080/00140130410001670417)).
- Eye-tracking evidence found that color or underlining can independently highlight hypertext without
  a detectable first-pass reading penalty; again, it supports salience rather than one mandatory
  decoration (Scharinger et al., 2016,
  [PMC5036113](https://pmc.ncbi.nlm.nih.gov/articles/PMC5036113/)).

The production decision is therefore to replace the underline with **persistent shape and weight**,
then amplify those cues on interaction.

### Three link components

| Context | Resting state | Hover state | Keyboard focus | Rationale |
| --- | --- | --- | --- | --- |
| Global/local navigation | Medium/semibold label, generous padded target; active item has a rounded tinted background or marker band. | Highlight fill becomes stronger; optional arrow/indicator shifts by 1–2px. | Existing 3px high-contrast outline plus the same highlight. | Placement already establishes navigation; active shape supplies a non-color cue. |
| Inline prose link | Semibold text plus a **permanent low marker band** behind the lower 35–45% of the glyphs; optional `↗` only for a true external destination. | Marker expands to a soft full-height fill, text remains readable. | Marker expands and the 3px outline remains visible. | Preserves the home page's highlight character without underlines and remains identifiable before hover. |
| Card/CTA link | Visible border/background and action label; the whole card may be clickable if it contains one destination. | Border/accent glow or small elevation change; no layout shift. | 3px outline around the whole target. | Component shape is the affordance; no underline is necessary. |

Illustrative behavior, not a required literal implementation:

```css
.highlight-link {
  color: var(--moe-accent-strong);
  font-weight: 700;
  text-decoration: none;
  background: linear-gradient(
    transparent 58%,
    color-mix(in srgb, var(--moe-accent) 24%, transparent) 58%
  );
  border-radius: 0.18em;
}

.highlight-link:is(:hover, :focus-visible) {
  background: color-mix(in srgb, var(--moe-accent) 18%, transparent);
}

.highlight-link:focus-visible {
  outline: 3px solid var(--moe-accent-strong);
  outline-offset: 3px;
}
```

The light and dark palettes must be measured independently. A color token name is not evidence of
contrast. Text should meet WCAG text contrast; focus rings and component boundaries should meet
non-text contrast against adjacent colors.

### Interaction rules

- Never make essential identification available only on hover; touch and keyboard users may never
  see it.
- Use `:focus-visible` rather than removing the user-agent outline. WCAG 2.2 requires visible keyboard
  focus; its AAA focus appearance guidance uses an area at least equivalent to a 2px perimeter and
  3:1 state contrast ([W3C Focus Appearance](https://www.w3.org/WAI/WCAG22/Understanding/focus-appearance.html)).
- Make primary control/link targets at least 24×24 CSS pixels or provide the specified spacing;
  larger targets are preferable for primary navigation
  ([W3C Target Size Minimum](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum)).
- Do not animate font weight, because it can reflow text. Animate background position/opacity or a
  contained arrow transform instead.
- Disable decorative motion under `prefers-reduced-motion: reduce`.
- Use descriptive link labels. “Read v1.0.1 release” is better than a repeated “Learn more.”
- An external-link arrow is a visual hint, not a substitute for a descriptive name. Do not force a
  new tab solely because the destination is external.

## Screenshot and live-observation record

The following untracked research captures were generated with Playwright/Chromium at 1440×1000 in
light mode (except Vite, whose stored preference rendered dark on its home page). They are retained
under `.temp/research-product-sites/` for local inspection and are intentionally not production
assets.

| Capture | Observation |
| --- | --- |
| `astro-home.png` | A compact release chip sits above a centered marketing hero; Documentation and Blog remain global destinations, not hero content. |
| `astro-release.png` | A release detail opens with a strong version image, date/title/author, then a compact outcome summary before technical sections. |
| `bun-home.png` | Home shares the same global Home/Docs/Guides/Reference/Blog shell as docs, but the body is benchmarks, workflow, trust, and installation—clearly a product page. |
| `bun-docs.png` | The reference entry starts with four capability cards and a concrete “get started with” action before the long sidebar taxonomy. |
| `vite-home.png` | The hero contains value proposition, two CTAs, a package-manager command, and social proof; version and resources live in the header. |
| `vite-releases.png` | “Releases” is a policy/support page with a right-side table of contents and GitHub link, not a chronological product feed. Useful separation, but too documentation-like for the target surface. |
| `cloudflare-docs.png` | A docs landing can still feel product-like when it leads with primitives, runnable action, and visual grouping rather than prose. |
| `cloudflare-changelog.png` | Chronological date rail, product tags, outcome-led headings, filters, and RSS make high-volume change history scannable. |
| `tailwind-home.png` | Version badge is subordinate to the brand; the hero demonstrates the core mental model directly. |
| `rust-home.png` | Version is prominent beside Get Started and links to a release announcement; the rest of the home page remains an outcome-oriented product explanation. |

Computed-style sampling of the visible top navigation found no text underline for the sampled Vite,
Astro, Bun, Tailwind, and Cloudflare navigation links. Bun supplied the strongest authored focus
treatment in the sample: a 2px pink outline and highlighted background. This sampling does **not**
prove that every link on those sites follows the same rule—indeed, Vite and Cloudflare still underline
many inline prose links. xmlsquish should use its own coherent highlight system across both navigation
and prose rather than copying any one site wholesale.

## Adopt / avoid checklist

### Adopt now

- [ ] One shared product shell for Home, Releases, and Namespace.
- [ ] Exactly three content destinations in primary navigation.
- [ ] Current version badge/chip that deep-links to the latest release.
- [ ] Release index led by current release, then a chronological rail.
- [ ] Typed release data reused by UI and machine surfaces.
- [ ] Product-owned namespace identity page plus stable raw/versioned specs.
- [ ] Permanent marker/highlight link treatment with stronger hover/focus states.
- [ ] Visible 3px `:focus-visible` outline, verified in light and dark themes.
- [ ] Full primary navigation available on mobile through an accessible menu.

### Avoid now

- [ ] No generic Docs top-level destination.
- [ ] No permanent left sidebar or right “On this page” rail on the main product surfaces.
- [ ] No mega-menu, search, release filter, or RSS unless content volume justifies and owns it.
- [ ] No duplicated release claims maintained in separate source files.
- [ ] No hidden release text behind client-only accordions.
- [ ] No color-only or hover-only link affordance.
- [ ] No automatic import of commit messages as customer-facing release copy.
- [ ] No redesign of the existing home page into a documentation index.

## Verification plan for the implementation

| Requirement | Authoritative check |
| --- | --- |
| Product rather than docs feel | Visual review at 1440, 1024, 768, 390, and 320 CSS-pixel widths; no documentation sidebar; home composition retained. |
| IA is coherent | Header exposes Home, Releases, Namespace on every localized page; active destination is visible without relying only on color. |
| Release history is complete | Every shipped version in repository release notes appears in chronological order and deep-links to its GitHub Release/tag. |
| v1.0.1 is current everywhere | Visible badge, releases hero, JSON metadata, structured data, install command, GitHub Release URL, and downloads agree. |
| Namespace identity is stable | Fetch canonical URI and raw/versioned specification URLs; verify status, content type, URI string, and persistence/versioning copy. |
| Links have no authored underlines | Browser-computed `text-decoration-line` check across nav, prose, cards, footer, hover, focus, light, and dark. |
| Links remain discoverable | Static-state screenshots in grayscale plus keyboard-only review; inline links retain weight/marker/shape before interaction. |
| Focus is usable | Tab through every interactive element; focus is not clipped/obscured and meets visible contrast in both themes. |
| Touch targets are usable | Automated geometry assertion for primary navigation, theme/locale controls, copy buttons, and CTAs; minimum 24×24 CSS px with spacing. |
| No-JavaScript path works | Disable JavaScript: content, navigation, release history, raw spec links, and direct downloads remain available. |
| Motion respects users | Emulate `prefers-reduced-motion: reduce`; nonessential transforms/transitions are removed. |

## Bottom line

The useful lesson from mature sites is not “add docs navigation.” It is **make product identity,
current state, and technical truth mutually reachable without collapsing them into one page type**.
For xmlsquish, the simplest faithful expression is Home / Releases / Namespace in one visual system:
home persuades and demonstrates, releases establish current state and trust, and namespace establishes
stable technical identity. The shared highlight interaction can make all three feel like the existing
home page while satisfying the no-underline request without sacrificing discoverability or keyboard
access.
