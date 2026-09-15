/** Build and inspect the live project-manager showcase. / 构建并检查当前项目管理器演示。 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
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

/** Run the workspace CLI with project-local manager state. / 使用项目内管理器状态运行工作区 CLI。 */
function cli(project, args, quiet = false) {
  const storage = join(project, ".manager-state").replaceAll("\\", "/");
  const result = spawnSync("cargo", ["run", "--quiet", "--locked", "-p", "xmlsquish", "--", "--config", `manager.storage-root='${storage}'`, "--color", "never", "--plain", ...(quiet ? ["--quiet"] : []), ...args], {
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

/** Return a stable digest summary instead of pretending binary artifacts are text. / 返回稳定摘要，不把二进制产物伪装成文本。 */
async function binarySummary(label, base, paths) {
  const lines = [];
  for (const path of paths.sort()) {
    const bytes = await readFile(path);
    const physical = relative(base, path).replaceAll("\\", "/");
    const logical = physical.split("/artifacts/").at(-1);
    lines.push(`${logical}  ${bytes.length} bytes  sha256:${createHash("sha256").update(bytes).digest("hex").slice(0, 16)}`);
  }
  return `${label}\n${lines.join("\n")}`;
}

/** Walk only the generated publication tree. / 仅遍历本次生成的发布目录。 */
async function filesBelow(directory) {
  const found = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) found.push(...await filesBelow(path));
    else found.push(path);
  }
  return found;
}

/** Build an explicit-argument variant through manifest discovery, then inspect its products. / 经清单发现构建显式参数变体并检查产物。 */
async function scenario(mode) {
  const project = join(temporary, mode);
  await mkdir(project, { recursive: true });
  const audience = mode === "parent" ? "researchers" : "everyone";
  const entry = sources["agent.xml"].replace('value="researchers"', `value="${audience}"`);
  for (const [name, text] of Object.entries({ ...sources, "agent.xml": entry })) await writeFile(join(project, name), text, "utf8");
  const manifest = join(project, "xmlsquish.toml");
  const report = cli(project, ["--message-format", "short", "build", "--manifest-path", manifest, "--emit", "prompt", "--emit", "ir", "--emit", "debug"]);
  for (const kind of ["compile", "link", "instantiate", "backend", "publish"]) assert(report.includes(`ok:${kind}`), `Missing ${kind} evidence:\n${report}`);

  const publication = join(project, "target/xmlsquish");
  const currentFiles = (await filesBelow(join(publication, ".squish-publish/targets"))).filter((path) => path.endsWith("current.json"));
  assert.equal(currentFiles.length, 1, "Build must publish exactly one selected target generation");
  const generation = JSON.parse(await readFile(currentFiles[0], "utf8"));
  const artifactOf = (kind) => generation.artifacts.find((item) => item.kind.type === kind);
  const promptRecord = artifactOf("prompt");
  const debugRecord = artifactOf("debug_info");
  assert(promptRecord && debugRecord);
  const promptPath = join(publication, promptRecord.uri);
  const debugPath = join(publication, debugRecord.uri);
  const prompt = lf(await readFile(promptPath, "utf8"));
  const expected = `<prompt> <Persona> <audience> ${audience} </audience> <voice> clear &amp; kind </voice> </Persona> <task> Explain the trade-offs. </task> </prompt>`;
  assert.equal(prompt, expected);
  const irPaths = generation.artifacts
    .filter((item) => item.kind.type === "binary_ir")
    .map((item) => join(publication, item.uri));
  assert.equal(irPaths.length, 3, "Every project XML module must publish one reusable XSIR companion");
  await readFile(debugPath);

  // Inspect uses the manager catalog rather than opening arbitrary bytes directly.
  // inspect 通过管理器 catalog 验证，而不是直接信任任意字节。
  const irRecord = generation.artifacts.find((item) => item.kind.type === "binary_ir");
  assert(irRecord);
  const inspectedIr = cli(project, ["inspect", "--manifest-path", manifest, "ir", irRecord.id, "--format", "json"], true);
  const inspectedLink = cli(project, ["inspect", "--manifest-path", manifest, "link", "agent", "--format", "json"], true);
  assert.doesNotThrow(() => JSON.parse(inspectedIr));
  assert.doesNotThrow(() => JSON.parse(inspectedLink));
  const stages = {
    manifest: sources["xmlsquish.toml"], source: entry,
    xsir: await binarySummary("XSIR/1 canonical binary module set", publication, irPaths),
    link: `target site-demo:agent\ncompile ${irPaths.length} modules → link → instantiate → backend → publish`,
    prompt,
    debug: await binarySummary("PSDBG/1 self-contained debug companion", publication, [debugPath]),
  };
  return { stages, metrics: { actions: [...report.matchAll(/^ok:/gm)].length, modules: irPaths.length, finalBytes: Buffer.byteLength(prompt) } };
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
    diagnostic: '{"version":{"major":2,"minor":1},"sequence":0,"payload":{"type":"planning_started"}}\n{"version":{"major":2,"minor":1},"sequence":1,"payload":{"type":"action_succeeded","data":{"kind":"compile","cache":"persistent"}}}\n{"version":{"major":2,"minor":1},"sequence":2,"payload":{"type":"job_finished","data":{"status":"success"}}}',
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
