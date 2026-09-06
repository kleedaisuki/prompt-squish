export const locales = ["zh-CN", "en"] as const;
export type Locale = (typeof locales)[number];

type Feature = { title: string; body: string; eyebrow: string };
type State = { code: string; name: string; body: string };

type Messages = {
  meta: { title: string; description: string };
  nav: { how: string; cli: string; stats: string; github: string; language: string };
  hero: { badge: string; title: string; lead: string; primary: string; secondary: string; footnote: string; warning: string };
  transform: { before: string; after: string; caption: string };
  principles: { eyebrow: string; title: string; intro: string; items: Feature[] };
  fsm: { eyebrow: string; title: string; intro: string; states: State[]; transition: string };
  cli: { eyebrow: string; title: string; intro: string; install: string; run: string; note: string };
  stats: { eyebrow: string; title: string; intro: string; labels: string[]; values: string[]; note: string };
  footer: { tagline: string; source: string; style: string; license: string; notices: string };
  theme: { label: string; auto: string; light: string; dark: string };
};

const zh = {
  meta: {
    title: "xmlsquish — 为 Agent 压紧 XML 提示词",
    description: "先编译 XML 提示词宏，再用 Rust 有限状态机压缩布局空白。",
  },
  nav: { how: "原理", cli: "命令行", stats: "统计", github: "GitHub", language: "English" },
  hero: {
    badge: "Rust 2024 · 语义编译 + FSM",
    title: "可读地写，紧凑地喂给 Agent。",
    lead: "xmlsquish 递归处理 XML 提示词，压缩布局空白，并在标签与标签、标签与单词、单词与标签之间保留且只保留一个空格。",
    primary: "查看命令行用法",
    secondary: "理解状态机",
    footnote: "输入保持不变；-I 生成 .i.xml，默认 -O 生成 .o.xml。",
    warning: "它不是保持 XML 语义的通用压缩器：会改写普通字符数据中的空白，可能改变混合内容，且不遵守 xml:space=\"preserve\"。请只处理已确认空白是布局噪声的提示词。",
  },
  transform: {
    before: "<role>\n  You are a careful agent.\n</role>\n<task>\n  Summarize this.\n</task>",
    after: "<role> You are a careful agent. </role> <task> Summarize this. </task>",
    caption: "回车、换行、制表符与空格归并为词法单元边界上的一个空格。",
  },
  principles: {
    eyebrow: "做得少，做得准",
    title: "为提示词管线而设计",
    intro: "先消除编译期宏、注释与元信息，再规范化普通字符数据的空白；它不是 XML 信息集压缩器。",
    items: [
      { eyebrow: "确定性", title: "单遍扫描", body: "空白压缩逐字符推进；宏编译在一次运行中共享系统与环境快照，不改写普通内容中的变量。" },
      { eyebrow: "批处理", title: "路径、目录与通配符", body: "一次接收多个路径；目录递归发现 *.xml，通配符匹配你的现有工作流。" },
      { eyebrow: "可追踪", title: "输入从不覆盖", body: "每份结果写到对应的 *.o.xml，原始提示词仍是可读、可审阅的事实来源。" },
    ],
  },
  fsm: {
    eyebrow: "机制，而非魔法",
    title: "先形成 atom，再统一发射",
    intro: "扫描器把普通字符数据形成 Word atom，把完整 markup 形成 Markup atom；连续空白延迟为候选分隔符，再由相邻 atom 统一决定输出。",
    states: [
      { code: "DATA", name: "字符数据", body: "形成 Word atom，并暂存 XML S 空白游程。" },
      { code: "TAG", name: "标签", body: "属性引号中的 > 不会提前结束标签。" },
      { code: "COMMENT / CDATA / PI", name: "定界结构", body: "识别各自的结束序列，内部字节原样保留。" },
      { code: "DOCTYPE", name: "文档类型", body: "跟踪引号、注释、PI 与内部子集方括号深度。" },
    ],
    transition: "边界规则：TAG ↔ TAG、TAG ↔ WORD 以及 WORD ↔ TAG，均输出恰好一个空格。",
  },
  cli: {
    eyebrow: "零仪式批处理",
    title: "把路径交给它",
    intro: "传入任意数量的文件、目录或通配符。没有参数时，xmlsquish 会打印帮助信息。",
    install: "cargo install --path crates/xmlsquish-cli --locked",
    run: "xmlsquish ./prompts \"templates/*.xml\"",
    note: "目录递归查找 *.xml，跳过 *.i.xml 与 *.o.xml。-I 只编译；默认 -O 压缩并清理对应中间文件。",
  },
  stats: {
    eyebrow: "每次运行都有账",
    title: "看见真正省下的内容",
    intro: "完成后汇总文件、token、字符与空白，让压缩效果可验证而非凭感觉。",
    labels: ["Tokenizer 编码", "处理文件数", "输入 token", "输出 token", "输入字符", "输出字符", "识别到的空白", "移除的空白", "插入的空白", "Token 压缩率"],
    values: ["o200k_base", "1", "12", "9", "18", "15", "6", "4", "1", "25.00%"],
    note: "示例数据仅用于展示统计界面；实际数值由你的文件决定。",
  },
  footer: {
    tagline: "让 XML 对人友好，对 Agent 也克制。",
    source: "查看源码",
    style: "Built with MoeSegfault Style",
    license: "许可",
    notices: "第三方声明",
  },
  theme: { label: "外观", auto: "跟随系统", light: "浅色", dark: "深色" },
} satisfies Messages;

const en: Messages = {
  meta: {
    title: "xmlsquish — Compact XML prompts for agents",
    description: "Compile XML prompt macros, then compact layout whitespace with a Rust finite-state machine.",
  },
  nav: { how: "How it works", cli: "CLI", stats: "Stats", github: "GitHub", language: "中文" },
  hero: {
    badge: "Rust 2024 · compiler + FSM",
    title: "Write for humans. Feed agents less.",
    lead: "xmlsquish recursively processes XML prompts, collapsing layout whitespace while keeping exactly one space between tag/tag, tag/word, and word/tag boundaries.",
    primary: "See the CLI",
    secondary: "Explore the FSM",
    footnote: "Sources stay untouched; -I writes .i.xml, while default -O writes .o.xml.",
    warning: "This is not a semantics-preserving XML minifier. It rewrites ordinary character-data whitespace, may change mixed-content meaning, and does not honor xml:space=\"preserve\". Use it only where that whitespace is known layout noise.",
  },
  transform: {
    before: "<role>\n  You are a careful agent.\n</role>\n<task>\n  Summarize this.\n</task>",
    after: "<role> You are a careful agent. </role> <task> Summarize this. </task>",
    caption: "Carriage returns, newlines, tabs, and spaces collapse to one separator at lexical-unit boundaries.",
  },
  principles: {
    eyebrow: "Do less, precisely",
    title: "Made for prompt pipelines",
    intro: "Compile away macros, comments, and metadata before normalizing ordinary text whitespace. This is not an XML Infoset minifier.",
    items: [
      { eyebrow: "Deterministic", title: "One-pass scanning", body: "The squasher advances character by character. Compilation shares system/environment snapshots within a run and leaves payload variables literal." },
      { eyebrow: "Batch-ready", title: "Paths, folders, and globs", body: "Pass many paths at once. Directories discover *.xml recursively; globs fit into existing workflows." },
      { eyebrow: "Traceable", title: "Inputs are never replaced", body: "Each result lands in a matching *.o.xml file, leaving readable source prompts as the reviewable source of truth." },
    ],
  },
  fsm: {
    eyebrow: "Mechanism, not magic",
    title: "Form atoms, then emit once",
    intro: "The scanner forms Word atoms from ordinary character data and complete Markup atoms from structures. It defers whitespace runs, then one emitter joins adjacent atoms.",
    states: [
      { code: "DATA", name: "Character data", body: "Form Word atoms and defer XML S whitespace runs." },
      { code: "TAG", name: "Tag", body: "A > inside a quoted attribute does not end the tag." },
      { code: "COMMENT / CDATA / PI", name: "Delimited structures", body: "Recognize each terminator and preserve interior bytes verbatim." },
      { code: "DOCTYPE", name: "Document type", body: "Track quotes, comments, PIs, and internal-subset bracket depth." },
    ],
    transition: "Boundary rule: TAG ↔ TAG, TAG ↔ WORD, and WORD ↔ TAG all emit exactly one space.",
  },
  cli: {
    eyebrow: "Zero-ceremony batching",
    title: "Give it paths",
    intro: "Pass any number of files, directories, or globs. With no arguments, xmlsquish prints its help.",
    install: "cargo install --path crates/xmlsquish-cli --locked",
    run: "xmlsquish ./prompts \"templates/*.xml\"",
    note: "Directories recursively find *.xml, skipping *.i.xml and *.o.xml. -I compiles only; default -O compresses and cleans its intermediate.",
  },
  stats: {
    eyebrow: "Account for every run",
    title: "See what you actually saved",
    intro: "A summary covers files, tokens, characters, and whitespace, so compression stays measurable rather than anecdotal.",
    labels: ["Tokenizer encoding", "Files processed", "Input tokens", "Output tokens", "Input characters", "Output characters", "Recognized whitespace", "Removed whitespace", "Inserted whitespace", "Token compression rate"],
    values: ["o200k_base", "1", "12", "9", "18", "15", "6", "4", "1", "25.00%"],
    note: "Illustrative values only; actual results depend on your files.",
  },
  footer: {
    tagline: "Friendly to authors. Frugal for agents.",
    source: "View source",
    style: "Built with MoeSegfault Style",
    license: "License",
    notices: "Third-party notices",
  },
  theme: { label: "Theme", auto: "System", light: "Light", dark: "Dark" },
};

export const messages: Record<Locale, Messages> = { "zh-CN": zh, en };
