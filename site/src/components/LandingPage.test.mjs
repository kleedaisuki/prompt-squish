/** End-to-end contracts for the localized product, release log, and DSL manual. / 本地化产品、发布记录与 DSL 手册的端到端合同。 */
import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { access, readFile, stat } from "node:fs/promises";
import { dirname, extname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const dist = resolve(dirname(fileURLToPath(import.meta.url)), "../../dist");
const origin = "https://xmlsquish.moesegfault.dev";
const namespaceUri = `${origin}/ns`;
const versions = ["1.0.4", "1.0.2", "1.0.1", "1.0.0", "0.3.0", "0.2.0"];
const dates = ["2026-09-20", "2026-09-17", "2026-09-16", "2026-09-15", "2026-09-11", "2026-09-11"];
const chapters = ["getting-started", "source-model", "composition", "control-and-scope", "build-and-artifacts", "reference", "limits-and-invariants"];
const expectedAssets = [
  "xmlsquish-1.0.4-x86_64-pc-windows-msvc.zip", "xmlsquish-1.0.4-aarch64-pc-windows-msvc.zip",
  "xmlsquish-1.0.4-x86_64-unknown-linux-gnu.tar.gz", "xmlsquish-1.0.4-aarch64-unknown-linux-gnu.tar.gz",
  "xmlsquish-1.0.4-x86_64-apple-darwin.tar.gz", "xmlsquish-1.0.4-aarch64-apple-darwin.tar.gz",
];

/** Create one bilingual route pair while retaining its top-level section. / 创建一组双语路由并保留顶层栏目。 */
function pair(zh, en, section) {
  return [
    { path: zh, peer: en, locale: "zh-CN", section },
    { path: en, peer: zh, locale: "en", section },
  ];
}

const routePairs = [
  ["/", "/en/", "home"],
  ["/releases/", "/en/releases/", "releases"],
  ...versions.map((version) => [`/releases/${version}/`, `/en/releases/${version}/`, "releases"]),
  ["/ns", "/en/ns/", "namespace"],
  ...chapters.map((chapter) => [`/ns/${chapter}/`, `/en/ns/${chapter}/`, "namespace"]),
];
const routes = routePairs.flatMap(([zh, en, section]) => pair(zh, en, section));
assert.equal(routes.length, 32);

const types = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".png": "image/png", ".json": "application/json", ".woff2": "font/woff2", ".md": "text/markdown; charset=utf-8", ".txt": "text/plain; charset=utf-8" };
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, "http://localhost").pathname);
    let file = resolve(dist, `.${pathname}`);
    assert(file === dist || file.startsWith(dist + sep));
    if ((await stat(file)).isDirectory()) file = join(file, "index.html");
    response.writeHead(200, { "Content-Type": types[extname(file)] ?? "application/octet-stream" });
    response.end(await readFile(file));
  } catch { response.writeHead(404).end("Not found"); }
});
let browser;
let base;

before(async () => {
  await access(join(dist, "index.html"));
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  base = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  await new Promise((done) => server.close(done));
});

/** Remove a trailing slash without changing the root path. / 删除末尾斜线但不改变根路径。 */
function normalizedPath(url) {
  const pathname = new URL(url, origin).pathname;
  return pathname === "/" ? pathname : pathname.replace(/\/$/, "");
}

/** Convert a published route into its expected canonical absolute URL. / 将发布路由转换为预期 canonical 绝对 URL。 */
function canonicalFor(path) {
  return origin + (path === "/ns" ? "/ns" : path);
}

test("all 32 human routes have localized identity and one active global destination", async () => {
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 390, height: 900 } });
  for (const route of routes) {
    const response = await page.goto(base + route.path, { waitUntil: "domcontentloaded" });
    assert.equal(response.status(), 200, route.path);
    assert.equal(await page.locator("html").getAttribute("lang"), route.locale, route.path);
    assert.equal(await page.locator("main#main").count(), 1, route.path);
    assert.equal(await page.locator("main h1").count(), 1, route.path);
    assert.equal(await page.locator(".product-header, .product-footer").count(), 2, route.path);
    assert.equal(await page.locator('.product-nav a[aria-current="page"]').count(), 1, route.path);
    const active = await page.locator('.product-nav a[aria-current="page"]').getAttribute("href");
    const topLevel = route.section === "home" ? (route.locale === "en" ? "/en/" : "/") : route.section === "releases" ? (route.locale === "en" ? "/en/releases/" : "/releases/") : (route.locale === "en" ? "/en/ns/" : "/ns");
    assert.equal(normalizedPath(active), normalizedPath(topLevel), route.path);
    assert.equal(normalizedPath(await page.locator(".language-link").getAttribute("href")), normalizedPath(route.peer), route.path);
    assert.equal(await page.locator('link[rel="canonical"]').getAttribute("href"), canonicalFor(route.path), route.path);
    assert.equal(await page.locator('link[rel="alternate"][hreflang="zh-CN"]').count(), 1, route.path);
    assert.equal(await page.locator('link[rel="alternate"][hreflang="en"]').count(), 1, route.path);
  }
  await page.close();
});

test("global controls remain in one vertically aligned header row", async () => {
  for (const width of [320, 390, 768, 1440]) {
    const page = await browser.newPage({ viewport: { width, height: 900 } });
    for (const path of ["/", "/releases/", "/releases/1.0.4/", "/ns", "/ns/reference/"]) {
      await page.goto(base + path, { waitUntil: "domcontentloaded" });
      const geometry = await page.locator(".header-row").evaluate((row) => {
        const children = [row.querySelector(".product-brand"), row.querySelector(".product-nav"), row.querySelector(".header-actions")];
        const rect = row.getBoundingClientRect();
        return { height: rect.height, centers: children.map((item) => { const box = item.getBoundingClientRect(); return box.top + box.height / 2; }) };
      });
      assert(geometry.height < 78, `${path} at ${width}px wrapped to a second row`);
      assert(Math.max(...geometry.centers) - Math.min(...geometry.centers) <= 1.5, `${path} at ${width}px is not vertically aligned`);
    }
    await page.close();
  }
});

test("home preserves its product layout, explorer, and direct latest-release cue", async () => {
  const demo = JSON.parse(await readFile(new URL("../data/build-demo.json", import.meta.url), "utf8"));
  for (const [path, releasePath] of [["/", "/releases/1.0.4/"], ["/en/", "/en/releases/1.0.4/"]]) {
    const page = await browser.newPage();
    await page.goto(base + path, { waitUntil: "domcontentloaded" });
    await page.waitForSelector("[data-build-explorer][data-ready]");
    assert.deepEqual(await page.locator("main > section").evaluateAll((sections) => sections.map((section) => section.id || (section.querySelector(".capabilities") ? "capabilities" : [...section.classList].find((name) => ["hero", "journey-band", "boundaries", "closing"].includes(name))))), ["hero", "how", "project-state", "build", "model", "capabilities", "cli", "stats", "boundaries", "closing"]);
    assert.equal(await page.locator(".hero-release-link").getAttribute("href"), releasePath);
    assert.match(await page.locator(".hero-release-link").textContent(), /v1\.0\.4/);
    await page.locator('[data-example-tab="prompt"]').click();
    await page.locator('[data-mode="self"]').click();
    assert.equal(await page.locator("[data-code-panel]:visible").getAttribute("data-source"), demo.scenarios.self.stages.prompt);
    await page.close();
  }
});

test("release indexes list six entries and every entry opens a same-locale detail", async () => {
  for (const prefix of ["", "/en"]) {
    const page = await browser.newPage();
    await page.goto(`${base}${prefix}/releases/`, { waitUntil: "domcontentloaded" });
    assert.equal(await page.locator(".log-entry").count(), 6);
    assert.equal(await page.locator(".log-entry.current").count(), 1, "exactly the latest release is current");
    assert.equal(await page.locator(".log-entry.current .version-line code").textContent(), "v1.0.4");
    const links = await page.locator(".entry-body h2 a").evaluateAll((items) => items.map((item) => item.getAttribute("href")));
    assert.deepEqual(links, versions.map((version) => `${prefix}/releases/${version}/`));
    for (let index = 0; index < links.length; index += 1) {
      await page.goto(base + links[index], { waitUntil: "domcontentloaded" });
      const eyebrow = await page.locator(".release-eyebrow").textContent();
      assert.match(eyebrow, new RegExp(`^v${versions[index].replaceAll(".", "\\.")}`));
      assert(eyebrow.includes(index === 0 ? (prefix ? "Current" : "当前") : (prefix ? "Historical" : "历史")), `${links[index]} has the wrong release channel`);
      assert.equal(await page.locator('.product-nav a[aria-current="page"]').getAttribute("href"), `${prefix}/releases/`);
    }
    await page.close();
  }
});

test("each immutable release detail has exact date, acquisition, metadata, and pagination contracts", async () => {
  const page = await browser.newPage();
  for (let index = 0; index < versions.length; index += 1) {
    const version = versions[index];
    await page.goto(`${base}/releases/${version}/`, { waitUntil: "domcontentloaded" });
    assert.equal(await page.locator(".release-facts time").getAttribute("datetime"), dates[index], version);
    assert.equal(await page.locator('script[type="application/ld+json"]').count(), 1, version);
    const schema = JSON.parse(await page.locator('script[type="application/ld+json"]').textContent());
    assert.equal(schema.softwareVersion, version);
    assert.equal(schema.datePublished, dates[index]);
    const metadata = page.locator('link[rel="alternate"][type="application/json"]');
    assert.equal(await metadata.count(), index < 4 ? 1 : 0, version);
    if (index < 4) assert.equal(await metadata.getAttribute("href"), `${origin}/releases/${version}.json`);
    if (version === "0.2.0") {
      assert.equal(await page.locator(".download-table").count(), 0);
      assert.match(await page.locator(".release-article").textContent(), /Rust 1\.88|Rust 1.88/);
      assert(schema.downloadUrl === undefined);
    } else {
      assert.equal(await page.locator(".download-table a[download]").count(), 6, version);
      assert.equal(schema.downloadUrl.length, 6, version);
    }
  }
  await page.close();
});

test("current release acquisition and machine metadata agree exactly", async () => {
  const page = await browser.newPage();
  await page.goto(base + "/releases/1.0.4/", { waitUntil: "domcontentloaded" });
  const hrefs = await page.locator(".download-table a[download]").evaluateAll((links) => links.map((link) => link.href));
  const root = "https://github.com/kleedaisuki/prompt-squish/releases/download/v1.0.4/";
  assert.deepEqual([...hrefs].sort(), expectedAssets.map((asset) => root + asset).sort());
  assert.equal(await page.locator('.resource-button[href$="SHA256SUMS"]').getAttribute("href"), root + "SHA256SUMS");
  const metadata = JSON.parse(await readFile(join(dist, "releases/1.0.4.json"), "utf8"));
  assert.deepEqual([metadata.release.version, metadata.release.tag, metadata.release.date], ["1.0.4", "v1.0.4", "2026-09-20"]);
  assert.equal(metadata.cli.machineProtocol, "3.1");
  assert.deepEqual(metadata.artifacts.map((artifact) => artifact.url).sort(), [...hrefs].sort());
  await page.close();
});

test("sticky release navigation never overlaps its external resources", async () => {
  const page = await browser.newPage({ viewport: { width: 1440, height: 700 } });
  await page.goto(base + "/releases/1.0.0/", { waitUntil: "domcontentloaded" });
  await page.locator("#acquisition").scrollIntoViewIfNeeded();
  const overlapArea = await page.evaluate(() => {
    const nav = document.querySelector(".release-story-rail nav").getBoundingClientRect();
    const resources = document.querySelector(".release-resources").getBoundingClientRect();
    return Math.max(0, Math.min(nav.right, resources.right) - Math.max(nav.left, resources.left))
      * Math.max(0, Math.min(nav.bottom, resources.bottom) - Math.max(nav.top, resources.top));
  });
  assert.equal(overlapArea, 0);
  await page.close();
});

test("manual overview and all seven chapters expose desktop and mobile local navigation", async () => {
  for (const prefix of ["", "/en"]) {
    const overview = `${prefix}/ns${prefix ? "/" : ""}`;
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    for (const [slug, path] of [[null, overview], ...chapters.map((chapter) => [chapter, `${prefix}/ns/${chapter}/`])]) {
      await page.goto(base + path, { waitUntil: "domcontentloaded" });
      assert.equal(await page.locator("[data-namespace-uri]").count(), slug === null ? 1 : 0, path);
      if (slug === null) assert.equal(await page.locator("[data-namespace-uri]").textContent(), namespaceUri);
      assert.equal(await page.locator(".chapter-rail nav a").count(), 8, path);
      assert.equal(await page.locator('.chapter-rail nav a[aria-current="page"]').count(), 1, path);
      assert.equal(await page.locator(".mobile-jump nav a").count(), 8, path);
      assert.equal(await page.locator('.mobile-jump nav a[aria-current="page"]').count(), 1, path);
      assert.equal(await page.locator(".chapter-rail nav").getAttribute("aria-label"), prefix === "" ? "手册章节" : "Manual chapters", path);
      if (slug !== null) {
        const localLabel = prefix === "" ? "本页目录" : "On this page";
        assert.equal(await page.locator(".manual-toc nav").getAttribute("aria-label"), localLabel, path);
        assert.equal(await page.locator(".mobile-toc nav").getAttribute("aria-label"), localLabel, path);
        assert.notEqual(await page.locator(".manual-toc nav").getAttribute("aria-label"), await page.locator(".chapter-rail nav").getAttribute("aria-label"), path);
      }
      assert.equal(await page.locator(".chapter-cards a").count(), slug === null ? 7 : 0, path);
      assert.equal(await page.locator("[data-directive]").count(), slug === "reference" ? 11 : 0, path);
    }
    await page.setViewportSize({ width: 390, height: 900 });
    await page.goto(base + `${prefix}/ns/reference/`, { waitUntil: "domcontentloaded" });
    assert(await page.locator(".mobile-jump").isVisible());
    assert(await page.locator(".mobile-toc").isVisible());
    assert((await page.locator(".mobile-toc nav a").count()) > 0);
    assert.equal(await page.locator(".chapter-rail").isVisible(), false);
    await page.setViewportSize({ width: 960, height: 900 });
    await page.goto(base + `${prefix}/ns/reference/`, { waitUntil: "domcontentloaded" });
    assert(await page.locator(".mobile-toc").isVisible(), "the local outline must remain available between 851px and 1080px");
    assert.equal(await page.locator(".manual-toc").isVisible(), false);
    assert(await page.locator(".chapter-rail").isVisible());
    await page.close();
  }
});

test("manual heading fragments clear the sticky header in both locales", async () => {
  const page = await browser.newPage({ viewport: { width: 1440, height: 700 } });
  for (const prefix of ["", "/en"]) {
    for (const chapter of chapters) {
      const path = `${prefix}/ns/${chapter}/`;
      await page.goto(base + path, { waitUntil: "domcontentloaded" });
      const ids = await page.locator(".manual-article h2[id]").evaluateAll((nodes) => nodes.map((node) => node.id));
      // The directive reference is card-addressed; prose chapters expose heading fragments.
      if (chapter === "reference") assert.equal(ids.length, 0, path);
      else assert(ids.length > 0, path);
      for (const id of ids) {
        await page.evaluate((hash) => { location.hash = hash; }, id);
        const [headingTop, headerBottom] = await page.evaluate((headingId) => [document.getElementById(headingId).getBoundingClientRect().top, document.querySelector(".product-header").getBoundingClientRect().bottom], id);
        assert(headingTop >= headerBottom - 1, `${path}#${id} is hidden by the header: ${headingTop}/${headerBottom}`);
      }
    }
  }
  await page.close();
});

test("getting started closes the build-to-inspect loop in both locales", async () => {
  const page = await browser.newPage();
  for (const path of ["/ns/getting-started/", "/en/ns/getting-started/"]) {
    await page.goto(base + path, { waitUntil: "domcontentloaded" });
    const text = await page.locator(".manual-article").textContent();
    for (const claim of ["prompt.xml", "xmlsquish build --target prompt", "xmlsquish inspect artifact target/xmlsquish/artifacts/prompt.prompt --format=raw", "<Prompt>Hello, xmlsquish.</Prompt>", "target/xmlsquish/artifacts/prompt.prompt"])
      assert(text.includes(claim), `${path}: ${claim}`);
  }
  await page.close();
});

test("all eleven directive entries expose deep syntax contracts", async () => {
  const page = await browser.newPage();
  await page.goto(base + "/en/ns/reference/", { waitUntil: "domcontentloaded" });
  const cards = page.locator("[data-directive]");
  assert.equal(await cards.count(), 11);
  assert.equal(await page.locator("[data-directive] > header h2").count(), 11, "every directive must participate in the level-2 heading outline");
  for (const card of await cards.all()) {
    assert.equal(await card.locator(".contract-grid section").count(), 2);
    assert.equal(await card.locator(".attribute-contract").count(), 1);
    assert.equal(await card.locator("section pre code").count(), 1);
    assert((await card.locator(".constraints li").count()) >= 2);
    assert.match(await card.locator(".related-link").getAttribute("href"), /^\/en\/ns\/.+\/$/);
  }
  const slot = page.locator("#slot .attribute-contract");
  assert.match(await slot.textContent(), /required.*optional.*default: false/s);
  const arg = await page.locator("#arg .attribute-contract").textContent();
  assert.match(arg, /name.*required/s); assert.match(arg, /value.*exclusive choice/s); assert.match(arg, /get.*exclusive choice/s);
  const ifr = await page.locator("#ifr .attribute-contract").textContent();
  assert.match(ifr, /get.*exclusive choice/s); assert.match(ifr, /str.*exclusive choice/s); assert.match(ifr, /pattern.*required/s);
  await page.close();
});

test("manual reference search, URI copy, fragments, and clipboard denial remain operable", async () => {
  const page = await browser.newPage({ viewport: { width: 390, height: 900 } });
  await page.addInitScript(() => Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async (text) => { window.copiedNamespace = text; } } }));
  await page.goto(base + "/en/ns/reference/", { waitUntil: "domcontentloaded" });
  assert.equal(await page.locator("[data-count]").textContent(), "11 / 11");
  const ids = await page.locator("[data-directive]").evaluateAll((nodes) => nodes.map((node) => node.id));
  assert.equal(new Set(ids).size, 11);
  await page.locator("[data-search]").fill("look-around");
  assert.equal(await page.locator("[data-directive]:visible").count(), 1);
  assert.equal(await page.locator("[data-directive]:visible code").first().textContent(), "xs:ifr");
  await page.locator("[data-search]").press("Escape");
  assert.equal(await page.locator("[data-directive]:visible").count(), 11);
  await page.locator('.directive-list [href="#ifr"]').click();
  assert.equal(new URL(page.url()).hash, "#ifr");
  await page.goto(base + "/en/ns/", { waitUntil: "domcontentloaded" });
  await page.locator("[data-copy]").click();
  assert.equal(await page.evaluate(() => window.copiedNamespace), namespaceUri);
  await page.close();

  const denied = await browser.newPage();
  await denied.addInitScript(() => Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async () => { throw new DOMException("denied", "NotAllowedError"); } } }));
  await denied.goto(base + "/ns", { waitUntil: "domcontentloaded" });
  await denied.locator("[data-copy]").click();
  assert.equal(await denied.evaluate(() => getSelection()?.toString()), namespaceUri);
  assert((await denied.locator("[data-copy-status]").textContent()).trim().length > 0);
  await denied.close();
});

test("manual no-JavaScript fallback and raw specifications remain complete", async () => {
  const page = await browser.newPage({ javaScriptEnabled: false });
  for (const path of ["/ns/reference/", "/en/ns/reference/"]) {
    await page.goto(base + path, { waitUntil: "domcontentloaded" });
    assert.equal(await page.locator("[data-directive]:visible").count(), 11);
    assert.equal(await page.locator("[data-search-ui]:visible, [data-copy]:visible").count(), 0);
    assert.equal(await page.locator('link[rel="describedby"]').getAttribute("href"), "/ns/dsl.md");
  }
  const current = await readFile(join(dist, "ns/dsl.md"), "utf8");
  assert.equal(current, await readFile(new URL("../../../docs/dsl.md", import.meta.url), "utf8"));
  for (const [version, digest] of [["0.3.0", "45beace4d8782c0ec4024c56bf581a7e65cc8bf9fadd97dcc3cdf2a856368945"], ["0.2.0", "2a53e352223e393e3669c5e5ef47328bee459a3a6bbfd45b1b88ce2e73015fad"]]) {
    const contents = await readFile(join(dist, `ns/${version}/dsl.md`), "utf8");
    assert.equal(createHash("sha256").update(contents.replaceAll("\r\n", "\n")).digest("hex"), digest);
  }
  await page.close();
});

test("sitemap covers exactly 32 reciprocal human routes and llms navigation separates artifacts", async () => {
  const sitemap = await readFile(join(dist, "sitemap.xml"), "utf8");
  const locations = [...sitemap.matchAll(/<loc>([^<]+)<\/loc>/g)].map((match) => match[1]);
  assert.equal(locations.length, 32);
  assert.equal(new Set(locations).size, 32);
  assert.deepEqual(new Set(locations), new Set(routes.map((route) => canonicalFor(route.path))));
  for (const [zhPath, enPath] of routePairs) {
    const zh = canonicalFor(zhPath); const en = canonicalFor(enPath);
    for (const loc of [zh, en]) {
      const block = sitemap.match(new RegExp(`<url>\\s*<loc>${loc.replaceAll(".", "\\.")}<\\/loc>[\\s\\S]*?<\\/url>`))?.[0];
      assert(block?.includes(`hreflang="zh-CN" href="${zh}"`), loc);
      assert(block?.includes(`hreflang="en" href="${en}"`), loc);
    }
  }
  assert(!sitemap.includes(".json") && !sitemap.includes(".md"));
  const llms = await readFile(join(dist, "llms.txt"), "utf8");
  for (const claim of ["English release index", "/en/releases/1.0.4/", "manual overview", "/en/ns/reference/", "/ns/dsl.md", "/ns/0.3.0/dsl.md"]) assert(llms.includes(claim), claim);
  const currentMachineRecord = llms.match(/\[Machine-readable v(\d+\.\d+\.\d+) metadata\]\([^)]*\/releases\/(\d+\.\d+\.\d+)\.json\)/);
  assert.deepEqual(currentMachineRecord?.slice(1), ["1.0.4", "1.0.4"], "llms label and current metadata URL must identify the same release");
});

test("all route links are underline-free and retain a 3px keyboard focus indicator", async () => {
  const page = await browser.newPage();
  for (const route of routes) {
    await page.goto(base + route.path, { waitUntil: "domcontentloaded" });
    const links = page.locator("a:visible");
    assert((await links.count()) > 4, route.path);
    assert.deepEqual(await links.evaluateAll((items) => [...new Set(items.map((link) => getComputedStyle(link).textDecorationLine))]), ["none"], route.path);
    for (const link of [page.locator(".product-brand"), page.locator("main a:visible").first(), page.locator(".product-footer a:visible").first()]) {
      await link.hover();
      assert.equal(await link.evaluate((node) => getComputedStyle(node).textDecorationLine), "none", route.path);
      await link.focus();
      assert.deepEqual(await link.evaluate((node) => [getComputedStyle(node).outlineStyle, getComputedStyle(node).outlineWidth]), ["solid", "3px"], route.path);
    }
  }
  await page.close();
});

test("all 32 routes reflow without document overflow at four widths in both themes", async () => {
  for (const theme of ["light", "dark"]) {
    const page = await browser.newPage();
    await page.addInitScript((value) => localStorage.setItem("xmlsquish-theme", value), theme);
    for (const width of [320, 390, 768, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      for (const route of routes) {
        await page.goto(base + route.path, { waitUntil: "domcontentloaded" });
        assert.equal(await page.evaluate(() => document.documentElement.dataset.moeTheme), theme);
        const [scrollWidth, innerWidth] = await page.evaluate(() => [document.documentElement.scrollWidth, window.innerWidth]);
        assert(scrollWidth <= innerWidth + 1, `${route.path}: ${theme} ${width}px overflow ${scrollWidth}/${innerWidth}`);
      }
    }
    await page.close();
  }
});

test("theme control remains operable when storage is blocked", async () => {
  const page = await browser.newPage({ reducedMotion: "reduce" });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(() => { Storage.prototype.getItem = () => { throw new Error("blocked storage"); }; Storage.prototype.setItem = () => { throw new Error("blocked storage"); }; });
  await page.goto(base + "/en/releases/", { waitUntil: "domcontentloaded" });
  await page.getByRole("button", { name: "Dark", exact: true }).click();
  assert.equal(await page.evaluate(() => document.documentElement.dataset.moeTheme), "dark");
  assert.deepEqual(errors, []);
  await page.close();
});
