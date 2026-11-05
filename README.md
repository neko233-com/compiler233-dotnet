# compiler233-dotnet

面向生产环境的 Rust .NET 编译器项目，**以 [Microsoft Roslyn](https://github.com/dotnet/roslyn) 为直接对标对象**。项目目标是在可验证的 C# 语言兼容性、程序集行为和工具链集成基础上，持续优化编译延迟、吞吐量与内存占用；性能比较以相同功能、相同输入和正确输出为前提。

**当前状态：早期开发，尚不适用于生产。** 现有版本仅能编译下文列出的极小 C# 子集，尚未实现 Roslyn 完整语言、程序集或 API 兼容，也没有完整编译性能对标结论。不要将其用于生产构建。

## 产品目标

- **兼容性**：按明确的 C# / .NET 版本逐步实现语法、绑定、类型检查、诊断、CLR 元数据和 CIL；以 Roslyn 的编译结果和实际运行行为进行差分验证。
- **性能**：在等价的正确性和功能覆盖下，对标 Roslyn 的冷启动、完整与增量编译、并发吞吐、p50/p95/p99、分配量及峰值内存，目标是在主要工作负载上显著领先。
- **集成**：逐步覆盖多文件项目、引用解析、CLI/MSBuild、可复现构建及开发工具接口。达到兼容性要求前，不宣称可以替代 `csc`、MSBuild 或 Roslyn API。
- **平台**：Windows x86-64、Linux x86-64、macOS **ARM64**。macOS Intel 不在支持范围；项目会明确拒绝该编译目标。平台 CI 通过只说明当前已实现功能通过该平台的检查。

## 当前已实现

| 能力 | 状态和边界 |
| --- | --- |
| 原生 Rust 语法前端 | 支持有限 C# 子集，构造 token 与语法树；超出范围直接报错。 |
| 受限编译与 DLL 生成 | 将支持的单文件子集编译为 .NET 10 DLL；使用 Rust 的 `hao` 库生成 PE、元数据与 CIL。 |
| 语义检查 | 检查重复类型/方法/参数、未定义参数引用、受限整型常量表达式等；尚非完整 C# 类型系统和诊断。 |
| 运行时对照 | 测试将输出 DLL 由 .NET 实际加载和执行，与 Roslyn 的结果和异常比较；覆盖范围见 `tests/fixtures`。 |
| 对照工具 | tree-sitter C# 用于独立的语法实验；Roslyn 使用本机 .NET 10 SDK 自带程序集作为参考。 |

受限编译器只接受 ASCII 源码、顶层 `class`，其中包含 `static int` 方法、`int` 参数，以及每个方法中一条 `return`。表达式支持参数、`int` 范围内的十进制常量、括号和二元 `+ - * /`。输出为库而非可直接启动的程序；不支持预处理、泛型、引用程序集、跨方法调用、调试信息或 VB。语法检查通过不代表程序通过了语义检查。

## 构建与使用

准备 Rust 稳定工具链、C 编译工具链（tree-sitter 依赖），以及测试所需的 .NET 10 SDK 和 PowerShell 7。macOS 请使用 Apple Silicon 原生工具链。

```powershell
cargo build --release --locked
cargo test --locked
./target/release/compiler233-dotnet compile tests/fixtures/Arithmetic.cs Arithmetic.dll
./target/release/compiler233-dotnet check-subset tests/fixtures/Arithmetic.cs
```

`compile` 在完整解析、语义检查和内存生成成功后才替换输出文件；失败时保留已有输出。命令成功返回 0，语法或语义错误返回 1，参数或 I/O 错误返回 2。`check` 使用 tree-sitter 检查更宽的 C# 语法，不会进行语义分析或生成程序集。

在 PowerShell 7 中运行针对性验证：

```powershell
./tests/runtime.ps1
./benchmarks/run.ps1 -RustMode Subset -Source benchmarks/corpus/Subset.cs -Iterations 500 -Warmup 50
./benchmarks/run.ps1 -RustMode Subset -Stage Compile -Source tests/fixtures/Arithmetic.cs -Iterations 100 -Warmup 30
./benchmarks/check-subset.ps1
```

运行时测试使用真实 .NET 运行时加载生成的程序集，并对照 Roslyn 检查结果、异常和部分元数据；同时检查无效输入不会覆盖旧产物。这些用例不构成完整语言兼容性证明。

## 性能基准

```powershell
./benchmarks/generate-subset.ps1 -Output benchmarks/results/generated.cs -Methods 200
./benchmarks/run.ps1 -RustMode Subset -Source benchmarks/results/generated.cs -Iterations 300 -Warmup 100
```

脚本对相同 UTF-8 输入检查 SHA-256，预热后记录所有样本及 p50/p95。两种实现分别在独立进程中运行，使用的 Roslyn 随本机 SDK 版本变化；原始 JSON 需要和输入一起保存。默认只测语法解析；`-Stage Compile` 测量内存中的解析、绑定与 DLL 生成。两种模式均不包含进程启动、文件读取或执行。

原生前端的语法覆盖和语法树内容比 Roslyn 少，因此该微基准**不是功能等价的速度比较**。[本机实验记录](benchmarks/RESULTS.md)仅用于跟踪早期实现。正式性能结论需要固定工具版本、代表性项目、正确性验证、多次独立运行、内存与完整编译测量。

## 后续里程碑

1. 定义具体 C# 版本与诊断兼容目标，扩大语法、语义和 Roslyn 差分测试语料。
2. 实现完整的符号绑定、类型系统、程序集引用、CIL 与元数据，并覆盖异常、泛型和跨程序集行为。
3. 打通多文件项目、增量编译、MSBuild/CLI 和可复现构建；按目标平台发布并验收。
4. 建立正确性门禁和等价工作负载基准，持续跟踪编译延迟分布、吞吐及内存，与固定版本 Roslyn 对比。

MIT License；依赖项遵循各自许可证。
