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
      "把提示词组织成多文件源码，通过宏、独立文件环境与显式元数据继承，构建可检查的中间表示和紧凑的最终产物。",
  },
  nav: {
    build: "看一次构建",
    model: "环境与继承",
    cli: "开始使用",
    github: "GitHub",
    language: "English",
    label: "主要导航",
  },
  hero: {
    badge: "XML PROMPT BUILD SYSTEM",
    title: "提示词，也值得",
    accent: "好好构建。",
    lead: "拆成文件，组合内容，显式传递上下文。xmlsquish 把可维护的 XML 源码，构建成给 Agent 的最终提示词。压紧空白，只是最后一步。",
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
        body: "Persona、任务与规则各自维护；用 mount / import 组合，而不是复制粘贴。",
      },
      {
        label: "02 / COMPILE",
        title: "展开成可检查的 XML",
        body: "执行条件与宏、合并元数据。-I 停在中间表示（Intermediate Representation, IR）。",
      },
      {
        label: "03 / DELIVER",
        title: "再生成紧凑产物",
        body: "默认 -O 规范化空白，写出 .o.xml；成功后清理对应 .i.xml。",
      },
    ],
    note: "SOURCE → SEMANTICS → ARTIFACT",
  },
  build: {
    eyebrow: "SHOW, DON'T JUST MINIFY",
    title: "从三个文件，看懂一次构建。",
    intro:
      "打开源码，再看展开结果：根变成 Persona，父方 audience 覆盖子默认值，任务脱离包装接入。点击 .i.xml / .o.xml 查看展开和最终结果。",
    label: "这条 mount 边的元数据策略",
    parent: "parent · 父方优先",
    self: "self · 子方独立",
    proof:
      "由仓库 Rust CLI 预编译并校验的示例；切换展示已有结果，不在浏览器里运行编译器。",
    resultParent: "meta:audience = researchers，来自父文件。",
    resultSelf: "meta:audience = everyone，来自子文件。",
    tabLabel: "示例源码与构建产物",
    noScript: "未启用 JavaScript：以下完整列出两种策略的源码与结果。",
    source: "源码",
    intermediate: "展开结果 · -I",
    output: "最终产物 · -O",
    copy: "复制代码",
    copied: "已复制",
    copyFailed: "无法复制，请选中文本",
  },
  model: {
    eyebrow: "EXPLICIT CONTEXT. LOCAL STATE.",
    title: "传递上下文，不泄漏局部变量。",
    intro:
      "每个物理文件拥有独立文件环境（File Frame）。可继承的 meta 与物理身份 file 分开，复用同一份内容也不会混淆它来自哪里。",
    parent: "父文件 / agent.xml",
    child: "子文件 / persona.xml",
    inherited: "父字段覆盖子默认值",
    physical: "物理身份始终不变",
    note: "openat 按每条引用边独立决定。省略始终是 self；只有连续显式 parent 边才继续传递合并后的 meta。相对路径仍从当前物理文件解析。",
    fields: [
      {
        name: "locals",
        title: "局部变量",
        body: "let 声明，set 更新。条件共享当前文件环境；引用文件不继承或导出这些变量。",
        sample: "let voice → set voice",
      },
      {
        name: "file",
        title: "物理身份",
        body: "name / path / dir 只描述当前物理文件，不能被继承元数据覆盖。",
        sample: "$file:name = persona.xml",
      },
      {
        name: "meta",
        title: "可组合的元数据",
        body: "处理指令（Processing Instruction, PI）定义字段；parent 新增缺失字段、覆盖同名字段。",
        sample: "$meta:audience = researchers",
      },
      {
        name: "sys / env",
        title: "运行内快照",
        body: "系统与环境变量只读，一次编译共享快照；这不是跨运行结果不变的承诺。",
        sample: "$sys:platform · $env:NAME",
      },
    ],
  },
  capabilities: {
    eyebrow: "SMALL LANGUAGE. USEFUL BOUNDARIES.",
    title: "组合、选择、输出，各司其职。",
    items: [
      {
        title: "带根挂载，也能只取内容",
        body: "mount 保留根，rename 仅重命名接入根；import 去掉根包装，接入其中内容。属性与子树不会被全局文本替换。",
        code: '<xmlsquish:mount path="persona.xml"\n  rename="Persona"/>\n<xmlsquish:import path="tasks.xml"/>',
      },
      {
        title: "在编译时做选择",
        body: "if / ifn 比较字符串；ifr 执行正则表达式（regular expression）匹配：str 展开变量，pattern 保持原样。未选分支不加载文件，不执行内部宏。",
        code: '<xmlsquish:ifr str="$file:name"\n  pattern="^tasks\\.xml$">\n  <task>Explain the trade-offs.</task>\n</xmlsquish:ifr>',
      },
      {
        title: "只有显式 insert 才输出变量",
        body: "普通文本里的 $name 保持字面值。insert 将变量作为转义后的 XML 文本输出，不重新解析为标签、变量或宏。",
        code: '<xmlsquish:let text="A &amp; B"/>\n<answer><xmlsquish:insert get="text"/></answer>\n<!-- result: <answer>A &amp; B</answer> -->',
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
    optimize: "编译并压紧，清理对应中间文件",
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
      "这是上方 parent 示例的真实结果。主文件只是入口，IR 才包含全部组装内容；空白优化以 IR 为基线，不输出误导的负“压缩率”。",
    source: "主源文件",
    ir: "已组装 IR",
    final: "最终提示词",
    saved: "空白优化节省",
    dependencies: "引用加载",
    files: "个不同文件",
    note: "固定 o200k_base；字节数不含 BOM。增长显示 added，-I 显示未优化。Token 大小不等于模型质量、实际账单或推理速度。",
  },
  limits: {
    title: "有边界，才可依赖。",
    body: '仅编译可信源：宏可以读取本地文件与环境变量。安全 insert 防止值变成 XML 或宏，不防提示词注入（prompt injection）。-O 会改变普通文本空白，不遵守 xml:space="preserve"。',
    details: "底层仍是那个严谨的空白状态机",
    engine: "语义编译在前，FSM 在后。",
    engineBody:
      "有限状态机（Finite-State Machine, FSM）仅规范化 XML S 空白，保留剩余 markup 内部字节。它是构建流水线的最后一层，不是整个产品。底层 squish API 仍保留独立词法契约。",
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
      "Compose multi-file XML prompts with macros, isolated file frames, and explicit metadata inheritance. Inspect the expanded IR, then ship a compact artifact.",
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
    lead: "Split files. Compose content. Pass context explicitly. xmlsquish turns maintainable XML source into a finished prompt for your agent. Compacting whitespace is only the last step.",
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
        body: "Maintain personas, tasks, and rules separately. Compose with mount / import instead of copy-paste.",
      },
      {
        label: "02 / COMPILE",
        title: "Inspect expanded XML",
        body: "Evaluate macros and conditions, merge metadata. -I stops at the intermediate representation (IR).",
      },
      {
        label: "03 / DELIVER",
        title: "Ship a compact artifact",
        body: "Default -O normalizes whitespace and writes .o.xml, removing its matching .i.xml after success.",
      },
    ],
    note: "SOURCE → SEMANTICS → ARTIFACT",
  },
  build: {
    eyebrow: "SHOW, DON'T JUST MINIFY",
    title: "Three files. One understandable build.",
    intro:
      "Read the sources, then inspect the expansion: a root becomes Persona, the parent's audience replaces a child default, and a task is imported without its wrapper. Select .i.xml / .o.xml to see the results.",
    label: "Metadata policy on this mount edge",
    parent: "parent · parent wins",
    self: "self · child stays local",
    proof:
      "Precompiled and verified with the repository's Rust CLI. Controls switch recorded results; no compiler runs in your browser.",
    resultParent: "meta:audience = researchers, from the parent.",
    resultSelf: "meta:audience = everyone, from the child.",
    tabLabel: "Example sources and build artifacts",
    noScript:
      "JavaScript is disabled: both policies and all sources and outputs are shown below.",
    source: "Source",
    intermediate: "Expanded · -I",
    output: "Final · -O",
    copy: "Copy code",
    copied: "Copied",
    copyFailed: "Copy unavailable; select the text",
  },
  model: {
    eyebrow: "EXPLICIT CONTEXT. LOCAL STATE.",
    title: "Pass context. Keep locals local.",
    intro:
      "Every physical file owns a File Frame. Inheritable meta is separate from physical file identity, so reusing content never obscures where it came from.",
    parent: "Parent / agent.xml",
    child: "Child / persona.xml",
    inherited: "Parent field overrides the child default",
    physical: "Physical identity never changes",
    note: "openat is a per-edge decision. Omission always means self; only consecutive explicit parent edges propagate merged meta. Relative paths still resolve from the physical source file.",
    fields: [
      {
        name: "locals",
        title: "Local variables",
        body: "Declare with let, update with set. Conditions share the current frame; includes neither inherit nor export locals.",
        sample: "let voice → set voice",
      },
      {
        name: "file",
        title: "Physical identity",
        body: "name / path / dir describe the physical source. Inherited metadata cannot overwrite them.",
        sample: "$file:name = persona.xml",
      },
      {
        name: "meta",
        title: "Composable metadata",
        body: "Processing instructions define fields. parent adds absent fields and wins same-name collisions.",
        sample: "$meta:audience = researchers",
      },
      {
        name: "sys / env",
        title: "Per-run snapshots",
        body: "System and environment values are read-only snapshots shared within a compilation, not a promise of identical output across runs.",
        sample: "$sys:platform · $env:NAME",
      },
    ],
  },
  capabilities: {
    eyebrow: "SMALL LANGUAGE. USEFUL BOUNDARIES.",
    title: "Compose. Select. Emit. Explicitly.",
    items: [
      {
        title: "Keep the root. Or just its contents.",
        body: "mount retains the root; rename changes only that root's name. import inserts its contents without the wrapper. No global replacement of attributes or descendants.",
        code: zh.capabilities.items[0].code,
      },
      {
        title: "Make choices at compile time.",
        body: "if / ifn compare strings. ifr matches a regular expression: str expands variables, pattern stays literal. Unselected branches load no files and execute no inner macros.",
        code: zh.capabilities.items[1].code,
      },
      {
        title: "Only insert emits a variable.",
        body: "Ordinary $name text stays literal. insert writes XML-escaped variable text, never reinterpreting it as markup, a reference, or a macro.",
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
    optimize: "Compile, compact, and clean its intermediate",
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
      "Actual numbers from the parent example above. The primary file is just an entry point; IR contains the assembled content. Whitespace savings use IR as their baseline.",
    source: "Primary source",
    ir: "Assembled IR",
    final: "Final prompt",
    saved: "Saved by whitespace optimization",
    dependencies: "Dependency loads",
    files: "unique files",
    note: "Fixed o200k_base; bytes exclude BOM. Growth is reported as added; -I skips optimization. Token size is not model quality, actual billing, or inference speed.",
  },
  limits: {
    title: "Clear boundaries. Fewer surprises.",
    body: 'Compile trusted sources only: macros can access local files and environment values. Safe insert prevents XML/macro interpretation, not prompt injection. -O changes ordinary text whitespace and does not honor xml:space="preserve".',
    details: "Under the hood: the same careful whitespace engine",
    engine: "Semantic compilation first. FSM second.",
    engineBody:
      "The finite-state machine (FSM) normalizes XML S whitespace and preserves remaining markup interiors. It is the final layer of the pipeline, not the whole product. The low-level squish API keeps its independent lexical contract.",
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
