import type { Locale } from "./messages";

/** Complete reference-page translations; never mix locales in a rendered document.
 * 参考页面的完整翻译；同一文档不混排语言。 */
const en = {
  "nav": {
    "label": "Documentation navigation",
    "namespace": "XML namespace",
    "releases": "Releases",
    "license": "License",
    "maintained": "Maintained by the xmlsquish project"
  },
  "ns": {
    "title": "xmlsquish XML namespace",
    "description": "Namespace identity, vocabulary and versioned specification resources for xmlsquish.",
    "eyebrow": "XML namespace",
    "heading": "XML is data. Macros are computation.",
    "name": "Namespace name",
    "introduction": "This namespace document describes the built-in XML vocabulary of xmlsquish. It is not an XML Schema (XSD), and is not fetched when compiling a program.",
    "identity": "Identity",
    "identityBody": "Use the exact HTTPS URI above, without a trailing slash. Prefixes such as xs are lexical aliases. Namespace names are compared as case-sensitive strings: HTTP, HTTPS, a trailing slash and a version suffix identify different namespaces. A browser redirect does not change this identity.",
    "namesBody": "Directive attributes such as name, src and get are unqualified. Macro name/ref values are prefixed QNames resolved at their lexical site. User macros must use a different namespace.",
    "resources": "Resources",
    "specification": "Current development DSL specification (Simplified Chinese)",
    "tagged": "Historical 0.2.0 specification",
    "migration": "Release notes and migration",
    "examples": "Executable examples",
    "vocabulary": "Current development vocabulary (not the 0.2.0 snapshot)",
    "element": "Element",
    "contract": "Contract",
    "elements": [
      "Declaration-only container; entry explicitly selects a named macro.",
      "Load definitions without executing a frame.",
      "Define an immutable namespace-qualified macro.",
      "Declare a required Unicode-string parameter.",
      "Recursively expand a statically resolved macro with isolated by-value inputs.",
      "Pass a literal, binding or text-only expanded body.",
      "Evaluate an XML sequence in the caller.",
      "Insert an explicitly provided XML sequence.",
      "Emit an escaped scalar as text, never markup.",
      "Match a Unicode string with lexical named captures."
    ],
    "validation": "Validation",
    "validationBody": "The compiler validates the complete static source closure, names, signatures, regex syntax and expansion results. No standalone XSD is advertised as a substitute: XML grammar alone cannot establish cross-file symbol identity, lexical captures or termination. Final .o.xml whitespace compression remains product behavior.",
    "versioning": "Version policy",
    "versionBody": "The vocabulary URI is not the executable version. This page may gain documentation and resource links; published specification snapshots remain at their versioned paths. 0.x releases may introduce breaking changes, recorded in release notes. Pin the executable tag and specification together; do not derive a namespace URI from a release number.",
    "standards": "Standards basis",
    "standardLabels": [
      "W3C Namespaces in XML 1.0 — namespace identity and QName rules",
      "W3C Web Architecture §4.5.4 — namespace documentation",
      "Semantic Versioning 2.0.0 — development-version policy"
    ]
  },
  "release": {
    "title": "xmlsquish 0.2.0 release notes",
    "description": "Installation, breaking changes, migration and resource limits for xmlsquish 0.2.0.",
    "eyebrow": "A new language for your prompts",
    "heroTitle": "Compose with clarity.",
    "heroAccent": "Build with confidence.",
    "heroBody": "Meet xmlsquish 0.2.0. Turn reusable XML modules into compact prompts—with explicit inputs, composable macros and a build you can inspect.",
    "installCta": "Get 0.2.0",
    "changesCta": "Explore what’s new",
    "sourceInstall": "Install from source",
    "sourceNote": "Rust 1.88+ · GPL-3.0-or-later · Git tag v0.2.0",
    "highlightsLabel": "Built for prompts that grow",
    "highlightsTitle": "Small pieces. One deliberate build.",
    "highlights": [
      { "number": "01", "title": "Compose, don’t duplicate", "body": "Define named macros once. Reuse them across files with stable namespace identity.", "code": "xs:import → xs:call" },
      { "number": "02", "title": "Make every input explicit", "body": "Pass text with arguments and XML with slots. No hidden caller state or accidental inheritance.", "code": "arg : String · slot : XML" },
      { "number": "03", "title": "See how your prompt was built", "body": "Inspect source locations and call frames. Keep recursive expansion under configurable budgets.", "code": "--explain · --max-depth" }
    ],
    "pipelineTitle": "From readable source to a focused prompt.",
    "pipelineSteps": ["Compose modules", "Inspect expansion", "Ship compact XML"],
    "pipelineNote": "Final whitespace compression stays. The new macro language changes how you compose—not the compact output you expect.",
    "breakingLabel": "Upgrading from 0.1?",
    "breakingBody": "0.2.0 introduces a new DSL and a CLI-only architecture. Existing templates need migration; there is no legacy compatibility layer.",
    "technicalDetails": "Release details & migration",
    "closeTitle": "Ready to build your next prompt?",
    "closeBody": "Start with the tagged release. Keep the source readable, the inputs explicit and the result compact.",
    "copy": "Copy command",
    "copied": "Copied",
    "copyFailed": "Copy failed; select the command manually",
    "dateLabel": "Release date",
    "date": "September 11, 2026",
    "tagLabel": "Git tag",
    "rustLabel": "Minimum supported Rust version",
    "github": "View on GitHub",
    "introduction": "0.2.0 implements the new XML macro language and consolidates the project into a single binary. This is a breaking language release with no compatibility layer for 0.1 syntax or the former Rust library interface.",
    "installation": "Installation",
    "installBody": "Use Rust 1.88 or newer to install from the pinned tag and lockfile.",
    "installNote": "Expected version: xmlsquish 0.2.0. Installation uses the Git tag; this release does not include a crates.io publication.",
    "migration": "Language and migration",
    "contract": "0.2.0 contract",
    "action": "Migration",
    "migrationRows": [
      [
        "Builtins use namespace identity",
        "Wrap sources in xs:module and bind the exact namespace URI below."
      ],
      [
        "Macros use namespace-qualified names",
        "Declare xs:macro name=\"app:name\" and invoke xs:call ref=\"app:name\"."
      ],
      [
        "No inherited arguments",
        "Declare xs:param and supply xs:arg; pass entry values with --arg NAME=VALUE."
      ],
      [
        "Explicit XML slots",
        "Pass node sequences with xs:fill and xs:slot, not implicit context."
      ],
      [
        "No $ text interpolation",
        "Read scalars with xs:insert get=\"arg.name\"."
      ],
      [
        "Single executable",
        "Use the CLI; the compiler is an internal module, not a public Rust library."
      ]
    ],
    "example": "Minimal program",
    "exampleNote": "Save as hello.xml, then run the command below.",
    "execution": "Execution and output",
    "outputRules": [
      "Validate the complete static source closure, including inactive branches. Imports load definitions; mounts execute module bodies. Import cycles are legal; execution recursion is budgeted.",
      "-I emits provenance-bearing .i.xml; default -O emits .o.xml. --debug and --explain retain diagnostic information without changing the final output.",
      "Final .o.xml continues to squish whitespace as insignificant formatting. This established product behavior is unchanged; text preservation during macro evaluation does not imply whitespace fidelity in the final product.",
      "Sources are not overwritten, and failed expansion does not commit partial output. Independent inputs may continue, but any failure causes a nonzero exit status."
    ],
    "limits": "Resource and security boundaries",
    "option": "Option",
    "default": "Default",
    "scope": "Scope",
    "budgetScopes": [
      "Simultaneously active macro frames",
      "Total macro frame creations",
      "Serialized bytes of final output and each temporary argument/fill sequence"
    ],
    "security": "The byte guard checks buffers separately, not aggregate allocations or all provenance-IR overhead. It is not a process-memory limit. The loader supports only file: URIs representable as native paths, rejecting other schemes, queries and fragments. Paths do not dereference symbolic links. This is not a filesystem sandbox: untrusted sources need external filesystem and process-resource isolation.",
    "verification": "Verification",
    "verifyBody": "Reproduce validation from the release source using the commands below. Refer to the release commit's CI logs for recorded results.",
    "links": "Further reading",
    "changelog": "Changelog",
    "design": "Language design and migration rationale",
    "readme": "Project documentation"
  }
};

const zh: typeof en = {
  "nav": {
    "label": "文档导航",
    "namespace": "XML 命名空间",
    "releases": "版本发布",
    "license": "许可证",
    "maintained": "由 xmlsquish 项目维护"
  },
  "ns": {
    "title": "xmlsquish XML 命名空间",
    "description": "xmlsquish 的命名空间身份、核心元素与版本规范资源。",
    "eyebrow": "XML 命名空间",
    "heading": "XML 是数据，宏是计算。",
    "name": "命名空间名称",
    "introduction": "本页是内建 XML 词汇的命名空间文档（Namespace Document），不是 XML Schema，也不是编译时需要访问的服务。",
    "identity": "身份规则",
    "identityBody": "必须使用上方精确的 HTTPS URI，不加末尾斜杠。xs 等前缀仅是词法别名。命名空间 URI 按区分大小写的字符串比较；HTTP、HTTPS、末尾斜杠和版本后缀都会形成不同身份。浏览器对文档地址的重定向不改变命名空间身份。",
    "namesBody": "name、src、get 等指令属性不带命名空间。宏名与引用是带前缀的限定名（QName），按词法位置解析。用户宏必须定义在其他命名空间中。",
    "resources": "规范资源",
    "specification": "当前开发版语言规范（简体中文）",
    "tagged": "历史 0.2.0 规范",
    "migration": "发布与迁移说明",
    "examples": "可运行示例",
    "vocabulary": "当前开发版元素（不同于 0.2.0 快照）",
    "element": "元素",
    "contract": "契约",
    "elements": [
      "仅容纳声明；entry 显式选择具名宏。",
      "装载定义，不执行调用帧。",
      "定义不可重定义的命名宏。",
      "声明必需的 Unicode 字符串参数。",
      "独立作用域按值接收输入，递归展开静态绑定的宏。",
      "传入字面量、绑定或纯文本展开主体。",
      "在调用方求值 XML 序列。",
      "插入显式传入的 XML 序列。",
      "将标量转义为文本，不重新解析为标记。",
      "匹配 Unicode 字符串，建立词法命名捕获。"
    ],
    "validation": "验证边界",
    "validationBody": "编译器验证完整静态源码闭包、名称、签名、正则语法与展开结果。XML 文法不能替代跨文件符号、词法作用域与停机行为的验证，因此本页不把 XSD 冒充完整语言验证器。最终 .o.xml 继续执行产品规定的空白压缩。",
    "versioning": "版本策略",
    "versionBody": "词汇 URI 与程序版本分离。页面可补充说明和链接，已发布的规范快照保持在固定版本路径下。0.x 版本仍可能包含破坏性变更，必须在发布说明中列出。应同时锁定程序标签与规范，不要自行给命名空间 URI 拼接版本号。",
    "standards": "标准依据",
    "standardLabels": [
      "W3C XML 命名空间 1.0：命名空间身份与限定名规则",
      "W3C Web 架构 §4.5.4：命名空间文档",
      "语义化版本 2.0.0：开发阶段的版本策略"
    ]
  },
  "release": {
    "title": "xmlsquish 0.2.0 发布说明",
    "description": "xmlsquish 0.2.0 的安装方法、破坏性变更、迁移指南及资源边界。",
    "eyebrow": "为提示词组合，带来新的语言",
    "heroTitle": "清晰地组合，",
    "heroAccent": "有依据地构建。",
    "heroBody": "认识 xmlsquish 0.2.0。把可复用的 XML 模块构建成紧凑提示词：输入显式传递，宏自由组合，展开过程可供检查。",
    "installCta": "获取 0.2.0",
    "changesCta": "看看有哪些新变化",
    "sourceInstall": "从源码安装",
    "sourceNote": "Rust 1.88+ · GPL-3.0-or-later · Git 标签 v0.2.0",
    "highlightsLabel": "为不断成长的提示词而设计",
    "highlightsTitle": "小模块，构建完整提示词。",
    "highlights": [
      { "number": "01", "title": "组合，而非重复复制", "body": "定义一次命名宏，在不同文件间复用；命名空间为每个宏提供稳定身份。", "code": "xs:import → xs:call" },
      { "number": "02", "title": "让每一份输入都明确", "body": "参数传文本，插槽传 XML。不依赖隐藏的调用方状态，也不发生意外继承。", "code": "arg : String · slot : XML" },
      { "number": "03", "title": "看清提示词如何生成", "body": "检查来源位置与调用帧，用可配置预算约束递归展开。", "code": "--explain · --max-depth" }
    ],
    "pipelineTitle": "从可读源码，到紧凑提示词。",
    "pipelineSteps": ["组合源码模块", "检查展开过程", "生成紧凑 XML"],
    "pipelineNote": "最终空白压缩保持不变。新的宏语言改变组合方式，不改变你期望的紧凑产物。",
    "breakingLabel": "正在从 0.1 升级？",
    "breakingBody": "0.2.0 引入新的 DSL，并改为纯 CLI 架构。现有模板需要迁移，不提供旧语法兼容层。",
    "technicalDetails": "发布细节与迁移指南",
    "closeTitle": "准备好构建下一份提示词了吗？",
    "closeBody": "从固定版本开始，让源码可读、输入明确、产物紧凑。",
    "copy": "复制安装命令",
    "copied": "已复制",
    "copyFailed": "复制失败，请手动选中命令",
    "dateLabel": "发布日期",
    "date": "2026 年 9 月 11 日",
    "tagLabel": "Git 标签",
    "rustLabel": "最低 Rust 版本",
    "github": "在 GitHub 查看",
    "introduction": "0.2.0 实现新的 XML 宏语言，并统一为单二进制架构。这是一次破坏性语言更新，不提供 0.1 语法或原 Rust 库接口的兼容层。",
    "installation": "安装",
    "installBody": "使用 Rust 1.88 或更高版本，从固定标签及锁文件安装。",
    "installNote": "预期版本为 xmlsquish 0.2.0。本次发布使用 Git 标签安装，不包含 crates.io 发布。",
    "migration": "语言与迁移",
    "contract": "0.2.0 契约",
    "action": "迁移操作",
    "migrationRows": [
      [
        "内建操作按命名空间身份识别",
        "将源码包在 xs:module 中，并绑定下方精确 URI。"
      ],
      [
        "宏使用命名空间限定名",
        "用 xs:macro name=\"app:name\" 声明，用 xs:call ref=\"app:name\" 调用。"
      ],
      [
        "参数不继承",
        "用 xs:param 声明、xs:arg 传入；入口使用 --arg NAME=VALUE。"
      ],
      [
        "XML 插槽显式传递",
        "用 xs:fill 和 xs:slot 传递节点序列，而不是依赖隐式上下文。"
      ],
      [
        "普通文本无 $ 插值",
        "用 xs:insert get=\"arg.name\" 读取标量。"
      ],
      [
        "单二进制程序",
        "通过 CLI 使用；编译器是内部模块，不再提供公共 Rust 库。"
      ]
    ],
    "example": "最小程序",
    "exampleNote": "保存为 hello.xml 后运行下方命令。",
    "execution": "执行与输出",
    "outputRules": [
      "验证完整静态源码闭包，包括未选中的分支。导入只装载定义，挂载执行模块主体。纯导入环合法，执行递归受预算约束。",
      "-I 输出带来源信息（Provenance）的 .i.xml，默认 -O 输出 .o.xml。--debug 与 --explain 保留诊断信息，但不改变最终输出。",
      "最终 .o.xml 继续压缩空白，空白视为无意义的格式内容。这是既有产品语义，本版本保持不变；宏求值时保留文本，不代表最终产物保留空白。",
      "不覆盖输入源码，失败的展开不提交部分输出。独立输入可以继续处理，但任一失败都会使命令返回非零退出码。"
    ],
    "limits": "资源与安全边界",
    "option": "选项",
    "default": "默认值",
    "scope": "范围",
    "budgetScopes": [
      "同时活动的宏展开帧",
      "宏帧创建总数",
      "最终输出与每个临时 argument/fill 序列的序列化字节数"
    ],
    "security": "字节预算分别检查缓冲区，不是全部活动分配量之和，也不包含所有来源中间表示（Intermediate Representation, IR）的开销，因此不是进程总内存限制。加载器仅支持可表示为本机路径的 file: URI，拒绝其他协议、查询参数和片段；路径规范化不解引用符号链接。这不是文件访问沙箱，不可信源码需要外部文件权限与进程资源隔离。",
    "verification": "验证",
    "verifyBody": "可在发布源码中运行下方命令复现检查；已记录的运行结果请查看发布提交的 CI 日志。",
    "links": "延伸阅读",
    "changelog": "更新日志",
    "design": "语言设计与迁移依据",
    "readme": "项目文档"
  }
};

export type ReferenceMessages = typeof en;
export const referenceMessages: Record<Locale, ReferenceMessages> = { "zh-CN": zh, en };
