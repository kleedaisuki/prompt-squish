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
    "description": "Namespace identity and XML frontend vocabulary within the xmlsquish project manager.",
    "eyebrow": "XML namespace",
    "heading": "A stable XML vocabulary inside a project build.",
    "name": "Namespace name",
    "introduction": "This document describes the XML frontend vocabulary used by project targets. The namespace is not an XSD and is never fetched during a build; project discovery, dependencies, caching, linking, and artifacts are manager concerns.",
    "identity": "Identity",
    "identityBody": "Use the exact HTTPS URI above, without a trailing slash. Prefixes such as xs are lexical aliases. Namespace names are compared as case-sensitive strings: HTTP, HTTPS, a trailing slash and a version suffix identify different namespaces. A browser redirect does not change this identity.",
    "namesBody": "Directive attributes such as name, src and get are unqualified. Macro name/ref values are prefixed QNames resolved locally. Import loads definitions, not prefix bindings: declare your chosen prefix with the same namespace URI. User macros must use a different namespace.",
    "resources": "Resources",
    "specification": "Current working DSL specification (Simplified Chinese)",
    "tagged": "Historical specification snapshot at v0.3.0",
    "migration": "Release notes and migration",
    "examples": "Current runnable project examples",
    "vocabulary": "Current XML frontend vocabulary",
    "element": "Element",
    "contract": "Contract",
    "elements": [
      "Library containing imports and named macro definitions only.",
      "Independent build document: imports modules and constructs the final prompt.",
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
    "validationBody": "The build frontend validates the frozen source closure and lowers modules to canonical binary XSIR. The linker resolves imports and relocations before an entry is instantiated. XSD alone cannot establish cross-file symbols, scopes, or termination; final products are .prompt files, with optional .xsir and .psdbg companions.",
    "versioning": "Version policy",
    "versionBody": "The vocabulary URI is not the executable version. This page may gain documentation and resource links; published specification snapshots remain at their versioned paths. 0.x releases may introduce breaking changes, recorded in release notes. Pin the executable tag and specification together; do not derive a namespace URI from a release number.",
    "standards": "Standards basis",
    "standardLabels": [
      "W3C Namespaces in XML 1.0 — namespace identity and QName rules",
      "W3C Web Architecture §4.5.4 — namespace documentation",
      "Semantic Versioning 2.0.0 — development-version policy"
    ],
    "history": "Historical 0.2.0 specification"
  },
  "release": {
    "title": "xmlsquish v1.0.0 — Prompt project manager",
    "description": "xmlsquish v1.0.0 turns the XML Prompt compiler into a Cargo-like project manager with creation, formatting, builds, dependencies, workspaces, and inspectable artifacts.",
    "date": "September 15, 2026",
    "heroTitle": "Your prompts are projects.",
    "heroAccent": "Manage them like code.",
    "heroBody": "xmlsquish v1.0.0 is a local-first, deterministic project manager for XML Prompts. Create a project, manage dependencies, build linked artifacts, and inspect how every result was made.",
    "releaseActions": "Release actions",
    "installCta": "Get v1.0.0",
    "exploreCta": "See the workflow",
    "platformLine": "Available now · Windows, Linux, and macOS · No Rust toolchain required",
    "pipelineTitle": "The v1 product model",
    "pipeline": [
      { "title": "Project", "body": "Workspace, manifest, XML sources, and locked dependencies" },
      { "title": "XSIR", "body": "Validated, canonical binary intermediate representation" },
      { "title": "Link", "body": "Resolve imports, symbols, and relocations" },
      { "title": "Artifacts", "body": ".prompt output with optional .xsir and .psdbg evidence" }
    ],
    "journeyLabel": "One tool, the whole local loop",
    "journeyTitle": "Six commands from idea to evidence.",
    "journeyBody": "Each command enters the same typed manager, scheduler, cancellation, and event pipeline—no hidden second implementation in the CLI.",
    "journey": [
      { "command": "new support", "title": "Create", "body": "Start a complete project and optionally register it in the enclosing workspace." },
      { "command": "fmt", "title": "Format", "body": "Canonicalize project XML without changing its meaning." },
      { "command": "add common --path ../common", "title": "Add", "body": "Resolve and record a real local project dependency; registry or Git sources may be used instead." },
      { "command": "remove common", "title": "Remove", "body": "Remove that declared dependency while keeping project state coherent." },
      { "command": "build", "title": "Build", "body": "Lower, link, and publish deterministic Prompt artifacts." },
      { "command": "inspect artifact target/xmlsquish/prompt.prompt", "title": "Inspect", "body": "Trace a concrete artifact, its inputs, and provenance without rebuilding." }
    ],
    "quickstartLabel": "Runnable quickstart",
    "quickstartTitle": "Create, verify, build, inspect.",
    "quickstartBody": "This minimal project needs no Git repository or network dependency. Run the five commands in order.",
    "featuresLabel": "Project management, not command accumulation",
    "featuresTitle": "A coherent manager around the compiler.",
    "features": [
      { "code": "new → commit", "title": "Transactional project creation", "body": "Stage and validate before an exclusive publish. Interrupted operations recover without exposing a half-written project." },
      { "code": "workspace + lock", "title": "Workspace and dependencies", "body": "Discover package context, inherit configuration, register members, and keep dependency decisions reproducible." },
      { "code": "prompt + xsir + psdbg", "title": "Inspectable artifacts", "body": "Ship the Prompt while retaining optional binary IR and provenance evidence for tools and debugging." }
    ],
    "outputLabel": "Humans and automation share one truth",
    "outputTitle": "One event stream, three renderers.",
    "outputBody": "Output format changes presentation, never execution semantics. Terminals, scripts, IDEs, and robot agents observe the same ordered operation.",
    "outputs": [
      { "name": "human", "body": "Readable terminal diagnostics and progress." },
      { "name": "short", "body": "Compact, stable lines for logs and shell tools." },
      { "name": "ndjson", "body": "Structured newline-delimited JSON for IDEs and agents." }
    ],
    "downloadsTitle": "Native v1.0.0 builds",
    "downloadsBody": "Choose your operating system and CPU architecture. Each v1.0.0 archive contains the executable and license; no Rust toolchain is required.",
    "platformNotes": ["Windows 10/11 · ZIP", "glibc 2.35+ · tar.gz", "macOS 11+ · tar.gz"],
    "checksums": "SHA-256 checksum manifest",
    "binaryNote": "Native archives are unsigned and not notarized. SHA-256 verifies integrity, not publisher identity.",
    "sourceLabel": "Build from source",
    "sourceTitle": "Install the tagged v1.0.0 source.",
    "sourceBody": "Rust 1.88 or newer is required. The lockfile and v1.0.0 tag pin the complete source installation.",
    "github": "View the v1.0.0 release on GitHub",
    "migrationLabel": "Moving from the compiler-era CLI",
    "migrationTitle": "Adopt the project model deliberately.",
    "migrationBody": "v1.0.0 preserves the XML language work while changing the primary user model from individual compilation invocations to managed projects.",
    "migrationRows": [
      { "title": "Create a package", "body": "Use xmlsquish new PATH for new work. It produces a manifest and src/prompt.xml that format and build offline immediately." },
      { "title": "Build from project context", "body": "Run fmt and build inside the package or workspace. Treat target output as derived state, not source." },
      { "title": "Manage dependencies", "body": "Use add and remove instead of editing resolved state by hand; commit the manifest and lockfile decisions." },
      { "title": "Integrate structured output", "body": "Automation should consume NDJSON rather than parse decorated human output." }
    ],
    "boundaryTitle": "Durability boundary",
    "boundaryBody": "Controlled process-kill recovery is tested across creation commit boundaries. v1.0.0 does not claim proof against sudden Windows power loss, storage-controller cache loss, or arbitrary remote filesystems. Filesystems without exclusive atomic rename support are rejected rather than given a racy fallback.",
    "closeTitle": "Build prompts as durable projects.",
    "closeBody": "Start with new, keep dependencies explicit, and ship artifacts whose origin you can inspect."
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
    "description": "xmlsquish 项目管理器中的命名空间身份与 XML 前端词汇。",
    "eyebrow": "XML 命名空间",
    "heading": "稳定的 XML 词汇，服务于项目级构建。",
    "name": "命名空间名称",
    "introduction": "本页描述项目目标使用的 XML 前端词汇。它不是 XML Schema，构建也不会联网获取它；项目发现、依赖、缓存、链接和产物由项目管理器负责。",
    "identity": "身份规则",
    "identityBody": "必须使用上方精确的 HTTPS URI，不加末尾斜杠。xs 等前缀仅是词法别名。命名空间 URI 按区分大小写的字符串比较；HTTP、HTTPS、末尾斜杠和版本后缀都会形成不同身份。浏览器对文档地址的重定向不改变命名空间身份。",
    "namesBody": "name、src、get 等指令属性不带命名空间。宏名与引用使用本地解析的限定名（QName）。import 只导入定义，不继承前缀绑定；引用方须将自选前缀绑定到相同 URI。用户宏须使用其他命名空间。",
    "resources": "规范资源",
    "specification": "当前工作版语言规范（简体中文）",
    "tagged": "v0.3.0 历史规范快照",
    "migration": "发布与迁移说明",
    "examples": "当前可运行的项目示例",
    "vocabulary": "当前 XML 前端核心元素",
    "element": "元素",
    "contract": "契约",
    "elements": [
      "宏库，仅包含 import 与具名宏定义。",
      "独立构建文档：导入模块并组织最终提示词。",
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
    "validationBody": "构建前端验证冻结的源码闭包并将模块降低为规范二进制 XSIR；链接器随后解析导入与重定位，再实例化入口。XSD 无法验证跨文件符号、作用域或停机行为；最终产品是 .prompt，并可附带 .xsir 与 .psdbg。",
    "versioning": "版本策略",
    "versionBody": "词汇 URI 与程序版本分离。页面可补充说明和链接，已发布的规范快照保持在固定版本路径下。0.x 版本仍可能包含破坏性变更，必须在发布说明中列出。应同时锁定程序标签与规范，不要自行给命名空间 URI 拼接版本号。",
    "standards": "标准依据",
    "standardLabels": [
      "W3C XML 命名空间 1.0：命名空间身份与限定名规则",
      "W3C Web 架构 §4.5.4：命名空间文档",
      "语义化版本 2.0.0：开发阶段的版本策略"
    ],
    "history": "历史 0.2.0 规范"
  },
  "release": {
    "title": "xmlsquish v1.0.0 — Prompt 项目管理器",
    "description": "xmlsquish v1.0.0 将 XML Prompt 编译器升级为 Cargo 式项目管理器，统一提供创建、格式化、构建、依赖、工作区与可检查产物。",
    "date": "2026 年 9 月 15 日",
    "heroTitle": "提示词，也是项目。",
    "heroAccent": "像代码一样管理它。",
    "heroBody": "xmlsquish v1.0.0 是本地优先、确定性的 XML Prompt 项目管理器。创建项目、管理依赖、构建链接后的产物，并检查每个结果如何生成。",
    "releaseActions": "发布操作",
    "installCta": "获取 v1.0.0",
    "exploreCta": "查看工作流",
    "platformLine": "现已发布 · Windows、Linux 与 macOS · 无需 Rust 工具链",
    "pipelineTitle": "v1 产品模型",
    "pipeline": [
      { "title": "项目", "body": "工作区、清单、XML 源码与锁定依赖" },
      { "title": "XSIR", "body": "经过验证的规范二进制中间表示" },
      { "title": "链接", "body": "解析导入、符号与重定位" },
      { "title": "产物", "body": ".prompt 输出，以及可选 .xsir 与 .psdbg 证据" }
    ],
    "journeyLabel": "一个工具，覆盖完整本地循环",
    "journeyTitle": "六个命令，从想法走到证据。",
    "journeyBody": "每条命令都进入同一个类型化管理器、调度、取消与事件管线；CLI 中没有隐藏的第二套实现。",
    "journey": [
      { "command": "new support", "title": "创建", "body": "创建完整项目，并可登记到外围工作区。" },
      { "command": "fmt", "title": "格式化", "body": "规范化项目 XML，不改变语义。" },
      { "command": "add common --path ../common", "title": "添加", "body": "解析并记录真实的本地项目依赖；也可改用 Registry 或 Git 来源。" },
      { "command": "remove common", "title": "移除", "body": "移除这个已声明依赖，同时保持项目状态一致。" },
      { "command": "build", "title": "构建", "body": "降低、链接并发布确定性的 Prompt 产物。" },
      { "command": "inspect artifact target/xmlsquish/prompt.prompt", "title": "检查", "body": "无需重建即可追踪具体产物、输入与来源。" }
    ],
    "quickstartLabel": "可执行快速开始",
    "quickstartTitle": "创建、验证、构建、检查。",
    "quickstartBody": "这个最小项目不需要 Git 仓库或网络依赖；请依次运行五条命令。",
    "featuresLabel": "项目管理，不是命令堆积",
    "featuresTitle": "编译器之外，是一个完整管理器。",
    "features": [
      { "code": "new → commit", "title": "事务式项目创建", "body": "先暂存和验证，再排他发布；操作中断后可以恢复，不暴露只写了一半的项目。" },
      { "code": "workspace + lock", "title": "工作区与依赖", "body": "发现包上下文、继承配置、登记成员，并让依赖选择可复现。" },
      { "code": "prompt + xsir + psdbg", "title": "可检查产物", "body": "交付 Prompt，同时可保留二进制 IR 与来源证据，服务于工具和调试。" }
    ],
    "outputLabel": "人类与自动化共享同一事实",
    "outputTitle": "一条事件流，三种渲染器。",
    "outputBody": "输出格式只改变呈现，不改变执行语义。终端、脚本、IDE 与 robot agent 观察同一个有序操作。",
    "outputs": [
      { "name": "human", "body": "面向终端的可读诊断与进度。" },
      { "name": "short", "body": "面向日志与 Shell 工具的紧凑稳定行。" },
      { "name": "ndjson", "body": "面向 IDE 与 Agent 的换行分隔结构化 JSON。" }
    ],
    "downloadsTitle": "v1.0.0 原生构建",
    "downloadsBody": "选择操作系统与 CPU 架构。每个 v1.0.0 压缩包都包含可执行文件与许可证，无需 Rust 工具链。",
    "platformNotes": ["Windows 10/11 · ZIP", "glibc 2.35+ · tar.gz", "macOS 11+ · tar.gz"],
    "checksums": "SHA-256 校验清单",
    "binaryNote": "原生压缩包未签名、未经 Apple 公证；SHA-256 校验完整性，不证明发布者身份。",
    "sourceLabel": "从源码构建",
    "sourceTitle": "安装已标记的 v1.0.0 源码。",
    "sourceBody": "需要 Rust 1.88 或更高版本；锁文件与 v1.0.0 标签共同固定完整源码安装。",
    "github": "在 GitHub 查看 v1.0.0 发布",
    "migrationLabel": "从编译器时代的 CLI 迁移",
    "migrationTitle": "有意识地采用项目模型。",
    "migrationBody": "v1.0.0 延续 XML 语言能力，同时把主要用户模型从单次文件编译升级为受管理项目。",
    "migrationRows": [
      { "title": "创建包", "body": "新项目使用 xmlsquish new PATH；生成的清单与 src/prompt.xml 可立即离线格式化和构建。" },
      { "title": "在项目上下文构建", "body": "在包或工作区中运行 fmt 与 build；target 输出是派生状态，不是源码。" },
      { "title": "管理依赖", "body": "使用 add 与 remove，而不是手工编辑解析状态；提交清单与锁文件中的选择。" },
      { "title": "接入结构化输出", "body": "自动化应消费 NDJSON，不要解析带装饰的人类输出。" }
    ],
    "boundaryTitle": "持久性边界",
    "boundaryBody": "我们测试了项目创建提交边界上的受控进程终止恢复。v1.0.0 不宣称已证明 Windows 突然断电、存储控制器缓存丢失或任意远程文件系统上的持久性；不支持排他原子重命名的文件系统会被拒绝，而不是使用有竞争条件的回退。",
    "closeTitle": "把提示词构建成持久项目。",
    "closeBody": "从 new 开始，保持依赖显式，交付来源可检查的产物。"
  }
};

export type ReferenceMessages = typeof en;
export const referenceMessages: Record<Locale, ReferenceMessages> = { "zh-CN": zh, en };
