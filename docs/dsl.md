# xmlsquish DSL 设计规范

> **XML is data. Macros are computation.**
> **源码路径定义身份，XML 命名空间定义符号，展开帧定义执行。**

本文定义 xmlsquish 的语言模型、模块与宏系统、作用域、展开语义、诊断信息以及资源边界。本文是 DSL 的规范性设计；实现应服从本文给出的语义不变量，而不是从现有代码行为反推语言规则。

文中的“必须”“不得”“应当”是规范要求。

---

## 1. 设计目标

xmlsquish 是一个带有小型、纯函数式且计算完备宏核的 XML 结构预处理器。它不是通用脚本宿主，也不是简化版 XSLT。

语言由两个彼此正交的层组成：

| 层 | 值域 | 职责 | 核心操作 |
|---|---|---|---|
| XML 结构层 | XML 节点序列 | 组合和填充文档结构 | `expand`、`slot`、`fill` |
| 标量宏层 | Unicode 字符串 | 参数传递、字符串构造与析构、条件和递归 | `arg`、`insert`、`ifr`、`expand` |

模块（Module）只组织定义；宏（Macro）是唯一可展开单位；片段（Fragment）仅是展开结果的有序节点序列，不是独立声明、文件类型或可调用对象。

两层通过以下显式桥梁相连：

1. 标量字符串可以由 `xs:insert` 产生一个 XML 文本节点；
2. `xs:fill` 可以向 `xs:slot` 传递 XML 节点序列；
3. 宏的纯文本展开结果可以在 `xs:arg` body 中拼接为字符串，再按值传给另一个宏。

XML 节点不得成为宏语言的通用运行时值。标量宏不得使用 XPath、XML AST 捕获或结构模式匹配来计算；slot 也不得冒充字符串变量。

这一边界消除了“XML 结构既是待处理文档、又是程序运行时数据”的双重语义。

---

## 2. 内建命名空间与 QName

### 2.1 内建命名空间

xmlsquish 的内建命名空间 URI 为：

```text
https://xmlsquish.moesegfault.dev/ns
```

本文约定使用 `xs` 前缀：

```xml
xmlns:xs="https://xmlsquish.moesegfault.dev/ns"
```

前缀仅是词法别名，不参与身份判断。任何绑定到该 URI 的前缀都表示同一组内建操作。用户不得在该命名空间中定义宏。

### 2.2 宏名使用 XML 扩展名

宏名是 XML 限定名（QName），其语义身份是 XML 扩展名（Expanded Name）：

\[
q=(\text{NamespaceURI},\text{LocalName})
\]

例如：

```xml
<xs:module
    xmlns:xs="https://xmlsquish.moesegfault.dev/ns"
    xmlns:str="https://xmlsquish.moesegfault.dev/macro/string">

    <xs:macro name="str:head">
        <!-- ... -->
    </xs:macro>
</xs:module>
```

在另一源码中，下面的引用仍指向同一个宏：

```xml
<xs:module
    xmlns:xs="https://xmlsquish.moesegfault.dev/ns"
    xmlns:s="https://xmlsquish.moesegfault.dev/macro/string">

    <xs:import src="./string.xml"/>
    <xs:macro name="s:example">
        <xs:expand ref="s:head"/>
    </xs:macro>
</xs:module>
```

宏 `name` 与 `ref` 的值必须是带前缀的词法 QName；前缀必须在其 XML 词法位置有效。宏身份不得以原始字符串或前缀比较。

参数名、slot 名和 capture 名是局部 NCName，不带命名空间。

### 2.3 符号不可重定义

一次 xmlsquish invocation 具有一张全局宏符号表：

\[
\operatorname{SymbolTable}:q\mapsto MacroDefId
\]

注册规则只有三种：

| 当前状态 | 新定义 | 结果 |
|---|---|---|
| 不存在该 QName | 任意定义 | 注册 |
| 已指向同一 `MacroDefId` | 同一源码被再次引用 | no-op |
| 已指向其他 `MacroDefId` | 来自同一或不同源码的另一处定义 | hard error |

语言不提供 override、overload、宏级 shadowing 或“后定义覆盖前定义”。重复 import 同一源码不会产生重定义；同一源码文本中声明两个同名宏则是两个不同定义，必须报错。

---

## 3. 源码、模块与身份

### 3.1 SourceUnit

每个源码资源形成一个不可变的源码单元（SourceUnit）：

源码单元具有两种互斥种类，由 XML 根元素显式区分：

| 根元素 | 职责 | 内容 |
|---|---|---|
| `xs:module` | 组织可复用宏定义 | `xs:import` 与零个或多个 `xs:macro` |
| `xs:entry` | 构造一个产品文档 | 声明区的 `xs:import`、可选 `xs:param`，随后是输出构造正文 |

模块示例 `macros.xml`：

```xml
<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"
           xmlns:app="https://example.com/app/macros">
    <xs:macro name="app:badge">
        <xs:param name="text"/>
        <Badge><xs:insert get="arg.text"/></Badge>
    </xs:macro>
</xs:module>
```

入口示例 `prompt.xml`：

```xml
<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns"
          xmlns:app="https://example.com/app/macros">
    <xs:import src="./macros.xml"/>
    <xs:param name="title"/>
    <Prompt><xs:expand ref="app:badge"><xs:arg name="text" get="arg.title"/></xs:expand></Prompt>
</xs:entry>
```

模块直接子元素只能是 `xs:import` 与 `xs:macro`；声明之间的格式空白、注释及处理指令不产生输出。普通 XML 元素、非空白文本、顶层 `xs:param` 或展开指令均非法。模块没有可执行正文、入口属性或隐式 `main`。宏定义不得嵌套；模块内导入与宏声明可以交错。

入口不是宏，不注册符号，不拥有 `MacroDefId`，不能被 `expand` 引用。入口正文在独立的构造上下文（Construction Context）中求值：`file.*` 来自入口源码，`arg.*` 来自显式命令行参数，初始匹配作用域为空。`xs:param` 声明必需字符串参数，规则与宏参数相同；`--arg` 只提供入口参数，不隐式传给宏。入口声明必须在正文之前；入口不得定义宏，也不得声明根 slot，因为命令行不提供 fill。正文允许普通 XML、`expand`、`insert` 和 `ifr`，并按后续章节相同规则组合结果。

`import` 的目标只能是模块，不能是入口。入口只能作为编译根使用；直接指定模块进行编译必须报错，目录或 glob 批量发现时跳过合法模块。不得根据文件名、宏个数或声明顺序猜测入口，不接受旧的模块 `entry` 属性。

宏可以前向引用，也可以直接或相互递归，不依赖文本声明顺序。根 invocation 的最终结果必须是一个格式良好的 XML 文档；宏和内部展开过程可以暂时产生 XML 节点序列。

### 3.2 SourceId

源码身份定义为：

\[
SourceId=Canonicalize(Resolve(DefinitionBase,src))
\]

规范化包括：

- 按 URI 规则将相对 `src` 解析到定义位置；
- 消除 `.` 与 `..` 路径段；
- 使用 loader 对应 URI scheme 的稳定分隔符与规范形式。

源码身份不根据以下信息折叠：

- 文件内容 hash；
- inode 或其他物理文件标识；
- symlink 解引用后的真实路径。

因此 `./lib/../lib/a.xml` 与 `./lib/a.xml` 是同一 SourceId；`./a.xml` 与 `./alias-to-a.xml` 即使最终指向同一物理文件，仍是两个逻辑资源。内容 hash 只能用于缓存失效，不得参与语言身份。

`src` 必须是静态 URI 字面量，不得由标量计算动态生成。这使编译器能够在执行前发现、装载并验证完整源码闭包。

### 3.3 一次运行内冻结

一个 SourceId 在一次 invocation 中最多读取和解析一次：

```text
SourceId
  ↓ first load
source bytes
  ↓ parse and validate
SourceUnit
  ↓ freeze
shared immutable definition
```

即使运行期间磁盘内容发生变化，本次 invocation 中后续的 import 和 expand 也必须继续引用已经冻结的 SourceUnit。下一次 invocation 才重新读取源码。

### 3.4 定义折叠不等于调用折叠

同一源码被引用多次时，SourceUnit 与其中的 `MacroDef` 只存在一份；每次调用仍创建新的展开帧（Expansion Frame）：

```text
MacroDef #42
  ↑         ↑         ↑
Frame #7  Frame #8  Frame #9
```

这些 frame 可以具有不同的参数、slot、capture、父调用和递归深度。实现不得因为 `MacroDefId`、参数或源码相同而把多个调用视作同一个执行实例；语言也不规定隐式 memoization。

---

## 4. 唯一装载与统一展开

### 4.1 `xs:import`

`import` 是唯一的源码装载操作，使另一个 SourceUnit 的命名宏参与本次编译和全局符号解析：

```xml
<xs:import src="./string.xml"/>
```

它只执行 `resolve SourceId → load/intern SourceUnit → register MacroDefs`。

- 只能作为模块的直接子元素，或入口声明区的直接子元素；
- 目标必须为模块；入口不得被导入；
- 不产生输出、不执行宏、不创建运行时 frame；
- 不接受 `xs:arg` 或 `xs:fill`；
- 同一 SourceId 只装载一次，重复导入和导入环不重复注册定义；
- 递归发现全部静态导入，在开始展开前冻结闭包。

### 4.2 `xs:expand`

`expand` 是唯一的宏展开操作，名字表示递归展开而不是文件挂载或文本粘贴：

```xml
<xs:expand ref="str:head">
    <xs:arg name="input" get="arg.value"/>
</xs:expand>
```

`ref` 必须是静态 QName，不能来自 `get`、字符串拼接或正则捕获。展开自身不装载源码；目标必须已存在于冻结符号表。宏不是一等值（First-class Value），没有动态分派或闭包。

每次展开创建独立 frame，将宏体递归求值得到的有序节点序列返回至当前位置。入口构造上下文消费这些返回值，最终产品必须是单一 XML 文档；入口自身不创建宏展开帧。

语言不再接受 `xs:call`、`xs:mount` 或 `xs:fragment`，也不提供兼容别名。

### 4.3 定义位置语义

相对导入地址绑定到声明所在源码；宏中 `file.*` 绑定宏定义源码，而不是展开位置。QName 同样在其源码词法位置解析。例如 `/lib/a.xml` 中的 `<xs:import src="./helper.xml"/>` 总是装载 `/lib/helper.xml`，不受谁引用该模块影响。

展开者信息只存在于诊断 frame，不作为宏可读的动态环境。宏需要外部数据时必须通过 `xs:arg` 或 `xs:fill` 显式传入。

---

## 5. 宏定义与调用契约

### 5.1 定义

命名宏使用 `xs:macro` 定义：

```xml
<xs:macro name="str:surround">
    <xs:param name="left"/>
    <xs:param name="value"/>
    <xs:param name="right"/>

    <xs:insert get="arg.left"/>
    <xs:insert get="arg.value"/>
    <xs:insert get="arg.right"/>
</xs:macro>
```

`xs:param`：

- 必须位于宏体其他内容之前；
- 名字在单个宏内唯一；
- 声明的参数均为必需参数；
- 参数类型固定为 Unicode 字符串。

`xs:param` 可声明在所属宏或入口声明区中；模块不具有参数。

调用必须恰好提供所有声明参数。缺失参数、未知参数和重复参数都是 hard error。

### 5.2 参数值

`xs:arg` 有三种互斥形式：

```xml
<xs:arg name="x" value="literal"/>
```

```xml
<xs:arg name="x" get="arg.other"/>
```

```xml
<xs:arg name="x">prefix-<xs:insert get="match.tail"/>-suffix</xs:arg>
```

`value`、`get` 和 body 三者必须且只能出现一种。

参数 body 在调用者的词法环境中展开，最终结果必须只包含 character data。若留下元素、注释、processing instruction 或其他非文本节点，则报错：

```text
scalar argument expansion produced a non-text node
```

XML 字符数据在解析实体后按原样保留；实现不得自动 trim 或折叠空白。因此对空白敏感的参数 body 应写成紧凑的内联形式。

### 5.3 调用求值顺序

一次 `expand` 按以下语义执行：

1. 静态解析目标 MacroDef；
2. 在调用者环境中分别求值每个 `xs:arg`，得到不可变字符串；
3. 在调用者环境中分别展开每个 `xs:fill`，得到不可变 XML 节点序列；
4. 验证参数和 slot 契约；
5. 创建新的 callee frame；
6. 在定义位置的 `file.*` 与本次调用的 `arg.*` 下展开宏体。

新 frame 不继承调用者的 `arg.*`、`match.*` 或 slot；其匹配作用域栈初始为空。宏自身的 `file.*` 是定义信息，不是对调用者变量的捕获。值传递（Pass by Value）规定的是不可变值语义，不要求实现物理复制字符串或节点；安全的只读共享允许，但不得观察到别名写入。

参数与 fill 都恰好求值一次。同一调用中正在构造的新参数不会彼此形成隐式可见性；若一个值依赖另一个值，应在调用者 frame 中先显式构造，或通过一个小宏完成组合。

### 5.4 返回值与组合

宏的唯一返回值是宏体展开得到的有序 XML 节点序列，不存在隐式写回、`return` 指令、返回寄存器或调用者变量绑定。正文中的兄弟节点按顺序拼接；嵌套 `expand` 的结果插入该表达式的位置。

结果的使用方式由外围构造明确决定：

| 位置 | 消费方式 |
|---|---|
| 普通宏体 | 将结果作为节点序列拼接 |
| `xs:arg` body | 展开完成后检查全部节点均为文本，再连接成字符串 |
| `xs:fill` body | 先展开成不可变节点序列，再传入目标 slot |

例如，把内层宏的返回文本按值传入另一个宏：

```xml
<!-- result-macros.xml -->
<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"
           xmlns:m="urn:example:result">
    <xs:macro name="m:prefix"><xs:param name="text"/><xs:insert get="arg.text"/></xs:macro>
    <xs:macro name="m:wrap">
        <xs:param name="text"/>
        <Message><xs:insert get="arg.text"/></Message>
    </xs:macro>
</xs:module>
```

```xml
<!-- result.xml -->
<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns"
          xmlns:m="urn:example:result">
    <xs:import src="./result-macros.xml"/>
    <xs:expand ref="m:wrap">
        <xs:arg name="text">Hello, <xs:expand ref="m:prefix"><xs:arg name="text" value="Klee"/></xs:expand>!</xs:arg>
    </xs:expand>
</xs:entry>
```

这里 `m:prefix` 的返回值先完成求值，才形成 `m:wrap` 的 `arg.text`。若内层返回 `<Name>Klee</Name>`，这不是字符串返回值，必须报非文本参数错误，不能偷偷去标签或序列化为字符串。普通源码的排版空白仍按第 5.2 节参与标量拼接；最终产品压缩只在整个展开结束后发生。

---

## 6. Slot 与 Fill

slot 只承载 XML 节点序列，不属于标量环境。

宏使用 `xs:slot` 声明插入点：

```xml
<xs:macro name="ui:panel">
    <Panel>
        <Header><xs:slot name="header" required="true"/></Header>
        <Body><xs:slot name="body"/></Body>
    </Panel>
</xs:macro>
```

调用者使用 `xs:fill` 提供内容：

```xml
<xs:expand ref="ui:panel">
    <xs:fill name="header"><Title>Hello</Title></xs:fill>
    <xs:fill name="body"><Paragraph>World</Paragraph></xs:fill>
</xs:expand>
```

规则如下：

- slot 名在一个宏内唯一；
- fill 名必须对应目标宏声明的 slot；
- 同一次调用不得重复 fill 同一个 slot；
- 缺少 `required="true"` 的 fill 是 hard error；
- 未填充的非 required slot 展开为空节点序列；
- fill 在调用者词法环境中展开，随后作为不可变节点序列传入 callee；
- slot 不能通过 `get`、`insert` 或 regex 读取，也不能隐式转成字符串。

这保证 `arg : String` 与 `slot : XMLSequence` 的边界明确，而不需要引入通用类型系统。

---

## 7. 标量环境与作用域

宏与入口构造正文可见的标量环境只有三层；入口不是隐式宏：

| 命名空间 | 生命周期 | 可变性 | 含义 |
|---|---|---|---|
| `file.*` | definition frame | 只读 | 当前宏定义源码的反射信息 |
| `arg.*` | invocation frame | 只读 | 本次调用显式传入的参数 |
| `match.*` | `xs:ifr` lexical block | 只读 | 当前正则匹配产生的命名 capture |

实现至少提供：

| Binding | 含义 |
|---|---|
| `file.uri` | 当前宏定义所在 SourceUnit 的 canonical URI |
| `file.dir` | 用于解析相对 `src` 的 definition base URI |
| `file.name` | SourceId 的最后一个路径段 |

不存在可继承的动态 `meta.*` 环境，也不存在参数的隐式向下传播。文件 processing instruction 或诊断 metadata 不得偷偷改变 callee 的业务参数；跨 frame 数据只能通过 `arg` 与 `fill` 传递。

语言不提供 `set`、`assign`、`mut` 或 global variable。状态变化只能表示为：构造新参数，然后调用新的 frame。

\[
S_{n+1}=F(S_n)
\]

而不是：

\[
S\leftarrow F(S)
\]

对不存在的 binding 执行 `get` 是 hard error，不返回空字符串。

---

## 8. 插入、匹配与递归

### 8.1 `xs:insert`

`xs:insert` 将一个标量 binding 插入为 XML 文本节点：

```xml
<xs:insert get="arg.name"/>
```

插入内容永远不按 XML markup 重新解析。输出器必须正常转义 `<`、`>`、`&` 等字符。这样 scalar 到 XML 的桥梁始终是安全、单向且可预测的。

### 8.2 `xs:ifr`

`xs:ifr` 对标量字符串执行正则匹配：

```xml
<xs:ifr
    get="arg.value"
    pattern="^(?&lt;head&gt;.)(?&lt;tail&gt;.*)$">

    <xs:insert get="match.head"/>
</xs:ifr>
```

输入有两种互斥形式：

```xml
<xs:ifr get="arg.value" pattern="...">...</xs:ifr>
```

```xml
<xs:ifr str="literal" pattern="...">...</xs:ifr>
```

匹配失败时，`xs:ifr` 产生空节点序列；匹配成功时，展开其 body，并在该 lexical scope 中绑定命名 capture：

```text
match.head
match.tail
```

规则如下：

- 只暴露命名 capture，不暴露 `match.1`、`match.2` 等位置编号；
- 普通捕获组是非法语法；仅分组但不捕获时应使用 `(?:...)`；
- 未参与匹配的可选命名 capture 不产生 binding；
- 离开 `xs:ifr` 后，本层 `match.*` 立即消失；
- sibling 不共享 capture；
- 嵌套 `xs:ifr` 的同名 capture 仅在内层临时遮蔽外层 binding，离开后恢复外层值；
- regex 的输入和输出都只能是 scalar string，不得捕获 XML 节点。

正则方言采用不含 backreference 与 look-around 的 Unicode 正则子集，并支持 `(?<name>...)` 命名 capture。实现应保证匹配时间不因回溯产生灾难性指数增长。

### 8.3 字符串构造与析构

宏核不需要额外的 `concat()`、`replace()` 或 arithmetic：

| 能力 | 语言机制 |
|---|---|
| 构造字符串 | 参数 body 中的 XML character data 拼接 |
| 析构字符串 | regex named capture |
| 分支 | `xs:ifr` |
| 迭代 | recursive `xs:expand` |

例如给一元自然数加一：

```xml
<xs:expand ref="machine:next">
    <xs:arg name="counter">1<xs:insert get="arg.counter"/></xs:arg>
</xs:expand>
```

从非零计数器减一：

```xml
<xs:ifr get="arg.counter" pattern="^1(?&lt;rest&gt;1*)$">
    <xs:expand ref="machine:next">
        <xs:arg name="counter" get="match.rest"/>
    </xs:expand>
</xs:ifr>
```

---

## 9. 函数抽象与组合

命名宏提供一阶函数抽象：小宏可以调用小宏，也可以组合成更大的结构宏。

```xml
<xs:macro name="str:emit-items">
    <xs:param name="list"/>

    <xs:ifr
        get="arg.list"
        pattern="^(?&lt;head&gt;[^,]+),(?&lt;tail&gt;.*)$">

        <Item><xs:insert get="match.head"/></Item>

        <xs:expand ref="str:emit-items">
            <xs:arg name="list" get="match.tail"/>
        </xs:expand>
    </xs:ifr>

    <xs:ifr
        get="arg.list"
        pattern="^(?&lt;last&gt;[^,]+)$">

        <Item><xs:insert get="match.last"/></Item>
    </xs:ifr>
</xs:macro>
```

允许：

- 直接递归；
- 相互递归；
- 跨 SourceUnit 调用静态命名宏；
- 在 scalar argument body 中调用宏，但最终结果仍必须只有文本节点。

不允许：

- 将宏名存入 `arg.*`；
- 从字符串计算 `ref`；
- 把宏作为参数或返回值；
- closure、lambda 或高阶函数；
- 根据调用位置动态捕获文件环境。

当前需要的是 named abstraction、composition 与 recursion，而不是另一套通用函数值系统。

---

## 10. 环、递归与图灵完备性

### 10.1 源码关系不是 DAG

xmlsquish 不要求 source/macro 关系构成有向无环图（Directed Acyclic Graph, DAG）。必须区分两个完全不同的过程：

| 过程 | 遇到环时的行为 |
|---|---|
| SourceUnit 装载 | 通过 SourceId interning 折叠；同一单元不再解析 |
| 宏执行 | 每次调用创建新 frame；环表示真实递归 |

纯 import cycle 只会形成有限的定义装载闭包，不会产生运行时递归。expand cycle 则会不断产生 frame，直到程序停机或触及实现资源 guard。

因此不能用“检测到图上的环”来拒绝宏程序，也不能把递归调用误当作重复定义。

### 10.2 两计数器机编码

使用一元字符串表示自然数：

\[
n\equiv 1^n
\]

则：

| Minsky 两计数器机 | xmlsquish |
|---|---|
| Program counter | 当前命名 MacroDef |
| Counter \(C_0,C_1\) | `arg.c0`、`arg.c1` |
| `INC` | 在参数 body 前置字符 `1` |
| `DEC` | regex capture 去掉一个 `1` |
| `JZ` | `ifr pattern="^$"` |
| goto | 静态 `expand` |
| loop | recursive expansion |

因此以下能力已经足以模拟 Minsky two-counter machine：

\[
\text{recursive expansion}
+\text{conditional}
+\text{regex capture}
+\text{scalar construction}
\]

对任意有限两计数器程序，为每条指令建立一个命名宏，统一声明 `c0`、`c1` 两个参数。初始入口传入编码后的两个计数器；以下是指令翻译模板（`m:next`、`m:zero`、`m:nonzero` 替换为原程序相应跳转标签）：

```xml
<!-- INC C0; goto next -->
<xs:macro name="m:inc">
    <xs:param name="c0"/><xs:param name="c1"/>
    <xs:expand ref="m:next">
        <xs:arg name="c0">1<xs:insert get="arg.c0"/></xs:arg>
        <xs:arg name="c1" get="arg.c1"/>
    </xs:expand>
</xs:macro>

<!-- If C0 = 0 goto zero; otherwise DEC C0 and goto nonzero -->
<xs:macro name="m:test">
    <xs:param name="c0"/><xs:param name="c1"/>
    <xs:ifr get="arg.c0" pattern="^$">
        <xs:expand ref="m:zero">
            <xs:arg name="c0" value=""/><xs:arg name="c1" get="arg.c1"/>
        </xs:expand>
    </xs:ifr>
    <xs:ifr get="arg.c0" pattern="^1(?&lt;rest&gt;1*)$">
        <xs:expand ref="m:nonzero">
            <xs:arg name="c0" get="match.rest"/><xs:arg name="c1" get="arg.c1"/>
        </xs:expand>
    </xs:ifr>
</xs:macro>

<!-- HALT: return the counters as one XML document -->
<xs:macro name="m:halt">
    <xs:param name="c0"/><xs:param name="c1"/>
    <Result><C0><xs:insert get="arg.c0"/></C0><C1><xs:insert get="arg.c1"/></C1></Result>
</xs:macro>
```

对 `C1` 的指令交换两参数的角色即可。非负一元编码在转移中保持不变，零与非零条件互斥且完备；每条机器指令对应一个新的宏展开帧，两个计数器都显式传递。停机标签返回单根文档，其余指令只递归转发结果（允许文档根外格式空白）。因此这一翻译保持状态转移及停机行为；它不依赖文件挂载、隐式入口、外部变量或高阶宏。

宏核在抽象语义上是图灵完备的（Turing-complete），无需 `eval`、Python、整数、算术、mutable variable、`while` 或通用 expression language。

图灵完备只回答“原则上能否表达计算”，不意味着常见宏任务应当以计数器机方式编写。后续 builtin 是否值得存在，应由真实 prompt metaprogramming 的可读性与频率决定，而不是为了堆叠理论能力。

---

## 11. 编译与展开管线

xmlsquish 的逻辑管线为：

```text
*.xml
  ↓ source discovery / compile
frozen SourceUnits + SymbolTable
  ↓ expansion
*.i.xml
  ↓ lowering / cleanup
attribute-free XML with local element names
  ↓ final product whitespace compression
*.o.xml
```

### 11.1 Source discovery / compile

编译阶段：

1. 解析 `xs:entry` 根源码；
2. 静态解析入口与模块声明区中 `import` 的 `src`；
3. 按 SourceId intern 并冻结完整源码闭包；
4. 按 Expanded Name 注册所有 MacroDef；
5. 校验重定义、QName、宏签名、静态展开引用与 regex 语法；
6. 验证所有导入目标都是模块，并检查入口参数契约。

定义解析与文本顺序无关；不存在“调用发生后才偶然注册宏”的运行时副作用。完整冻结闭包和静态宏边为链接时优化（Link-time Optimization, LTO）提供分析基础，但不表示已经实现内联、常量折叠或死代码消除。优化不得改变错误、预算或来源信息契约。

### 11.2 Expansion 与 `*.i.xml`

展开阶段创建入口构造上下文，求值其正文；遇到 `expand` 才创建命名宏展开帧，执行参数求值、slot 替换、正则匹配与递归。入口诊断上下文应与宏帧明确区分，不得伪造隐式 `main` 或宏定义标识。

`*.i.xml` 是带完整 provenance 的中间表示（Intermediate Representation, IR）的可序列化视图。实现应为每个生成节点保留：

- originating SourceId 与 source span；
- 若节点由宏生成，记录 MacroDefId；入口直接生成节点不伪造此标识；
- 当前 frame id；
- parent/caller frame；
- 触发调用的 `expand` 位置；
- 子文件的 file frame 信息。

### 11.3 Lowering 与 `*.o.xml`

从 `*.i.xml` 降低（lowering）为最终提示词 XML 时必须：

- 移除所有 xmlsquish 控制节点；
- 移除**所有属性**，包括普通用户属性、内部属性、`xml:*` 属性，以及默认和带前缀的命名空间声明（namespace declaration）；
- 移除 debug/provenance/file-frame metadata；
- 元素名仅保留局部名（local name），移除前缀，不得产生未绑定的前缀；不同命名空间下的同名元素在产品输出中不再区分；
- 保留节点顺序，且在最终空白压缩前保留文本内容；
- 验证最终结果是格式良好的 XML 文档。

属性不属于最终提示词的产品语义，没有启用或保留属性的输出选项。此规则不禁用源码中 DSL 指令的 `name`、`ref`、`src`、`get` 等属性；它们仍用于编译和展开。源码命名空间仍用于解析符号，中间表示仍保留来源与诊断信息；这些信息不得泄漏到 `*.o.xml`。

例如，`<p:task xmlns:p="urn:example" role="user" xml:space="preserve">Explain.</p:task>` 的最终输出为 `<task> Explain. </task>`。

**产品阶段说明：** 最终产品阶段必须继续压缩空白，生成 `*.o.xml`；提示词中的空白按无意义的格式字符串处理，这是固定产品语义，不得关闭或因 `xml:space` 等属性而改变。空白压缩不属于标量求值或 XML 结构展开，不得提前应用到参数 body、capture 或 fill。最终输出有意丢弃属性、命名空间身份和格式空白，不承诺与输入 XML 的通用数据语义等价。

`--debug` 与 `--explain` 表示同一诊断能力：它们可以保留或展示中间 provenance 和完整 frame chain，但不得改变正常 `.o.xml` 的语义输出。

XML declaration 只作为输入/输出文档声明处理，不参与宏展开。普通用户 processing instruction 作为 XML 数据保留；实现专用诊断信息不得伪装成会泄漏到最终输出的用户节点。

---

## 12. 抽象语义与资源限制

语言抽象语义中的递归深度、展开次数和输出长度不设固定上限。否则语言会因规范常数上界而失去计算完备性。

实现必须提供可配置的资源 guard。`max-depth` 与 `max-expansions` 统计执行帧，包含一个入口构造帧；入口帧不具有宏身份。初始深度和展开计数均为 1，每次 `expand` 再创建一个宏帧。

可配置项：

```text
--max-depth
--max-expansions
--max-output-bytes
```

这些 guard：

- 是 invocation 级运行预算，不是语言语义常量；
- 可以由用户提高；
- 对任意最终停机的程序，都存在足够大的有限预算使其运行完成；
- 触发时产生带完整 frame chain 的确定性错误；
- 不得把部分 `.o.xml` 当作成功结果提交。

于是可以同时保持：

\[
\boxed{\text{抽象语义上图灵完备}}
\]

以及：

\[
\boxed{\text{实现上具有明确资源边界}}
\]

---

## 13. 确定性与错误模型

在入口源码、完整冻结源码闭包、入口参数和资源预算相同的条件下，展开结果必须确定。语言核心不得读取时间、随机数、环境变量或启动 subprocess。

所有规范错误都是 hard error；实现不得猜测、降级或静默选择某个定义。

| 错误类别 | 示例 |
|---|---|
| Source | `src` 无法解析、资源无法读取、最终文档不完整 |
| Namespace | QName 未绑定、用户试图定义 builtin namespace、宏重定义 |
| Signature | 缺失/未知/重复 argument 或 fill |
| Scalar | argument body 产生非文本节点、读取不存在的 binding |
| Regex | pattern 非法、出现位置 capture、重复命名 capture |
| Expansion | 超出 depth/expansion/output budget |
| Output | 最终结果不是格式良好的单一 XML 文档 |

每条诊断至少包含：

- 错误类型和直接原因；
- 当前源码 URI 与 source span；
- 若涉及重定义，同时显示 first definition 与 second definition；
- 若发生在展开阶段，显示从入口到当前节点的 frame chain；
- 若涉及 callee 契约，同时显示 call site 与 definition site。

---

## 14. 核心形式模型

对每个 canonical source URI：

\[
u=CanonicalURI(src)
\]

一次 invocation 只构造一个不可变模块：

\[
M_u=parse(bytes_u)
\]

其中：

\[
M_u=(imports_u,\{f_1,f_2,\ldots,f_n\})
\]

入口另表示为 `E = (SourceId, imports, params, body)`；它不属于宏符号表，也不具有 `MacroDefId`。入口构造上下文 `Context(E, args)` 的求值启动整个文档构造，随后仅 `expand` 创建宏帧。

每个命名宏具有唯一扩展名：

\[
q=(NamespaceURI,LocalName)
\]

并满足：

\[
q\mapsto f
\]

是不可重定义的单值绑定。

一次调用创建：

\[
Frame(f,args,slots,parent)
\]

其可见环境为：

\[
Env=File(definition(f))\oplus Arg(args)\oplus MatchStack
\]

递归表示为：

\[
f\rightarrow Frame_1(f)\rightarrow Frame_2(f)\rightarrow\cdots
\]

MacroDef 不复制，Frame 随执行增长；SourceUnit 的装载闭包与运行时调用链是两个不同的数据结构。

---

## 15. 明确排除的能力

为维持语言边界，核心 DSL 明确不包含：

- `eval`、Python 或任意 subprocess；
- 整数、浮点数、arithmetic 与通用类型系统；
- mutable variable、global state、assignment 与 `while`；
- XPath、XQuery、XML tree capture 或 term rewriting；
- 将 XML slot 当作 scalar value；
- 动态 `src`、动态 `ref` 与 first-class macro；
- lambda、closure、callback 和高阶函数；
- 宏 override、overload、redefinition 或后定义覆盖；
- caller metadata、argument 或 file environment 的隐式传播；
- 固定在语言规范中的递归或展开上限。

这些排除项不是尚未补齐的空白，而是 xmlsquish 的设计边界。

---

## 16. 语言不变量

实现、重构和新 builtin 都必须保持以下不变量：

1. **XML 是数据，宏是计算。** XML 节点不是宏语言的通用值。
2. **Source defines identity.** 同一 canonical `src` 在一次 invocation 中只形成一个冻结 SourceUnit。
3. **Namespace defines names.** 宏符号按 XML Expanded Name 解析，prefix 不参与身份。
4. **Frames define execution.** 定义可以共享，调用 frame 必须彼此独立。
5. **Definitions are immutable.** 宏不能重定义、覆盖或按调用顺序改变。
6. **Invocation data is explicit.** frame 之间只通过 `arg` 与 `fill` 传递数据。
7. **Scalar state is lexical and immutable.** `file.*`、`arg.*`、`match.*` 都只读且生命周期明确。
8. **Relative references bind at definition site.** 同一 MacroDef 在不同 caller 下具有一致的源码解析语义。
9. **Cycles are legal.** Source loading 通过 interning 终止；execution cycle 表示递归。
10. **Abstract semantics are unbounded.** 资源 guard 属于实现预算，不属于语言表达能力上限。
11. **Final output contains only user XML.** 所有控制结构和 provenance 在 lowering 阶段消失。
12. **Errors are explicit.** 不存在隐式 fallback、猜测性解析或静默覆盖。

这组不变量共同定义 xmlsquish 的风格：它不是一门把 XML 重新包装成通用编程语言的 DSL，而是一个以 XML 结构组合为中心、拥有最小但完整宏计算核的预处理系统。
