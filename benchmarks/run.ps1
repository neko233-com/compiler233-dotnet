param(
    [string]$Source = (Join-Path $PSScriptRoot 'corpus/Example.cs'),
    [ValidateRange(1, 1000000)][int]$Iterations = 200,
    [ValidateRange(0, 1000000)][int]$Warmup = 30,
    [ValidateSet('TreeSitter', 'Subset')][string]$RustMode = 'TreeSitter'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$sourcePath = (Resolve-Path -LiteralPath $Source).Path
Push-Location $root
try {
    $rustBuild = & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "Rust release build failed: $rustBuild" }
    $dotnetBuild = & dotnet build benchmarks/RoslynBench/RoslynBench.csproj -c Release --nologo
    if ($LASTEXITCODE -ne 0) { throw "Roslyn benchmark build failed: $dotnetBuild" }
    $binary = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
    $command = if ($RustMode -eq 'Subset') { 'bench-subset' } else { 'bench' }
    $rust = & (Join-Path $root "target/release/$binary") $command $sourcePath $Iterations $Warmup | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Rust benchmark failed' }
    $roslyn = & dotnet (Join-Path $root 'benchmarks/RoslynBench/bin/Release/net10.0/RoslynBench.dll') $sourcePath $Iterations $Warmup | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Roslyn benchmark failed' }
    if ($rust.source_sha256 -ne $roslyn.source_sha256 -or $rust.bytes -ne $roslyn.bytes) { throw 'Input mismatch' }
    if ($rust.has_parse_issues -or $roslyn.has_parse_issues) { throw 'Input has syntax issues in at least one parser; comparisons require clean input' }
    [pscustomobject]@{
        description = 'Full syntax parse only; grammar and syntax tree representation differ.'
        dotnet_sdk = (& dotnet --version)
        rustc = (& rustc --version)
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
        source = $sourcePath
        rust = $rust
        roslyn = $roslyn
    } | ConvertTo-Json -Depth 5
} finally { Pop-Location }
