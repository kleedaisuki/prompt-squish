/** Locale-neutral identity for one public DSL directive.
 * 一条公共 DSL 指令的语言无关身份。
 */
export type Directive = {
  readonly name: "module" | "entry" | "import" | "macro" | "param" | "expand" | "arg" | "fill" | "slot" | "insert" | "ifr";
  readonly group: "project" | "composition" | "values" | "control";
  readonly attributes: readonly { readonly name: string; readonly requirement: "required" | "optional" | "exclusive"; readonly default?: string }[];
  readonly syntax: string;
  readonly relatedChapter: ManualChapter;
};

/** The complete public vocabulary, ordered for the manual reference.
 * 完整公共词汇表，按用户手册参考章节排序。
 *
 * Each name occurs exactly once so navigation, search, and stable fragments share
 * one source of truth. / 每个名称只出现一次，使导航、搜索和稳定片段共享单一事实来源。
 */
export const directives = [
  { name: "entry", group: "project", attributes: [], syntax: `<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns">\n  <Prompt>Hello</Prompt>\n</xs:entry>`, relatedChapter: "getting-started" },
  { name: "module", group: "project", attributes: [], syntax: `<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"\n           xmlns:app="urn:example:app">\n  <xs:macro name="app:hello"/>\n</xs:module>`, relatedChapter: "source-model" },
  { name: "import", group: "project", attributes: [{ name: "src", requirement: "required" }], syntax: `<xs:import src="./macros.xml"/>`, relatedChapter: "source-model" },
  { name: "macro", group: "composition", attributes: [{ name: "name", requirement: "required" }], syntax: `<xs:macro name="app:hello">\n  <Message>Hello</Message>\n</xs:macro>`, relatedChapter: "composition" },
  { name: "param", group: "composition", attributes: [{ name: "name", requirement: "required" }], syntax: `<xs:param name="title"/>`, relatedChapter: "composition" },
  { name: "expand", group: "composition", attributes: [{ name: "ref", requirement: "required" }], syntax: `<xs:expand ref="app:hello">\n  <xs:arg name="title" value="Hello"/>\n</xs:expand>`, relatedChapter: "composition" },
  { name: "arg", group: "values", attributes: [{ name: "name", requirement: "required" }, { name: "value", requirement: "exclusive" }, { name: "get", requirement: "exclusive" }], syntax: `<xs:arg name="title" get="arg.heading"/>`, relatedChapter: "composition" },
  { name: "fill", group: "values", attributes: [{ name: "name", requirement: "required" }], syntax: `<xs:fill name="body"><Message>Hello</Message></xs:fill>`, relatedChapter: "composition" },
  { name: "slot", group: "values", attributes: [{ name: "name", requirement: "required" }, { name: "required", requirement: "optional", default: "false" }], syntax: `<xs:slot name="body" required="true"/>`, relatedChapter: "composition" },
  { name: "insert", group: "values", attributes: [{ name: "get", requirement: "required" }], syntax: `<xs:insert get="arg.name"/>`, relatedChapter: "control-and-scope" },
  { name: "ifr", group: "control", attributes: [{ name: "get", requirement: "exclusive" }, { name: "str", requirement: "exclusive" }, { name: "pattern", requirement: "required" }], syntax: `<xs:ifr get="arg.value" pattern="^(?&lt;head&gt;.)$">\n  <xs:insert get="match.head"/>\n</xs:ifr>`, relatedChapter: "control-and-scope" },
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
