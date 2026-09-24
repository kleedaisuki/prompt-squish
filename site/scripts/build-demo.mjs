/** Build and inspect the live project-manager showcase. / 构建并检查当前项目管理器演示。 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(import.meta.url);
const root = resolve(dirname(script), "../..");
const fixture = join(root, "examples/site-demo");
const artifact = join(root, "site/src/data/build-demo.json");
const temporaryRoot = join(root, ".temp");
const temporary = join(temporaryRoot, `site-demo-${process.pid}`);
const check = process.argv.slice(2).includes("--check");
assert(process.argv.slice(2).every((arg) => arg === "--check"), "Usage: node site/scripts/build-demo.mjs [--check]");
const lf = (text) => text.replace(/\r\n?/g, "\n");
const sourceNames = ["xmlsquish.toml", "agent.xml", "persona.xml", "tasks.xml"];
const sources = Object.fromEntries(await Promise.all(sourceNames.map(async (name) => [name, lf(await readFile(join(fixture, name), "utf8"))])));
const visibleArtifacts = [
  { stage: "manifest", name: "xmlsquish.toml", kind: "source", language: "toml" },
  { stage: "source", name: "agent.xml", kind: "source", language: "xml" },
  { stage: "xsir", name: "ir/agent/site-demo/*.xsir", kind: "intermediate", language: "plaintext" },
  { stage: "link", name: "linked target", kind: "intermediate", language: "plaintext" },
  { stage: "prompt", name: "agent.prompt", kind: "output", language: "xml" },
  { stage: "debug", name: "agent.psdbg", kind: "intermediate", language: "plaintext" },
];

/** Run the workspace CLI; the fixture's target-dir owns all derived state. / 运行工作区 CLI；夹具的 target-dir 拥有全部派生状态。 */
function cli(args, quiet = false) {
  const result = spawnSync("cargo", ["run", "--quiet", "--locked", "-p", "xmlsquish", "--", "--color", "never", "--plain", ...(quiet ? ["--quiet"] : []), ...args], {
    cwd: root, encoding: "utf8", windowsHide: true, maxBuffer: 8 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `CLI failed:\n${result.stderr}\n${result.stdout}`);
  if (quiet) {
    assert.equal(result.stderr, "", `Unexpected CLI diagnostic:\n${result.stderr}`);
    return lf(result.stdout);
  }
  assert.equal(result.stdout, "", `Operation events must stay on stderr:\n${result.stdout}`);
  return lf(result.stderr);
}

/** Run a build in native NDJSON mode and return its typed build result. / 以原生 NDJSON 模式运行构建并返回类型化结果。 */
function buildResult(manifest) {
  const output = cli(["--message-format", "json", "build", "--manifest-path", manifest, "--emit", "prompt", "--emit", "ir", "--emit", "debug"], true);
  const events = output.trim().split("\n").map((line) => JSON.parse(line));
  const completed = events.find((event) => event.payload?.type === "operation_completed");
  assert(completed, "Build NDJSON must contain operation_completed");
  assert.equal(completed.payload.data.result.type, "build");
  return { result: completed.payload.data.result.result, events };
}

/** Summarize typed descriptors without opening publisher-private paths. / 汇总类型化描述符，不打开发布器私有路径。 */
function descriptorSummary(label, artifacts) {
  const lines = artifacts.slice().sort((left, right) => left.locator.localeCompare(right.locator)).map((item) =>
    `${item.locator}  ${item.size} bytes  ${item.digest.algorithm.type}:${item.digest.hex.slice(0, 16)}`,
  );
  return `${label}\n${lines.join("\n")}`;
}

/** Build an explicit-argument variant through manifest discovery, then inspect its products. / 经清单发现构建显式参数变体并检查产物。 */
async function scenario(mode) {
  const project = join(temporary, mode);
  await mkdir(project, { recursive: true });
  const audience = mode === "parent" ? "researchers" : "everyone";
  const entry = sources["agent.xml"].replace('value="researchers"', `value="${audience}"`);
  for (const [name, text] of Object.entries({ ...sources, "agent.xml": entry })) await writeFile(join(project, name), text, "utf8");
  const manifest = join(project, "xmlsquish.toml");
  const report = cli(["--message-format", "short", "build", "--manifest-path", manifest, "--emit", "prompt", "--emit", "ir", "--emit", "debug"]);
  assert(report.includes("plan:ready") && report.includes("result build published=1"), `Missing build evidence:\n${report}`);

  const built = buildResult(manifest);
  const build = built.result;
  assert.equal(build.published.length, 1, "Build must publish exactly one selected target generation");
  const generation = build.published[0];
  const artifactOf = (kind) => generation.artifacts.find((item) => item.kind.type === kind);
  const promptRecord = artifactOf("prompt");
  const debugRecord = artifactOf("debug_info");
  assert(promptRecord && debugRecord);
  const expected = `<prompt> <Persona> <audience> ${audience} </audience> <voice> clear &amp; kind </voice> </Persona> <task> Explain the trade-offs. </task> </prompt>`;
  // 通过公开 typed locator 读取并验证真实 backend 字节，不推断 publisher 物理布局。
  // Read and verify real backend bytes through the public typed locator without inferring the
  // publisher's physical layout.
  const prompt = cli(["inspect", "--manifest-path", manifest, "artifact", promptRecord.locator, "--format", "raw"], true);
  assert.equal(prompt, expected);
  const irRecords = generation.artifacts.filter((item) => item.kind.type === "binary_ir");
  assert.equal(irRecords.length, 3, "Every project XML module must publish one reusable XSIR companion");
  for (const item of generation.artifacts) {
    assert(!item.locator.includes(".squish-publish"));
    const inspected = cli(["inspect", "--manifest-path", manifest, "artifact", item.locator, "--format", "json"], true);
    assert.doesNotThrow(() => JSON.parse(inspected));
  }
  const inspectedIr = cli(["inspect", "--manifest-path", manifest, "ir", irRecords[0].id, "--format", "json"], true);
  const inspectedLink = cli(["inspect", "--manifest-path", manifest, "link", "agent", "--format", "json"], true);
  assert.doesNotThrow(() => JSON.parse(inspectedIr));
  assert.doesNotThrow(() => JSON.parse(inspectedLink));
  const stages = {
    manifest: sources["xmlsquish.toml"], source: entry,
    xsir: descriptorSummary("XSIR/1 canonical binary module set", irRecords),
    link: `target site-demo:agent\ncompile ${irRecords.length} modules → link → instantiate → backend → publish`,
    prompt,
    debug: descriptorSummary("PSDBG/1 self-contained debug companion", [debugRecord]),
  };
  return { stages, metrics: { actions: built.events.filter((event) => event.payload?.type === "action_succeeded").length, modules: irRecords.length, finalBytes: Buffer.byteLength(prompt) } };
}

await mkdir(temporaryRoot, { recursive: true });
await rm(temporary, { recursive: true, force: true });
try {
  const scenarios = { parent: await scenario("parent"), self: await scenario("self") };
  const data = {
    artifacts: visibleArtifacts,
    metricStages: [
      { label: "source", value: 3, unit: "XML files", file: "xmlsquish.toml" },
      { label: "ir", value: scenarios.parent.metrics.modules, unit: "XSIR modules", file: "*.xsir" },
      { label: "final", value: 1, unit: "prompt", file: "agent.prompt" },
    ],
    files: sources, scenarios,
    diagnostic: '{"version":{"major":3,"minor":0},"sequence":0,"payload":{"type":"planning_started"}}\n{"version":{"major":3,"minor":0},"sequence":1,"payload":{"type":"action_succeeded","data":{"kind":"compile","cache":"persistent"}}}\n{"version":{"major":3,"minor":0},"sequence":2,"payload":{"type":"job_finished","data":{"status":"success"}}}',
  };
  const generated = `${JSON.stringify(data, null, 2)}\n`;
  if (check) assert.equal(lf(await readFile(artifact, "utf8")), generated, "Site demo drifted. Run npm --prefix site run demo:generate.");
  else await writeFile(artifact, generated, "utf8");
  console.log(`${check ? "Verified" : "Generated"} site/src/data/build-demo.json with build + inspect`);
} finally {
  const child = relative(temporaryRoot, temporary);
  assert(child.startsWith("site-demo-") && !child.includes(sep), "Unsafe temporary cleanup path");
  await rm(temporary, { recursive: true, force: true });
}
