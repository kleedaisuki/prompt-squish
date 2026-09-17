import type { Locale } from "./messages";
import type { ReleaseVersion } from "../data/releases";

/** Localized prose for one immutable release entry.
 * 单个不可变发布条目的本地化文案。 */
export interface ReleaseDetailCopy {
  title: string;
  description: string;
  date: string;
  status: string;
  summary: string;
  changes: ReadonlyArray<{ id: string; title: string; items: ReadonlyArray<string> }>;
  compatibility: ReadonlyArray<string>;
}

interface ReleaseUiCopy {
  index: {
    title: string;
    description: string;
    eyebrow: string;
    heading: string;
    introduction: string;
    current: string;
    historical: string;
    read: string;
    protocol: string;
    acquisitionNative: string;
    acquisitionSource: string;
  };
  detail: {
    back: string;
    onThisPage: string;
    overview: string;
    changes: string;
    acquisition: string;
    compatibility: string;
    github: string;
    metadata: string;
    date: string;
    distribution: string;
    protocol: string;
    nativeTitle: string;
    nativeBody: string;
    sourceTitle: string;
    sourceBody: string;
    checksums: string;
    sourceInstall: string;
    unsignedNote: string;
    previous: string;
    next: string;
    allReleases: string;
  };
  versions: Record<ReleaseVersion, ReleaseDetailCopy>;
}

const en: ReleaseUiCopy = {
  index: {
    title: "xmlsquish release log",
    description: "A reverse-chronological log of xmlsquish product releases, compatibility boundaries, downloads, and migrations.",
    eyebrow: "Release log",
    heading: "Every release, in order.",
    introduction: "Open a version for its exact changes, acquisition contract, and upgrade boundary. This log is the human history; immutable JSON metadata remains available where it was published.",
    current: "Current",
    historical: "Historical",
    read: "Open release",
    protocol: "Machine protocol",
    acquisitionNative: "6 native archives",
    acquisitionSource: "Source tag only",
  },
  detail: {
    back: "Release log",
    onThisPage: "On this page",
    overview: "Overview",
    changes: "Changes",
    acquisition: "Get this version",
    compatibility: "Compatibility",
    github: "GitHub Release",
    metadata: "Release metadata",
    date: "Released",
    distribution: "Distribution",
    protocol: "Machine protocol",
    nativeTitle: "Native archives",
    nativeBody: "Choose the archive matching your operating system and processor. Verify its exact filename against SHA256SUMS before extraction.",
    sourceTitle: "Install the immutable source tag",
    sourceBody: "This release did not publish a native archive matrix. Install the reviewed tag and lockfile with Rust 1.88 or newer.",
    checksums: "SHA-256 manifest",
    sourceInstall: "Pinned source install",
    unsignedNote: "Native archives are unsigned and not Apple-notarized. Checksums establish byte integrity, not publisher identity.",
    previous: "Older release",
    next: "Newer release",
    allReleases: "All releases",
  },
  versions: {
    "1.0.2": {
      title: "Readable outputs and recoverable cleanup",
      description: "xmlsquish 1.0.2 exposes stable artifact paths, adds a typed clean operation, and advances the additive machine protocol to 3.1.",
      date: "September 17, 2026",
      status: "Current stable release",
      summary: "Build products now appear directly at their declared target paths while hashes, generations, and journals remain private. The new clean command removes project build state and only dependency-cache entries proven invalid.",
      changes: [
        { id: "target-layout", title: "Readable target layout", items: ["Prompt, IR, and debug products materialize at stable manifest-derived paths.", "Publication generations, content hashes, journals, and build records live in project-private manager state.", "Existing target/xmlsquish locators and custom workspace target directories remain compatible."] },
        { id: "clean", title: "Typed, recoverable clean", items: ["xmlsquish clean removes the complete project output root and private catalog through the normal manager lifecycle.", "A redo journal, shared project lock, and publication epoch make build/clean races and interrupted cleanup recoverable.", "Healthy shared dependencies, the global CAS, and the action index are retained; only provably invalid cache entries are pruned."] },
        { id: "agents-and-release", title: "Agent and release workflow", items: ["The root SKILL.md provides a compact DSL, manifest, configuration, and command reference.", "Linux, Windows, and macOS CI verifies exact output layout, clean, and offline rebuild behavior.", "Six native archives and SHA256SUMS are published only after the complete release matrix succeeds."] },
      ],
      compatibility: ["Manifest version, DSL namespace, artifact locators, locked/offline modes, and exit codes remain unchanged.", "Machine protocol 3.1 adds typed clean requests and results without changing existing 3.0 result shapes.", "Legacy v1.0.1 publication state is migrated and validated under the project lock before removal."],
    },
    "1.0.1": {
      title: "Publication boundaries become product contracts",
      description: "xmlsquish 1.0.1 introduces typed artifact locators, verified raw inspection, capability-lazy runtime services, and machine protocol 3.0.",
      date: "September 16, 2026",
      status: "Historical stable release",
      summary: "Build outputs now carry typed, project-relative locators instead of exposing publisher storage paths. Raw artifact inspection validates digest and size before any bytes reach stdout.",
      changes: [
        { id: "publication-catalog", title: "Typed publication catalog", items: ["Published targets expose target and generation identities plus typed artifact records.", "Every artifact carries an immutable id, kind, stable locator, size, and digest.", "Manager code no longer constructs or scans private generation directories."] },
        { id: "runtime", title: "Capability-lazy runtime", items: ["One invocation-scoped production runtime is injected at the composition boundary.", "CAS, action index, target publisher, and catalog publisher initialize independently and only when required.", "An initialization failure in one capability does not poison unrelated capabilities."] },
        { id: "protocol", title: "Protocol 3.0 migration", items: ["Automation must decode PublishedArtifact rather than the protocol 2.1 Artifact shape.", "Keep and pass the stable locator instead of reading a physical uri.", "Use inspect artifact LOCATOR --format=raw when exact bytes are required."] },
      ],
      compatibility: ["The XML DSL namespace, vocabulary, and semantics are unchanged.", "The six direct commands, manifest, locked/offline modes, exit codes, and artifact roles remain stable.", "Machine-event consumers must migrate from protocol 2.1 to 3.0 together with the CLI."],
    },
    "1.0.0": {
      title: "A project manager for reproducible prompt builds",
      description: "xmlsquish 1.0.0 introduces manifests, lockfiles, workspaces, inspectable artifacts, and a stable automation contract.",
      date: "September 15, 2026",
      status: "Historical stable release",
      summary: "The first stable release turns loose XML compilation into a Cargo-style project workflow with reproducible dependency resolution, atomic publication, and inspectable build evidence.",
      changes: [
        { id: "projects", title: "Projects and workspaces", items: ["xmlsquish.toml declares packages, targets, dependencies, and workspace membership.", "A deterministic lockfile and locked/offline modes make dependency resolution inspectable.", "new, add, remove, fmt, build, and inspect form the stable command surface."] },
        { id: "artifacts", title: "Inspectable artifacts", items: ["Builds publish .prompt by default and can also emit typed .xsir and self-contained .psdbg evidence.", "Content-addressed build state and atomic publication prevent partial results from becoming current.", "Human, short, and versioned NDJSON renderers separate interactive and automation output."] },
        { id: "stability", title: "The 1.0 contract", items: ["Stable exit codes and machine protocol 2.1 define the automation boundary.", "The 0.3 XML vocabulary remains unchanged while the project and command model evolves.", "Future compatible releases will not silently break declared commands, manifests, artifacts, protocols, or exit codes."] },
      ],
      compatibility: ["Projects migrating from 0.3.0 need a manifest and declared build targets.", "Consume products from project target state instead of legacy files beside the source.", "The DSL namespace stays stable; the old loose-file CLI workflow does not."],
    },
    "0.3.0": {
      title: "Libraries provide reuse; entries build the product",
      description: "xmlsquish 0.3.0 separates build entries from macro libraries and unifies recursive expansion.",
      date: "September 11, 2026",
      status: "Historical release",
      summary: "Explicit xs:entry build roots are separated from reusable xs:module libraries. xs:expand becomes the single composition operation over prepared, reusable compilation snapshots.",
      changes: [
        { id: "language-model", title: "One composition model", items: ["xs:entry builds a product; xs:module contains imports and named macros only.", "xs:import loads definitions and xs:expand is the sole recursive expansion operation.", "Inputs evaluate in the caller and pass by value into isolated macro scopes."] },
        { id: "compiler", title: "Reusable compilation", items: ["Compiler::prepare freezes and links source once for repeated expansion.", "Immutable static payloads, escaping, and provenance serialization are reused.", "Execution frames and complete macro results remain fresh for each expansion."] },
        { id: "migration", title: "Breaking 0.2 migration", items: ["Replace document-building modules with explicit xs:entry roots.", "Replace xs:call with xs:expand and extract former mounts into named macros.", "Final output removes attributes, namespace declarations, and prefixes, then squishes whitespace."] },
      ],
      compatibility: ["This language release is not syntax-compatible with 0.2.0.", "Linux archives require glibc 2.35 or newer and do not target Alpine/musl.", "The pinned 0.3.0 namespace specification remains permanently addressable."],
    },
    "0.2.0": {
      title: "A namespace-aware XML macro language",
      description: "xmlsquish 0.2.0 introduces immutable macros, explicit scalar arguments and XML slots in a single CLI binary.",
      date: "September 11, 2026",
      status: "Historical source release",
      summary: "The language moves to URI-identified builtins, namespace-qualified immutable macros, explicit value passing, and bounded expansion in a single command-line program.",
      changes: [
        { id: "language", title: "Explicit language contracts", items: ["Builtins are recognized by namespace identity and macros use namespace-qualified names.", "Parameters do not inherit; xs:arg and xs:fill pass scalar values and XML sequences explicitly.", "xs:insert emits escaped scalar text rather than parsing markup."] },
        { id: "execution", title: "Validated execution", items: ["The complete static source closure is validated before execution, including unselected branches.", "Imports load definitions while actual recursion remains bounded by depth and expansion budgets.", "Failed expansion publishes no partial output and never overwrites source files."] },
        { id: "distribution", title: "Source-tag distribution", items: ["0.2.0 was installed from its pinned Git tag and lockfile.", "It did not publish the later six-platform native archive matrix.", "The former Rust library interface and 0.1 syntax have no compatibility layer."] },
      ],
      compatibility: ["This is a breaking language release relative to 0.1.", "Rust 1.88 or newer is required to install the source tag.", "Resource budgets are per guarded buffer, not a process-wide memory sandbox."],
    },
  },
};

const zh: ReleaseUiCopy = {
  index: {
    title: "xmlsquish 发布记录",
    description: "按时间倒序浏览 xmlsquish 产品发布、兼容边界、下载方式与迁移要求。",
    eyebrow: "发布记录",
    heading: "每次发布，依次归档。",
    introduction: "打开具体版本，阅读准确变更、获取方式与升级边界。这里是面向人的版本历史；已发布的不可变 JSON 元数据仍原址保留。",
    current: "当前版本",
    historical: "历史版本",
    read: "打开发布记录",
    protocol: "机器协议",
    acquisitionNative: "6 个原生归档",
    acquisitionSource: "仅源码标签",
  },
  detail: {
    back: "发布记录",
    onThisPage: "本页目录",
    overview: "概览",
    changes: "版本变化",
    acquisition: "获取此版本",
    compatibility: "兼容边界",
    github: "GitHub Release",
    metadata: "发布元数据",
    date: "发布日期",
    distribution: "分发方式",
    protocol: "机器协议",
    nativeTitle: "原生归档",
    nativeBody: "选择与操作系统及处理器相符的归档。解压前，请用 SHA256SUMS 中完全相同的文件名进行校验。",
    sourceTitle: "安装不可变源码标签",
    sourceBody: "此版本没有发布六平台原生归档矩阵。请使用 Rust 1.88 或更高版本安装已经审阅的标签与锁文件。",
    checksums: "SHA-256 校验清单",
    sourceInstall: "固定源码安装",
    unsignedNote: "原生归档未签名，也未经 Apple 公证。校验和证明字节完整性，不证明发布者身份。",
    previous: "更早版本",
    next: "更新版本",
    allReleases: "全部发布",
  },
  versions: {
    "1.0.2": {
      title: "可读产物与可恢复清理",
      description: "xmlsquish 1.0.2 提供稳定直观的产物路径，新增有类型 clean 操作，并将可加性机器协议提升到 3.1。",
      date: "2026 年 9 月 17 日",
      status: "当前稳定版本",
      summary: "构建产品现在直接出现在清单声明的 target 路径；hash、generation 与 journal 留在私有状态。新的 clean 命令删除项目构建状态，并且只清理可证明失效的依赖缓存。",
      changes: [
        { id: "target-layout", title: "可读的 target 布局", items: ["Prompt、IR 与调试产品物化到由清单确定的稳定路径。", "发布 generation、内容 hash、journal 与 build record 迁入 manager 的项目私有状态。", "现有 target/xmlsquish locator 与自定义工作区 target-dir 保持兼容。"] },
        { id: "clean", title: "有类型、可恢复的 clean", items: ["xmlsquish clean 通过正常 manager 生命周期删除完整项目输出根与私有目录。", "redo journal、共享项目锁和 publication epoch 使 build/clean 竞态及中断清理都能恢复。", "健康共享依赖、全局 CAS 与 action index 保留；只裁剪可证明失效的缓存条目。"] },
        { id: "agents-and-release", title: "Agent 与发布工作流", items: ["根目录 SKILL.md 提供精简的 DSL、清单、配置与命令速查。", "Linux、Windows 与 macOS CI 验证精确输出布局、clean 和离线重建。", "只有完整发布矩阵通过后才发布六个原生归档及 SHA256SUMS。"] },
      ],
      compatibility: ["清单版本、DSL 命名空间、产物 locator、locked/offline 模式及退出码保持不变。", "机器协议 3.1 增加有类型 clean 请求和结果，不改变既有 3.0 结果结构。", "v1.0.1 的旧发布状态会在项目锁内迁移并验证，成功后才移除。"],
    },
    "1.0.1": {
      title: "发布边界成为产品契约",
      description: "xmlsquish 1.0.1 引入有类型产物定位符、经验证的原始检查、按能力延迟初始化的运行时，以及机器协议 3.0。",
      date: "2026 年 9 月 16 日",
      status: "历史稳定版本",
      summary: "构建输出改用有类型、项目相对的定位符，不再暴露发布器存储路径。原始产物检查会在任何字节进入 stdout 前验证摘要与大小。",
      changes: [
        { id: "publication-catalog", title: "有类型的发布目录", items: ["已发布目标公开目标与 generation 身份，以及有类型产物记录。", "每个产物携带不可变 id、kind、稳定 locator、size 与 digest。", "manager 不再构造或扫描私有 generation 目录。"] },
        { id: "runtime", title: "按能力延迟初始化的运行时", items: ["组合边界注入一个调用级生产运行时。", "CAS、action index、目标发布器与目录发布器相互独立，只在需要时初始化。", "一种能力初始化失败不会污染无关能力。"] },
        { id: "protocol", title: "协议 3.0 迁移", items: ["自动化必须解码 PublishedArtifact，而不是协议 2.1 的 Artifact 结构。", "保存并传递稳定 locator，不再读取物理 uri。", "需要精确字节时调用 inspect artifact LOCATOR --format=raw。"] },
      ],
      compatibility: ["XML DSL 命名空间、词汇与语义均未改变。", "六个直接命令、清单、锁定/离线模式、退出码与产物角色保持稳定。", "机器事件消费者必须与 CLI 同步从协议 2.1 迁移到 3.0。"],
    },
    "1.0.0": {
      title: "面向可复现 Prompt 构建的项目管理器",
      description: "xmlsquish 1.0.0 引入清单、锁文件、工作区、可检查产物与稳定自动化契约。",
      date: "2026 年 9 月 15 日",
      status: "历史稳定版本",
      summary: "首个稳定版本把松散 XML 编译升级为 Cargo 式项目工作流，提供可复现依赖解析、原子发布与可检查的构建证据。",
      changes: [
        { id: "projects", title: "项目与工作区", items: ["xmlsquish.toml 声明包、目标、依赖和工作区成员。", "确定性锁文件及 locked/offline 模式让依赖解析可以检查。", "new、add、remove、fmt、build 与 inspect 构成稳定命令面。"] },
        { id: "artifacts", title: "可检查的产物", items: ["构建默认发布 .prompt，也可输出有类型 .xsir 与自包含 .psdbg 证据。", "内容寻址构建状态与原子发布避免部分结果成为 current。", "human、short 与版本化 NDJSON renderer 分离交互输出和自动化输出。"] },
        { id: "stability", title: "1.0 契约", items: ["稳定退出码与机器协议 2.1 定义自动化边界。", "项目与命令模型演进时，0.3 XML 词汇保持不变。", "后续兼容版本不会无预警破坏已声明的命令、清单、产物、协议或退出码。"] },
      ],
      compatibility: ["从 0.3.0 迁移时需要添加清单并声明构建目标。", "从项目 target 状态消费产品，不再读取源码旁边的旧文件。", "DSL 命名空间保持稳定；旧的松散文件 CLI 工作流不保留。"],
    },
    "0.3.0": {
      title: "宏库负责复用，入口负责产品",
      description: "xmlsquish 0.3.0 分离构建入口与宏库，并统一递归展开。",
      date: "2026 年 9 月 11 日",
      status: "历史版本",
      summary: "显式 xs:entry 构建根与可复用 xs:module 宏库分离；xs:expand 成为基于可复用编译快照的唯一组合操作。",
      changes: [
        { id: "language-model", title: "一套组合模型", items: ["xs:entry 构建产品；xs:module 只包含 import 与具名宏。", "xs:import 装载定义，xs:expand 是唯一递归展开操作。", "输入在调用方求值，再按值传入隔离的宏作用域。"] },
        { id: "compiler", title: "可复用编译", items: ["Compiler::prepare 一次冻结并链接源码，供重复展开。", "不可变静态载荷、转义和来源序列化可以复用。", "每次展开仍使用全新的执行帧，不缓存完整宏结果。"] },
        { id: "migration", title: "破坏性的 0.2 迁移", items: ["把生成文档的 module 改为显式 xs:entry 根。", "用 xs:expand 替换 xs:call，并把旧 mount 提取成具名宏。", "最终输出会移除属性、命名空间声明与前缀，然后压缩空白。"] },
      ],
      compatibility: ["此语言版本不兼容 0.2.0 语法。", "Linux 归档要求 glibc 2.35 或更高版本，不面向 Alpine/musl。", "固定的 0.3.0 命名空间规范永久可引用。"],
    },
    "0.2.0": {
      title: "命名空间感知的 XML 宏语言",
      description: "xmlsquish 0.2.0 在单一 CLI 二进制中引入不可变宏、显式标量参数与 XML slot。",
      date: "2026 年 9 月 11 日",
      status: "历史源码版本",
      summary: "语言转向以 URI 识别的内建操作、命名空间限定的不可变宏、显式值传递，以及单一命令行程序中的有界展开。",
      changes: [
        { id: "language", title: "显式语言契约", items: ["内建操作按命名空间身份识别，宏使用命名空间限定名。", "参数不继承；xs:arg 与 xs:fill 显式传递标量值和 XML 序列。", "xs:insert 输出转义后的标量文本，而不会解析标记。"] },
        { id: "execution", title: "经过验证的执行", items: ["执行前验证完整静态源码闭包，包括未选中的分支。", "import 装载定义，实际递归受深度和展开次数预算约束。", "失败展开不会发布部分输出，也绝不覆盖源码。"] },
        { id: "distribution", title: "源码标签分发", items: ["0.2.0 通过固定 Git 标签与锁文件安装。", "它没有发布后来形成的六平台原生归档矩阵。", "旧 Rust 库接口与 0.1 语法不提供兼容层。"] },
      ],
      compatibility: ["相对 0.1，这是破坏性语言发布。", "安装源码标签需要 Rust 1.88 或更高版本。", "资源预算按受检缓冲区计算，并不是进程级内存沙箱。"],
    },
  },
};

/** Keyed translations prevent chronology from becoming a positional coupling.
 * 键控翻译避免时间顺序与数组位置形成耦合。 */
export const releaseMessages: Record<Locale, ReleaseUiCopy> = { "zh-CN": zh, en };
