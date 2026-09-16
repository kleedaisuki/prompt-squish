/** Product-site browser contracts / 产品站浏览器合同。 */
import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { access, readFile, stat } from "node:fs/promises";
import { dirname, extname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const dist = resolve(dirname(fileURLToPath(import.meta.url)), "../../dist");
const demo = JSON.parse(await readFile(new URL("../data/build-demo.json", import.meta.url), "utf8"));
const types = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".png": "image/png", ".json": "application/json", ".woff2": "font/woff2" };
const routes = [
  { path: "/", locale: "zh-CN", page: "home", peer: "/en/" },
  { path: "/en/", locale: "en", page: "home", peer: "/" },
  { path: "/releases/", locale: "zh-CN", page: "releases", peer: "/en/releases/" },
  { path: "/en/releases/", locale: "en", page: "releases", peer: "/releases/" },
  { path: "/ns/", canonicalPath: "/ns", locale: "zh-CN", page: "namespace", peer: "/en/ns/" },
  { path: "/en/ns/", locale: "en", page: "namespace", peer: "/ns" },
];
const expectedAssets = [
  "xmlsquish-1.0.1-x86_64-pc-windows-msvc.zip", "xmlsquish-1.0.1-aarch64-pc-windows-msvc.zip",
  "xmlsquish-1.0.1-x86_64-unknown-linux-gnu.tar.gz", "xmlsquish-1.0.1-aarch64-unknown-linux-gnu.tar.gz",
  "xmlsquish-1.0.1-x86_64-apple-darwin.tar.gz", "xmlsquish-1.0.1-aarch64-apple-darwin.tar.gz",
];
const namespaceUri = "https://xmlsquish.moesegfault.dev/ns";
const directiveGroups = [["module", "entry"], ["import", "macro", "param", "expand"], ["arg", "fill", "slot", "insert"], ["ifr"]];
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

/** Normalize route spelling while preserving root / 规范化路由拼写并保留根路径。 */
function normalizedPath(url) {
  const pathname = new URL(url, "https://example.invalid").pathname;
  return pathname === "/" ? pathname : pathname.replace(/\/$/, "");
}

test("all six human routes share one localized product shell", async () => {
  const page = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 390, height: 900 } });
  for (const route of routes) {
    const response = await page.goto(base + route.path, { waitUntil: "networkidle" });
    assert.equal(response.status(), 200, route.path);
    assert.equal(await page.locator("html").getAttribute("lang"), route.locale);
    assert.equal(await page.locator(".product-header, .product-footer").count(), 2);
    assert.equal(await page.locator("main#main").count(), 1);
    assert.equal(await page.locator("main h1").count(), 1);
    assert.equal(await page.locator(".skip-link").getAttribute("href"), "#main");
    assert.equal(await page.locator(".product-nav a").count(), 3);
    const current = page.locator('.product-nav a[aria-current="page"]');
    assert.equal(await current.count(), 1);
    assert.equal(normalizedPath(await current.getAttribute("href")), normalizedPath(route.path));
    assert(await current.evaluate((link) => {
      const style = getComputedStyle(link);
      return style.backgroundColor !== "rgba(0, 0, 0, 0)" || style.boxShadow !== "none";
    }), `${route.path}: active route needs a persistent non-color cue`);
    assert.equal(await page.locator('link[rel="canonical"]').getAttribute("href"), "https://xmlsquish.moesegfault.dev" + (route.canonicalPath ?? route.path));
    assert.equal(normalizedPath(await page.locator(".language-link").getAttribute("href")), normalizedPath(route.peer));
    assert.equal(await page.locator('meta[property="og:locale"]').getAttribute("content"), route.locale === "en" ? "en_US" : "zh_CN");
    const text = await page.locator("main").textContent();
    if (route.locale === "en") assert(!/\p{Script=Han}/u.test(text), `${route.path}: untranslated Chinese copy`);
    else assert(/\p{Script=Han}/u.test(text), `${route.path}: missing Chinese copy`);
    for (const selector of [".product-brand", ".language-link", ".product-nav"]) assert(await page.locator(selector).isVisible(), `${route.path}: ${selector}`);
  }
  await page.close();
});

test("language switch preserves the current destination", async () => {
  const page = await browser.newPage();
  for (const route of routes) {
    await page.goto(base + route.path, { waitUntil: "networkidle" });
    await page.locator(".language-link").click();
    assert.equal(normalizedPath(page.url()), normalizedPath(route.peer), route.path);
    assert.equal(await page.locator("html").getAttribute("lang"), route.locale === "en" ? "zh-CN" : "en");
  }
  await page.close();
});

for (const locale of ["zh-CN", "en"]) {
  test(`${locale}: home preserves section order and build explorer behavior`, async () => {
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.addInitScript(() => Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async (text) => { window.copiedCode = text; } } }));
    await page.goto(base + (locale === "en" ? "/en/" : "/"), { waitUntil: "networkidle" });
    await page.waitForSelector("[data-build-explorer][data-ready]");
    assert.deepEqual(await page.locator("main > section").evaluateAll((sections) => sections.map((section) =>
      section.id || (section.querySelector(".capabilities") ? "capabilities" : [...section.classList].find((name) => ["hero", "journey-band", "boundaries", "closing"].includes(name))),
    )), ["hero", "how", "build", "model", "capabilities", "cli", "stats", "boundaries", "closing"]);
    assert.match((await page.locator(".hero-release-link").textContent()).replace(/\s+/g, " "), /v1\.0\.1/);
    assert.equal(await page.locator("[data-code-panel]:visible").count(), 1);
    await page.locator('[data-example-tab="source"]').focus();
    await page.keyboard.press("End");
    assert.equal(await page.locator('[data-example-tab="debug"]').getAttribute("aria-selected"), "true");
    await page.locator('[data-example-tab="prompt"]').click();
    await page.locator('[data-mode="self"]').click();
    assert.equal(await page.locator("[data-code-panel]:visible").getAttribute("data-source"), demo.scenarios.self.stages.prompt);
    await page.locator("[data-copy]").click();
    assert.equal(await page.evaluate(() => window.copiedCode), demo.scenarios.self.stages.prompt);
    await page.locator('[data-mode="parent"]').click();
    assert.equal(await page.locator("[data-code-panel]:visible").getAttribute("data-source"), demo.scenarios.parent.stages.prompt);
    const commands = await page.locator(".command-terminal pre code").allTextContents();
    for (const command of ["xmlsquish new hello-prompts", "xmlsquish fmt --check", "xmlsquish build --offline"])
      assert(commands.some((candidate) => candidate.includes(command)), command);
    assert.equal(JSON.parse(await page.locator('script[type="application/ld+json"]').textContent()).softwareVersion, "1.0.1");
    assert.deepEqual(errors, []);
    await page.close();
  });
}

test("home no-JavaScript fallback keeps every recorded source and output readable", async () => {
  const page = await browser.newPage({ javaScriptEnabled: false });
  await page.goto(base, { waitUntil: "networkidle" });
  assert.equal(await page.locator("[data-code-panel]:visible").count(), demo.artifacts.length * 2);
  assert.equal(await page.locator("[data-copy]:visible").count(), 0);
  assert.equal(await page.locator("[data-mode]:visible").count(), 0);
  assert(await page.locator("[data-fallback]").isVisible());
  await page.close();
});

for (const path of ["/releases/", "/en/releases/"]) {
  test(`${path}: v1.0.1 release, migration, downloads and history agree`, async () => {
    const page = await browser.newPage();
    await page.goto(base + path, { waitUntil: "networkidle" });
    const text = await page.locator("main").textContent();
    for (const token of ["v1.0.1", "3.0", "2.1"]) assert(text.includes(token), token);
    assert.equal(await page.locator(".release-hero time").getAttribute("datetime"), "2026-09-16");
    const downloadBase = "https://github.com/kleedaisuki/prompt-squish/releases/download/v1.0.1/";
    const assets = await page.locator(".download-actions a[download]").evaluateAll((links) => links.map((link) => link.href));
    assert.deepEqual([...assets].sort(), expectedAssets.map((asset) => downloadBase + asset).sort());
    assert.equal(await page.locator('a[href$="SHA256SUMS"]').getAttribute("href"), downloadBase + "SHA256SUMS");
    assert(text.includes("cargo install --git https://github.com/kleedaisuki/prompt-squish --tag v1.0.1 --locked"));
    assert.equal(await page.locator("#v1-0-0").count(), 1);
    assert.equal(await page.locator('#v1-0-0 a[href="/releases/1.0.0.json"]').count(), 1);
    assert.equal(await page.locator('link[rel="alternate"][type="application/json"]').getAttribute("href"), "https://xmlsquish.moesegfault.dev/releases/1.0.1.json");
    const jsonLd = await page.locator('script[type="application/ld+json"]').allTextContents();
    assert.equal(jsonLd.length, 1);
    const application = JSON.parse(jsonLd[0]);
    assert.equal(application["@type"], "SoftwareApplication");
    assert.equal(application.softwareVersion, "1.0.1");
    assert.equal(application.datePublished, "2026-09-16");
    assert.equal(application.creativeWorkStatus, "Published");
    assert.deepEqual([...application.downloadUrl].sort(), [...assets].sort());
    assert.equal(await page.locator("[itemscope], [itemprop]").count(), 0);
    await page.close();
  });
}

test("release discovery artifacts expose one machine-readable v1.0.1 contract", async () => {
  const metadata = JSON.parse(await readFile(join(dist, "releases/1.0.1.json"), "utf8"));
  assert.equal(metadata.schemaVersion, 1);
  assert.deepEqual([metadata.release.version, metadata.release.tag, metadata.release.date, metadata.release.releaseStatus], ["1.0.1", "v1.0.1", "2026-09-16", "published"]);
  assert.equal(metadata.release.githubRelease, "https://github.com/kleedaisuki/prompt-squish/releases/tag/v1.0.1");
  assert.equal(metadata.release.checksums, "https://github.com/kleedaisuki/prompt-squish/releases/download/v1.0.1/SHA256SUMS");
  assert.equal(metadata.cli.machineProtocol, "3.0");
  assert.equal(metadata.cli.sourceInstall, "cargo install --git https://github.com/kleedaisuki/prompt-squish --tag v1.0.1 --locked");
  assert.equal(metadata.artifacts.length, 6);
  assert.deepEqual(metadata.artifacts.map((artifact) => artifact.url.split("/").at(-1)).sort(), [...expectedAssets].sort());
  assert(metadata.artifacts.every((artifact) => artifact.url.includes("/releases/download/v1.0.1/")));
  assert.equal(JSON.parse(await readFile(join(dist, "releases/1.0.0.json"), "utf8")).release.version, "1.0.0");
  const robots = await readFile(join(dist, "robots.txt"), "utf8");
  assert.match(robots, /^User-agent: \*$/m);
  assert.match(robots, /^Sitemap: https:\/\/xmlsquish\.moesegfault\.dev\/sitemap\.xml$/m);
  const sitemap = await readFile(join(dist, "sitemap.xml"), "utf8");
  for (const route of routes) assert(sitemap.includes("https://xmlsquish.moesegfault.dev" + (route.canonicalPath ?? route.path)));
  const llms = await readFile(join(dist, "llms.txt"), "utf8");
  for (const claim of ["/releases/1.0.1.json", "published v1.0.1 release", "protocol 3.0", "v1.0.0"]) assert(llms.includes(claim), claim);
});

for (const path of ["/ns/", "/en/ns/"]) {
  test(`${path}: namespace exposes eleven unique directives, resources, filtering and copy`, async () => {
    const page = await browser.newPage();
    await page.addInitScript(() => Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async (text) => { window.copiedNamespace = text; } } }));
    await page.goto(base + path, { waitUntil: "networkidle" });
    assert.equal(await page.locator("[data-namespace-uri]").textContent(), namespaceUri);
    assert.equal(await page.locator("[data-group]").count(), 4);
    assert.deepEqual(await page.locator("[data-group]").evaluateAll((groups) => groups.map((group) =>
      [...group.querySelectorAll("[data-directive] > code")].map((code) => code.textContent.replace("xs:", "")),
    )), directiveGroups);
    const directives = await page.locator("[data-directive]").evaluateAll((cards) => cards.map((card) => card.id));
    assert.equal(directives.length, 11);
    assert.equal(new Set(directives).size, 11);
    for (const directive of directives) assert.equal(await page.locator(`[data-directive]#${directive} .directive-anchor[href="#${directive}"]`).count(), 1);
    const resources = await page.locator(".resource-card").evaluateAll((links) => links.map((link) => link.getAttribute("href")));
    for (const href of ["/ns/dsl.md", "https://github.com/kleedaisuki/prompt-squish/tree/main/examples", path.startsWith("/en/") ? "/en/releases/" : "/releases/", "/ns/0.3.0/dsl.md", "/ns/0.2.0/dsl.md", "https://www.w3.org/TR/REC-xml-names/", "/LICENSE.txt"])
      assert(resources.includes(href), href);
    assert.equal(await page.locator('link[rel="describedby"]').getAttribute("href"), "/ns/dsl.md");
    assert(await page.locator("[data-search-ui]").isVisible());
    assert.equal(await page.locator("[data-count]").textContent(), "11 / 11");
    await page.locator("[data-search]").fill("ifr");
    assert.equal(await page.locator("[data-directive]:visible").count(), 1);
    assert.equal(await page.locator("[data-directive]:visible > code").textContent(), "xs:ifr");
    assert.equal(await page.locator("[data-count]").textContent(), "1 / 11");
    await page.locator("[data-search]").press("Escape");
    assert.equal(await page.locator("[data-directive]:visible").count(), 11);
    await page.locator("[data-copy]").click();
    assert.equal(await page.evaluate(() => window.copiedNamespace), namespaceUri);
    assert((await page.locator("[data-copy-status]").textContent()).trim().length > 0);
    await page.close();
  });
}

test("namespace fragment and clipboard-denial fallbacks remain operable", async () => {
  const page = await browser.newPage({ viewport: { width: 390, height: 900 } });
  await page.addInitScript(() => Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: async () => { throw new DOMException("denied", "NotAllowedError"); } },
  }));
  await page.goto(base + "/en/ns/", { waitUntil: "networkidle" });
  await page.locator('[href="#ifr"]').click();
  assert.equal(new URL(page.url()).hash, "#ifr");
  const positions = await page.evaluate(() => ({
    card: document.querySelector("#ifr").getBoundingClientRect().top,
    header: document.querySelector(".product-header").getBoundingClientRect().bottom,
  }));
  assert(positions.card >= positions.header, `fragment hidden by sticky header: ${JSON.stringify(positions)}`);
  await page.locator("[data-copy]").click();
  assert.equal(await page.evaluate(() => getSelection()?.toString()), namespaceUri);
  assert((await page.locator("[data-copy-status]").textContent()).trim().length > 0);
  await page.close();
});

test("release primary CTA keeps filled contrast on hover", async () => {
  const page = await browser.newPage();
  await page.goto(base + "/en/releases/", { waitUntil: "networkidle" });
  const button = page.locator(".actions .button.primary");
  const resting = await button.evaluate((node) => [getComputedStyle(node).color, getComputedStyle(node).backgroundColor]);
  await button.hover();
  const hovered = await button.evaluate((node) => [getComputedStyle(node).color, getComputedStyle(node).backgroundColor]);
  assert.equal(hovered[0], resting[0], "hover must retain the on-accent foreground");
  assert.notEqual(hovered[1], "rgba(0, 0, 0, 0)");
  assert.notEqual(hovered[1], resting[1], "hover must retain a distinct filled state");
  await page.close();
});

test("namespace progressive enhancement leaves complete content without JavaScript", async () => {
  const page = await browser.newPage({ javaScriptEnabled: false });
  for (const path of ["/ns/", "/en/ns/"]) {
    await page.goto(base + path, { waitUntil: "networkidle" });
    assert.equal(await page.locator("[data-namespace-uri]").textContent(), namespaceUri);
    assert.equal(await page.locator("[data-directive]:visible").count(), 11);
    assert.equal(await page.locator("[data-search-ui]:visible, [data-copy]:visible").count(), 0);
    assert((await page.locator(".use-layout pre code").textContent()).includes(`xmlns:xs="${namespaceUri}"`));
  }
  await page.close();
});

test("all product links remain underline-free at rest, hover and keyboard focus", async () => {
  const page = await browser.newPage();
  for (const route of routes) {
    await page.goto(base + route.path, { waitUntil: "networkidle" });
    const links = page.locator("a:visible");
    assert((await links.count()) > 5);
    assert.deepEqual(await links.evaluateAll((items) => [...new Set(items.map((link) => getComputedStyle(link).textDecorationLine))]), ["none"], `${route.path}: resting underline`);
    for (const selector of [".product-nav a", "main a", ".product-footer a"]) {
      const link = page.locator(`${selector}:visible`).first();
      await link.hover();
      assert.equal(await link.evaluate((node) => getComputedStyle(node).textDecorationLine), "none", `${route.path}: hover ${selector}`);
      await link.focus();
      assert.equal(await link.evaluate((node) => getComputedStyle(node).textDecorationLine), "none", `${route.path}: focus ${selector}`);
      assert.deepEqual(await link.evaluate((node) => [getComputedStyle(node).outlineStyle, getComputedStyle(node).outlineWidth]), ["solid", "3px"], `${route.path}: focus ${selector}`);
    }
  }
  await page.close();
});

test("every route reflows without page overflow in light and dark themes", async () => {
  for (const theme of ["light", "dark"]) {
    const page = await browser.newPage();
    await page.addInitScript((value) => localStorage.setItem("xmlsquish-theme", value), theme);
    for (const width of [320, 390, 768, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      for (const route of routes) {
        await page.goto(base + route.path, { waitUntil: "networkidle" });
        assert.equal(await page.evaluate(() => document.documentElement.dataset.moeTheme), theme);
        const dimensions = await page.evaluate(() => [document.documentElement.scrollWidth, innerWidth]);
        assert(dimensions[0] <= dimensions[1] + 1, `${route.path}: ${theme} ${width}px overflow ${dimensions.join("/")}`);
      }
    }
    await page.close();
  }
});

test("home hero stays two-column on desktop and reflows on mobile", async () => {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(base, { waitUntil: "networkidle" });
  assert.equal((await page.locator(".hero").evaluate((node) => getComputedStyle(node).gridTemplateColumns.split(" ").length)), 2);
  await page.setViewportSize({ width: 390, height: 900 });
  assert.equal((await page.locator(".hero").evaluate((node) => getComputedStyle(node).gridTemplateColumns.split(" ").length)), 1);
  await page.close();
});

test("current and historical namespace specifications retain their contracts", async () => {
  const current = await readFile(join(dist, "ns/dsl.md"), "utf8");
  assert.equal(current, await readFile(new URL("../../../docs/dsl.md", import.meta.url), "utf8"));
  assert(current.includes("xs:expand"));
  const snapshots = [
    ["0.3.0", "45beace4d8782c0ec4024c56bf581a7e65cc8bf9fadd97dcc3cdf2a856368945"],
    ["0.2.0", "2a53e352223e393e3669c5e5ef47328bee459a3a6bbfd45b1b88ce2e73015fad"],
  ];
  for (const [version, digest] of snapshots) {
    const published = await readFile(join(dist, `ns/${version}/dsl.md`), "utf8");
    assert.equal(createHash("sha256").update(published.replaceAll("\r\n", "\n")).digest("hex"), digest);
  }
});

test("theme controls survive blocked storage and reduced motion", async () => {
  const page = await browser.newPage({ reducedMotion: "reduce" });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(() => {
    Storage.prototype.getItem = () => { throw new Error("blocked storage"); };
    Storage.prototype.setItem = () => { throw new Error("blocked storage"); };
  });
  await page.goto(base + "/en/", { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Dark", exact: true }).click();
  assert.equal(await page.evaluate(() => document.documentElement.dataset.moeTheme), "dark");
  assert.equal(await page.locator(".action").first().evaluate((element) => getComputedStyle(element).transitionDuration), "0s");
  await page.locator(".engine-details summary").click();
  assert(await page.locator(".engine-details h3").isVisible());
  assert.deepEqual(errors, []);
  await page.close();
});
