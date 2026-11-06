param(
    [ValidateRange(10, 10000)][int]$Iterations = 100,
    [ValidateRange(0, 1000)][int]$Warmup = 5,
    [ValidateRange(0, 10000)][int]$ExtraMethods = 200,
    [string]$ReferenceVersion = ''
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne 'X64') {
    throw 'This benchmark currently requires native Windows x64 PowerShell.'
}
if (-not ('Compiler233Bench.Win32MemoryV1' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
namespace Compiler233Bench {
    public static class Win32MemoryV1 {
        [StructLayout(LayoutKind.Sequential)]
        private struct Counters {
            public uint Size;
            public uint PageFaultCount;
            public UIntPtr PeakWorkingSetSize;
            public UIntPtr WorkingSetSize;
            public UIntPtr QuotaPeakPagedPoolUsage;
            public UIntPtr QuotaPagedPoolUsage;
            public UIntPtr QuotaPeakNonPagedPoolUsage;
            public UIntPtr QuotaNonPagedPoolUsage;
            public UIntPtr PagefileUsage;
            public UIntPtr PeakPagefileUsage;
        }
        [DllImport("psapi.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetProcessMemoryInfo(IntPtr process, ref Counters counters, uint size);
        public static long[] Read(IntPtr process) {
            var counters = new Counters { Size = (uint)Marshal.SizeOf<Counters>() };
            if (!GetProcessMemoryInfo(process, ref counters, counters.Size))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "GetProcessMemoryInfo failed");
            return new[] { checked((long)counters.PeakWorkingSetSize.ToUInt64()),
                           checked((long)counters.PeakPagefileUsage.ToUInt64()) };
        }
    }
}
'@
}
$root = Split-Path $PSScriptRoot -Parent
$scriptPath = $PSCommandPath
$scriptHash = (Get-FileHash -LiteralPath $scriptPath -Algorithm SHA256).Hash.ToLowerInvariant()
$source = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'corpus/Equivalent.cs')).Path
$dotnet = (Get-Command dotnet -CommandType Application | Select-Object -First 1).Source
Push-Location $root
try {
    $sdkVersion = (& $dotnet --version).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $sdkVersion.StartsWith('10.')) {
        throw 'The benchmark requires a .NET 10 SDK, matching CompileOracle.'
    }
    $sdkListing = & $dotnet --list-sdks
    if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate .NET SDKs.' }
    $sdkLine = $sdkListing | Where-Object { $_ -match "^$([regex]::Escape($sdkVersion)) \[(.+)\]$" } | Select-Object -First 1
    if (-not $sdkLine -or $sdkLine -notmatch "\[(.+)\]$") { throw "Could not locate SDK $sdkVersion." }
    $sdkParent = $Matches[1]
    $dotnetRoot = Split-Path $sdkParent -Parent
    $roslyn = Join-Path $sdkParent "$sdkVersion/Roslyn/bincore/csc.dll"
    $hostInfo = & $dotnet --info
    $architecture = ($hostInfo | Where-Object { $_ -match '^\s*Architecture:\s*(\S+)' } | Select-Object -First 1)
    if (-not $architecture -or $architecture -notmatch '^\s*Architecture:\s*x64\s*$') {
        throw 'The selected .NET host must run natively as x64.'
    }
    $rustHost = (& rustc -vV | Where-Object { $_ -match '^host: ' } | Select-Object -First 1)
    if (-not $rustHost -or -not ($rustHost -match '^host: (x86_64-pc-windows-\S+)$')) {
        throw 'The selected Rust toolchain must use a native Windows x64 host.'
    }
    $rustTarget = $Matches[1]
    $packRoot = Join-Path $dotnetRoot 'packs/Microsoft.NETCore.App.Ref'
    if (-not $ReferenceVersion) {
        $versions = @(
            Get-ChildItem -LiteralPath $packRoot -Directory | ForEach-Object {
                $parsed = $null
                if ([version]::TryParse($_.Name, [ref]$parsed) -and $parsed.Major -eq 10) {
                    [pscustomobject]@{ Name = $_.Name; Version = $parsed }
                }
            } | Sort-Object Version -Descending
        )
        if ($versions.Count -eq 0) { throw 'No .NET 10 reference pack was found.' }
        $ReferenceVersion = $versions[0].Name
    }
    $reference = Join-Path $packRoot "$ReferenceVersion/ref/net10.0/System.Runtime.dll"
    if (-not (Test-Path -LiteralPath $roslyn -PathType Leaf) -or -not (Test-Path -LiteralPath $reference -PathType Leaf)) {
        throw 'The selected Roslyn compiler or System.Runtime reference assembly is missing.'
    }
    $metadata = & cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Could not determine the Cargo target directory.' }
    & cargo build --release --locked --target $rustTarget
    if ($LASTEXITCODE -ne 0) { throw 'compiler233 release build failed.' }
    & $dotnet build (Join-Path $root 'tests/CompileOracle/CompileOracle.csproj') -c Release --nologo -v quiet
    if ($LASTEXITCODE -ne 0) { throw 'CompileOracle build failed.' }
    $compiler = Join-Path $metadata.target_directory "$rustTarget/release/compiler233-dotnet.exe"
    if (-not (Test-Path -LiteralPath $compiler -PathType Leaf)) { throw 'The native compiler binary is missing.' }
    $oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'
    $work = Join-Path (Join-Path $root 'benchmarks/results/equivalent-compile') ([guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force (Join-Path $work 'compiler233'), (Join-Path $work 'roslyn') | Out-Null
    $sources = [Collections.Generic.List[string]]::new()
    $sources.Add($source)
    if ($ExtraMethods -gt 0) {
        $generated = Join-Path $work 'Generated.cs'
        $builder = [Text.StringBuilder]::new()
        [void]$builder.AppendLine('namespace CompilerBench { public static class Generated {')
        [void]$builder.AppendLine('public static int Run() { int result = 0;')
        for ($index = 0; $index -lt $ExtraMethods; $index++) {
            $argument = ($index % 3) + 1
            [void]$builder.AppendLine("result += M$index($argument);")
        }
        [void]$builder.AppendLine('return result; }')
        for ($index = 0; $index -lt $ExtraMethods; $index++) {
            [void]$builder.AppendLine("static int M$index(int x) { int sum = x; if (x > 0) sum += 1; for (int i = 0; i < 3; i++) sum += i; return sum; }")
        }
        [void]$builder.AppendLine('} }')
        [IO.File]::WriteAllText($generated, $builder.ToString(), [Text.UTF8Encoding]::new($false))
        $sources.Add($generated)
    }
    $sourceHashes = @($sources | ForEach-Object {
        [pscustomobject]@{ path = $_; sha256 = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $referenceHash = (Get-FileHash -LiteralPath $reference -Algorithm SHA256).Hash.ToLowerInvariant()
    $compilerHash = (Get-FileHash -LiteralPath $compiler -Algorithm SHA256).Hash.ToLowerInvariant()
    $dotnetHash = (Get-FileHash -LiteralPath $dotnet -Algorithm SHA256).Hash.ToLowerInvariant()
    $roslynFiles = @('csc.dll', 'csc.deps.json', 'csc.runtimeconfig.json', 'Microsoft.CodeAnalysis.dll', 'Microsoft.CodeAnalysis.CSharp.dll') | ForEach-Object {
        $path = Join-Path (Split-Path $roslyn -Parent) $_
        [pscustomobject]@{ path = $path; sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
    }
    $runtimeConfig = Get-Content -LiteralPath (Join-Path (Split-Path $roslyn -Parent) 'csc.runtimeconfig.json') -Raw | ConvertFrom-Json
    $installedRuntimes = @(& $dotnet --list-runtimes)
    if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate .NET runtimes.' }
    $performanceEnvironment = [ordered]@{}
    foreach ($name in @('DOTNET_TieredCompilation', 'DOTNET_TieredPGO', 'DOTNET_ReadyToRun', 'DOTNET_gcServer', 'DOTNET_GCHeapHardLimit', 'DOTNET_PROCESSOR_COUNT', 'COMPlus_TieredCompilation', 'COMPlus_ReadyToRun', 'COMPlus_gcServer')) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if ($null -ne $value) { $performanceEnvironment[$name] = $value }
    }
    $manifest = Join-Path $work 'manifest.txt'
    [IO.File]::WriteAllLines($manifest, $sources, [Text.UTF8Encoding]::new($false))
    $roslynVersion = (& $dotnet $roslyn -version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not determine Roslyn compiler version.' }

    function Invoke-Compile([string]$kind, [string]$output) {
        $absoluteOutput = [IO.Path]::GetFullPath($output)
        $workPrefix = [IO.Path]::GetFullPath($work).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
        if (-not $absoluteOutput.StartsWith($workPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Benchmark output must stay inside its unique work directory.'
        }
        if (Test-Path -LiteralPath $absoluteOutput) { Remove-Item -LiteralPath $absoluteOutput -Force }
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = if ($kind -eq 'roslyn') { $dotnet } else { $compiler }
        $info.WorkingDirectory = $root
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $info.ArgumentList.Add($(if ($kind -eq 'roslyn') { $roslyn } else { 'csc' }))
        foreach ($argument in @('-noconfig', '-nologo', '-target:library', '-langversion:9.0', "-reference:$reference", "-out:$output")) {
            $info.ArgumentList.Add($argument)
        }
        foreach ($path in $sources) { $info.ArgumentList.Add($path) }
        $watch = [Diagnostics.Stopwatch]::StartNew()
        $process = [Diagnostics.Process]::Start($info)
        if (-not $process) { throw "Failed to start $kind." }
        try {
            # Keep the process handle open through exit so Win32 can report its
            # lifetime high-water counters without periodic sampling.
            $handle = $process.Handle
            $stdoutTask = $process.StandardOutput.ReadToEndAsync()
            $stderrTask = $process.StandardError.ReadToEndAsync()
            $process.WaitForExit()
            $watch.Stop()
            $stdout = $stdoutTask.GetAwaiter().GetResult()
            $stderr = $stderrTask.GetAwaiter().GetResult()
            if ($process.ExitCode -ne 0) { throw "$kind compilation failed: $stdout $stderr" }
            if (-not (Test-Path -LiteralPath $absoluteOutput -PathType Leaf)) { throw "$kind did not produce its output." }
            $stream = [IO.File]::OpenRead($absoluteOutput)
            try {
                if ($stream.ReadByte() -ne 0x4d -or $stream.ReadByte() -ne 0x5a) {
                    throw "$kind output is not a PE file."
                }
            } finally {
                $stream.Dispose()
            }
            $memory = [Compiler233Bench.Win32MemoryV1]::Read($handle)
            if ($memory[0] -le 0 -or $memory[1] -le 0) { throw "Could not read $kind process peak memory." }
            [pscustomobject]@{
                milliseconds = $watch.Elapsed.TotalMilliseconds
                peak_working_set_bytes = $memory[0]
                peak_commit_bytes = $memory[1]
            }
        } finally {
            $process.Dispose()
        }
    }
    function Get-Percentiles([double[]]$values) {
        $ordered = @($values | Sort-Object)
        $count = $ordered.Count
        [pscustomobject]@{
            p50 = $ordered[[Math]::Ceiling(0.50 * $count) - 1]
            p95 = $ordered[[Math]::Ceiling(0.95 * $count) - 1]
            p99 = $ordered[[Math]::Ceiling(0.99 * $count) - 1]
        }
    }
    $checks = @(
        @{ Type = 'CompilerBench.Runner'; Method = 'Arithmetic' },
        @{ Type = 'CompilerBench.Runner'; Method = 'Loops' },
        @{ Type = 'CompilerBench.Runner'; Method = 'Dispatch' },
        @{ Type = 'CompilerBench.Runner'; Method = 'Text' },
        @{ Type = 'CompilerBench.Runner'; Method = 'Arrays' }
    )
    if ($ExtraMethods -gt 0) { $checks += @{ Type = 'CompilerBench.Generated'; Method = 'Run' } }
    $generatedExpected = 0
    for ($index = 0; $index -lt $ExtraMethods; $index++) { $generatedExpected += ($index % 3) + 5 }
    $validation = [Collections.Generic.List[object]]::new()
    foreach ($kind in @('compiler233', 'roslyn')) {
        $output = Join-Path (Join-Path $work $kind) 'Bench.dll'
        $null = Invoke-Compile $kind $output
        foreach ($check in $checks) {
            $result = & $dotnet $oracle $manifest $output $check.Type $check.Method 2>&1 | Out-String
            if ($LASTEXITCODE -ne 0) { throw "$kind runtime validation failed: $result" }
            if ($check.Type -eq 'CompilerBench.Generated' -and
                $result -notmatch "Roslyn=$generatedExpected, compiler233=$generatedExpected") {
                throw "$kind generated workload did not execute every expected method: $result"
            }
            $validation.Add([pscustomobject]@{ compiler = $kind; type = $check.Type; method = $check.Method; result = $result.Trim() })
        }
    }
    $samples = @{ compiler233 = [Collections.Generic.List[object]]::new(); roslyn = [Collections.Generic.List[object]]::new() }
    $output = Join-Path $work 'Timed.dll'
    for ($run = -$Warmup; $run -lt $Iterations; $run++) {
        $order = if ($run % 2 -eq 0) { @('compiler233', 'roslyn') } else { @('roslyn', 'compiler233') }
        foreach ($kind in $order) {
            $result = Invoke-Compile $kind $output
            if ($run -ge 0) { $samples[$kind].Add($result) }
        }
    }
    foreach ($entry in $sourceHashes) {
        if ((Get-FileHash -LiteralPath $entry.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $entry.sha256) {
            throw 'A benchmark source changed during measurement.'
        }
    }
    if ((Get-FileHash -LiteralPath $reference -Algorithm SHA256).Hash.ToLowerInvariant() -ne $referenceHash) {
        throw 'The reference assembly changed during measurement.'
    }
    if ((Get-FileHash -LiteralPath $compiler -Algorithm SHA256).Hash.ToLowerInvariant() -ne $compilerHash -or
        (Get-FileHash -LiteralPath $dotnet -Algorithm SHA256).Hash.ToLowerInvariant() -ne $dotnetHash) {
        throw 'A compiler executable changed during measurement.'
    }
    foreach ($entry in $roslynFiles) {
        if ((Get-FileHash -LiteralPath $entry.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $entry.sha256) {
            throw 'A Roslyn compiler assembly changed during measurement.'
        }
    }
    if ((Get-FileHash -LiteralPath $scriptPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $scriptHash) {
        throw 'The benchmark script changed during measurement.'
    }
    $cpu = $null
    try {
        $cpu = Get-CimInstance Win32_Processor -ErrorAction Stop |
            Select-Object -First 1 -Property Name, NumberOfCores, NumberOfLogicalProcessors
    } catch {
        # CPU identification is optional; never substitute a guessed value.
    }
    if ($cpu -and $cpu.Name) { $cpu.Name = $cpu.Name.Trim() }
    $gitHead = (& git rev-parse HEAD).Trim()
    $gitDirty = [bool](@(& git status --porcelain --untracked-files=no).Count)
    $report = [pscustomobject]@{
        schema_version = 1
        description = 'Windows x64 process startup, file input, parsing, binding, and DLL emission for one synthetic supported C# subset.'
        limitations = @(
            'Both commands use the same sources, target kind and single reference assembly; compiler capabilities and emitted metadata remain different.',
            'Every timed run creates a fresh PE; only the listed entry points were executed against a Roslyn oracle before timing, so this is not complete language or API equivalence.',
            'Memory values are Windows per-process lifetime peaks for working set and commit charge; parent and child processes are excluded.',
            'Each timed run starts a new process after warmups; filesystem caches and machine load are not isolated.',
            'The workload is synthetic and does not measure real projects, incremental builds, analyzers, generators or full Roslyn replacement.'
        )
        generated_utc = [DateTime]::UtcNow.ToString('o')
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
        logical_processors = [Environment]::ProcessorCount
        cpu = $cpu
        dotnet_sdk = $sdkVersion
        installed_dotnet_runtimes = $installedRuntimes
        roslyn_runtime_framework = $runtimeConfig.runtimeOptions.framework
        performance_environment = $performanceEnvironment
        roslyn_version = $roslynVersion
        rustc = (& rustc --version | Out-String).Trim()
        rust_target = $rustTarget
        benchmark_script_sha256 = $scriptHash
        git_head = $gitHead
        git_tracked_files_dirty = $gitDirty
        compiler233 = [pscustomobject]@{
            executable = $compiler
            sha256 = $compilerHash
            arguments = @('csc', '-noconfig', '-nologo', '-target:library', '-langversion:9.0', "-reference:$reference", "-out:$output") + @($sources)
            samples = @($samples.compiler233)
            wall_percentiles_ms = Get-Percentiles @($samples.compiler233.milliseconds)
            peak_working_set_percentiles_bytes = Get-Percentiles @($samples.compiler233.peak_working_set_bytes)
            peak_commit_percentiles_bytes = Get-Percentiles @($samples.compiler233.peak_commit_bytes)
        }
        roslyn = [pscustomobject]@{
            executable = $dotnet
            executable_sha256 = $dotnetHash
            assemblies = $roslynFiles
            arguments = @($roslyn, '-noconfig', '-nologo', '-target:library', '-langversion:9.0', "-reference:$reference", "-out:$output") + @($sources)
            samples = @($samples.roslyn)
            wall_percentiles_ms = Get-Percentiles @($samples.roslyn.milliseconds)
            peak_working_set_percentiles_bytes = Get-Percentiles @($samples.roslyn.peak_working_set_bytes)
            peak_commit_percentiles_bytes = Get-Percentiles @($samples.roslyn.peak_commit_bytes)
        }
        reference = [pscustomobject]@{ path = $reference; version = $ReferenceVersion; sha256 = $referenceHash }
        sources = $sourceHashes
        generated_methods = $ExtraMethods
        generated_methods_executed = $ExtraMethods
        generated_expected_result = $generatedExpected
        warmup = $Warmup
        iterations = $Iterations
        memory_source = 'GetProcessMemoryInfo: PeakWorkingSetSize and PeakPagefileUsage after process exit.'
        order = 'Alternating compiler233-first and roslyn-first.'
        validation = @($validation)
    }
    $reportPath = Join-Path $work 'report.json'
    [IO.File]::WriteAllText($reportPath, ($report | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
    Write-Output $reportPath
    $report | Select-Object @{ Name = 'compiler233_ms'; Expression = { $_.compiler233.wall_percentiles_ms } }, @{ Name = 'roslyn_ms'; Expression = { $_.roslyn.wall_percentiles_ms } } | ConvertTo-Json -Depth 4
} finally {
    Pop-Location
}
