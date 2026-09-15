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
    "title": "xmlsquish 0.3.0 release notes",
    "description": "Installation, breaking changes, migration and resource limits for xmlsquish 0.3.0.",
    "eyebrow": "A new language for your prompts",
    "heroTitle": "Macros for reuse.",
    "heroAccent": "Entries for building.",
    "heroBody": "Meet xmlsquish 0.3.0. Keep reusable macros in libraries, compose each prompt in an explicit entry, and ship only the structure and text your agent needs.",
    "installCta": "Get 0.3.0",
    "changesCta": "Explore what’s new",
    "sourceInstall": "Prefer to build from source?",
    "downloadsTitle": "Download. Extract. Run.",
    "downloadsBody": "Choose your operating system and CPU architecture. Each archive includes the executable and license. No Rust toolchain required.",
    "download": "Download",
    "platformNotes": ["Windows 10/11 · ZIP", "glibc 2.35+ · tar.gz", "macOS 11+ · tar.gz"],
    "checksum": "SHA-256 checksums",
    "downloadHelp": "Extract the archive, run ./xmlsquish --version (Windows: .\\xmlsquish.exe --version), then add its folder to PATH to use it anywhere.",
    "unsignedNote": "Binaries are not code-signed or notarized. SHA-256 verifies download integrity, not publisher identity. Your OS may ask you to approve an unrecognized executable.",
    "sourceNote": "Windows · Linux · macOS · No Rust installation needed",
    "highlightsLabel": "Built for prompts that grow",
    "highlightsTitle": "Clear boundaries. Less repeated work.",
    "highlights": [
      {
        "number": "01",
        "title": "Libraries are not entry points",
        "body": "Define macros in modules. Import them into a separate entry and expand exactly what you need. No implicit main.",
        "code": "module → import → entry"
      },
      {
        "number": "02",
        "title": "Keep the prompt, drop the metadata",
        "body": "Final XML keeps structure and text, removes every attribute and namespace declaration, and compresses formatting whitespace.",
        "code": ".o.xml = structure + text"
      },
      {
        "number": "03",
        "title": "Reuse work, not execution state",
        "body": "Shared static IR payloads avoid repeated serialization. Recursive frames, explicit inputs and diagnostic origins remain independent.",
        "code": "prepare once · expand again"
      }
    ],
    "pipelineTitle": "From readable source to a focused prompt.",
    "pipelineSteps": [
      "Compose an entry",
      "Inspect provenance",
      "Ship compact XML"
    ],
    "pipelineNote": "Macros still recurse and return values. Arguments stay isolated and explicit. Metadata belongs in the diagnostic IR—not in the prompt you send.",
    "breakingLabel": "Upgrading from 0.2?",
    "breakingBody": "0.3.0 separates entry documents from macro libraries and replaces mount/call with expand. Migrate your sources; old syntax and module entry attributes are not supported.",
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
    "introduction": "0.3.0 introduces separate xs:entry and xs:module sources, one import operation and recursive xs:expand. There is no implicit main, module entry selector or fragment construct. The compiler remains an internal module of the CLI binary.",
    "installation": "Installation",
    "installBody": "Use Rust 1.88 or newer to install from the pinned tag and lockfile.",
    "installNote": "Expected version: xmlsquish 0.3.0. This optional source installation requires Rust 1.88+. Not published on crates.io.",
    "migration": "Language and migration",
    "contract": "0.3.0 contract",
    "action": "Migration",
    "migrationRows": [
      [
        "Separate libraries and builds",
        "Use xs:entry for imports, input parameters and output construction. Keep macro definitions in xs:module files."
      ],
      [
        "One loading and one expansion operation",
        "Replace call with expand. Replace mount with import plus a named macro expansion. An entry cannot be imported."
      ],
      [
        "Explicit inputs and returned values",
        "Pass Unicode text with arg and node sequences with fill/slot. Recursive results compose without capturing caller variables."
      ],
      [
        "Local namespace bindings",
        "Import definitions, then bind a local prefix to the macro namespace URI. Prefix spellings need not match across files."
      ],
      [
        "Attribute-free final prompts",
        "All attributes, namespace declarations and element prefixes are removed from .o.xml. Move meaningful attribute content into text elements."
      ],
      [
        "Reusable compiler snapshots",
        "Internal prepare/expand APIs reuse frozen sources and static event payloads. Reprepare to observe source edits; no global cache or new CLI flag."
      ]
    ],
    "example": "Minimal program",
    "exampleNote": "Save as hello.xml, then run the command below.",
    "execution": "Execution and output",
    "outputRules": [
      "The complete import closure is frozen, validated and linked before entry execution. Imports only load modules; expand only targets named macros. Module import cycles are legal, recursive expansion is budgeted.",
      "-I emits provenance-bearing .i.xml; default -O emits clean .o.xml. --debug and --explain retain diagnostics without changing the final prompt.",
      "Final .o.xml removes all attributes, namespace declarations and element prefixes, then squishes whitespace. Intermediate diagnostics preserve origin and expansion frames.",
      "Sources are not overwritten. Failed expansion publishes no partial result. Directory/glob builds skip valid library modules; explicitly compiling a module is an error."
    ],
    "limits": "Resource and security boundaries",
    "option": "Option",
    "default": "Default",
    "scope": "Scope",
    "budgetScopes": [
      "Active execution frames, including one entry frame",
      "Total execution frames, including one entry frame",
      "Serialized final-output bytes and each temporary argument/fill buffer"
    ],
    "security": "The byte guard checks buffers separately, not aggregate allocations or all provenance-IR overhead. It is not a process-memory limit. The loader supports only file: URIs representable as native paths, rejecting other schemes, queries and fragments. Paths do not dereference symbolic links. This is not a filesystem sandbox: untrusted sources need external filesystem and process-resource isolation.",
    "verification": "Verification",
    "verifyBody": "Reproduce validation from the release source using the commands below. Refer to the release commit's CI logs for recorded results.",
    "links": "Further reading",
    "changelog": "Changelog",
    "design": "Language design and migration rationale",
    "readme": "Project documentation",
    "performance": "Performance, with context",
    "performanceBody": "Seven paired release-build microbenchmarks reduced full in-memory compilation time by 17–81%. This includes parsing, expansion and IR serialization, but excludes CLI startup, token counting and file I/O. The small GSP CLI build was effectively unchanged within run-to-run noise. Some preparation-only cases got slower; cached payloads remain in memory until the snapshot is dropped.",
    "performanceLink": "Inspect workloads, raw samples and trade-offs",
    "history": "Previous release: 0.2.0"
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
    "title": "xmlsquish 0.3.0 发布说明",
    "description": "xmlsquish 0.3.0 的安装方法、破坏性变更、迁移指南及资源边界。",
    "eyebrow": "为提示词组合，带来新的语言",
    "heroTitle": "宏，负责复用。",
    "heroAccent": "入口，负责构建。",
    "heroBody": "认识 xmlsquish 0.3.0。宏库维护可复用定义，独立入口组织每一份提示词；交给 Agent 的，只留下需要的结构与文本。",
    "installCta": "获取 0.3.0",
    "changesCta": "看看有哪些新变化",
    "sourceInstall": "也可以自行编译",
    "downloadsTitle": "下载，解压，直接运行。",
    "downloadsBody": "选择操作系统和 CPU 架构。压缩包包含可执行文件与许可证，无需安装 Rust 工具链。",
    "download": "下载",
    "platformNotes": ["Windows 10/11 · ZIP", "glibc 2.35+ · tar.gz", "macOS 11+ · tar.gz"],
    "checksum": "SHA-256 校验文件",
    "downloadHelp": "解压后运行 ./xmlsquish --version（Windows：.\\xmlsquish.exe --version）；将所在目录加入 PATH，即可在任意位置使用。",
    "unsignedNote": "二进制尚未做代码签名或 macOS 公证。SHA-256 用于校验下载完整性，不证明发布者身份；系统可能提示你确认运行未知来源程序。",
    "sourceNote": "Windows · Linux · macOS · 无需安装 Rust",
    "highlightsLabel": "为不断成长的提示词而设计",
    "highlightsTitle": "职责更清楚，重复工作更少。",
    "highlights": [
      {
        "number": "01",
        "title": "宏库不是程序入口",
        "body": "module 定义宏，独立 entry 导入并展开所需内容。没有隐式 main，也不把文件偷偷当作宏。",
        "code": "module → import → entry"
      },
      {
        "number": "02",
        "title": "保留提示词，移除元数据",
        "body": "最终 XML 保留结构与文本，移除全部属性和命名空间声明，并压缩无意义的格式空白。",
        "code": ".o.xml = structure + text"
      },
      {
        "number": "03",
        "title": "复用工作，不复用执行状态",
        "body": "共享静态中间表示（IR）载荷，减少重复序列化；递归执行帧、显式输入和诊断来源仍各自独立。",
        "code": "prepare once · expand again"
      }
    ],
    "pipelineTitle": "从可读源码，到紧凑提示词。",
    "pipelineSteps": [
      "组织独立入口",
      "检查来源信息",
      "生成紧凑 XML"
    ],
    "pipelineNote": "宏仍可递归展开并传递返回值，参数保持显式且相互隔离。元数据留在诊断 IR 中，不混入最终发送的提示词。",
    "breakingLabel": "正在从 0.2 升级？",
    "breakingBody": "0.3.0 将入口文档与宏库分离，用 expand 替代 mount/call。源码需要迁移，不再接受旧语法或 module 的 entry 属性。",
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
    "introduction": "0.3.0 区分 xs:entry 与 xs:module，只保留 import 装载定义、xs:expand 递归展开宏。没有隐式 main、模块入口选择器或独立 fragment 构造。编译器仍是 CLI 二进制的内部模块。",
    "installation": "安装",
    "installBody": "使用 Rust 1.88 或更高版本，从固定标签及锁文件安装。",
    "installNote": "预期版本为 xmlsquish 0.3.0。此备选源码安装方式需要 Rust 1.88+，尚未发布到 crates.io。",
    "migration": "语言与迁移",
    "contract": "0.3.0 契约",
    "action": "迁移操作",
    "migrationRows": [
      [
        "分离宏库与构建入口",
        "xs:entry 放置导入、输入参数与输出结构；宏定义放入 xs:module 文件。"
      ],
      [
        "一个装载操作，一个展开操作",
        "call 改为 expand；mount 改为 import 加具名宏展开。不能 import 入口文件。"
      ],
      [
        "显式输入与返回值",
        "arg 传 Unicode 文本，fill/slot 传节点序列；递归结果可组合，不捕获调用方变量。"
      ],
      [
        "前缀在本地声明",
        "import 引入定义后，将本地前缀绑定到宏的命名空间 URI；文件间前缀拼写不必一致。"
      ],
      [
        "最终提示词没有属性",
        "从 .o.xml 移除所有属性、命名空间声明与元素前缀；有意义的属性内容应迁移成文本节点。"
      ],
      [
        "复用编译器快照",
        "内部 prepare/expand 接口复用冻结源码和静态事件载荷；源码变化后重新准备，不引入全局缓存或新 CLI 选项。"
      ]
    ],
    "example": "最小程序",
    "exampleNote": "保存为 hello.xml 后运行下方命令。",
    "execution": "执行与输出",
    "outputRules": [
      "入口执行前冻结、验证并链接完整导入闭包。import 只装载模块，expand 只展开具名宏；模块导入环合法，递归展开受预算约束。",
      "-I 生成带来源信息的 .i.xml；默认 -O 生成干净的 .o.xml。--debug 与 --explain 保留诊断，不改变最终提示词。",
      "最终 .o.xml 移除全部属性、命名空间声明和元素前缀，再执行固定空白压缩；中间诊断保留生成来源与展开帧。",
      "不覆盖源码，展开失败不发布部分结果。目录与 glob 构建跳过合法宏库；显式编译 module 文件会报错。"
    ],
    "limits": "资源与安全边界",
    "option": "选项",
    "default": "默认值",
    "scope": "范围",
    "budgetScopes": [
      "活动执行帧数，包含一个入口帧",
      "累计执行帧数，包含一个入口帧",
      "最终输出及每个临时参数/fill 缓冲区的序列化字节数"
    ],
    "security": "字节预算分别检查缓冲区，不是全部活动分配量之和，也不包含所有来源中间表示（Intermediate Representation, IR）的开销，因此不是进程总内存限制。加载器仅支持可表示为本机路径的 file: URI，拒绝其他协议、查询参数和片段；路径规范化不解引用符号链接。这不是文件访问沙箱，不可信源码需要外部文件权限与进程资源隔离。",
    "verification": "验证",
    "verifyBody": "可在发布源码中运行下方命令复现检查；已记录的运行结果请查看发布提交的 CI 日志。",
    "links": "延伸阅读",
    "changelog": "更新日志",
    "design": "语言设计与迁移依据",
    "readme": "项目文档",
    "performance": "性能数据，也说明边界",
    "performanceBody": "7 组配对 release 微基准中，完整内存编译耗时下降 17%–81%。范围包含解析、展开与 IR 序列化，不含 CLI 启动、token 计数及文件 I/O。小型 GSP 的实际 CLI 构建变化接近运行波动。部分单独准备阶段变慢，缓存载荷也会保留到快照释放；这些取舍均列入报告。",
    "performanceLink": "查看工作负载、原始样本与取舍",
    "history": "上一版本：0.2.0"
  }
};

export type ReferenceMessages = typeof en;
export const referenceMessages: Record<Locale, ReferenceMessages> = { "zh-CN": zh, en };
