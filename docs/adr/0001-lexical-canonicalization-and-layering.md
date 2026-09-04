# ADR 0001：面向提示词的词法规范化与分层边界

- 状态：已接受
- 日期：2026-09-05

## 背景

XML 提示词通常保留缩进与换行以方便人类维护，但这些布局字符也会进入模型上下文。项目需要在不引入完整 XML 解析器的前提下，以可预测、可测试的方式压紧这类文档。

这里存在一个无法回避的语义选择：XML 的字符数据空白并不普遍等价于“排版”。`xml:space="preserve"`、混合内容（mixed content）、CDATA、属性、注释和 DTD 都可能使空白具有意义。因此，xmlsquish 不能宣称自己是保持 XML 信息集（XML Information Set, XML Infoset）的通用压缩器。

## 决策

### 1. 产品定位

xmlsquish 是面向大语言模型提示词的**词法规范化器（prompt-oriented lexical canonicalizer）**，不是验证型 XML 解析器（validating XML parser）。它有意规范普通字符数据中的 XML `S`，可能改变混合内容的含义，并且不遵守 `xml:space`。调用方必须只把它用于“空白是布局噪声”的提示词。

[XML 1.0 `S` 产生式](https://www.w3.org/TR/xml/#sec-common-syn)定义且仅定义以下四个字符：

- U+0020 SPACE
- U+0009 CHARACTER TABULATION
- U+000D CARRIAGE RETURN
- U+000A LINE FEED

扫描器不得用语言层面更宽泛的 Unicode 空白分类替代该集合。

### 2. Atom 与统一发射规则

输入被扫描为两类词法单元（atom）：

1. `Word`：普通字符数据中最大的连续非 `S` 片段；
2. `Markup`：一个完整标签、注释、CDATA、处理指令（Processing Instruction, PI）或 DOCTYPE。

`Markup` 内部原样保留；普通字符数据中的 `S` 游程由同一个发射器处理。文件首尾不输出空格，任意两个相邻 atom 之间输出且只输出一个 U+0020。这个规则统一覆盖标签—标签、标签—单词、单词—标签和单词—单词边界，不在各扫描状态中复制特判。

例如：

```text
<a>x</a><b/>  ->  <a> x </a> <b/>
a\n\tb        ->  a b
```

该变换应满足确定性、线性时间 `O(n)` 与幂等性：

```text
squish(squish(x)) = squish(x)
```

### 3. 手写有限状态机

核心使用单遍手写有限状态机（Finite-State Machine, FSM），不构造 DOM 或语法树：

| 状态/扫描器 | 结束条件与不变量 |
| --- | --- |
| Data / Text | 识别 `Word` 与待处理的 XML `S` 游程；`<` 转入 markup 分派 |
| Tag | 仅未处于单双引号时的 `>` 结束标签；属性值中的空白和 `>` 原样保留 |
| Comment | 仅 `-->` 结束 |
| CDATA | 仅 `]]>` 结束 |
| PI | 仅 `?>` 结束 |
| DOCTYPE | 跟踪单双引号、内部子集方括号深度以及嵌套注释/PI；只有深度为零且不在受保护结构中的 `>` 才结束声明 |

扫描器只识别边界，不维护元素栈，不验证名称或起止标签配对，也不展开 DTD/实体。未闭合结构以类型和原输入字节偏移报告错误。

### 4. 统计账本

所有字符计数均指去除可选 UTF-8 BOM 后的 Unicode 标量值（Unicode scalar value）数量，而非 UTF-8 字节数或用户感知字符（grapheme cluster）数量。

- `recognized`：输入全部区域中属于 XML `S` 的字符数；markup 内的空白也计入，但受到保护。
- `removed`：从字符账本中消去的输入 `S` 槽位数；atom 间游程可复用一个槽位输出规范化空格，因此它不是字符身份的逐一追踪。
- `inserted`：为了分隔原本直接相邻的两个 atom 而新增的 U+0020 数。

若 Data 空白游程长度为 `k`：

- 位于文件开头或结尾：删除全部，`removed += k`；
- 位于两个 atom 之间：输出一个 U+0020，`removed += k - 1`，`inserted` 不变；
- 两个 atom 原本直接相邻：输出一个 U+0020，`inserted += 1`。

实现和测试必须保持以下可复核恒等式：

```text
output_chars = input_chars - removed + inserted
```

Token 数通过应用层端口提供，命令行适配器固定使用 [`o200k_base`](https://github.com/openai/tiktoken)，只计算文件逻辑文本，不包含消息封装。主压缩率按 Token 计算：

```text
compression = 1 - output_tokens / input_tokens
```

当 `input_tokens = 0` 时显示 `N/A`，避免产生 `NaN`。负数表示规范化后 Token 反而增加。批量统计只累加成功写出的文件，再由总数重算比率，而不是平均逐文件百分比。

### 5. 编码、输出与失败策略

- 输入必须是严格 UTF-8；不进行有损解码（lossy decoding）。
- 可选 UTF-8 BOM 在进入 core 前移除，写出时保留原 BOM；BOM 不参与字符、Token 或空白统计。
- UTF-16LE/BE 等编码不进行解码；字节序列不是合法 UTF-8（包括常见的带 BOM UTF-16 文件）时按文件报告错误，但不阻止其他独立文件继续处理。
- 输出与输入同目录，`name.xml` 对应 `name.o.xml`；发现阶段排除既有 `*.o.xml`，避免生成 `.o.o.xml`。
- 发现结果排序、去重；源文件不被覆盖。写文件使用同目录临时文件再持久化，以免暴露半写结果。
- 发现、读取、扫描、计数或写入失败均携带路径与原因。一个文件失败不应把其统计混入成功汇总，也不应阻止其余文件处理；批次最终以非零状态码反映存在失败。

### 6. 依赖方向

Rust 使用 [2024 Edition](https://doc.rust-lang.org/edition-guide/editions/creating-a-new-project.html)，工作区依赖只能向领域内侧流动：

```text
                   -> xmlsquish-app   (ports / use cases)
xmlsquish-cli  ----|
                   -> xmlsquish-core  (pure FSM / domain)
```

- `xmlsquish-core`：纯文本领域算法、FSM、错误与空白账本；不依赖 app、CLI、文件系统或 tokenizer。
- `xmlsquish-app`：批处理用例、报告模型及 `FileStore`、`Squasher`、`TokenCounter` 端口；不依赖 core 的具体实现。
- `xmlsquish-cli`：组合根（composition root），依赖 app 与 core，并提供参数解析、glob/目录发现、UTF-8/BOM、原子文件 I/O、core `Squasher` 适配器、tiktoken 和终端展示。
- `site/`：独立的 Astro/TypeScript/React 静态网站。网站不复制 FSM；未来若加入交互式规范化，只能复用 core（例如经 WASM），以免产生两套不一致语义。

## 后果

### 正面

- 单遍扫描避免 DOM 内存成本，复杂度和输出规则容易验证。
- 发射器集中处理边界，减少特殊分支，并自然提供幂等性。
- 端口将 tokenizer 与文件系统隔离，领域测试快速且可复现。
- 错误继续策略兼顾批处理可用性和可观测性。

### 代价与限制

- 输出可能不是 XML 语义等价变换；混合内容与 `xml:space` 是明确边界，而不是待隐藏的缺陷。
- 词法扫描不等同于完整 XML 合规验证；成功输出不代表输入通过 XML 规范验证。
- 固定 tokenizer 有利于可复现，但它不代表所有模型的实际计费方式。
- 严格 UTF-8 会拒绝部分 XML 允许的其他编码；将来可在 CLI 适配层增加解码/回写，而不改变 core。
