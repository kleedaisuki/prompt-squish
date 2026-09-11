# 宏展开与 IR 复用 / Macro expansion and IR reuse

## 范围与设计 / Scope and design

本轮优化保留 `.o.xml`、`.i.xml`、诊断顺序和资源预算语义，不改变语言或产物格式。
This optimization preserves final/intermediate XML, diagnostic ordering and resource budgets; it does not change the language or artifact format.

1. **显式冻结快照（PreparedProgram）**：`Compiler::prepare` 只解析、验证、链接一次；`PreparedProgram::expand` 接受新的 `CompileOptions`。每次展开独立创建参数环境、帧和预算。源码改变后必须重新准备，没有全局路径缓存或隐式失效协议。
   **Explicit snapshots:** prepare once, expand with fresh inputs and budgets. Reprepare to observe source edits; no global path cache or implicit invalidation protocol exists.
2. **共享静态载荷（Payload）**：每个已执行的静态语法节点缓存 XML 事件载荷。重复宏展开复用载荷，但输出事件仍保留各自的位置和展开帧。动态 `insert` 不缓存为静态节点结果。
   **Shared static payloads:** repeated expansions reuse immutable event bytes, not occurrence provenance. Dynamic insertions remain invocation-specific.
3. **惰性 IR 转义（Lazy Escaping）**：仅在事件真正进入来源 IR 时进行第二层转义，并缓存结果。仅作为标量参数消费的临时返回值不做这一步。
   **Lazy IR escaping:** discarded scalar-return events do not pay diagnostic-embedding costs. Surviving payloads share their escaped representation.
4. **来源信息与输出分配**：一次序列化内按逻辑路径复用 URI、目录和文件名转义；直接写入目标字符串，最终 XML 按已计费长度预分配。
   **Serialization:** cache source metadata by logical path, write directly to output buffers, and reserve the already-accounted final XML length.

缓存只持有上下文无关的数据；**没有完整宏结果记忆化（Memoization）**，不会因缓存命中而省略递归展开、参数求值、预算检查或来源帧。
Only context-free data is cached. There is no whole-macro memoization that could suppress recursion, argument evaluation, accounting or provenance frames.

## 测量结果 / Measurements

基线为 `7c1e10d`，加入与候选完全相同的基准文件；基线在独立构建目录重新编译，避免 Cargo 构建缓存误复用。环境为 Windows 11、Core i9-12900H、Rust 1.97.1、release 构建。三轮独立进程交替 AB/BA，每组预热三次、采样十一次；微小输入每个样本批量执行一百次。
Baseline production code is `7c1e10d` plus the identical benchmark harness, rebuilt in an isolated target directory. Three alternating process pairs use three warmups and eleven samples per workload; tiny inputs run in batches of one hundred.

完整编译包含解析、链接、展开、IR 序列化及结果释放，**不包含 CLI 启动、token 计数、文件 I/O 或最终空白压缩**。下表时间是各轮中位数的中位数，降幅是各轮配对比值的中位数，因此不应从四舍五入后的时间重新推导精确百分比。
Full compilation includes parsing, linking, expansion, IR serialization and result destruction, but excludes CLI startup, token counting, filesystem I/O and final squishing. Times are medians of round medians; reductions use median paired ratios.

| 工作负载 / Workload | 基线 / Baseline | 优化后 / Optimized | 耗时下降 / Reduction |
|---|---:|---:|---:|
| 微小入口 / Tiny | 22.8 µs | 18.8 µs | 17.4% |
| 2,000 次静态宏 / Static expansions | 35.67 ms | 12.77 ms | 64.2% |
| 48 层填充转发 / Fill forwarding | 14.47 ms | 9.94 ms | 31.3% |
| 384 层递归 / Recursion | 3.60 ms | 2.13 ms | 40.7% |
| 1,000 次属性密集宏 / Attribute-heavy | 28.05 ms | 5.30 ms | 81.1% |
| 1,000 个非重复节点 / Unique nodes | 6.16 ms | 4.35 ms | 29.4% |
| 128 字符递归返回组合 / Scalar return composition | 1.37 ms | 1.11 ms | 18.1% |

原始逐轮样本、二进制 SHA-256 与阶段数据见 [paired-results.json](paired-results.json)；最初五组探索性基线保存在 [baseline.json](baseline.json)。样本是描述性测量，不是置信区间，也不承诺其他硬件上的相同比例。
Raw rounds, binary hashes and phase measurements are in the paired report. The initial five-case exploratory baseline is retained separately. These are descriptive results, not confidence intervals or guarantees for other machines.

### 实际 CLI / Real CLI

另用外部 GSP-4.0.0 的 17 个源码文件、7 个入口运行七轮 release CLI 配对测试，包括启动、token 计数和文件写入。基线中位数 **178.1 ms**，候选 **174.5 ms**，配对中位数仅改善约 **1.2%**；单轮波动跨越零，**不能据此宣称实际 GSP 构建显著提速**。这限制了微基准结果的外推范围。14 个最终及中间产物、完整报表和诊断均逐字节相同。
Seven CLI pairs on the real, external GSP dataset have medians of 178.1 ms and 174.5 ms, but only a 1.2% median paired improvement with individual rounds crossing zero. This does not establish a material end-to-end GSP speedup. All fourteen artifacts and console results are byte-identical. See [gsp-cli-results.json](gsp-cli-results.json); private prompt sources are not published.

### 代价与限制 / Costs and limitations

- 单独的源码发现/链接阶段并非全面加速：属性密集场景约从 0.44 ms 增至 0.66 ms，唯一节点场景约从 0.99 ms 增至 1.13 ms。阶段测量发生在同进程完整编译之后，分配器状态可能影响数据，不能把全部差异单独归因于节点大小。
  Discovery/link-only measurements regress in some cases. Allocator history can confound attribution because phases follow full compilation in the same process.
- 接受这一取舍的依据是：七组**包含准备成本**的完整编译均改善，且准备后的多次展开复用已编译正则、链接结果与静态载荷；不把阶段回归隐藏为“所有操作都更快”。
  The trade-off is accepted because all seven full-compilation workloads improve including preparation costs, and repeated expansions amortize preparation. Not every operation is faster.
- 快照保留已访问节点的缓存到释放时；尚未测量峰值内存，**不声称总内存下降**。缓存不随重复展开次数无界增长，但可能随新分支访问的静态节点增加。
  Reached-node caches remain until the snapshot is dropped. Peak memory has not been measured; no total-memory reduction is claimed.
- `Rc` 与 `OnceCell` 用于单线程快照复用，不提供跨线程共享同一 `PreparedProgram` 的承诺。不同编译可独立运行。
  Prepared snapshots are single-thread-owned, not a concurrent shared-program API.
- CLI 仍为每个独立入口准备自己的快照；本轮没有偷偷改变批量源码冻结边界。
  CLI entries still prepare independent snapshots; batch source-freezing semantics are unchanged.

## 正确性检查 / Correctness checks

- 136 项常规 Rust 测试通过；另有一个显式忽略的手动性能基准。
- 25 个基线/候选差分用例严格比较产物、退出码、完整报表和诊断；覆盖按值参数、填充来源、递归返回、Unicode/转义、导入去重及预算临界值。见 [semantic-results.json](semantic-results.json)。
- 新增快照复用、参数不泄漏、失败后恢复、重新准备观察新源码，以及共享载荷不合并帧的测试。
- 严格 Clippy、Rust 1.88 最低版本检查和真实 CLI 网站示例一致性检查通过。

136 regular Rust tests pass, plus one explicitly ignored manual benchmark. Differential tests compare actual bytes; microbenchmark fingerprints alone are not a proof of equivalence. Snapshot tests cover freezing, isolated inputs, failed-run recovery and cache/provenance separation.

## 复现 / Reproduction

```powershell
cargo test --all-features --locked
cargo test --release compiler::perf_tests::compiler_baseline -- --ignored --nocapture --test-threads=1
python docs/performance/compare.py BASELINE_TEST_EXE CANDIDATE_TEST_EXE --report paired-results.json --rounds 3
python docs/performance/differential.py BASELINE_CLI CANDIDATE_CLI
python docs/performance/cli_compare.py BASELINE_CLI CANDIDATE_CLI GSP_SOURCE_DIR --report gsp-results.json --rounds 7
```

重建基线时，将 `7c1e10d` 导出到独立目录，仅复制当前 `src/compiler/perf.test.rs` 并在其 `compiler/mod.rs` 注册测试模块；使用独立 `--target-dir` 构建，不在两个不同源码目录之间复用同一 crate 产物。从 Cargo 的 `compiler-artifact` JSON 记录取得测试可执行文件，不猜测哈希文件名。比较时使用同一工作目录，不并发运行构建或其他基准。
To rebuild the baseline, export `7c1e10d`, add only the identical harness and test-module registration, and build with a separate target directory. Obtain executable paths from Cargo artifact JSON rather than guessing hashed names. Run comparisons from one working directory without concurrent builds or benchmarks.

也可直接检出仅添加上述基准工具、尚未包含性能优化的 `e59963c` 提交。
Alternatively, use benchmark-only commit `e59963c`, which already includes the harness but not these optimizations.

GSP 是本地私有数据集；无该目录时仍可完全复现七个合成工作负载及差分用例。
The GSP corpus is private and external; all seven synthetic workloads and differential cases remain reproducible without it.

## 设计依据 / Design references

- [rustc 内存管理](https://rustc-dev-guide.rust-lang.org/memory.html)：支持编译生命周期内复用不可变数据，但不意味着需要全局缓存。
- [rust-analyzer 语法树设计](https://rust-analyzer.github.io/book/contributing/syntax.html)：区分共享内容与每个出现位置的身份，启发载荷与来源帧分离。
- [LLVM metadata](https://llvm.org/docs/LangRef.html#metadata)：内容相等不意味着具有独立身份的诊断节点可以合并。
- [Appel / Gonçalves 的 hash-consing 研究](https://www.cs.utexas.edu/~hunt/research/hash-cons/hash-cons-papers/Hash-Consing-GC.pdf)：驻留也有查找成本；本轮复用同一定义的载荷，不引入全树哈希去重。

These references motivate lifetime-scoped sharing and occurrence identity separation. They are design guidance, not evidence that a particular optimization is faster here; the measurements above provide that evidence.
