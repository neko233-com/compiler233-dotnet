# compiler233-dotnet

面向生产环境的 Rust .NET 编译器项目，**以 [Microsoft Roslyn](https://github.com/dotnet/roslyn) 为功能与生态兼容目标**。项目目标是在完整的 C# 语言支持、正确的程序集行为和工具链集成基础上，持续优化编译延迟、吞吐量与内存占用，力争性能领先。

**当前状态：早期开发，尚不适用于生产。** 现有版本仅能编译下文列出的极小 C# 子集，尚未实现完整语言、程序集或 API 兼容，也未完成生产环境验收。不要将其用于生产构建。

## 产品目标

- **兼容性**：按明确的 C# / .NET 版本逐步实现语法、绑定、类型检查、诊断、CLR 元数据和 CIL；按语言及运行时契约验证产物与实际运行行为。
- **性能**：持续优化冷启动、完整与增量编译、并发吞吐及内存占用，目标是在主要工作负载上显著领先；当前优先完成正确性与生产可用性，不要求竞品速度对比。
- **集成**：逐步覆盖多文件项目、引用解析、CLI/MSBuild、可复现构建及开发工具接口。达到兼容性要求前，不宣称可以替代 `csc`、MSBuild 或 Roslyn API。
- **替代目标**：包括 csc 命令行契约、C# 和 Visual Basic、分析器与源生成器，以及 `Microsoft.CodeAnalysis` 托管 API 兼容；这些能力目前只有少量 csc/C# 子集实现。HotReload233 的接入点是 csc 进程契约，待相关能力及实际链路验证后再接入。
- **平台**：当前优先实现并验收 Windows x86-64；macOS **ARM64** 在后续补全，macOS Intel 不在目标范围。现有 Linux 和 macOS CI / 发布配置保留，但不作为当前 Windows 阶段的验收前提。平台 CI 通过只说明当前已实现功能通过该平台的检查。

## 当前已实现

| 能力 | 状态和边界 |
| --- | --- |
| 原生 C# 语法前端 | 自研词法 + 递归下降解析：预处理指令与条件编译、命名空间、类型、成员、语句与表达式；不支持的构造带偏移报错。2026-11-06 对 `tuanjie-project-sf/Assets` 的 8534 个脚本做语法层差分，双方全部接受；结果不代表语义或编译覆盖。 |
| 受限编译与 DLL 生成 | `csc` 支持部分多文件 C# 和外部引用，输出 .NET DLL；`compile` 命令使用更小的独立子集。均使用 Rust 的 `hao` 库生成 PE、元数据与 CIL。 |
| 语义检查 | 检查重复类型/方法/参数、未定义参数引用、受限整型常量表达式等；尚非完整 C# 类型系统和诊断。 |
| 访问控制 | 部分支持顶层类型、字段、方法、构造函数与属性访问器的可见性和部分访问检查；友元程序集、嵌套类型及完整继承规则尚未覆盖。 |
| 运行时对照 | 测试将输出 DLL 由 .NET 实际加载和执行，与 Roslyn 的结果和异常比较；覆盖范围见 `tests/fixtures`。 |
| 对照工具 | tree-sitter C# 用于独立的语法实验；Roslyn 使用本机 .NET 10 SDK 自带程序集作为参考。 |
| csc 兼容前端 | 解析 `@响应文件`（引号、`#` 注释、嵌套与环检测）以及部分常用选项；`-define` 参与条件编译。诊断按 `path(line,col): error CODE: message` 输出，退出码 0/1；`-report:<path>` 写出机器可读报告。非 library 目标、签名与资源等未实现能力会拒绝；部分其他选项尚被忽略，不能认为命令行行为完整等价。 |
| HotReload233 接入 | 接入点已确认为 csc 进程契约；本仓库暂不改动 HotReload233，待语言覆盖达标后再接。 |
| 真实工程度量 | `benchmarks/compile-survey.ps1` 用 `-reference` 指向工程 `Library/ScriptAssemblies`，抽样编译真实脚本并统计失败原因分布 |
| 差分门禁 | 以 Roslyn 为 oracle 对同一批 `.cs` 双编译，输出 accept / reject / false_reject / false_accept 分类与覆盖率；false_accept 默认上限为 0。 |

独立的 `compile` 子集命令只接受 ASCII 源码、顶层 `class`，其中包含 `static int` 方法、`int` 参数，以及每个方法中一条 `return`。表达式支持参数、`int` 范围内的十进制常量、括号和二元 `+ - * /`。更广的 `csc` 能力见下文，但仍远未覆盖 C#；语法检查通过不代表程序通过了语义检查。

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

`csc` 在写入前检查输出、引用输出和报告是否与源文件、引用程序集、响应文件或其他输出指向同一路径；冲突时报错并保留已有文件。

`-refout:<path>` 可针对当前支持的 C# 子集额外生成引用程序集：保留实现程序集的身份和对外元数据，使用 `ReferenceAssemblyAttribute` 与无实现的方法体。测试使用 Roslyn 编译下游消费者，再绑定实现程序集执行。当前访问修饰符和元数据生成仍不完整，因此不能保证引用程序集的公开 API 与 Roslyn 完全一致；后续需要覆盖更多类型、属性、泛型、特性和程序集引用。

分析器、生成器相关输入、`-checked+`、调试信息、确定性输出、警告转错误、XML 文档等尚未实现；这些选项会报错。其他尚被忽略的选项会列入报告，不应将其视为与 Roslyn 等价。

`csc` 接受常见的调用形态（`@响应文件`、`-选项`、`.cs` 源文件路径），成功返回 0、诊断错误返回 1。诊断代码中，`CS####` 表示借用的 csc 错误代码，`CD233####` 表示本项目诊断；具体消息、行为和位置尚未完整对齐 Roslyn。

`frontend` 在一个进程里批量做语法校验，不写任何程序集，只输出（或按 `-report:` 写入）一份 JSON：每个文件是否通过、类型/成员计数与首个错误的行列。这是"超快增量验证"的入口，也是差分门禁 `-Mode frontend` 使用的命令。

在 PowerShell 7 中运行针对性验证：

```powershell
./tests/runtime.ps1
./tests/preprocessor.ps1
./tests/output-safety.ps1
./tests/enums.ps1
./benchmarks/check-subset.ps1
```

运行时测试使用真实 .NET 运行时加载生成的程序集，并对照 Roslyn 检查结果、异常和部分元数据；同时检查无效输入不会覆盖旧产物。这些用例不构成完整语言兼容性证明。

## Roslyn 替代边界（M0）

HotReload233 的补丁编译链是：Unity/Tuanjie 的 Bee 生成响应文件 → `HotReload233.Server` 执行 `mono csc.exe @rsp`（或 `dotnet csc.dll @rsp`）→ 反射读取补丁 DLL 的 IL。Roslyn 在这里以 **csc 进程契约**出现，不是托管 API，因此替代点在命令行与程序集产物，而不在 `Microsoft.CodeAnalysis`。

| 层次 | 状态 | 说明 |
| --- | --- | --- |
| csc 命令行契约 | 部分实现 | `@rsp`、部分选项、`file(line,col): error CODE: message`、退出码 0/1、`-report`；忽略的选项会记录在报告中，不能视为完整契约 |
| Unity/Tuanjie 脚本程序集全量编译 | 未实现 | 需要完整语法、绑定、CIL/元数据/PDB 与引用程序集；当前仅支持受限子集 |
| 多文件与引用解析 | 部分实现 | 支持多文件及部分外部程序集类型和成员，完整引用解析尚未实现 |
| 源生成器 / 分析器 | 未实现 | `-analyzer` 等开关明确报错，不会忽略分析器并报告编译成功 |
| `Microsoft.CodeAnalysis` API 兼容 | 未实现 | IDE、分析器宿主等托管 API 需要独立的兼容层及集成测试 |

计划中的接入点（尚未改动 HotReload233）：由 HotReload233.Server 直接启动本编译器并传 `@rsp`，替代 `mono csc.exe @rsp`。当前进程契约仍不完整，接入需等待语言覆盖和实际运行链路达标。

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

本机 2026-11-06 的语法层结果（`-Mode frontend`，`tuanjie-project-sf/Assets` 的 8534 个 `.cs`，C# 9，未额外指定条件编译符号）：`accept 8534 / false_reject 0 / false_accept 0`。这只覆盖当次所选条件分支的语法，不代表语义或程序集正确；产物需要另做编译与运行时验证。

## 真实编译（M3/M4 起点）

`csc` 模式不再只做子集校验：它构建 AST、解析名字与类型、发射可执行 CIL，并把多个源文件合并为一个程序集。当前支持的核心：

- 命名空间（块式与文件级）、类（含静态类）、静态/实例字段与初始化器（实例初始化器在构造函数，静态初始化器在 `.cctor`）
- 对象：`new T(...)`（按参数个数匹配构造函数）、`new T { Name = value }` 对象初始化器（字段与属性）、`obj.field` 读写、`obj.Method(...)` 与 `this.Method(...)` 调用
- 属性：`get`/`set` 访问器（自动属性生成后备字段）、`obj.Prop` 读取与 `obj.Prop = v` 赋值、静态属性
- 表达式体：在现有表达式和语句子集内支持方法、构造函数、只读属性及 `get`/`set` 访问器的 `=>` 写法；接口成员的表达式体和更广的表达式仍未覆盖。
- 基础类型实例调用：支持 `int`、`float`、`bool`、`string` 和 `object` 的无参 `ToString` / `GetHashCode`，以及 `int`、`float` 的 `ToString(string)`；其他重载尚未覆盖。
- 数值：支持部分 `int` 到 `float` 的隐式转换与混合运算；测试覆盖舍入、求值顺序和 `NaN` 的大小比较。完整数值类型与转换规则仍未实现。
- 异常处理：支持部分 `try`/`catch`/`finally`、`throw`、重新抛出及 `catch ... when` 筛选器；测试覆盖筛选器在栈展开前执行、筛选器抛异常、变量修改、嵌套异常及跨保护区的返回、`break`、`continue`。完整异常类型解析和语言语义仍未覆盖。
- 数组：一维数组构造、`a[i]` 读写、`a.Length` 和 `foreach` 遍历；`int`、`float`、`bool`、`string`、`object` 的元素指令与部分边界异常有运行时对照测试。
- 枚举：支持八种整数底层类型、具名成员、前向引用及部分常量表达式，生成真实 CLR 枚举元数据；支持本程序集和外部引用中的顶层公共枚举签名、常量、数组和部分方法调用，以及比较、按位运算、与受支持整数的加减和复合赋值、自增自减、`object`/`System.Enum`/`System.ValueType` 装箱拆箱、`ToString`/`GetHashCode`、显式转换、部分零值隐式转换及全限定的 `System.Flags` 特性。`System.Enum` 和 `System.ValueType` 需要引用对应的框架程序集。两个枚举值相减目前仅支持 `int` 底层类型；`uint`/`long`/`ulong` 底层类型的成员初始化常量算术、外部嵌套枚举、类型转发、其他枚举特性及完整运算规则尚未实现。
- 继承：基类（本地或引用程序集）、`virtual`/`override` 虚调用、`: base(args)` 基类构造、基类方法继承与基类-派生类赋值兼容
- 接口：`interface` 声明（抽象方法、无方法体）、类实现接口（`InterfaceImpl` + 隐式实现标记为 `virtual final newslot`）、接口类型变量与 `callvirt` 派发
- 外部基类继承：`class X : System.Exception` 可编译，构造函数 `: base(args)`、继承的字段与属性（`get_X`）沿引用程序集的 `extends` 链解析
- 静态/实例方法、参数，重载按参数个数解析
- 语句：局部声明、赋值、`if/else`、`while`、`do ... while`、`for`、`return`、`break`/`continue`、`base.M(...)` 基类调用、块
- `switch`：支持 `int`、`bool`、`string` 的字面量分支，以及枚举成员、零值和部分显式枚举常量分支；密集的 32 位整数或枚举分支使用 CLR 跳转表。更广的常量表达式、模式匹配和直接写在分支中的局部声明尚未支持。
- 普通字符串字面量支持常见字符转义、`\x`、`\u`、`\U` 和成对的 UTF-16 代理项；目前会拒绝包含未配对代理项的字面量。
- 表达式：`int/float/bool/string/object` 与 `null` 字面量、`enum` 常量、算术与比较（含字符串/布尔相等、`== null`）、`&&/||` 短路、`??`、`?:` 三元、`(T)x` 转换与 `as`、`!`、`++`/`--`、`+=`/`-=`（含字段与属性）、字符串 `+` 拼接、赋值表达式、`string.Empty`/`int.MaxValue` 等预定义成员
- 外部引用：`-reference:` 指定的程序集参与类型与方法解析（全名、唯一短名、`System.Text.Regex` 这类限定名），跨程序集静态/实例调用可用；`tests/compile.ps1` 会对 `System.Text.RegularExpressions` 与 Roslyn 构建的泛型库做真实对照
- 泛型方法调用：`Foo<T>(...)` 通过 MethodSpec 实例化（外部程序集的方法），返回类型按实参替换`MVar`
- 泛型类型实例：`List<int>` 等引用程序集里的泛型类型，字段/局部/参数签名用`GenericInst`，成员通过`TypeSpec`父的 MemberRef 调用（`list.Add(x)`、`list.Count`），`new List<int>()` 可用；泛型**类型定义**仍不支持
- 调用：本程序集的静态/实例方法与构造函数；BCL 的 `System.Math.Max/Min/Abs`、`System.Convert.ToString`、`System.String.Concat`、`System.Console.Write/WriteLine`（按需添加 `System.Console` 程序集引用）

尚未支持、会被明确拒绝而不是静默降级：泛型类型定义（`class Foo<T>`）、多维/交错数组、lambda、`async` 和 `switch` 模式匹配等扩展形式。

当前针对性测试包括 `./tests/enums.ps1`：检查枚举字段与方法元数据、引用程序集、实际运行结果、下游调用和失败时旧产物保护。仓库保留既有的差分测试及历史数据；这些有限用例不能代表完整功能或生产验收。

## 发布

完成 Windows 生产验收后才推送并发布。发布由 GitHub tag 触发，本地也可直接打包：

```powershell
./scripts/package.ps1 -Version 0.1.0
```

产物为 `compiler233-dotnet-<version>-<平台>.zip`（含可执行文件、README、LICENSE、THIRD_PARTY_NOTICES）与同名 `.sha256`。推送 `v*` tag 时 `.github/workflows/release.yml` 在 win-x64、linux-x64、macos-arm64 三个平台分别构建并上传 Release 资产。macOS Intel 不在支持范围。

## 性能与生产验证

性能领先是项目目标。当前以功能正确、真实项目编译、产物可用和生产链路验证为优先，不要求竞品速度对比。[历史实验记录](benchmarks/RESULTS.md)仅覆盖早期子集，不能证明完整编译器的性能或生产可用性。

## 后续里程碑

- **M0（进行中）**：csc 命令行前端、诊断与退出码、`-report` 报告、CLI 契约测试、tag 驱动的发布；当前以 Windows 验收，完整选项语义和产物兼容尚未完成。
- **M1（已完成）**：Roslyn 差分门禁——同输入双编译，输出 accept / reject / false_reject / false_accept 分类与覆盖率，false_accept 默认上限 0；语料可指向任意 `.cs` 目录。
- **M2（进行中）**：自研词法 + 递归下降解析已落地，覆盖预处理/条件编译、命名空间、类型、成员、语句与表达式；当次语料与条件分支的语法层结果见上文。剩余工作：trivia/span 保真、增量解析，以及更多语料和编译条件下的长尾构造。
- **M3（进行中）**：部分名字/类型检查与 CIL 发射已打通，由 `tests/compile.ps1` 与 Roslyn 执行结果对照；仍需完整的泛型定义、lambda、异常、async 及其语义验证。
- **M4**：CIL、元数据、便携 PDB、引用程序集，以及目标框架矩阵（netstandard2.0/2.1、net472、net8/9/10）。
- **M5**：优化性能并完成真实项目与生产链路验收，再进行推送和 GitHub tag 发布；当前先验证 win-x64，后续补全 macos-arm64。
- **M6**：Visual Basic、分析器与源生成器，以及 `Microsoft.CodeAnalysis` 托管 API 兼容。

MIT License；依赖项遵循各自许可证。
