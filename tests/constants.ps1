$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
& cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Compiler build failed' }
& dotnet build (Join-Path $root 'tests/EnumOracle/EnumOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw 'Runtime probe build failed' }
$work = Join-Path (Join-Path $root 'benchmarks/results/constants') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$output = Join-Path $work 'Constants.dll'
$reference = Join-Path (Join-Path $work 'ref') 'Constants.dll'
& $binary csc (Join-Path $root 'tests/fixtures/constants/Program.cs') "-out:$output" "-refout:$reference"
if ($LASTEXITCODE -ne 0) { throw 'Constant fixture failed to compile' }
$oracle = Join-Path $root 'tests/EnumOracle/bin/Release/net10.0/EnumOracle.dll'
& dotnet $oracle --constants $output $reference
if ($LASTEXITCODE -ne 0) { throw 'Constant metadata or runtime validation failed' }
$consumerSource = Join-Path $work 'ConstantConsumer.cs'
$consumerOutput = Join-Path $work 'ConstantConsumer.dll'
[IO.File]::WriteAllText($consumerSource, 'using ConstantFixture; public static class ConstantConsumer { public static int Run() { if (!Values.Enabled || Values.Text != "常量✓" || Values.Missing != null) return -1; switch (42) { case Values.Answer: return Values.Answer; default: return -2; } } }')
& $binary csc $consumerSource "-reference:$reference" "-out:$consumerOutput"
if ($LASTEXITCODE -ne 0) { throw 'Constant reference consumer failed to compile' }
& dotnet $oracle --constant-consumer $consumerOutput
if ($LASTEXITCODE -ne 0) { throw 'Constant reference consumer failed to execute' }
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$referenceBefore = (Get-FileHash -Algorithm SHA256 $reference).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'class C { public const int F; }',
    'class C { public static const int F = 1; }',
    'class C { public readonly const int F = 1; }',
    'class C { public const int F = 2147483647 + 1; }',
    'class C { public const int F = 10 / 0; }',
    'class C { public const int A = 1; public const int B = A; }',
    'class C { public const float F = 1f; }',
    'class C { public const bool F = 1; }',
    'class C { public const int F = 1; void M() { F = 2; } }',
    'class C { public const int F = 1; void M() { C.F++; } }',
    'class C { public const int F = 1; void M() { C.F += 2; } }',
    'class C { public const int F = 1; public const int F = 2; }',
    'class C { public const int F = 1; public int F; }',
    'class C { public const int F = 1; public int F() { return 0; } }',
    'class C { public const int C = 1; }',
    'class C { public const int M = 2147483647; int F() { return M + 1; } }',
    'class C { public const int M = 2147483647; int F() { return C.M + 1; } }',
    'class C { public const int Z = 0; int F() { return 10 / Z; } }',
    'enum E { Max = 2147483647 } class C { const int One = 1; E F() { return E.Max + One; } }',
    'class C { int F() { return 2147483647 + 1; } }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-out:$output" "-refout:$reference" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid constant code was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or
        (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $referenceBefore) {
        throw 'Invalid constant compilation changed a previous output'
    }
}
Write-Output 'Invalid constant definitions, writes and arithmetic rejected; previous outputs preserved.'
exit 0
