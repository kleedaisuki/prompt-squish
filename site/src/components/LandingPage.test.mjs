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
        .locator('[data-example-tab="debug"]')
        .getAttribute("aria-selected"),
      "true",
    );
    assert.equal(
      await page
        .locator("[data-code-panel]:visible")
        .getAttribute("data-source"),
      demo.scenarios.parent.stages.debug,
    );
    await page.locator('[data-example-tab="prompt"]').click();
    await page.locator('[data-mode="self"]').click();
    assert.equal(
      await page
        .locator("[data-code-panel]:visible")
        .getAttribute("data-stage"),
      "prompt",
    );
    assert.equal(
      await page
        .locator("[data-code-panel]:visible")
        .getAttribute("data-source"),
      demo.scenarios.self.stages.prompt,
    );
    await page.locator("[data-copy]").click();
    assert.equal(
      await page.evaluate(() => window.copiedCode),
      demo.scenarios.self.stages.prompt,
    );
    await page.locator('[data-example-tab="prompt"]').focus();
    await page.keyboard.press("Home");
    await page.keyboard.press("ArrowRight");
    assert.equal(
      await page
        .locator("[data-code-panel]:visible")
        .getAttribute("data-stage"),
      "source",
    );
    await page.locator('[data-example-tab="xsir"]').click();
    await page.locator("[data-copy]").click();
    assert.equal(
      await page.evaluate(() => window.copiedCode),
      demo.scenarios.self.stages.xsir,
    );
    await page.locator('[data-mode="parent"]').click();
    assert.equal(
      await page
        .locator("[data-code-panel]:visible")
        .getAttribute("data-source"),
      demo.scenarios.parent.stages.xsir,
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
  test(`${locale}: project creation flow and new/init boundary stay visible`, async () => {
    const page = await browser.newPage();
    await page.goto(base + (locale === "en" ? "/en/" : "/"), {
      waitUntil: "networkidle",
    });
    const commands = await page
      .locator(".command-terminal pre code")
      .allTextContents();
    assert(commands.some((command) => command.includes("xmlsquish new hello-prompts")));
    assert(commands.some((command) => command.includes("xmlsquish fmt --check")));
    assert(commands.some((command) => command.includes("xmlsquish build --offline")));
    const note = await page.locator(".cli-copy .small-note").textContent();
    assert(note.includes(locale === "en" ? "absent destination" : "目标尚不存在"));
    assert(note.includes("init"));
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
        await page.locator('[data-example-tab="prompt"]').click();
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
  assert.equal(
    await page.locator("[data-code-panel]:visible").count(),
    demo.artifacts.length * 2,
  );
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
      if (locale === "en") assert(!/\p{Script=Han}/u.test(text), "English page contains untranslated Chinese copy");
      else assert(/\p{Script=Han}/u.test(text), "Chinese page is missing localized copy");
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
      if (kind === "namespace") {
        assert(text.includes("0.3.0"));
        assert.equal(await page.locator("tbody tr").count(), 11);
        assert.equal(await page.locator('link[rel="describedby"]').getAttribute("href"), "/ns/dsl.md");
        assert.equal(await page.locator(".identity code").textContent(), "https://xmlsquish.moesegfault.dev/ns");
      } else {
        const commands = ["new", "fmt", "build", "add", "remove", "inspect"];
        assert(text.includes("1.0.0"));
        assert(!text.includes("0.3.0"), "v1 release page must not present the old release");
        assert(!text.includes(".o.xml"), "v1 release page must not preserve the old single-file artifact story");
        const journey = await page.locator(".command-journey > li > code").allTextContents();
        assert.equal(journey.length, 6);
        assert.deepEqual(
          journey.map(invocation => invocation.trim().split(/\s+/)[1]).sort(),
          [...commands].sort(),
        );
        assert.equal(await page.locator('.actions .primary[href="#install"]').count(), 1);

        // Candidate assets are labels, not deceptive links. / 候选资产只是标签，不伪装成下载链接。
        const expectedAssets = [
          "xmlsquish-1.0.0-x86_64-pc-windows-msvc.zip",
          "xmlsquish-1.0.0-aarch64-pc-windows-msvc.zip",
          "xmlsquish-1.0.0-x86_64-unknown-linux-gnu.tar.gz",
          "xmlsquish-1.0.0-aarch64-unknown-linux-gnu.tar.gz",
          "xmlsquish-1.0.0-x86_64-apple-darwin.tar.gz",
          "xmlsquish-1.0.0-aarch64-apple-darwin.tar.gz",
        ];
        const pending = page.locator('.download-actions [aria-disabled="true"]');
        assert.equal(await pending.count(), 6);
        assert.deepEqual(await pending.evaluateAll(nodes => nodes.map(node => node.title)), expectedAssets);
        assert.equal(await page.locator('a[href*="/releases/download/v1.0.0/"]').count(), 0);
        assert.equal(await page.locator('a[href$="SHA256SUMS"]').count(), 0);

        const candidateInstall = "cargo install --git https://github.com/kleedaisuki/prompt-squish --rev 2eb5834b15d47717a7b45092a3b72bfa475f4c79 --locked";
        const taggedInstall = "cargo install --git https://github.com/kleedaisuki/prompt-squish --tag v1.0.0 --locked";
        assert.deepEqual(
          await page.locator(".install-commands pre code").allTextContents(),
          [candidateInstall, taggedInstall],
        );
        assert(text.includes(locale === "en"
          ? "create and push the v1.0.0 tag first, then run the release workflow"
          : "先创建并推送 v1.0.0 标签，再运行验证既有标签并发布资产的 release workflow"));

        assert.deepEqual(
          (await page.locator(".quickstart pre code").textContent()).split("\n"),
          [
            "xmlsquish new support --vcs=none",
            "cd support",
            "xmlsquish fmt --check",
            "xmlsquish build --offline",
            "xmlsquish inspect artifact target/xmlsquish/prompt.prompt --format=json",
          ],
        );

        assert.equal(await page.locator('link[rel="alternate"][type="application/json"]').getAttribute("href"),
          "https://xmlsquish.moesegfault.dev/releases/1.0.0.json");
        assert.equal(await page.locator('meta[property="og:image"]').getAttribute("content"),
          "https://xmlsquish.moesegfault.dev/release-v1.png");
        const jsonLd = await page.locator('script[type="application/ld+json"]').allTextContents();
        assert.equal(jsonLd.length, 1, "release page has one authoritative SoftwareApplication entity");
        const application = JSON.parse(jsonLd[0]);
        assert.equal(application["@type"], "SoftwareApplication");
        assert.equal(application.softwareVersion, "1.0.0");
        assert.equal(application.creativeWorkStatus, "release candidate");
        assert.equal(application.sameAs, "https://github.com/kleedaisuki/prompt-squish");
        for (const dishonestClaim of ["downloadUrl", "aggregateRating", "rating", "review", "offers", "codeRepository", "additionalProperty"])
          assert.equal(dishonestClaim in application, false, dishonestClaim);
        assert.equal(await page.locator("[itemscope], [itemprop]").count(), 0,
          "release page must not duplicate JSON-LD with partial microdata");
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
  for (const width of [390, 1440]) {
    test(`${locale}: v1 release remains readable in light and dark at ${width}px`, async () => {
      const page = await browser.newPage({ viewport: { width, height: 1000 }, javaScriptEnabled: false });
      const path = locale === "en" ? "/en/releases/" : "/releases/";
      await page.goto(base + path, { waitUntil: "networkidle" });
      assert(await page.locator("main").isVisible());
      assert.equal(await page.locator(".command-journey > li").count(), 6);
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
      await page.close();

      for (const theme of ["light", "dark"]) {
        const themed = await browser.newPage({ viewport: { width, height: 1000 } });
        await themed.addInitScript(theme => localStorage.setItem("xmlsquish-theme", theme), theme);
        await themed.goto(base + path, { waitUntil: "networkidle" });
        assert.equal(await themed.evaluate(() => document.documentElement.dataset.moeTheme), theme);
        assert(await themed.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
        await themed.close();
      }
    });
  }
}

test("release discovery files expose prepared, machine-readable v1 state", async () => {
  const metadata = JSON.parse(await readFile(join(root, "releases/1.0.0.json"), "utf8"));
  assert.equal(metadata.schemaVersion, 1);
  assert.equal(metadata.release.version, "1.0.0");
  assert.equal(metadata.release.tag, "v1.0.0");
  assert.equal(metadata.release.releaseStatus, "prepared");
  assert.deepEqual(metadata.cli.commands, ["new", "fmt", "build", "add", "remove", "inspect"]);
  assert.equal(metadata.artifacts.length, 6);
  assert(metadata.artifacts.every(asset => asset.intendedUrl.includes("/releases/download/v1.0.0/")));

  const robots = await readFile(join(root, "robots.txt"), "utf8");
  assert.match(robots, /^User-agent: \*$/m);
  assert.match(robots, /^Allow: \/$/m);
  assert.match(robots, /^Sitemap: https:\/\/xmlsquish\.moesegfault\.dev\/sitemap\.xml$/m);

  const sitemap = await readFile(join(root, "sitemap.xml"), "utf8");
  assert(sitemap.includes("https://xmlsquish.moesegfault.dev/releases/"));
  assert(sitemap.includes("https://xmlsquish.moesegfault.dev/en/releases/"));
  assert.equal((sitemap.match(/<loc>https:\/\/xmlsquish\.moesegfault\.dev\/(?:en\/)?releases\/<\/loc>/g) ?? []).length, 2);

  const llms = await readFile(join(root, "llms.txt"), "utf8");
  assert(llms.includes("/releases/1.0.0.json"));
  assert(llms.includes("releaseStatus"));
  assert(llms.includes("Prepared asset URLs may not exist"));
  for (const command of metadata.cli.commands) assert(llms.includes(`\`${command}\``));
});

test("current namespace specification follows the working source", async () => {
  const published = await readFile(join(root, "ns/dsl.md"), "utf8");
  const source = await readFile(new URL("../../../docs/dsl.md", import.meta.url), "utf8");
  assert.equal(published, source);
  assert(published.includes("xs:expand"));
});

test("published namespace specification is a byte-exact 0.3.0 snapshot", async () => {
  const published = await readFile(join(root, "ns/0.3.0/dsl.md"), "utf8");
  // Pin the immutable release contract independently of the working specification.
  // 独立固定不可变发布契约，不随工作规范变动。
  assert.equal(createHash("sha256").update(published.replaceAll("\r\n", "\n")).digest("hex"),
    "45beace4d8782c0ec4024c56bf581a7e65cc8bf9fadd97dcc3cdf2a856368945");
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
