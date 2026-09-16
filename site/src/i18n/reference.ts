import type { Locale } from "./messages";

type LocalizedReference = {
  nav: {
    label: string;
    home: string;
    namespace: string;
    releases: string;
    license: string;
    maintained: string;
    footerLabel: string;
  };
  ns: {
    title: string;
    description: string;
    eyebrow: string;
    heading: string;
    lead: string;
    status: string;
    name: string;
    copy: string;
    copied: string;
    useTitle: string;
    useBody: string;
    vocabularyEyebrow: string;
    vocabularyTitle: string;
    vocabularyBody: string;
    groupLabels: Record<"roots" | "composition" | "content" | "control", string>;
    elements: Record<string, string>;
    pipelineEyebrow: string;
    pipelineTitle: string;
    pipelineBody: string;
    validationTitle: string;
    validationBody: string;
    resourcesEyebrow: string;
    resourcesTitle: string;
    resources: Array<{ title: string; body: string }>;
    versionTitle: string;
    versionRows: Array<{ label: string; value: string; body: string }>;
  };
  release: {
    title: string;
    description: string;
    eyebrow: string;
    date: string;
    status: string;
    heroTitle: string;
    heroAccent: string;
    heroBody: string;
    releaseActions: string;
    installCta: string;
    githubCta: string;
    impactLabel: string;
    impact: string;
    acquisitionEyebrow: string;
    acquisitionTitle: string;
    acquisitionBody: string;
    checksums: string;
    binaryNote: string;
    sourceLabel: string;
    sourceBody: string;
    changesEyebrow: string;
    changesTitle: string;
    highlights: Array<{ label: string; title: string; body: string }>;
    changeGroups: Array<{ title: string; items: string[] }>;
    migrationEyebrow: string;
    migrationTitle: string;
    migrationLead: string;
    migrationSteps: string[];
    supportTitle: string;
    supportBody: string;
    historyEyebrow: string;
    historyTitle: string;
    previous: Array<{ version: string; date: string; status: string; summary: string }>;
    releaseMetadata: string;
  };
};

/** Complete product-reference translations; a route never mixes locales.
 * 完整的产品参考页翻译；单个路由不会混用语言。 */
const en: LocalizedReference = {
  nav: {
    label: "Product navigation",
    home: "Home",
    namespace: "XML namespace",
    releases: "Release log",
    license: "License",
    maintained: "Maintained by the xmlsquish project",
    footerLabel: "Project resources",
  },
  ns: {
    title: "xmlsquish XML namespace — vocabulary explorer",
    description: "Explore the stable xmlsquish XML namespace, its eleven directives, build boundary, and immutable specification snapshots.",
    eyebrow: "Stable language surface",
    heading: "One namespace. Eleven precise building blocks.",
    lead: "Compose prompt projects with a compact XML vocabulary, then let xmlsquish validate the complete source closure before it publishes artifacts.",
    status: "Stable identity",
    name: "Exact namespace URI",
    copy: "Copy URI",
    copied: "URI copied",
    useTitle: "Bind it once, use it locally.",
    useBody: "The URI identifies the vocabulary. Builds compare it as an exact, case-sensitive string; they never fetch this address over the network.",
    vocabularyEyebrow: "Vocabulary explorer",
    vocabularyTitle: "Browse by the work each directive performs.",
    vocabularyBody: "Every directive is shown below in the static page. Stable fragment links make each contract addressable without creating eleven tiny documentation pages.",
    groupLabels: { roots: "Document roots", composition: "Definitions and composition", content: "Values and content", control: "Control" },
    elements: {
      module: "Define a library containing imports and named macros only.",
      entry: "Create an independent build document that constructs the final prompt.",
      import: "Load definitions from another module without executing a frame.",
      macro: "Define an immutable, namespace-qualified macro.",
      param: "Declare a required Unicode-string parameter.",
      expand: "Expand a statically resolved macro with isolated, by-value inputs.",
      arg: "Pass a literal, binding, or text-only expanded body.",
      fill: "Evaluate and provide an XML sequence in the caller.",
      slot: "Insert an explicitly provided XML sequence.",
      insert: "Emit an escaped scalar as text, never as parsed markup.",
      ifr: "Match a Unicode string and expose lexical named captures.",
    },
    pipelineEyebrow: "Build boundary",
    pipelineTitle: "A namespace is identity; the manager supplies the guarantees.",
    pipelineBody: "The frontend freezes and validates the source closure before lowering it to canonical binary XSIR. Linking resolves cross-file symbols and relocations; XSD alone cannot prove those project-level rules.",
    validationTitle: "Cross-file validation",
    validationBody: "Imports, macro scopes, recursion budgets, and final artifact publication are manager concerns. The result is a .prompt product with optional .xsir and .psdbg evidence.",
    resourcesEyebrow: "Resources and lineage",
    resourcesTitle: "Read the contract at the depth you need.",
    resources: [
      { title: "Current DSL specification", body: "The complete working language contract in Markdown." },
      { title: "Runnable examples", body: "Projects that exercise the current compiler and manager." },
      { title: "Release and migration log", body: "Product changes, including automation protocol migrations." },
      { title: "v0.3.0 snapshot", body: "Immutable historical language specification." },
      { title: "v0.2.0 snapshot", body: "The earlier immutable namespace specification." },
      { title: "Standards basis", body: "W3C namespace identity and Web Architecture guidance." },
      { title: "Project license", body: "The GPL-3.0-or-later terms shipped with xmlsquish." },
    ],
    versionTitle: "Three versions, three different jobs.",
    versionRows: [
      { label: "Namespace identity", value: "https://xmlsquish.moesegfault.dev/ns", body: "Stable and unversioned. Never append a release number or trailing slash." },
      { label: "Product release", value: "xmlsquish v1.0.1", body: "The executable and manager version you install." },
      { label: "Specification snapshot", value: "v0.3.0 / v0.2.0", body: "Immutable historical copies for auditing older language contracts." },
    ],
  },
  release: {
    title: "xmlsquish v1.0.1 — Release log",
    description: "Download xmlsquish v1.0.1 and review typed artifact locators, raw verified inspection, lazy runtime services, and the required machine protocol 3.0 migration.",
    eyebrow: "Latest stable release",
    date: "September 16, 2026",
    status: "Stable",
    heroTitle: "Artifacts now have",
    heroAccent: "public identities, not storage paths.",
    heroBody: "v1.0.1 gives build outputs typed, project-relative locators and makes raw artifact inspection verify bytes before writing them. Runtime services are also initialized only when an operation needs them.",
    releaseActions: "Release actions",
    installCta: "Get v1.0.1",
    githubCta: "View GitHub Release",
    impactLabel: "Upgrade impact",
    impact: "XML projects keep working. Automation that parses --message-format=json must migrate from protocol 2.1 to 3.0.",
    acquisitionEyebrow: "Acquire and verify",
    acquisitionTitle: "Six native builds, one checksum manifest.",
    acquisitionBody: "Choose the matching operating system and architecture, or install the exact tagged source. Native archives include the executable and license.",
    checksums: "SHA-256 checksum manifest",
    binaryNote: "Native archives are unsigned and not notarized. SHA-256 verifies integrity, not publisher identity.",
    sourceLabel: "Tagged source install",
    sourceBody: "Requires Rust 1.88 or newer. The tag and lockfile pin the complete source installation.",
    changesEyebrow: "What changed",
    changesTitle: "Sharper public boundaries with less hidden state.",
    highlights: [
      { label: "Artifact catalog", title: "Stable locators cross the publication boundary.", body: "Build results expose typed PublishedArtifact records while generation directories remain private." },
      { label: "Verified output", title: "Raw inspection verifies before it writes.", body: "inspect artifact LOCATOR --format=raw checks digest and size before exact bytes reach stdout." },
      { label: "Runtime", title: "Storage wakes only when work needs it.", body: "Invocation-scoped services lazily initialize CAS, indexes, and publishers by capability." },
    ],
    changeGroups: [
      { title: "Build and runtime", items: ["The production host injects one invocation-scoped build runtime.", "Operations with no storage capability no longer create storage state."] },
      { title: "Artifacts and inspection", items: ["Published artifacts carry id, kind, stable locator, size, and digest.", "Raw inspection resolves publisher-issued locators and verifies content before stdout."] },
      { title: "Reliability and maintenance", items: ["CI now enforces the reviewed workspace dependency-edge policy.", "The architecture checker tests itself and the real Cargo metadata graph."] },
    ],
    migrationEyebrow: "Required automation migration",
    migrationTitle: "Machine protocol 3.0 is intentionally not additive.",
    migrationLead: "v1.0.1 is a product patch release, but its machine event contract advances from 2.1 to 3.0. Consumers must update together with the CLI.",
    migrationSteps: ["Read target_id and generation_id from the completed build result.", "Consume PublishedArtifact { id, kind, locator, size, digest } records.", "Stop treating the v2 physical uri field as a public artifact contract."],
    supportTitle: "Compatibility boundary",
    supportBody: "The XML DSL and namespace identity do not change. Native targets remain Windows 10/11, glibc 2.35+ Linux, and macOS 11+. The published archives are not code-signed or notarized.",
    historyEyebrow: "Previous releases",
    historyTitle: "A chronological product record.",
    previous: [
      { version: "1.0.0", date: "September 15, 2026", status: "Stable", summary: "Introduced the Cargo-like project manager, reproducible dependencies, workspaces, and inspectable .prompt/.xsir/.psdbg artifacts." },
      { version: "0.3.0", date: "September 11, 2026", status: "Historical", summary: "Separated entry roots from module libraries and unified recursive expansion under xs:expand." },
      { version: "0.2.0", date: "September 11, 2026", status: "Historical", summary: "Introduced namespace-aware modules, immutable macros, scalar parameters, and XML slots." },
    ],
    releaseMetadata: "Versioned release metadata",
  },
};

const zh: LocalizedReference = {
  nav: { label: "产品导航", home: "首页", namespace: "XML 命名空间", releases: "发布记录", license: "许可证", maintained: "由 xmlsquish 项目维护", footerLabel: "项目资源" },
  ns: {
    title: "xmlsquish XML 命名空间 — 词汇浏览器",
    description: "浏览稳定的 xmlsquish XML 命名空间、十一条指令、构建边界与不可变规范快照。",
    eyebrow: "稳定语言表面",
    heading: "一个命名空间，十一块精确积木。",
    lead: "用紧凑的 XML 词汇组合提示词项目，再由 xmlsquish 验证完整源码闭包并发布产物。",
    status: "稳定身份",
    name: "精确命名空间 URI",
    copy: "复制 URI",
    copied: "URI 已复制",
    useTitle: "绑定一次，在本地使用。",
    useBody: "URI 用来标识词汇。构建按区分大小写的精确字符串比较它，绝不会通过网络获取这个地址。",
    vocabularyEyebrow: "词汇浏览器",
    vocabularyTitle: "按每条指令承担的任务浏览。",
    vocabularyBody: "静态页面完整呈现所有指令；稳定片段链接让每份契约都可被引用，而无需制造十一个零碎文档页。",
    groupLabels: { roots: "文档根", composition: "定义与组合", content: "值与内容", control: "控制" },
    elements: {
      module: "定义只包含导入与具名宏的库。", entry: "创建组织最终提示词的独立构建文档。", import: "从另一模块装载定义，不执行调用帧。", macro: "定义不可重定义、命名空间限定的宏。", param: "声明必需的 Unicode 字符串参数。", expand: "使用隔离的按值输入展开静态解析的宏。", arg: "传入字面量、绑定或纯文本展开主体。", fill: "在调用方求值并提供 XML 序列。", slot: "插入显式提供的 XML 序列。", insert: "把标量转义为文本，绝不重新解析为标记。", ifr: "匹配 Unicode 字符串并暴露词法命名捕获。",
    },
    pipelineEyebrow: "构建边界",
    pipelineTitle: "命名空间提供身份，管理器提供保证。",
    pipelineBody: "前端冻结并验证源码闭包，再降低为规范二进制 XSIR。链接阶段解析跨文件符号与重定位；单靠 XSD 无法证明这些项目级规则。",
    validationTitle: "跨文件验证",
    validationBody: "导入、宏作用域、递归预算和最终产物发布都属于管理器职责。结果是 .prompt 产品，并可附带 .xsir 与 .psdbg 证据。",
    resourcesEyebrow: "资源与沿革",
    resourcesTitle: "按你需要的深度阅读契约。",
    resources: [
      { title: "当前 DSL 规范", body: "完整的 Markdown 工作版语言契约。" }, { title: "可运行示例", body: "实际运行当前编译器与管理器的项目。" }, { title: "发布与迁移记录", body: "产品变更，包括自动化协议迁移。" }, { title: "v0.3.0 快照", body: "不可变的历史语言规范。" }, { title: "v0.2.0 快照", body: "更早的不可变命名空间规范。" }, { title: "标准依据", body: "W3C 命名空间身份与 Web 架构指南。" }, { title: "项目许可证", body: "xmlsquish 随附的 GPL-3.0-or-later 条款。" },
    ],
    versionTitle: "三种版本，三份不同职责。",
    versionRows: [
      { label: "命名空间身份", value: "https://xmlsquish.moesegfault.dev/ns", body: "稳定且不带版本。切勿追加发布号或末尾斜杠。" }, { label: "产品版本", value: "xmlsquish v1.0.1", body: "你安装的可执行程序与管理器版本。" }, { label: "规范快照", value: "v0.3.0 / v0.2.0", body: "用于审计旧语言契约的不可变历史副本。" },
    ],
  },
  release: {
    title: "xmlsquish v1.0.1 — 发布记录",
    description: "下载 xmlsquish v1.0.1，并了解有类型产物定位符、经验证的原始检查、惰性运行时服务，以及必须进行的机器协议 3.0 迁移。",
    eyebrow: "最新稳定版本", date: "2026 年 9 月 16 日", status: "稳定版",
    heroTitle: "产物拥有的是", heroAccent: "公共身份，而非存储路径。",
    heroBody: "v1.0.1 为构建输出提供有类型、项目相对的定位符，并让原始产物检查在写出字节前完成验证。运行时服务也只在操作确有需要时初始化。",
    releaseActions: "发布操作", installCta: "获取 v1.0.1", githubCta: "查看 GitHub Release", impactLabel: "升级影响", impact: "XML 项目无需迁移。解析 --message-format=json 的自动化必须从协议 2.1 迁移到 3.0。",
    acquisitionEyebrow: "获取与校验", acquisitionTitle: "六份原生构建，一份校验清单。", acquisitionBody: "选择匹配的操作系统与架构，或安装精确标签的源码。原生压缩包包含可执行文件与许可证。", checksums: "SHA-256 校验清单", binaryNote: "原生压缩包未签名、未经 Apple 公证。SHA-256 验证完整性，不证明发布者身份。", sourceLabel: "安装标签源码", sourceBody: "需要 Rust 1.88 或更高版本；标签和锁文件共同固定完整源码安装。",
    changesEyebrow: "本次变化", changesTitle: "公共边界更清晰，隐藏状态更少。",
    highlights: [
      { label: "产物目录", title: "稳定定位符跨越发布边界。", body: "构建结果公开有类型 PublishedArtifact 记录，而 generation 目录保持私有。" }, { label: "验证输出", title: "原始检查先验证，再写出。", body: "inspect artifact LOCATOR --format=raw 在精确字节进入 stdout 前检查摘要与大小。" }, { label: "运行时", title: "存储只在工作需要时唤醒。", body: "调用级服务按能力惰性初始化 CAS、索引与发布器。" },
    ],
    changeGroups: [
      { title: "构建与运行时", items: ["生产主机注入一个调用级构建运行时。", "不需要存储能力的操作不再创建存储状态。"] }, { title: "产物与检查", items: ["已发布产物携带 id、kind、稳定 locator、size 与 digest。", "原始检查解析发布器签发的定位符，并在 stdout 前验证内容。"] }, { title: "可靠性与维护", items: ["CI 现在强制执行已评审的工作区依赖边策略。", "架构检查器同时测试自身与真实 Cargo 元数据图。"] },
    ],
    migrationEyebrow: "必须进行的自动化迁移", migrationTitle: "机器协议 3.0 有意不是加性更新。", migrationLead: "v1.0.1 是产品补丁版本，但机器事件契约从 2.1 升级到 3.0；消费者必须与 CLI 同步升级。", migrationSteps: ["从完成的构建结果读取 target_id 与 generation_id。", "消费 PublishedArtifact { id, kind, locator, size, digest } 记录。", "停止把 v2 的物理 uri 字段当作公共产物契约。"],
    supportTitle: "兼容边界", supportBody: "XML DSL 与命名空间身份不变。原生目标仍为 Windows 10/11、glibc 2.35+ Linux 与 macOS 11+。发布的压缩包未进行代码签名或公证。",
    historyEyebrow: "历史版本", historyTitle: "按时间排列的产品记录。",
    previous: [
      { version: "1.0.0", date: "2026 年 9 月 15 日", status: "稳定版", summary: "引入 Cargo 式项目管理器、可复现依赖、工作区，以及可检查的 .prompt/.xsir/.psdbg 产物。" }, { version: "0.3.0", date: "2026 年 9 月 11 日", status: "历史版本", summary: "分离 entry 构建根与 module 宏库，并以 xs:expand 统一递归展开。" }, { version: "0.2.0", date: "2026 年 9 月 11 日", status: "历史版本", summary: "引入命名空间感知模块、不可变宏、标量参数与 XML slot。" },
    ],
    releaseMetadata: "版本化发布元数据",
  },
};

export type ReferenceMessages = LocalizedReference;
export const referenceMessages: Record<Locale, ReferenceMessages> = { "zh-CN": zh, en };
