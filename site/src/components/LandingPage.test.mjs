/** Browser regression for the landing page / 首页浏览器回归；只在临时端口提供构建产物。 */
import assert from "node:assert/strict";
import { before, after, test } from "node:test";
import { createServer } from "node:http";
import { createHash } from "node:crypto";
import { access, mkdir, readFile, stat } from "node:fs/promises";
import { dirname, extname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../dist");
const demo = JSON.parse(
  await readFile(
    new URL("../data/build-demo.json", import.meta.url),
    "utf8",
  ),
);
const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".css": "text/css",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".json": "application/json",
  ".woff2": "font/woff2",
};
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(
      new URL(request.url, "http://localhost").pathname,
    );
    let file = resolve(root, `.${pathname}`);
    assert(file === root || file.startsWith(root + sep));
    if ((await stat(file)).isDirectory()) file = join(file, "index.html");
    response.writeHead(200, {
      "Content-Type": types[extname(file)] ?? "application/octet-stream",
    });
    response.end(await readFile(file));
  } catch {
    response.writeHead(404).end("Not found");
  }
});
let browser;
let base;
before(async () => {
  await access(join(root, "index.html"));
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  base = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  await new Promise((resolve) => server.close(resolve));
});

for (const locale of ["zh", "en"]) {
  test(`${locale}: tabs, explicit arguments, exact copy, and keyboard navigation`, async () => {
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: {
          writeText: async (text) => {
            window.copiedCode = text;
          },
        },
      });
    });
    await page.goto(base + (locale === "en" ? "/en/" : "/"), {
      waitUntil: "networkidle",
    });
    await page.waitForSelector("[data-build-explorer][data-ready]");
    assert.equal(await page.locator("[data-code-panel]:visible").count(), 1);
    await page.locator('[data-example-tab="source"]').focus();
    await page.keyboard.press("End");
    assert.equal(
      await page
        .locator('[data-example-tab="output"]')
        .getAttribute("aria-selected"),
      "true",
    );
    assert.equal(
      await page
        .locator("[data-code-panel]:visible")
        .getAttribute("data-source"),
      demo.scenarios.parent.output,
    );
    await page.locator('[data-mode="self"]').click();
    assert.equal(
      await page
        .locator("[data-code-panel]:visible")
        .getAttribute("data-stage"),
      "output",
    );
    assert.equal(
      await page
        .locator("[data-code-panel]:visible")
        .getAttribute("data-source"),
      demo.scenarios.self.output,
    );
    await page.locator("[data-copy]").click();
    assert.equal(
      await page.evaluate(() => window.copiedCode),
      demo.scenarios.self.output,
    );
    await page.locator('[data-example-tab="output"]').focus();
    await page.keyboard.press("Home");
    await page.keyboard.press("ArrowRight");
    assert.equal(
      await page
        .locator("[data-code-panel]:visible")
        .getAttribute("data-stage"),
      "persona",
    );
    await page.locator('[data-example-tab="intermediate"]').click();
    await page.locator("[data-copy]").click();
    assert.equal(
      await page.evaluate(() => window.copiedCode),
      demo.scenarios.self.intermediate,
    );
    await page.locator('[data-mode="parent"]').click();
    assert.equal(
      await page
        .locator("[data-code-panel]:visible")
        .getAttribute("data-source"),
      demo.scenarios.parent.intermediate,
    );
    const anchors = await page
      .locator('a[href^="#"]')
      .evaluateAll((links) => links.map((link) => link.getAttribute("href")));
    for (const anchor of anchors)
      assert.equal(await page.locator(anchor).count(), 1, anchor);
    assert.deepEqual(errors, []);
    await page.close();
  });
}

for (const locale of ["zh", "en"]) {
  for (const width of [320, 390, 768, 1440]) {
    for (const theme of ["light", "dark"]) {
      test(`${locale}: ${width}px ${theme}, no document overflow`, async () => {
        const page = await browser.newPage({
          viewport: { width, height: 900 },
        });
        await page.addInitScript(
          (theme) => localStorage.setItem("xmlsquish-theme", theme),
          theme,
        );
        await page.goto(base + (locale === "en" ? "/en/" : "/"), {
          waitUntil: "networkidle",
        });
        await page.waitForSelector("[data-build-explorer][data-ready]");
        assert.equal(
          await page.evaluate(() => document.documentElement.dataset.moeTheme),
          theme,
        );
        assert(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
          `overflow at ${width}`,
        );
        assert(
          await page.evaluate(
            () =>
              getComputedStyle(document.documentElement)
                .getPropertyValue("--moe-accent-strong")
                .trim().length > 0,
          ),
          "theme stylesheet failed to load",
        );
        await page.locator('[data-example-tab="output"]').click();
        assert(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        );
        if (process.env.UI_SCREENSHOT_DIR && [390, 1440].includes(width)) {
          await mkdir(process.env.UI_SCREENSHOT_DIR, { recursive: true });
          await page.locator('[data-example-tab="source"]').click();
          await page.evaluate(() => scrollTo(0, 0));
          await page.screenshot({
            path: join(
              process.env.UI_SCREENSHOT_DIR,
              `${locale}-${width}-${theme}.png`,
            ),
            fullPage: true,
          });
          await page.screenshot({
            path: join(
              process.env.UI_SCREENSHOT_DIR,
              `${locale}-${width}-${theme}-hero.png`,
            ),
          });
        }
        await page.close();
      });
    }
  }
}

test("without JavaScript, all recorded sources and outputs remain readable", async () => {
  const page = await browser.newPage({ javaScriptEnabled: false });
  await page.goto(base, { waitUntil: "networkidle" });
  assert.equal(await page.locator("[data-code-panel]:visible").count(), 10);
  assert.equal(await page.locator("[data-copy]:visible").count(), 0);
  assert.equal(await page.locator("[data-mode]:visible").count(), 0);
  assert(await page.locator("[data-fallback]").isVisible());
  await page.close();
});

/** Localized reference and product pages preserve language, route and product identity.
 * 本地化文档与产品页保持语言、对应路由和产品身份。 */
for (const [path, locale, kind] of [
  ["/ns/", "zh-CN", "namespace"], ["/en/ns/", "en", "namespace"],
  ["/releases/", "zh-CN", "releases"], ["/en/releases/", "en", "releases"],
]) {
  for (const width of [390, 1440]) {
    test(path + ": localized metadata and layout at " + width, async () => {
      const page = await browser.newPage({ viewport: { width, height: 1000 }, javaScriptEnabled: false });
      const response = await page.goto(base + path, { waitUntil: "networkidle" });
      assert.equal(response.status(), 200);
      assert.equal(await page.locator("html").getAttribute("lang"), locale);
      assert.equal(await page.locator("main h1").count(), 1);
      const canonical = path === "/ns/" ? "/ns" : path;
      assert.equal(await page.locator('link[rel="canonical"]').getAttribute("href"), "https://xmlsquish.moesegfault.dev" + canonical);
      const zhPath = kind === "namespace" ? "/ns" : "/releases/";
      const enPath = kind === "namespace" ? "/en/ns/" : "/en/releases/";
      assert.equal(await page.locator('link[hreflang="zh-CN"]').getAttribute("href"), "https://xmlsquish.moesegfault.dev" + zhPath);
      assert.equal(await page.locator('link[hreflang="en"]').getAttribute("href"), "https://xmlsquish.moesegfault.dev" + enPath);
      assert.equal(await page.locator('link[hreflang="x-default"]').getAttribute("href"), "https://xmlsquish.moesegfault.dev" + zhPath);
      assert.equal(await page.locator(".language-link").getAttribute("href"), locale === "en" ? zhPath : enPath);
      assert.equal(await page.locator('meta[property="og:locale"]').getAttribute("content"), locale === "en" ? "en_US" : "zh_CN");
      const text = await page.locator("main").textContent();
      assert(text.includes("0.2.0"));
      if (locale === "en") assert(!/\p{Script=Han}/u.test(text), "English page contains untranslated Chinese copy");
      else assert(/\p{Script=Han}/u.test(text), "Chinese page is missing localized copy");
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
      if (kind === "namespace") {
        assert.equal(await page.locator("tbody tr").count(), 10);
        assert.equal(await page.locator('link[rel="describedby"]').getAttribute("href"), "/ns/dsl.md");
        assert.equal(await page.locator(".identity code").textContent(), "https://xmlsquish.moesegfault.dev/ns");
      } else {
        assert.equal(await page.locator(".release-highlights article").count(), 3);
        assert.equal(await page.locator(".release-details").getAttribute("open"), null);
        assert.equal(await page.locator(".release-intro .primary").getAttribute("href"), "#install");
        assert((await page.locator("[data-install-command]").textContent()).includes("--tag v0.2.0 --locked"));
      }
      if (process.env.UI_SCREENSHOT_DIR) {
        await mkdir(process.env.UI_SCREENSHOT_DIR, { recursive: true });
        await page.screenshot({ path: join(process.env.UI_SCREENSHOT_DIR, locale + "-" + kind + "-" + width + ".png"), fullPage: true });
      }
      await page.locator(".language-link").click();
      assert.equal(await page.locator("html").getAttribute("lang"), locale === "en" ? "zh-CN" : "en");
      assert.equal(new URL(page.url()).pathname.replace(/\/$/, ""), (locale === "en" ? zhPath : enPath).replace(/\/$/, ""));
      await page.close();
    });
  }
}

for (const locale of ["zh-CN", "en"]) {
  test(locale + ": release command copy and progressive details", async () => {
    const page = await browser.newPage({ viewport: { width: 390, height: 1000 } });
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async text => { window.copiedCommand = text; } } });
    });
    await page.goto(base + (locale === "en" ? "/en/releases/" : "/releases/"), { waitUntil: "networkidle" });
    await page.locator("[data-install-copy]").click();
    assert.equal(await page.evaluate(() => window.copiedCommand), await page.locator("[data-install-command]").textContent());
    assert.equal(await page.locator("[data-copy-status]").textContent(), locale === "en" ? "Copied" : "已复制");
    await page.locator(".release-details summary").click();
    assert(await page.locator(".release-detail-body").isVisible());
    await page.getByRole("button", { name: locale === "en" ? "Dark" : "深色", exact: true }).click();
    assert.equal(await page.evaluate(() => document.documentElement.dataset.moeTheme), "dark");
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
    if (process.env.UI_SCREENSHOT_DIR) {
      await mkdir(process.env.UI_SCREENSHOT_DIR, { recursive: true });
      await page.screenshot({ path: join(process.env.UI_SCREENSHOT_DIR, locale + "-releases-390-dark-expanded.png"), fullPage: true });
    }
    await page.goto(base + (locale === "en" ? "/en/ns/" : "/ns/"), { waitUntil: "networkidle" });
    assert.equal(await page.evaluate(() => document.documentElement.dataset.moeTheme), "dark");
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
    if (process.env.UI_SCREENSHOT_DIR) await page.screenshot({ path: join(process.env.UI_SCREENSHOT_DIR, locale + "-namespace-390-dark.png"), fullPage: true });
    await page.close();
  });
}

test("current namespace specification follows the working source", async () => {
  const published = await readFile(join(root, "ns/dsl.md"), "utf8");
  const source = await readFile(new URL("../../../docs/dsl.md", import.meta.url), "utf8");
  assert.equal(published, source);
  assert(published.includes("xs:expand"));
});

test("published namespace specification is a byte-exact 0.2.0 snapshot", async () => {
  const published = await readFile(join(root, "ns/0.2.0/dsl.md"), "utf8");
  // Pin the release snapshot, not the evolving working specification.
  // 固定已发布快照，不与持续演进的工作规范比较。
  assert.equal(createHash("sha256").update(published.replaceAll("\r\n", "\n")).digest("hex"),
    "2a53e352223e393e3669c5e5ef47328bee459a3a6bbfd45b1b88ce2e73015fad");
  assert(published.includes("https://xmlsquish.moesegfault.dev/ns"));
});


test("theme controls work with blocked storage and reduced motion", async () => {
  const page = await browser.newPage({ reducedMotion: "reduce" });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(() => {
    Storage.prototype.getItem = () => {
      throw new Error("blocked storage");
    };
    Storage.prototype.setItem = () => {
      throw new Error("blocked storage");
    };
  });
  await page.goto(base + "/en/", { waitUntil: "networkidle" });
  await page.getByRole("button", { name: "Dark", exact: true }).click();
  assert.equal(
    await page.evaluate(() => document.documentElement.dataset.moeTheme),
    "dark",
  );
  assert.equal(
    await page
      .locator(".action")
      .first()
      .evaluate((element) => getComputedStyle(element).transitionDuration),
    "0s",
  );
  await page.locator(".engine-details summary").click();
  assert(await page.locator(".engine-details h3").isVisible());
  assert.deepEqual(errors, []);
  await page.close();
});
