# compiler233-dotnet

面向生产环境的 Rust .NET 编译器项目，**以 [Microsoft Roslyn](https://github.com/dotnet/roslyn) 为直接对标对象**。项目目标是在可验证的 C# 语言兼容性、程序集行为和工具链集成基础上，持续优化编译延迟、吞吐量与内存占用；性能比较以相同功能、相同输入和正确输出为前提。

**当前状态：早期开发，尚不适用于生产。** 现有版本仅能编译下文列出的极小 C# 子集，尚未实现 Roslyn 完整语言、程序集或 API 兼容，也没有完整编译性能对标结论。不要将其用于生产构建。

## 产品目标

- **兼容性**：按明确的 C# / .NET 版本逐步实现语法、绑定、类型检查、诊断、CLR 元数据和 CIL；以 Roslyn 的编译结果和实际运行行为进行差分验证。
- **性能**：在等价的正确性和功能覆盖下，对标 Roslyn 的冷启动、完整与增量编译、并发吞吐、p50/p95/p99、分配量及峰值内存，目标是在主要工作负载上显著领先。
- **集成**：逐步覆盖多文件项目、引用解析、CLI/MSBuild、可复现构建及开发工具接口。达到兼容性要求前，不宣称可以替代 `csc`、MSBuild 或 Roslyn API。
- **替代边界**：可验收的"替代 Roslyn"以 **csc 命令行契约**为定义——参数字典、响应文件、诊断格式、退出码和产物；`Microsoft.CodeAnalysis` 托管 API 兼容不在范围内。HotReload233 的接入点是该契约（见下文"Roslyn 替代边界"），但本仓库当前不改动 HotReload233，接入放在语言覆盖达标之后。
- **平台**：Windows x86-64、Linux x86-64、macOS **ARM64**。macOS Intel 不在支持范围；项目会明确拒绝该编译目标。平台 CI 通过只说明当前已实现功能通过该平台的检查。

## 当前已实现

| 能力 | 状态和边界 |
| --- | --- |
| 原生 C# 语法前端 | 自研词法 + 递归下降解析：预处理指令与条件编译、命名空间、类型、成员、语句与表达式；不支持的构造带偏移报错。对 `tuanjie-project-sf` 全部 8530 个脚本语法层 8530/8530 通过，误接受 0。 |
| 受限编译与 DLL 生成 | 将支持的单文件子集编译为 .NET 10 DLL；使用 Rust 的 `hao` 库生成 PE、元数据与 CIL。 |
| 语义检查 | 检查重复类型/方法/参数、未定义参数引用、受限整型常量表达式等；尚非完整 C# 类型系统和诊断。 |
| 运行时对照 | 测试将输出 DLL 由 .NET 实际加载和执行，与 Roslyn 的结果和异常比较；覆盖范围见 `tests/fixtures`。 |
| 对照工具 | tree-sitter C# 用于独立的语法实验；Roslyn 使用本机 .NET 10 SDK 自带程序集作为参考。 |
| csc 兼容前端 | 解析 `@响应文件`（引号、`#` 注释、嵌套与环检测）以及 Unity/HotReload233 实际传入的选项；诊断按 `path(line,col): error CODE: message` 输出，退出码 0/1；`-report:<path>` 写出机器可读报告。多文件、非 library 目标、签名与资源等未实现能力明确拒绝，且不产出程序集。 |
| HotReload233 接入 | 接入点已确认为 csc 进程契约；本仓库暂不改动 HotReload233，待语言覆盖达标后再接。 |
| 真实工程度量 | `benchmarks/compile-survey.ps1` 用 `-reference` 指向工程 `Library/ScriptAssemblies`，抽样编译真实脚本并统计失败原因分布 |
| 差分门禁 | 以 Roslyn 为 oracle 对同一批 `.cs` 双编译，输出 accept / reject / false_reject / false_accept 分类与覆盖率；false_accept 默认上限为 0。 |

受限编译器只接受 ASCII 源码、顶层 `class`，其中包含 `static int` 方法、`int` 参数，以及每个方法中一条 `return`。表达式支持参数、`int` 范围内的十进制常量、括号和二元 `+ - * /`。输出为库而非可直接启动的程序；不支持预处理、泛型、引用程序集、跨方法调用、调试信息或 VB。语法检查通过不代表程序通过了语义检查。

## 构建与使用

准备 Rust 稳定工具链、C 编译工具链（tree-sitter 依赖），以及测试所需的 .NET 10 SDK 和 PowerShell 7。macOS 请使用 Apple Silicon 原生工具链。

```powershell
cargo build --release --locked
cargo test --locked
./target/release/compiler233-dotnet compile tests/fixtures/Arithmetic.cs Arithmetic.dll
./target/release/compiler233-dotnet check-subset tests/fixtures/Arithmetic.cs
./target/release/compiler233-dotnet csc @Library/HotReload233/Patches/compile.rsp -report:patch-report.json
./target/release/compiler233-dotnet frontend tests/fixtures/differential -report:syntax.json
./target/release/compiler233-dotnet version
```

`compile` 在完整解析、语义检查和内存生成成功后才替换输出文件；失败时保留已有输出。命令成功返回 0，语法或语义错误返回 1，参数或 I/O 错误返回 2。`check` 使用 tree-sitter 检查更宽的 C# 语法，不会进行语义分析或生成程序集。

`csc` 接受与 Roslyn 相同的调用形态（`@响应文件`、`-选项`、`.cs` 源文件路径），成功返回 0、诊断错误返回 1。诊断代码中，`CS####` 表示与 csc 语义一致的既有错误，`CD233####` 表示本项目尚未实现的边界（例如 `CD2330003` 多文件编译未实现）。

`frontend` 在一个进程里批量做语法校验，不写任何程序集，只输出（或按 `-report:` 写入）一份 JSON：每个文件是否通过、类型/成员计数与首个错误的行列。这是"超快增量验证"的入口，也是差分门禁 `-Mode frontend` 使用的命令。

在 PowerShell 7 中运行针对性验证：

```powershell
./tests/runtime.ps1
./benchmarks/run.ps1 -RustMode Subset -Source benchmarks/corpus/Subset.cs -Iterations 500 -Warmup 50
./benchmarks/run.ps1 -RustMode Subset -Stage Compile -Source tests/fixtures/Arithmetic.cs -Iterations 100 -Warmup 30
./benchmarks/check-subset.ps1
```

运行时测试使用真实 .NET 运行时加载生成的程序集，并对照 Roslyn 检查结果、异常和部分元数据；同时检查无效输入不会覆盖旧产物。这些用例不构成完整语言兼容性证明。

## Roslyn 替代边界（M0）

HotReload233 的补丁编译链是：Unity/Tuanjie 的 Bee 生成响应文件 → `HotReload233.Server` 执行 `mono csc.exe @rsp`（或 `dotnet csc.dll @rsp`）→ 反射读取补丁 DLL 的 IL。Roslyn 在这里以 **csc 进程契约**出现，不是托管 API，因此替代点在命令行与程序集产物，而不在 `Microsoft.CodeAnalysis`。

| 层次 | 状态 | 说明 |
| --- | --- | --- |
| csc 命令行契约 | M0 已实现 | `@rsp`、项目实际使用的选项、`file(line,col): error CODE: message`、退出码 0/1、`-report` |
| Unity/Tuanjie 脚本程序集全量编译 | 未实现 | 需要完整语法、绑定、CIL/元数据/PDB 与引用程序集；当前仅支持受限子集 |
| 多文件与引用解析 | 未实现 | 多文件请求以 `CD2330003` 拒绝，报告中给出前端扫描统计 |
| 源生成器 / 分析器 | 未承诺 | `-analyzer` 等开关被记录但不执行；源生成器依赖 Roslyn 自身 API，暂不在范围内 |
| `Microsoft.CodeAnalysis` API 兼容 | 不在范围 | IDE、分析器宿主等托管 API 不属于替代目标 |

计划中的接入点（尚未改动 HotReload233）：由 HotReload233.Server 直接启动本编译器并传 `@rsp`，替代 `mono csc.exe @rsp`。本仓库只保证该进程契约，接入留到语言覆盖达标后进行。

在真实项目上量化失败面（不产出程序集，只统计前端覆盖情况）：

```powershell
./target/release/compiler233-dotnet csc @<unity rsp> -report:benchmarks/results/unity-report.json
```

## 差分门禁（M1/M2）

以 Roslyn 作为 oracle，对同一批 `.cs` 分别编译并分类，量化"能编译多少"和"是否会误接受"：

```powershell
./tests/differential.ps1                  # 默认：语法层快速模式
./tests/differential.ps1 -Mode compile    # 完整 csc 产物模式（每文件一次进程）
./tests/differential.ps1 -Corpus D:\Game\Assets -References D:\Game\Library\ScriptAssemblies -LangVersion 9.0 -MaxFiles 500
```

| 模式 | 比较对象 | 用途 |
| --- | --- | --- |
| `frontend`（默认） | 本项目语法前端 vs Roslyn `--syntax-only` | 秒级反馈，衡量语法覆盖，不写 DLL |
| `compile` | 本项目 csc 产物 vs Roslyn 完整编译 | 端到端正确性，含语义与产物 |

| 分类 | 含义 |
| --- | --- |
| `accept` | 双方都接受，即当前真实覆盖率 |
| `reject` | 双方都拒绝 |
| `false_reject` | Roslyn 接受、本项目拒绝，即语言缺口 |
| `false_accept` | 本项目接受、Roslyn 拒绝，最危险；CI 默认上限 0，超出即失败 |

产物写入 `benchmarks/results/differential/roslyn.json` 与 `differential.json`（逐文件分类、双方首条诊断）；中间文件在结束时删除，`-KeepWork` 可保留。默认语料为 `tests/fixtures/differential`；对真实项目请用 `-Corpus` 指向源目录，`compile` 模式还需用 `-References` 提供引用程序集目录，否则 Roslyn 会因缺少 UnityEngine 等引用而误判。

本机基线（`-Mode frontend`，`tuanjie-project-sf` 全部 8530 个 `.cs`，C# 9）：`accept 8530 / false_reject 0 / false_accept 0`；前端单独处理全量耗时约 0.85 秒，差分端到端约 9 秒（含构建检查与 Roslyn 侧分析），报告约 1.8 MB。语法通过不代表语义或产物正确，产物正确性由 `-Mode compile` 与 `tests/runtime.ps1` 负责。

## 真实编译（M3/M4 起点）

`csc` 模式不再只做子集校验：它构建 AST、解析名字与类型、发射可执行 CIL，并把多个源文件合并为一个程序集。当前支持的核心：

- 命名空间（块式与文件级）、类（含静态类）、静态/实例字段与初始化器（实例初始化器在构造函数，静态初始化器在 `.cctor`）
- 对象：`new T(...)`（按参数个数匹配构造函数）、`new T { Name = value }` 对象初始化器（字段与属性）、`obj.field` 读写、`obj.Method(...)` 与 `this.Method(...)` 调用
- 属性：`get`/`set` 访问器（自动属性生成后备字段）、`obj.Prop` 读取与 `obj.Prop = v` 赋值、静态属性
- 数组：`new int[n]`、`a[i]` 读写、`a.Length`、`foreach` 遍历（`int`/`bool`/`string`/类元素）
- 继承：基类（本地或引用程序集）、`virtual`/`override` 虚调用、`: base(args)` 基类构造、基类方法继承与基类-派生类赋值兼容
- 接口：`interface` 声明（抽象方法、无方法体）、类实现接口（`InterfaceImpl` + 隐式实现标记为 `virtual final newslot`）、接口类型变量与 `callvirt` 派发
- 外部基类继承：`class X : System.Exception` 可编译，构造函数 `: base(args)`、继承的字段与属性（`get_X`）沿引用程序集的 `extends` 链解析
- 静态/实例方法、参数，重载按参数个数解析
- 语句：局部声明、赋值、`if/else`、`while`、`for`、`return`、`break`/`continue`、`base.M(...)` 基类调用、块
- 表达式：`int/float/bool/string/object` 与 `null` 字面量、`enum` 常量、算术与比较（含字符串/布尔相等、`== null`）、`&&/||` 短路、`??`、`?:` 三元、`(T)x` 转换与 `as`、`!`、`++`/`--`、`+=`/`-=`（含字段与属性）、字符串 `+` 拼接、赋值表达式、`string.Empty`/`int.MaxValue` 等预定义成员
- 外部引用：`-reference:` 指定的程序集参与类型与方法解析（全名、唯一短名、`System.Text.Regex` 这类限定名），跨程序集静态/实例调用可用；`tests/compile.ps1` 会对 `System.Text.RegularExpressions` 与 Roslyn 构建的泛型库做真实对照
- 泛型方法调用：`Foo<T>(...)` 通过 MethodSpec 实例化（外部程序集的方法），返回类型按实参替换`MVar`
- 泛型类型实例：`List<int>` 等引用程序集里的泛型类型，字段/局部/参数签名用`GenericInst`，成员通过`TypeSpec`父的 MemberRef 调用（`list.Add(x)`、`list.Count`），`new List<int>()` 可用；泛型**类型定义**仍不支持
- 调用：本程序集的静态/实例方法与构造函数；BCL 的 `System.Math.Max/Min/Abs`、`System.Convert.ToString`、`System.String.Concat`、`System.Console.Write/WriteLine`（按需添加 `System.Console` 程序集引用）

尚未支持、会被明确拒绝而不是静默降级：泛型类型定义（`class Foo<T>`）、多维/交错数组、lambda、`try`/`async`/`switch`。

验收方式：`./tests/compile.ps1` 用 Roslyn 编译同一批源码，分别加载两个程序集并执行 `Demo.Program.Run()`，结果不一致即失败。

## 发布

发布由 GitHub tag 触发，本地也可直接打包：

```powershell
./scripts/package.ps1 -Version 0.1.0
```

产物为 `compiler233-dotnet-<version>-<平台>.zip`（含可执行文件、README、LICENSE、THIRD_PARTY_NOTICES）与同名 `.sha256`。推送 `v*` tag 时 `.github/workflows/release.yml` 在 win-x64、linux-x64、macos-arm64 三个平台分别构建并上传 Release 资产。macOS Intel 不在支持范围。

## 性能基准

```powershell
./benchmarks/generate-subset.ps1 -Output benchmarks/results/generated.cs -Methods 200
./benchmarks/run.ps1 -RustMode Subset -Source benchmarks/results/generated.cs -Iterations 300 -Warmup 100
```

脚本对相同 UTF-8 输入检查 SHA-256，预热后记录所有样本及 p50/p95。两种实现分别在独立进程中运行，使用的 Roslyn 随本机 SDK 版本变化；原始 JSON 需要和输入一起保存。默认只测语法解析；`-Stage Compile` 测量内存中的解析、绑定与 DLL 生成。两种模式均不包含进程启动、文件读取或执行。

原生前端的语法覆盖和语法树内容比 Roslyn 少，因此该微基准**不是功能等价的速度比较**。[本机实验记录](benchmarks/RESULTS.md)仅用于跟踪早期实现。正式性能结论需要固定工具版本、代表性项目、正确性验证、多次独立运行、内存与完整编译测量。

## 后续里程碑

- **M0（已完成）**：csc 兼容命令行前端、诊断与退出码、`-report` 报告、CLI 契约测试、tag 驱动的多平台发布。
- **M1（已完成）**：Roslyn 差分门禁——同输入双编译，输出 accept / reject / false_reject / false_accept 分类与覆盖率，false_accept 默认上限 0；语料可指向任意 `.cs` 目录。
- **M2（进行中）**：自研词法 + 递归下降解析已落地，覆盖预处理/条件编译、命名空间、类型、成员、语句与表达式；真实 Unity 脚本语法层 8530/8530 通过，误接受 0。剩余工作：trivia/span 保真、增量解析，以及换更大语料后的长尾构造。
- **M3（进行中）**：名字/类型检查与 CIL 发射已打通（类、静态/实例字段与方法、控制流、BCL 调用），由 	ests/compile.ps1 与 Roslyn 执行结果对照；待办：
ew/对象与数组、泛型、接口继承、属性、lambda、异常与 async。
- **M4**：CIL、元数据、便携 PDB、引用程序集，以及目标框架矩阵（netstandard2.0/2.1、net472、net8/9/10）。
- **M5**：性能对标（冷启动、全量/增量 p50/p95、峰值内存）与 GitHub tag 发布（win-x64、linux-x64、macos-arm64）。

MIT License；依赖项遵循各自许可证。
