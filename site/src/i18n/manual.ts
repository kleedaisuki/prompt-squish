import type { Locale } from "./messages";
import type { DirectiveGroup, DirectiveName, ManualChapter } from "../data/dsl";

type ChapterCopy = { nav: string; title: string; intro: string };
type ManualMessages = {
  title: string; description: string; eyebrow: string; heading: string; lead: string;
  status: string; uriLabel: string; copy: string; copied: string; denied: string;
  onThisPage: string; jump: string; filter: string; searchPlaceholder: string; empty: string;
  chapters: Record<ManualChapter, ChapterCopy>;
  notes: { exactUri: string; roots: string; imports: string; evaluation: string; values: string; control: string; limits: string; build: string; version: string };
  groups: Record<DirectiveGroup, string>;
  directives: Record<DirectiveName, { summary: string; detail: string }>;
  resourcesTitle: string;
  resources: Record<"spec" | "examples" | "v030" | "v020" | "standard" | "license", { title: string; body: string }>;
};

const en: ManualMessages = {
  title: "xmlsquish DSL user manual", description: "Learn the xmlsquish XML DSL: project roots, imports, macros, values, content slots, control flow, builds, artifacts, and all eleven directives.",
  eyebrow: "DSL user manual", heading: "Compose structured prompts with a small XML language.", lead: "Start with a working entry, split reusable behavior into modules, then build deterministic .prompt artifacts. This guide follows the tasks you perform—not the compiler internals.",
  status: "Stable, unversioned identity", uriLabel: "Exact XML namespace URI", copy: "Copy URI", copied: "URI copied", denied: "Clipboard unavailable; the URI is selected for manual copy.",
  onThisPage: "Manual chapters", jump: "Jump to a chapter", filter: "Filter directives", searchPlaceholder: "Name, attribute, or purpose…", empty: "No directives match.",
  chapters: {
    "getting-started": { nav: "Getting started", title: "Create your first entry", intro: "Bind the exact namespace URI on xs:entry. Ordinary XML becomes output; xs:* elements control composition." },
    "source-model": { nav: "Source model", title: "Separate products from libraries", intro: "An entry builds one product. A module publishes macros. Imports make module definitions visible without executing them." },
    composition: { nav: "Macros & expansion", title: "Define contracts, then expand them", intro: "Macros have namespace-qualified names and explicit parameters. Expansion resolves a fixed target and creates an isolated invocation frame." },
    "control-and-scope": { nav: "Control & scope", title: "Match strings without leaking scope", intro: "xs:ifr provides regular-expression matching and lexical named captures. Scalar and XML content remain separate throughout evaluation." },
    "build-and-artifacts": { nav: "Build & artifacts", title: "Build through a frozen source closure", intro: "The manager resolves imports, validates the whole project, lowers canonical XSIR, links symbols, and publishes inspectable artifacts." },
    reference: { nav: "Directive reference", title: "All eleven directives", intro: "Use the filter as a shortcut. Every contract remains present in the HTML and addressable by a stable fragment." },
    "limits-and-invariants": { nav: "Limits & invariants", title: "Know the boundaries that keep builds predictable", intro: "Recursive programs run within explicit budgets, while stable identity and language invariants keep projects auditable." },
  },
  notes: {
    exactUri: "Comparison is exact and case-sensitive. The address identifies the vocabulary; the build never fetches it over the network.",
    roots: "xs:module may contain imports and macros only. xs:entry has declarations followed by an output-producing body; an entry cannot be imported.",
    imports: "Relative import paths are resolved from the file that declares them. The manager loads each source identity once and freezes the import closure before expansion.",
    evaluation: "At a call, args and fills are evaluated in the caller first. The macro body then runs at its definition location with only its explicit inputs.",
    values: "Use xs:insert to emit a scalar as escaped text. It never reparses <, >, or & as markup. Use xs:fill/xs:slot when structure must remain XML.",
    control: "A failed match emits nothing. A successful match exposes match.* captures inside the xs:ifr body only; sibling blocks cannot observe them.",
    limits: "Recursive expansion is legal, but not free: depth, steps, nodes, bytes, and regular-expression work are bounded. Treat a budget error as a program design signal.",
    build: "Typical outputs are the final .prompt plus optional .xsir intermediate representation and .psdbg provenance/debug evidence.",
    version: "Never append a version or trailing slash to the namespace URI. Use snapshots only when auditing the contract implemented by an older release.",
  },
  groups: { project: "Project structure", composition: "Definitions and calls", values: "Values and XML content", control: "Control" },
  directives: {
    entry: { summary: "Build root for one output document.", detail: "Accepts imports, optional required parameters, then the construction body." }, module: { summary: "Reusable macro library.", detail: "Contains imports and named macros; it has no executable body." }, import: { summary: "Load definitions from a module.", detail: "src is resolved relative to the declaring source; importing produces no output." }, macro: { summary: "Define an immutable named macro.", detail: "name is a namespace-qualified QName and cannot be redefined." }, param: { summary: "Declare a required string input.", detail: "Declarations precede the body; every call must provide each parameter exactly once." }, expand: { summary: "Invoke a statically resolved macro.", detail: "ref is a QName fixed before evaluation; calls receive only explicit args and fills." }, arg: { summary: "Provide one immutable string.", detail: "Choose exactly one of value, get, or a text-only evaluated body." }, fill: { summary: "Provide an XML node sequence.", detail: "Its body is evaluated by the caller, frozen, then supplied to the matching slot." }, slot: { summary: "Insert caller-provided XML content.", detail: "A required slot must be filled; an optional unfilled slot emits nothing." }, insert: { summary: "Emit one scalar as escaped text.", detail: "get reads a binding; its characters are never parsed as XML markup." }, ifr: { summary: "Match a string and conditionally emit.", detail: "Takes get or str plus pattern; named captures are lexical match.* bindings." },
  },
  resourcesTitle: "Specifications, examples, and lineage",
  resources: { spec: { title: "Complete DSL specification", body: "Formal semantics and implementation boundaries in Markdown." }, examples: { title: "Runnable projects", body: "Small projects exercised by the current manager." }, v030: { title: "v0.3.0 snapshot", body: "Immutable historical language contract." }, v020: { title: "v0.2.0 snapshot", body: "Earlier immutable namespace contract." }, standard: { title: "XML Namespaces", body: "The W3C standard behind namespace identity." }, license: { title: "Project license", body: "GPL-3.0-or-later terms shipped with xmlsquish." } },
};

const zh: ManualMessages = {
  title: "xmlsquish DSL 用户手册", description: "学习 xmlsquish XML DSL：项目根、导入、宏、值、内容槽、控制流、构建、产物与全部十一条指令。",
  eyebrow: "DSL 用户手册", heading: "用一门小型 XML 语言组合结构化提示词。", lead: "先写一个可工作的 entry，再把复用逻辑拆进 module，最终构建确定性的 .prompt 产物。本手册按用户任务组织，而不是照搬编译器内部设计。",
  status: "稳定、无版本号的身份", uriLabel: "精确 XML 命名空间 URI", copy: "复制 URI", copied: "URI 已复制", denied: "无法访问剪贴板；URI 已选中，请手动复制。",
  onThisPage: "手册章节", jump: "跳转到章节", filter: "筛选指令", searchPlaceholder: "名称、属性或用途…", empty: "没有匹配的指令。",
  chapters: {
    "getting-started": { nav: "快速开始", title: "创建第一个 entry", intro: "在 xs:entry 上绑定精确命名空间 URI。普通 XML 成为输出，xs:* 元素控制组合过程。" },
    "source-model": { nav: "源码模型", title: "把产品与库分开", intro: "entry 构建一个产品；module 发布宏；import 只让模块定义可见，并不会执行模块。" },
    composition: { nav: "宏、参数与展开", title: "先定义契约，再执行展开", intro: "宏拥有命名空间限定名称与显式参数。展开解析固定目标，并创建隔离的调用帧。" },
    "control-and-scope": { nav: "控制与作用域", title: "匹配字符串而不泄漏作用域", intro: "xs:ifr 提供正则匹配与词法命名捕获；标量与 XML 内容在整个求值过程中保持分离。" },
    "build-and-artifacts": { nav: "构建与产物", title: "基于冻结的源码闭包构建", intro: "管理器解析导入、验证完整项目、降低为规范 XSIR、链接符号并发布可检查产物。" },
    reference: { nav: "指令参考", title: "全部十一条指令", intro: "筛选框只是快捷入口；每条契约都完整存在于 HTML 中，并拥有稳定片段链接。" },
    "limits-and-invariants": { nav: "限制与不变量", title: "理解让构建保持可预测的边界", intro: "递归程序在显式预算内运行，稳定身份与语言不变量则让项目保持可审计。" },
  },
  notes: {
    exactUri: "比较严格区分大小写。该地址只标识词汇；构建过程绝不会通过网络获取它。", roots: "xs:module 只能包含 import 与 macro。xs:entry 先声明、后构造输出；entry 不能被导入。", imports: "相对导入路径以声明它的文件为基准。管理器按源码身份只装载一次，并在展开前冻结导入闭包。", evaluation: "调用时先在调用方求值 arg 与 fill；随后宏体在定义位置运行，只能看到显式输入。", values: "用 xs:insert 把标量作为转义文本输出；它绝不会把 <、> 或 & 重新解析成标记。需要保留结构时使用 xs:fill/xs:slot。", control: "匹配失败不产生输出；成功时只在 xs:ifr 正文内暴露 match.* 捕获，兄弟区块不可见。", limits: "递归展开合法但并非无限：深度、步数、节点、字节与正则工作量都有预算。预算错误应被视为程序设计信号。", build: "典型输出是最终 .prompt，以及可选的 .xsir 中间表示和 .psdbg 溯源/调试证据。", version: "命名空间 URI 后不要追加版本号或末尾斜线。只有审计旧版本实现的语言契约时才使用快照。",
  },
  groups: { project: "项目结构", composition: "定义与调用", values: "值与 XML 内容", control: "控制" },
  directives: {
    entry: { summary: "一个输出文档的构建根。", detail: "先接受导入与可选必需参数，再进入构造正文。" }, module: { summary: "可复用宏库。", detail: "只包含导入与具名宏，没有可执行正文。" }, import: { summary: "从模块装载定义。", detail: "src 相对声明源码解析；导入本身不产生输出。" }, macro: { summary: "定义不可变的具名宏。", detail: "name 是命名空间限定 QName，不可重定义。" }, param: { summary: "声明必需字符串输入。", detail: "声明必须位于正文前；调用必须恰好提供每个参数一次。" }, expand: { summary: "调用静态解析的宏。", detail: "ref 是求值前固定的 QName；调用只接收显式 arg 与 fill。" }, arg: { summary: "提供一个不可变字符串。", detail: "value、get、纯文本求值 body 三种形式恰选其一。" }, fill: { summary: "提供 XML 节点序列。", detail: "正文由调用方求值、冻结，再交给对应 slot。" }, slot: { summary: "插入调用方提供的 XML 内容。", detail: "required slot 必须被填充；未填充的可选 slot 不产生输出。" }, insert: { summary: "把一个标量输出为转义文本。", detail: "get 读取 binding；字符永远不会被解析为 XML 标记。" }, ifr: { summary: "匹配字符串并条件输出。", detail: "使用 get 或 str 加 pattern；命名捕获是词法 match.* binding。" },
  },
  resourcesTitle: "规范、示例与沿革",
  resources: { spec: { title: "完整 DSL 规范", body: "Markdown 形式语义与实现边界。" }, examples: { title: "可运行项目", body: "由当前管理器实际运行的小型项目。" }, v030: { title: "v0.3.0 快照", body: "不可变历史语言契约。" }, v020: { title: "v0.2.0 快照", body: "更早的不可变命名空间契约。" }, standard: { title: "XML 命名空间", body: "命名空间身份依据的 W3C 标准。" }, license: { title: "项目许可证", body: "xmlsquish 随附的 GPL-3.0-or-later 条款。" } },
};

/** Complete, monolingual manual translations. / 完整且单语的手册翻译。 */
export const manualMessages: Record<Locale, ManualMessages> = { "zh-CN": zh, en };
