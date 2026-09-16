import type { Locale } from "./messages";
import type { DirectiveGroup, DirectiveName, ManualChapter } from "../data/dsl";

type ChapterCopy = { nav: string; title: string; intro: string };
type DirectiveCopy = { summary: string; detail: string; context: string; children: string; attributes: Record<string, string>; constraints: string[] };
type ManualMessages = {
  title: string; description: string; eyebrow: string; heading: string; lead: string;
  status: string; uriLabel: string; copy: string; copied: string; denied: string;
  onThisPage: string; jump: string; filter: string; searchPlaceholder: string; empty: string;
  chapters: Record<ManualChapter, ChapterCopy>;
  notes: { exactUri: string; roots: string; imports: string; evaluation: string; values: string; control: string; limits: string; build: string; version: string };
  groups: Record<DirectiveGroup, string>;
  referenceLabels: { context: string; children: string; attributes: string; constraints: string; syntax: string; related: string; required: string; optional: string; exclusive: string; default: string };
  directives: Record<DirectiveName, DirectiveCopy>;
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
  referenceLabels: { context: "Allowed in", children: "Allowed content", attributes: "Attributes", constraints: "Constraints", syntax: "Valid syntax", related: "Read the chapter", required: "required", optional: "optional", exclusive: "exclusive choice", default: "default" },
  directives: {
    entry: { summary: "Build root for one output document.", detail: "Accepts declarations, then constructs the final document.", context: "The document root of a build target; never an import target.", children: "Leading xs:import and xs:param declarations, then ordinary XML, xs:expand, xs:insert, and xs:ifr.", attributes: {}, constraints: ["Declarations must precede the construction body.", "Cannot define macros or root slots, and cannot be expanded as a macro."] },
    module: { summary: "Reusable macro library.", detail: "Registers definitions but has no executable body.", context: "The document root of an imported source unit.", children: "xs:import and xs:macro only; they may be interleaved.", attributes: {}, constraints: ["Ordinary XML, non-whitespace text, top-level xs:param, and expansion are invalid.", "A module cannot be built as an entry and has no implicit main."] },
    import: { summary: "Load definitions from a module.", detail: "Freezes another source unit into the symbol closure without executing it.", context: "Direct child of xs:module, or the declaration region of xs:entry.", children: "Empty.", attributes: { src: "Static source reference resolved relative to the declaring file." }, constraints: ["The target must be a module, never an entry.", "Produces no output, frame, arg, or fill; each source identity is loaded once."] },
    macro: { summary: "Define an immutable named macro.", detail: "Publishes one namespace-qualified symbol.", context: "Direct child of xs:module; macro definitions cannot nest.", children: "Leading xs:param declarations, then ordinary XML and composition directives.", attributes: { name: "Prefixed lexical QName resolved at the definition site." }, constraints: ["The expanded QName cannot be redefined anywhere in the frozen closure.", "The body runs with definition-site file.* and explicit call inputs only."] },
    param: { summary: "Declare a required string input.", detail: "Adds one immutable Unicode-string contract.", context: "At the start of xs:macro or the declaration region of xs:entry.", children: "Empty.", attributes: { name: "Name unique within the containing macro or entry." }, constraints: ["All parameters are required and must be supplied exactly once.", "Missing, unknown, or duplicate call arguments are hard errors."] },
    expand: { summary: "Invoke a statically resolved macro.", detail: "Evaluates explicit inputs once and returns an ordered XML node sequence.", context: "A construction body where output nodes are allowed.", children: "xs:arg and xs:fill call inputs only.", attributes: { ref: "Static prefixed QName of a macro already in the frozen symbol table." }, constraints: ["ref cannot come from a binding, concatenation, or regular-expression capture.", "A new frame does not inherit caller arg.*, match.*, or slots."] },
    arg: { summary: "Provide one immutable string.", detail: "Evaluates in the caller before the callee frame exists.", context: "Direct child of xs:expand.", children: "Empty for value/get forms; the body form may compose text-producing content.", attributes: { name: "Declared parameter name in the target macro.", value: "Literal string form.", get: "Existing scalar binding form." }, constraints: ["Choose exactly one of value, get, or body.", "A body must evaluate only to character data; whitespace is preserved and XML nodes are an error."] },
    fill: { summary: "Provide an XML node sequence.", detail: "Evaluates in the caller, freezes the sequence, then supplies a slot.", context: "Direct child of xs:expand.", children: "Any valid construction body producing an XML node sequence.", attributes: { name: "Slot name declared by the target macro." }, constraints: ["Unknown slot names and duplicate fills in one call are hard errors.", "A fill is evaluated exactly once and cannot be read as a string."] },
    slot: { summary: "Insert caller-provided XML content.", detail: "Marks a named insertion point inside one macro.", context: "Inside an xs:macro construction body.", children: "Empty.", attributes: { name: "Slot name unique within the containing macro.", required: "Boolean flag; true requires a matching fill." }, constraints: ["Missing required fills are hard errors; an unfilled optional slot emits nothing.", "Slots cannot be read through get, xs:insert, or regular expressions."] },
    insert: { summary: "Emit one scalar as escaped text.", detail: "Creates a text node without reparsing markup.", context: "A construction body or a text-only xs:arg body.", children: "Empty.", attributes: { get: "Existing file.*, arg.*, or in-scope match.* binding." }, constraints: ["A missing binding is a hard error, not an empty string.", "Characters such as <, >, and & are escaped and never interpreted as XML markup."] },
    ifr: { summary: "Match a string and conditionally emit.", detail: "Exposes lexical named captures while its body runs.", context: "A construction body, including nested macro and ifr bodies.", children: "Any valid construction body evaluated only after a successful match.", attributes: { get: "Existing scalar binding input.", str: "Literal string input.", pattern: "Unicode regular expression using the supported subset." }, constraints: ["Choose exactly one of get or str; a failed match emits nothing.", "Only named captures are exposed. Positional capture groups, look-around, and backreferences are forbidden; use (?:…) for non-capturing groups.", "match.* bindings end with the lexical block; unmatched optional captures do not exist."] },
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
  referenceLabels: { context: "允许位置", children: "允许内容", attributes: "属性", constraints: "约束", syntax: "有效语法", related: "阅读章节", required: "必需", optional: "可选", exclusive: "互斥选择", default: "默认值" },
  directives: {
    entry: { summary: "一个输出文档的构建根。", detail: "先接受声明，再构造最终文档。", context: "构建目标的文档根；绝不能作为导入目标。", children: "开头的 xs:import 与 xs:param 声明，随后是普通 XML、xs:expand、xs:insert 与 xs:ifr。", attributes: {}, constraints: ["所有声明必须位于构造正文之前。", "不能定义宏或根 slot，也不能像宏一样被展开。"] },
    module: { summary: "可复用宏库。", detail: "注册定义，但没有可执行正文。", context: "被导入 SourceUnit 的文档根。", children: "只能是 xs:import 与 xs:macro，两者可以交错。", attributes: {}, constraints: ["普通 XML、非空白文本、顶层 xs:param 与展开均非法。", "module 不能作为 entry 构建，也没有隐式 main。"] },
    import: { summary: "从模块装载定义。", detail: "把另一个 SourceUnit 冻结进符号闭包，但不执行它。", context: "xs:module 的直接子元素，或 xs:entry 的声明区。", children: "空。", attributes: { src: "相对声明文件解析的静态源码引用。" }, constraints: ["目标必须是 module，绝不能是 entry。", "不产生输出、frame、arg 或 fill；每个源码身份只装载一次。"] },
    macro: { summary: "定义不可变的具名宏。", detail: "发布一个命名空间限定符号。", context: "xs:module 的直接子元素；宏定义不能嵌套。", children: "开头的 xs:param 声明，随后是普通 XML 与组合指令。", attributes: { name: "在定义位置解析的带前缀词法 QName。" }, constraints: ["展开后的 QName 在冻结闭包中不得重定义。", "宏体只能看到定义位置 file.* 与显式调用输入。"] },
    param: { summary: "声明必需字符串输入。", detail: "添加一份不可变 Unicode 字符串契约。", context: "xs:macro 开头或 xs:entry 声明区。", children: "空。", attributes: { name: "在所属宏或 entry 中唯一的名称。" }, constraints: ["所有参数都是必需的，并且必须恰好提供一次。", "缺失、未知或重复调用参数都是 hard error。"] },
    expand: { summary: "调用静态解析的宏。", detail: "把显式输入求值一次，并返回有序 XML 节点序列。", context: "允许产生输出节点的构造正文。", children: "只能是 xs:arg 与 xs:fill 调用输入。", attributes: { ref: "冻结符号表中已有宏的静态带前缀 QName。" }, constraints: ["ref 不能来自 binding、字符串拼接或正则捕获。", "新 frame 不继承调用方 arg.*、match.* 或 slot。"] },
    arg: { summary: "提供一个不可变字符串。", detail: "在 callee frame 创建前于调用方求值。", context: "xs:expand 的直接子元素。", children: "value/get 形式为空；body 形式可以组合只产生文本的内容。", attributes: { name: "目标宏声明的参数名。", value: "字面字符串形式。", get: "已有标量 binding 形式。" }, constraints: ["value、get、body 恰选其一。", "body 必须只产生字符数据；空白保留，出现 XML 节点即报错。"] },
    fill: { summary: "提供 XML 节点序列。", detail: "在调用方求值并冻结序列，再提供给 slot。", context: "xs:expand 的直接子元素。", children: "产生 XML 节点序列的任意有效构造正文。", attributes: { name: "目标宏声明的 slot 名称。" }, constraints: ["未知 slot 名与同一次调用中的重复 fill 都是 hard error。", "fill 恰好求值一次，也不能当作字符串读取。"] },
    slot: { summary: "插入调用方提供的 XML 内容。", detail: "在一个宏内标记具名插入点。", context: "xs:macro 构造正文内部。", children: "空。", attributes: { name: "在所属宏内唯一的 slot 名称。", required: "布尔标志；true 要求存在对应 fill。" }, constraints: ["缺失 required fill 是 hard error；未填充的可选 slot 不产生输出。", "slot 不能通过 get、xs:insert 或正则读取。"] },
    insert: { summary: "把一个标量输出为转义文本。", detail: "创建文本节点，不重新解析标记。", context: "构造正文或只允许文本的 xs:arg body。", children: "空。", attributes: { get: "已有的 file.*、arg.* 或作用域内 match.* binding。" }, constraints: ["binding 不存在是 hard error，而不是空字符串。", "<、> 与 & 等字符会被转义，绝不解释为 XML 标记。"] },
    ifr: { summary: "匹配字符串并条件输出。", detail: "在正文运行期间暴露词法命名捕获。", context: "构造正文，包括嵌套的宏与 ifr 正文。", children: "只在匹配成功后求值的任意有效构造正文。", attributes: { get: "已有标量 binding 输入。", str: "字面字符串输入。", pattern: "采用受支持子集的 Unicode 正则表达式。" }, constraints: ["get 与 str 恰选其一；匹配失败不产生输出。", "只暴露命名捕获。禁止位置捕获组、look-around 与 backreference；非捕获分组使用 (?:…)。", "match.* 在词法块结束时消失；未参与匹配的可选捕获不存在。"] },
  },
  resourcesTitle: "规范、示例与沿革",
  resources: { spec: { title: "完整 DSL 规范", body: "Markdown 形式语义与实现边界。" }, examples: { title: "可运行项目", body: "由当前管理器实际运行的小型项目。" }, v030: { title: "v0.3.0 快照", body: "不可变历史语言契约。" }, v020: { title: "v0.2.0 快照", body: "更早的不可变命名空间契约。" }, standard: { title: "XML 命名空间", body: "命名空间身份依据的 W3C 标准。" }, license: { title: "项目许可证", body: "xmlsquish 随附的 GPL-3.0-or-later 条款。" } },
};

/** Complete, monolingual manual translations. / 完整且单语的手册翻译。 */
export const manualMessages: Record<Locale, ManualMessages> = { "zh-CN": zh, en };
