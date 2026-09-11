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
    title: "xmlsquish — XML 提示词构建系统",
    description:
      "把提示词组织成多文件源码，通过宏、独立文件环境与显式参数与结构插槽，构建可检查的中间表示和紧凑的最终产物。",
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
    badge: "XML PROMPT BUILD SYSTEM",
    title: "提示词，也值得",
    accent: "好好构建。",
    lead: "拆成文件，组合内容，显式传递上下文。xmlsquish 把可维护的 XML 源码，构建成给 Agent 的最终提示词。宏展开保留文本语义，最终产物压紧空白。",
    primary: "看一次真实构建",
    secondary: "安装 CLI",
    footnote: "Rust 2024 · 源文件不改写 · 编译产物可检查",
    source: "组织源码",
    compile: "解析与组合",
    artifact: "交付产物",
    caption: "3 份源码 → 1 份提示词。不是把一段 XML 的换行删掉。",
  },
  journey: {
    steps: [
      {
        label: "01 / AUTHOR",
        title: "按职责拆分",
        body: "Persona、任务与规则各自维护；用 mount / call 组合，而不是复制粘贴。",
      },
      {
        label: "02 / COMPILE",
        title: "展开成可检查的 XML",
        body: "执行条件、宏与结构插槽，保留来源信息。-I 停在中间表示（Intermediate Representation, IR）。",
      },
      {
        label: "03 / DELIVER",
        title: "生成紧凑 XML 产物",
        body: "默认 -O 移除内部来源信息并压紧空白，写出 .o.xml；成功后清理对应 .i.xml。",
      },
    ],
    note: "SOURCE → SEMANTICS → ARTIFACT",
  },
  build: {
    eyebrow: "SHOW, DON'T JUST MINIFY",
    title: "从三个文件，看懂一次构建。",
    intro:
      "打开源码，再看展开结果：挂载 Persona，显式传递 audience，再调用任务宏。点击 .i.xml / .o.xml 查看展开和最终结果。",
    label: "显式传给 audience 的参数",
    parent: "researchers · 研究者",
    self: "everyone · 所有人",
    proof:
      "由仓库 Rust CLI 预编译并校验的示例；切换展示已有结果，不在浏览器里运行编译器。",
    resultParent: "arg.audience = researchers，由调用者显式传入。",
    resultSelf: "arg.audience = everyone，由调用者显式传入。",
    tabLabel: "示例源码与构建产物",
    noScript: "未启用 JavaScript：以下完整列出两组参数的源码与结果。",
    source: "源码",
    intermediate: "展开结果 · -I",
    output: "最终产物 · -O",
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
        title: "导入定义，挂载或调用内容",
        body: "import 只装载命名宏，不产生输出。mount 调用模块主体，call 调用静态命名宏；slot / fill 组合 XML 节点。",
        code: '<xs:import src="tasks.xml"/>\n<xs:mount src="persona.xml">\n  <xs:arg name="audience" value="researchers"/>\n</xs:mount>',
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
    title: "检查展开结果，再交给 Agent。",
    intro:
      "接收文件、目录或 glob。发现阶段跳过 .i.xml / .o.xml，原子替换产物；一个文件失败，不阻止其他独立输入。",
    install: "从仓库安装",
    inspect: "只编译，保留中间表示",
    optimize: "展开、清理来源信息并压紧最终 XML",
    color: "纯文本诊断，也适合 CI",
    note: "目录递归发现；源文件保持不变。示例路径对应本仓库 examples/site-demo。",
    diagnosticTitle: "错误回到源码，而不是一串重复路径。",
    diagnosticBody:
      "下面是独立错误用例的真实诊断，显示文件、行号和源码快照；支持 --color auto / always / never。没有精确列号，就不虚构插入符位置。",
  },
  stats: {
    eyebrow: "MEASURE THE RIGHT TRANSFORMATION",
    title: "先分清展开，再谈节省。",
    intro:
      "这是上方 researchers 示例的真实结果。IR 含来源信息，展示时归一化源码 URI；最终产物移除来源信息并压紧空白。",
    source: "主源文件",
    ir: "已组装 IR",
    final: "最终提示词",
    saved: "来源信息清理差额",
    dependencies: "引用加载",
    files: "个不同文件",
    note: "固定 o200k_base；字节数不含 BOM。IR 仅展示归一化文本的 UTF-8 字节数，不与实际 token 数作差。Token 大小不等于模型质量、实际账单或推理速度。",
  },
  limits: {
    title: "有边界，才可依赖。",
    body: '仅编译可信源：静态 src 可以读取本地源码。宏不能读取环境变量、时间或运行子进程。安全 insert 不防提示词注入（prompt injection）；-O 会压紧用户文本中的 XML 空白；空白敏感场景应检查最终产物。',
    details: "明确的计算边界与资源预算",
    engine: "XML 是数据，宏是计算。",
    engineBody:
      "标量宏支持递归、字符串构造与命名捕获；XML 结构通过 slot / fill 传递。--max-depth、--max-expansions 与 --max-output-bytes 控制运行预算，失败不提交部分产物。",
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
    title: "xmlsquish — A build system for XML prompts",
    description:
      "Compose multi-file XML prompts with macros, isolated file frames, and explicit arguments and structural slots. Inspect the expanded IR, then ship a compact artifact.",
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
    badge: "XML PROMPT BUILD SYSTEM",
    title: "Your prompts deserve",
    accent: "a proper build.",
    lead: "Split files. Compose content. Pass context explicitly. xmlsquish turns maintainable XML source into a finished prompt for your agent. Expansion preserves text semantics; the final product compacts whitespace.",
    primary: "See a real build",
    secondary: "Install the CLI",
    footnote: "Rust 2024 · Sources stay untouched · Inspectable artifacts",
    source: "Author sources",
    compile: "Resolve & compose",
    artifact: "Deliver artifacts",
    caption: "3 source files → 1 prompt. More than XML with fewer line breaks.",
  },
  journey: {
    steps: [
      {
        label: "01 / AUTHOR",
        title: "Split by responsibility",
        body: "Maintain personas, tasks, and rules separately. Compose with mount / call instead of copy-paste.",
      },
      {
        label: "02 / COMPILE",
        title: "Inspect expanded XML",
        body: "Evaluate macros and conditions, retain provenance. -I stops at the intermediate representation (IR).",
      },
      {
        label: "03 / DELIVER",
        title: "Ship compact XML",
        body: "Default -O removes internal provenance compacts whitespace, and writes .o.xml, removing its matching .i.xml after success.",
      },
    ],
    note: "SOURCE → SEMANTICS → ARTIFACT",
  },
  build: {
    eyebrow: "SHOW, DON'T JUST MINIFY",
    title: "Three files. One understandable build.",
    intro:
      "Read the sources, then inspect the expansion: Persona is mounted with an explicit audience argument, and a named task macro is called. Select .i.xml / .o.xml to see the results.",
    label: "Explicit audience argument",
    parent: "researchers · focused audience",
    self: "everyone · broad audience",
    proof:
      "Precompiled and verified with the repository's Rust CLI. Controls switch recorded results; no compiler runs in your browser.",
    resultParent: "arg.audience = researchers, explicitly passed by the caller.",
    resultSelf: "arg.audience = everyone, explicitly passed by the caller.",
    tabLabel: "Example sources and build artifacts",
    noScript:
      "JavaScript is disabled: both argument sets and all sources and outputs are shown below.",
    source: "Source",
    intermediate: "Expanded · -I",
    output: "Final · -O",
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
        title: "Import definitions. Invoke content.",
        body: "import loads named macros without output. mount invokes a module main; call invokes a static named macro. slot / fill compose XML nodes.",
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
    title: "Inspect the expansion. Then feed your agent.",
    intro:
      "Accept files, directories, or globs. Discovery skips .i.xml / .o.xml, output replacement is atomic, and one failed file does not stop independent inputs.",
    install: "Install from a repository checkout",
    inspect: "Compile only; keep the intermediate",
    optimize: "Expand, lower provenance, and compact final XML",
    color: "Plain diagnostics, ready for CI",
    note: "Directories are recursive; sources stay untouched. These paths use examples/site-demo in this repository.",
    diagnosticTitle: "Errors point to source, not a pile of repeated paths.",
    diagnosticBody:
      "This separate failing example shows the physical file, line, and original source snapshot. Choose --color auto / always / never; no caret is invented when a column isn't known.",
  },
  stats: {
    eyebrow: "MEASURE THE RIGHT TRANSFORMATION",
    title: "Assembly growth is not failed compression.",
    intro:
      "Actual numbers from the researchers example. IR contains provenance with display-normalized source URIs; final output removes metadata and compacts whitespace.",
    source: "Primary source",
    ir: "Assembled IR",
    final: "Final prompt",
    saved: "Removed provenance tokens",
    dependencies: "Dependency loads",
    files: "unique files",
    note: "Fixed o200k_base; bytes exclude BOM. IR shows normalized UTF-8 bytes only, not a token delta. Token size is not model quality, actual billing, or inference speed.",
  },
  limits: {
    title: "Clear boundaries. Fewer surprises.",
    body: 'Compile trusted sources only: static src can read local sources. Macros cannot read environment values, time, or launch subprocesses. Safe insert does not prevent prompt injection. -O compacts XML whitespace in user text; inspect the final artifact for whitespace-sensitive uses.',
    details: "Explicit computational boundaries and budgets",
    engine: "XML is data. Macros are computation.",
    engineBody:
      "Scalar macros support recursion, string construction, and named captures; slot / fill pass XML structure. --max-depth, --max-expansions, and --max-output-bytes bound each invocation. Failures never publish partial output.",
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

export const messages: Record<Locale, Messages> = { "zh-CN": zh, en };
