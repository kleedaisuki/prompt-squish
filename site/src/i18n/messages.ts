import { referenceMessages, type ReferenceMessages } from "./reference";

export const locales = ["zh-CN", "en"] as const;
export type Locale = (typeof locales)[number];

type Capability = { title: string; body: string; code: string };
type FrameField = { name: string; title: string; body: string; sample: string };
type Messages = {
  meta: { title: string; description: string };
  nav: {
    build: string;
    model: string;
    cli: string;
    github: string;
    language: string;
    label: string;
  };
  hero: {
    badge: string;
    revision: string;
    title: string;
    accent: string;
    lead: string;
    primary: string;
    secondary: string;
    footnote: string;
    source: string;
    compile: string;
    artifact: string;
    caption: string;
  };
  journey: {
    steps: { label: string; title: string; body: string }[];
    note: string;
  };
  build: {
    eyebrow: string;
    title: string;
    intro: string;
    label: string;
    parent: string;
    self: string;
    proof: string;
    resultParent: string;
    resultSelf: string;
    tabLabel: string;
    noScript: string;
    source: string;
    intermediate: string;
    output: string;
    copy: string;
    copied: string;
    copyFailed: string;
  };
  model: {
    eyebrow: string;
    title: string;
    intro: string;
    parent: string;
    child: string;
    inherited: string;
    physical: string;
    note: string;
    fields: FrameField[];
  };
  capabilities: { eyebrow: string; title: string; items: Capability[] };
  cli: {
    eyebrow: string;
    title: string;
    intro: string;
    install: string;
    format: string;
    build: string;
    add: string;
    remove: string;
    inspect: string;
    optimize: string;
    color: string;
    note: string;
    diagnosticTitle: string;
    diagnosticBody: string;
  };
  stats: {
    eyebrow: string;
    title: string;
    intro: string;
    source: string;
    ir: string;
    final: string;
    saved: string;
    dependencies: string;
    files: string;
    note: string;
  };
  limits: {
    title: string;
    body: string;
    details: string;
    engine: string;
    engineBody: string;
    docs: string;
  };
  footer: {
    tagline: string;
    source: string;
    style: string;
    license: string;
    notices: string;
  };
  theme: { label: string; auto: string; light: string; dark: string };
};

const zh: Messages = {
  meta: {
    title: "xmlsquish — XML 提示词项目管理器",
    description:
      "用统一项目管理器格式化、解析依赖并构建多文件 XML 提示词；复用二进制 XSIR，链接后发布 .prompt 与可选 .psdbg。",
  },
  nav: {
    build: "看一次构建",
    model: "作用域与参数",
    cli: "开始使用",
    github: "GitHub",
    language: "English",
    label: "主要导航",
  },
  hero: {
    badge: "XML PROMPT PROJECT MANAGER",
    revision: "项目管理器 · fmt / build / add / remove / inspect",
    title: "提示词项目，也值得",
    accent: "好好构建。",
    lead: "从 xmlsquish.toml 发现项目与工作区：格式化源码、解析锁定依赖、把 XML 编译成可缓存的二进制 XSIR，再链接并发布 .prompt 与 .psdbg。",
    primary: "看一次真实构建",
    secondary: "安装 CLI",
    footnote: "Rust 2024 · 项目级增量构建 · Human / Short / NDJSON",
    source: "组织源码",
    compile: "编译 XSIR 并链接",
    artifact: "交付产物",
    caption: "项目清单 + XML 模块 → 二进制 XSIR → 链接 → .prompt + .psdbg。",
  },
  journey: {
    steps: [
      {
        label: "01 / AUTHOR",
        title: "声明项目与依赖",
        body: "在 xmlsquish.toml 声明包、目标与依赖；add / remove 以事务方式更新清单和锁文件。",
      },
      {
        label: "02 / COMPILE",
        title: "编译并缓存 XSIR",
        body: "每个 XML 模块生成规范二进制中间表示（XSIR）；内容寻址缓存复用未变化的纯变换。",
      },
      {
        label: "03 / DELIVER",
        title: "链接并发布提示词",
        body: "链接冻结的模块图并实例化入口，原子发布 .prompt；按 --emit 生成 .xsir 与自包含 .psdbg 伴随文件。",
      },
    ],
    note: "PROJECT → RESOLVE → XSIR → LINK → ARTIFACT",
  },
  build: {
    eyebrow: "SHOW, DON'T JUST MINIFY",
    title: "从项目清单，看懂一次可复用构建。",
    intro:
      "切换项目清单、入口源码、二进制 XSIR 摘要、链接结果、最终 .prompt 与 .psdbg。示例命令可直接在仓库根目录运行。",
    label: "显式传给 audience 的参数",
    parent: "researchers · 研究者",
    self: "everyone · 所有人",
    proof:
      "数据记录当前项目管理器的工件契约；浏览器只展示结果，不把二进制 XSIR 伪装成 XML。",
    resultParent: "arg.audience = researchers，由调用者显式传入。",
    resultSelf: "arg.audience = everyone，由调用者显式传入。",
    tabLabel: "示例源码与构建产物",
    noScript: "未启用 JavaScript：以下完整列出两组参数的源码与结果。",
    source: "源码",
    intermediate: "构建阶段 / 伴随产物",
    output: "最终 .prompt",
    copy: "复制代码",
    copied: "已复制",
    copyFailed: "无法复制，请选中文本",
  },
  model: {
    eyebrow: "EXPLICIT CONTEXT. LOCAL STATE.",
    title: "定义共享，调用相互独立。",
    intro:
      "源码单元（SourceUnit）按规范 URI 冻结；每次调用建立独立展开帧（Expansion Frame），仅显式传递 arg 与 fill。",
    parent: "父文件 / agent.xml",
    child: "子文件 / persona.xml",
    inherited: "显式参数进入新的调用帧",
    physical: "定义位置始终不变",
    note: "相对 src 与 file.* 绑定到宏的定义位置，不随调用者变化。参数不会隐式继承；所有声明参数都必须显式提供。",
    fields: [
      { name: "arg", title: "不可变参数", body: "必需参数由调用者显式传入，类型始终为 Unicode 字符串。", sample: "arg.audience = researchers" },
      { name: "file", title: "定义位置", body: "uri / dir / name 只描述当前宏的定义源码。", sample: "file.name = persona.xml" },
      { name: "match", title: "词法捕获", body: "ifr 的命名捕获仅在当前词法块内有效，离开后恢复外层绑定。", sample: "match.head · match.tail" },
      { name: "slot / fill", title: "结构通道", body: "XML 节点序列与标量参数严格分离，不隐式转换为字符串。", sample: "<xs:slot name=\"body\"/>" },
    ],
  },
  capabilities: {
    eyebrow: "SMALL LANGUAGE. USEFUL BOUNDARIES.",
    title: "组合、选择、输出，各司其职。",
    items: [
      {
        title: "导入定义，递归展开宏",
        body: "module 只存放宏定义；独立的 entry 导入模块、构建提示词。expand 递归展开宏并按值返回结果。",
        code: '<xs:import src="persona.xml"/>\n<xs:expand ref="demo:persona">\n  <xs:arg name="audience" value="researchers"/>\n</xs:expand>',
      },
      {
        title: "在编译时做选择",
        body: "ifr 匹配标量字符串，命名捕获只在词法块内可见。完整源码闭包在执行前加载与验证，包括未执行分支。",
        code: '<xs:ifr get="file.name" pattern="^tasks[.]xml$">\n  <task>Explain the trade-offs.</task>\n</xs:ifr>',
      },
      {
        title: "只有显式 insert 才输出变量",
        body: "insert 将 arg.*、file.* 或 match.* 作为转义后的 XML 文本输出，不重新解析为标签或宏。",
        code: '<answer><xs:insert get="arg.text"/></answer>\n<!-- arg.text is text, never markup. -->',
      },
    ],
  },
  cli: {
    eyebrow: "FROM SOURCE TO YOUR WORKFLOW",
    title: "管理整个项目，再交付给 Agent。",
    intro:
      "fmt、build、add、remove 与 inspect 共享同一项目发现、配置、依赖解析和事件协议；build 默认继续独立工作，并原子发布成功目标。",
    install: "从当前工作区源码安装（Rust 1.88+）",
    format: "格式化项目源码；--check 只检查",
    build: "构建 prompt、XSIR 与调试伴随文件",
    add: "添加依赖并更新清单与锁文件",
    remove: "移除直接依赖别名",
    inspect: "检查类型化 IR、链接、来源、缓存或产物",
    optimize: "构建 prompt、XSIR 与调试伴随文件",
    color: "NDJSON 事件流，适合 CI 与工具",
    note: "命令向上发现 xmlsquish.toml，也可显式传 --manifest-path；Human 面向终端，Short 稳定逐行，json 为 NDJSON。",
    diagnosticTitle: "一种事件协议，三种终端体验。",
    diagnosticBody:
      "Human 提供进度与摘要，Short 逐行且可 grep，--message-format=json 输出带版本与序号的换行分隔 JSON（NDJSON）事件。--plain 提供无装饰追加输出。",
  },
  stats: {
    eyebrow: "MEASURE THE RIGHT TRANSFORMATION",
    title: "缓存模块，链接目标，原子发布。",
    intro:
      "构建计划把扫描、编译、链接、实例化、后端与发布分开；只有确定性的纯变换进入持久动作缓存。",
    source: "项目源码",
    ir: "可复用 XSIR",
    final: "发布目标",
    saved: "来源信息清理差额",
    dependencies: "构建动作",
    files: "个模块",
    note: "计数描述演示项目的结构，不声称二进制 XSIR 是文本或把缓存命中等同于端到端加速；最终字节数不含 BOM。",
  },
  limits: {
    title: "有边界，才可依赖。",
    body: '仅构建可信源码与依赖：本地及已解析包可被前端读取。宏不能读取环境、时间或运行子进程；insert 也不能阻止提示词注入（prompt injection）。--locked / --offline / --frozen 控制解析，不是文件系统沙箱。',
    details: "明确的计算边界与资源预算",
    engine: "项目负责边界，XSIR 负责复用。",
    engineBody:
      "解析器保留 XML 语义；规范二进制 XSIR 跨目标复用；链接器解析导入与重定位；发布器只提交完整的一代 .prompt / .psdbg。缓存可删除，不是权威项目状态。",
    docs: "阅读完整语义与边界",
  },
  footer: {
    tagline: "让提示词可维护，让产物可交付。",
    source: "查看源码与示例",
    style: "Built with MoeSegfault Style",
    license: "许可",
    notices: "第三方声明",
  },
  theme: { label: "外观", auto: "跟随系统", light: "浅色", dark: "深色" },
};

const en: Messages = {
  meta: {
    title: "xmlsquish — The project manager for XML prompts",
    description:
      "Format sources, resolve dependencies, and build multi-file XML prompts with one project manager. Reuse binary XSIR, link, then publish .prompt and optional .psdbg artifacts.",
  },
  nav: {
    build: "See a build",
    model: "Frames & context",
    cli: "Get started",
    github: "GitHub",
    language: "中文",
    label: "Main navigation",
  },
  hero: {
    badge: "XML PROMPT PROJECT MANAGER",
    revision: "Project manager · fmt / build / add / remove / inspect",
    title: "Your prompt projects deserve",
    accent: "a proper build.",
    lead: "Discover projects and workspaces from xmlsquish.toml, format sources, resolve locked dependencies, compile XML to cacheable binary XSIR, then link and publish .prompt and .psdbg.",
    primary: "See a real build",
    secondary: "Install the CLI",
    footnote: "Rust 2024 · Project-level incremental builds · Human / Short / NDJSON",
    source: "Author sources",
    compile: "Compile XSIR & link",
    artifact: "Deliver artifacts",
    caption: "Project manifest + XML modules → binary XSIR → link → .prompt + .psdbg.",
  },
  journey: {
    steps: [
      {
        label: "01 / AUTHOR",
        title: "Declare project and dependencies",
        body: "Declare packages, targets, and dependencies in xmlsquish.toml; add / remove update manifests and lockfiles transactionally.",
      },
      {
        label: "02 / COMPILE",
        title: "Compile and cache XSIR",
        body: "Compile each XML module to canonical binary XSIR; the content-addressed cache reuses unchanged pure transformations.",
      },
      {
        label: "03 / DELIVER",
        title: "Link and publish the prompt",
        body: "Link the frozen module graph and instantiate the entry, atomically publishing .prompt plus .xsir and self-contained .psdbg companions selected by --emit.",
      },
    ],
    note: "PROJECT → RESOLVE → XSIR → LINK → ARTIFACT",
  },
  build: {
    eyebrow: "SHOW, DON'T JUST MINIFY",
    title: "One project manifest. One reusable build.",
    intro:
      "Switch among the manifest, entry source, binary XSIR summary, linked target, final .prompt, and .psdbg. The example command runs from the repository root.",
    label: "Explicit audience argument",
    parent: "researchers · focused audience",
    self: "everyone · broad audience",
    proof:
      "The data records the current manager artifact contract. The browser only presents it; binary XSIR is never passed off as XML.",
    resultParent: "arg.audience = researchers, explicitly passed by the caller.",
    resultSelf: "arg.audience = everyone, explicitly passed by the caller.",
    tabLabel: "Example sources and build artifacts",
    noScript:
      "JavaScript is disabled: both argument sets and all sources and outputs are shown below.",
    source: "Source",
    intermediate: "Build stage / companion",
    output: "Final .prompt",
    copy: "Copy code",
    copied: "Copied",
    copyFailed: "Copy unavailable; select the text",
  },
  model: {
    eyebrow: "EXPLICIT CONTEXT. LOCAL STATE.",
    title: "Share definitions. Isolate invocations.",
    intro:
      "SourceUnits are frozen by canonical URI. Every invocation creates a new expansion frame; only explicit arguments and fills cross the boundary.",
    parent: "Parent / agent.xml",
    child: "Child / persona.xml",
    inherited: "Explicit argument enters a fresh frame",
    physical: "Definition-site identity never changes",
    note: "Relative src and file.* bind to the definition site, not the caller. Arguments are never inherited; every declared parameter must be explicitly supplied.",
    fields: [
      { name: "arg", title: "Immutable arguments", body: "Required Unicode string parameters are supplied explicitly by the caller.", sample: "arg.audience = researchers" },
      { name: "file", title: "Definition site", body: "uri / dir / name describe the source that defines the current macro.", sample: "file.name = persona.xml" },
      { name: "match", title: "Lexical captures", body: "Named captures live inside an ifr block; outer bindings return when it exits.", sample: "match.head · match.tail" },
      { name: "slot / fill", title: "Structural channel", body: "XML node sequences remain separate from scalar arguments, with no implicit string conversion.", sample: "<xs:slot name=\"body\"/>" },
    ],
  },
  capabilities: {
    eyebrow: "SMALL LANGUAGE. USEFUL BOUNDARIES.",
    title: "Compose. Select. Emit. Explicitly.",
    items: [
      {
        title: "Import modules. Expand macros.",
        body: "Modules contain macro definitions. A separate entry imports modules and constructs the prompt; expand recursively produces macro return values.",
        code: zh.capabilities.items[0].code,
      },
      {
        title: "Make choices at compile time.",
        body: "ifr matches scalar strings with lexically scoped named captures. The complete source closure is loaded and validated before execution, including unselected branches.",
        code: zh.capabilities.items[1].code,
      },
      {
        title: "Only insert emits a variable.",
        body: "insert emits arg.*, file.*, or match.* as XML-escaped text, never reinterpreting it as markup or macros.",
        code: zh.capabilities.items[2].code,
      },
    ],
  },
  cli: {
    eyebrow: "FROM SOURCE TO YOUR WORKFLOW",
    title: "Manage the project. Then ship to your agent.",
    intro:
      "fmt, build, add, remove, and inspect share project discovery, configuration, dependency resolution, and one event protocol. Build keeps independent work moving and publishes successful targets atomically.",
    install: "Install from the current workspace source (Rust 1.88+)",
    format: "Format project sources; --check only reports",
    build: "Build prompt, XSIR, and debug companion",
    add: "Add a dependency and update manifest plus lockfile",
    remove: "Remove a direct dependency alias",
    inspect: "Inspect typed IR, links, sources, cache, or artifacts",
    optimize: "Build prompt, XSIR, and debug companion",
    color: "NDJSON events for CI and tools",
    note: "Commands discover xmlsquish.toml upward or accept --manifest-path. Human targets terminals, Short is stable line output, and json is NDJSON.",
    diagnosticTitle: "One event protocol. Three terminal experiences.",
    diagnosticBody:
      "Human offers progress and summaries; Short is line-oriented and grep-friendly; --message-format=json emits versioned, sequenced newline-delimited JSON (NDJSON). --plain is undecorated and append-only.",
  },
  stats: {
    eyebrow: "MEASURE THE RIGHT TRANSFORMATION",
    title: "Cache modules. Link targets. Publish atomically.",
    intro:
      "The plan separates scan, compile, link, instantiate, backend, and publish. Only deterministic pure transformations enter the persistent action cache.",
    source: "Project sources",
    ir: "Reusable XSIR",
    final: "Published target",
    saved: "Removed provenance tokens",
    dependencies: "Build actions",
    files: "modules",
    note: "Counts describe the demo project structure. They do not pretend binary XSIR is text or equate a cache hit with end-to-end speed; final bytes exclude BOM.",
  },
  limits: {
    title: "Clear boundaries. Fewer surprises.",
    body: 'Build trusted sources and dependencies only: the frontend can read local and resolved package sources. Macros cannot read environment values, time, or launch subprocesses, and insert does not prevent prompt injection. --locked / --offline / --frozen control resolution; they are not a filesystem sandbox.',
    details: "Explicit computational boundaries and budgets",
    engine: "Projects define boundaries. XSIR enables reuse.",
    engineBody:
      "The frontend preserves XML semantics; canonical binary XSIR is reusable across targets; the linker resolves imports and relocations; the publisher commits only a complete .prompt / .psdbg generation. Cache state is disposable, never authoritative project state.",
    docs: "Read the full semantics and boundaries",
  },
  footer: {
    tagline: "Maintainable sources. Deliverable prompts.",
    source: "Explore the source & examples",
    style: "Built with MoeSegfault Style",
    license: "License",
    notices: "Third-party notices",
  },
  theme: { label: "Theme", auto: "System", light: "Light", dark: "Dark" },
};

export const messages: Record<Locale, Messages & { reference: ReferenceMessages }> = {
  "zh-CN": { ...zh, reference: referenceMessages["zh-CN"] },
  en: { ...en, reference: referenceMessages.en },
};
