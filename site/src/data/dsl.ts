/** Locale-neutral identity for one public DSL directive.
 * 一条公共 DSL 指令的语言无关身份。
 */
export type Directive = {
  readonly name: "module" | "entry" | "import" | "macro" | "param" | "expand" | "arg" | "fill" | "slot" | "insert" | "ifr";
  readonly group: "project" | "composition" | "values" | "control";
  readonly attributes: readonly string[];
};

/** The complete public vocabulary, ordered for the manual reference.
 * 完整公共词汇表，按用户手册参考章节排序。
 *
 * Each name occurs exactly once so navigation, search, and stable fragments share
 * one source of truth. / 每个名称只出现一次，使导航、搜索和稳定片段共享单一事实来源。
 */
export const directives = [
  { name: "entry", group: "project", attributes: [] },
  { name: "module", group: "project", attributes: [] },
  { name: "import", group: "project", attributes: ["src"] },
  { name: "macro", group: "composition", attributes: ["name"] },
  { name: "param", group: "composition", attributes: ["name"] },
  { name: "expand", group: "composition", attributes: ["ref"] },
  { name: "arg", group: "values", attributes: ["name", "value | get | body"] },
  { name: "fill", group: "values", attributes: ["name"] },
  { name: "slot", group: "values", attributes: ["name", "required?"] },
  { name: "insert", group: "values", attributes: ["get"] },
  { name: "ifr", group: "control", attributes: ["get | str", "pattern"] },
] as const satisfies readonly Directive[];

export type DirectiveName = (typeof directives)[number]["name"];
export type DirectiveGroup = (typeof directives)[number]["group"];

/** Stable manual chapters used by desktop and mobile navigation.
 * 桌面与移动导航共用的稳定手册章节。
 */
export const manualChapters = [
  "getting-started", "source-model", "composition", "control-and-scope", "build-and-artifacts", "reference", "limits-and-invariants",
] as const;
export type ManualChapter = (typeof manualChapters)[number];

/** Public resources whose URLs are language independent.
 * URL 与语言无关的公共资源。
 */
export const dslResources = [
  { key: "spec", href: "/ns/dsl.md", kind: "Markdown", rel: "describedby" },
  { key: "examples", href: "https://github.com/kleedaisuki/prompt-squish/tree/main/examples", kind: "GitHub" },
  { key: "v030", href: "/ns/0.3.0/dsl.md", kind: "Snapshot" },
  { key: "v020", href: "/ns/0.2.0/dsl.md", kind: "Snapshot" },
  { key: "standard", href: "https://www.w3.org/TR/REC-xml-names/", kind: "W3C" },
  { key: "license", href: "/LICENSE.txt", kind: "GPL-3.0", rel: "license" },
] as const;
