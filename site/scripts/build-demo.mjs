/** Rebuild the site showcase with the actual CLI / 用真实 CLI 重建网站示例。 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const script = fileURLToPath(import.meta.url);
const root = resolve(dirname(script), "../..");
const fixtureDirectory = join(root, "examples/site-demo");
const artifact = join(root, "site/src/data/build-demo.json");
const check = process.argv.slice(2).includes("--check");
assert(
  process.argv.slice(2).every((arg) => arg === "--check"),
  "Usage: node site/scripts/build-demo.mjs [--check]",
);
const lf = (text) => text.replace(/\r\n?/g, "\n");
const sources = Object.fromEntries(
  await Promise.all(
    ["agent.xml", "persona.xml", "tasks.xml"].map(async (name) => [
      name,
      lf(await readFile(join(fixtureDirectory, name), "utf8")),
    ]),
  ),
);

/** Normalize display-only source locations / 仅归一化展示用源码位置，避免机器路径泄漏。 */
function normalizePaths(text, directory) {
  return text.replaceAll(pathToFileURL(directory).href, "file:///examples/site-demo")
    .replaceAll(directory.replaceAll("\\", "/"), "examples/site-demo")
    .replaceAll(directory, "examples/site-demo");
}

/** Run one real CLI stage and reject diagnostics / 执行真实 CLI 阶段并拒绝意外诊断。 */
function invoke(directory, stage) {
  const result = spawnSync(
    "cargo",
    [
      "run",
      "--quiet",
      "--locked",
      "-p",
      "xmlsquish",
      "--",
      "--color",
      "never",
      stage,
      join(directory, "agent.xml"),
    ],
    {
      cwd: root,
      encoding: "utf8",
      windowsHide: true,
      maxBuffer: 4 * 1024 * 1024,
    },
  );
  if (result.error) throw result.error;
  assert.equal(
    result.status,
    0,
    `CLI ${stage} failed:\n${result.stderr}\n${result.stdout}`,
  );
  assert.equal(
    result.stderr,
    "",
    `Unexpected CLI diagnostic: ${result.stderr}`,
  );
  return lf(result.stdout);
}

/** Read an asserted integer metric from the CLI report / 从 CLI 报表读取已验证的数值指标。 */
function number(report, pattern) {
  const match = report.match(pattern);
  assert(match, `Missing CLI metric ${pattern}:\n${report}`);
  return Number(match[1]);
}

/** Compile explicit argument variants and verify product semantics / 编译显式参数变体并验证产物语义。 */
async function scenario(temporary, mode) {
  const directory = join(temporary, mode);
  await mkdir(directory);
  const source = sources["agent.xml"].replace(
    'value="researchers"',
    `value="${mode === "parent" ? "researchers" : "everyone"}"`,
  );
  for (const [name, text] of Object.entries({
    ...sources,
    "agent.xml": source,
  })) {
    await writeFile(join(directory, name), text, "utf8");
  }
  const irReport = invoke(directory, "-I");
  const intermediate = normalizePaths(lf(
    await readFile(join(directory, "agent.i.xml"), "utf8"),
  ), directory);
  const report = invoke(directory, "-O");
  const output = lf(await readFile(join(directory, "agent.o.xml"), "utf8"));
  await assert.rejects(readFile(join(directory, "agent.i.xml")), {
    code: "ENOENT",
  });
  assert.equal(await readFile(join(directory, "agent.xml"), "utf8"), source);
  assert(irReport.includes("Optimization: not run (-I)"));
  assert(report.includes("Succeeded: 1") && report.includes("Failed: 0"));

  const metrics = {
    sourceTokens: number(report, /^Primary source\s+(\d+)\s+\d+$/m),
    irBytes: Buffer.byteLength(intermediate, "utf8"),
    finalTokens: number(report, /^Final prompt tokens: (\d+)$/m),
    finalBytes: number(report, /^Final prompt UTF-8 bytes: (\d+)$/m),
    dependencyLoads: number(report, /^Dependency loads: (\d+)$/m),
    uniqueDeps: number(report, /^Unique dependency files: (\d+)$/m),
  };
  assert.equal(metrics.finalBytes, Buffer.byteLength(output, "utf8"));
  assert.equal(metrics.dependencyLoads, 2);
  assert.equal(metrics.uniqueDeps, 2);

  // Check semantics, not a simulated browser transformation / 验证真实编译语义。
  const audience = mode === "parent" ? "researchers" : "everyone";
  assert(output.includes(audience));
  assert(output.includes("clear &amp; kind"));
  assert(output.includes("Explain the trade-offs."));
  assert(!output.includes("\n"), "Final product compacts XML whitespace");
  assert(!output.includes("<xs:") && !output.includes("<?xmlsquish"));
  assert.notEqual(intermediate, output, "IR must retain provenance absent from output");
  assert(intermediate.includes("file:///examples/site-demo/persona.xml"));
  assert(intermediate.includes("frame="), "IR must retain invocation identity");
  assert(!intermediate.includes(temporary), "Display IR must not leak temporary paths");
  assert(!output.includes("urn:xmlsquish:provenance"));
  return { source, intermediate, output, metrics };
}

/** Record an actual failing invocation with its source chain / 记录真实失败调用及源码链。 */
async function diagnosticExample(temporary) {
  const directory = join(temporary, "diagnostic");
  await mkdir(directory);
  await writeFile(
    join(directory, "agent.xml"),
    '<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><prompt>\n  <xs:mount src="broken.xml"/>\n</prompt></xs:module>\n',
    "utf8",
  );
  await writeFile(
    join(directory, "broken.xml"),
    '<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><role>\n  <xs:insert get="arg.voice"/>\n</role></xs:module>\n',
    "utf8",
  );
  const result = spawnSync(
    "cargo",
    [
      "run",
      "--manifest-path",
      join(root, "Cargo.toml"),
      "--quiet",
      "--locked",
      "-p",
      "xmlsquish",
      "--",
      "--color",
      "never",
      "-I",
      "agent.xml",
    ],
    {
      cwd: directory,
      encoding: "utf8",
      windowsHide: true,
      maxBuffer: 4 * 1024 * 1024,
    },
  );
  if (result.error) throw result.error;
  assert.equal(
    result.status,
    1,
    `Expected compilation failure:\n${result.stderr}\n${result.stdout}`,
  );
  const diagnostic = normalizePaths(lf(result.stderr).trimEnd(), directory);
  assert(diagnostic.includes("error[compile]"));
  assert(diagnostic.includes("voice"));
  assert(diagnostic.includes("broken.xml"));
  assert(diagnostic.includes("agent.xml"));
  assert(!diagnostic.includes(temporary) && !diagnostic.includes(root));
  assert(!diagnostic.includes("\x1b"));
  return diagnostic;
}

const temporary = await mkdtemp(join(tmpdir(), "xmlsquish-site-demo-"));
try {
  const scenarios = {};
  for (const mode of ["parent", "self"])
    scenarios[mode] = await scenario(temporary, mode);
  const data = {
    files: {
      "persona.xml": sources["persona.xml"],
      "tasks.xml": sources["tasks.xml"],
    },
    scenarios,
    diagnostic: await diagnosticExample(temporary),
  };
  const generated = `${JSON.stringify(data, null, 2)}\n`;
  if (check) {
    assert.equal(
      lf(await readFile(artifact, "utf8")),
      generated,
      "Site demo drifted. Run node site/scripts/build-demo.mjs to rebuild it from the current locked CLI.",
    );
  } else {
    await writeFile(artifact, generated, "utf8");
  }
  // No machine paths or timestamps in the artifact / 产物不含机器路径和时间戳。
  const provenance = createHash("sha256")
    .update(lf(await readFile(script, "utf8")))
    .update(JSON.stringify(sources))
    .update(generated)
    .digest("hex");
  console.log(
    `${check ? "Verified" : "Generated"} site/src/data/build-demo.json`,
  );
  console.log(`Fixture + generator + result SHA-256: ${provenance}`);
  console.log(
    "Provenance: current xmlsquish via cargo run --quiet --locked; temporary LF-normalized source copies.",
  );
  for (const [mode, value] of Object.entries(scenarios))
    console.log(`${mode}: ${JSON.stringify(value.metrics)}`);
} finally {
  // Delete only the exact directory created above / 仅删除本次创建的临时目录。
  const child = relative(resolve(tmpdir()), resolve(temporary));
  assert(
    child.startsWith("xmlsquish-site-demo-") && !child.includes(sep),
    "Unsafe temporary cleanup path",
  );
  await rm(temporary, { recursive: true, force: true });
}
