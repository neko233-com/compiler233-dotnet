$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$resultDir = Join-Path $root 'benchmarks/results/runtime'
New-Item -ItemType Directory -Force $resultDir | Out-Null
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$fixture = Join-Path $root 'tests/fixtures/Arithmetic.cs'
$output = Join-Path $resultDir 'Arithmetic.dll'
$buildOutput = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Rust build failed: $buildOutput" }
$buildOutput = & dotnet build (Join-Path $root 'tests/RuntimeOracle/RuntimeOracle.csproj') -c Release --nologo
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $buildOutput" }
& $binary compile $fixture $output
if ($LASTEXITCODE -ne 0) { throw 'Compiler failed on valid fixture' }
& dotnet (Join-Path $root 'tests/RuntimeOracle/bin/Release/net10.0/RuntimeOracle.dll') $fixture $output
if ($LASTEXITCODE -ne 0) { throw 'Runtime differential test failed' }
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$invalid = Join-Path $resultDir 'invalid.cs'
$oracle = Join-Path $root 'tests/RuntimeOracle/bin/Release/net10.0/RuntimeOracle.dll'
$badCases = @(
    'class A { static int F() { return missing; } }',
    'class A { static int F(int a, int a) { return a; } }',
    'class A { static int F() { return 2147483647 + 1; } }',
    'class A { static int F() { return 1 / 0; } }',
    'class A { static int F() { return 1; } static int F() { return 2; } }',
    'class A {} class A {}'
)
foreach ($case in $badCases) {
    [System.IO.File]::WriteAllText($invalid, $case)
    $null = & dotnet $oracle $invalid --diagnose
    if ($LASTEXITCODE -ne 1) { throw "Roslyn unexpectedly accepted: $case" }
    $null = & $binary compile $invalid $output 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Compiler unexpectedly accepted: $case" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Failed compilation modified the existing output' }
}
$generatedSource = Join-Path $resultDir 'generated.cs'
$generatedOutput = Join-Path $resultDir 'Generated.dll'
& (Join-Path $root 'benchmarks/generate-subset.ps1') -Output $generatedSource -Methods 200
& $binary compile $generatedSource $generatedOutput
if ($LASTEXITCODE -ne 0) { throw 'Compilation failed on generated benchmark corpus' }
& dotnet $oracle $generatedSource $generatedOutput
if ($LASTEXITCODE -ne 0) { throw 'Generated benchmark corpus runtime comparison failed' }
Write-Output 'Runtime differential and failed-output preservation checks passed.'
