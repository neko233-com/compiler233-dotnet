$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
& cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Compiler build failed' }
& dotnet build (Join-Path $root 'tests/EnumOracle/EnumOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw 'Runtime probe build failed' }
$work = Join-Path (Join-Path $root 'benchmarks/results/readonly') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$output = Join-Path $work 'Readonly.dll'
$reference = Join-Path (Join-Path $work 'ref') 'Readonly.dll'
& $binary csc (Join-Path $root 'tests/fixtures/readonly/Program.cs') "-out:$output" "-refout:$reference"
if ($LASTEXITCODE -ne 0) { throw 'Readonly fixture failed to compile' }
$oracle = Join-Path $root 'tests/EnumOracle/bin/Release/net10.0/EnumOracle.dll'
& dotnet $oracle --readonly $output $reference
if ($LASTEXITCODE -ne 0) { throw 'Readonly metadata or runtime validation failed' }
$consumerSource = Join-Path $work 'ReadonlyConsumer.cs'
$consumerOutput = Join-Path $work 'ReadonlyConsumer.dll'
[IO.File]::WriteAllText($consumerSource, 'using ReadonlyFixture; public static class ReadonlyConsumer { public static int Run() { return Holder.Global + Holder.Inline; } }')
& $binary csc $consumerSource "-reference:$reference" "-out:$consumerOutput"
if ($LASTEXITCODE -ne 0) { throw 'Readonly reference consumer failed to compile' }
& dotnet $oracle --invoke $consumerOutput 'ReadonlyConsumer' 'Run' '14'
if ($LASTEXITCODE -ne 0) { throw 'Readonly reference consumer failed to execute' }
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$referenceBefore = (Get-FileHash -Algorithm SHA256 $reference).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'class C { public readonly int F; void M() { F = 1; } }',
    'class C { public readonly int F; void M() { this.F = 1; } }',
    'class C { public readonly int F; void M() { F++; } }',
    'class C { public readonly int F; void M() { this.F += 1; } }',
    'class C { public readonly int F; C(C other) { other.F = 1; } }',
    'class C { public readonly int F; C(C other) { other.F++; } }',
    'class C { public readonly int F; } class D { void M(C c) { c.F = 1; } }',
    'class C { public readonly int F; } class D { C M() { return new C { F = 1 }; } }',
    'class C { public static readonly int F; C() { F = 1; } }',
    'class C { public static readonly int F; static void M() { F = 1; } }',
    'class C { public static readonly int F; static void M() { C.F++; } }',
    'class C { public static readonly int F; static void M() { C.F += 1; } }',
    'class C { public readonly int P { get; set; } }',
    'class C { public readonly void M() { } }',
    'class C { public readonly void F; }',
    'class C { public readonly void[] F; }',
    'class C { public virtual readonly int F; }',
    'class C { public async readonly int F; }',
    'interface I { readonly int F; }',
    'class C { readonly readonly int F; }',
    'readonly struct S { public readonly int F; }',
    'record R { }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-out:$output" "-refout:$reference" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid readonly code was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or
        (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $referenceBefore) {
        throw 'Invalid readonly compilation changed a previous output'
    }
}
[IO.File]::WriteAllText($bad, 'using ReadonlyFixture; class C { void M() { Holder.Global = 1; } }')
$null = & $binary csc $bad "-reference:$reference" "-out:$output" 2>&1
if ($LASTEXITCODE -ne 1 -or (Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) {
    throw 'External readonly write was accepted or changed a previous output'
}
$runtime = & dotnet --list-runtimes | Where-Object { $_ -match '^Microsoft\.NETCore\.App ' } | Select-Object -Last 1
if ($runtime -notmatch '^Microsoft\.NETCore\.App (\S+) \[(.+)\]$') { throw 'No .NET runtime found' }
$core = Join-Path (Join-Path $Matches[2] $Matches[1]) 'System.Private.CoreLib.dll'
foreach ($invalid in @(
    'class C { public readonly System.DateTime F; }',
    'class C { public readonly System.Nullable<int> F; }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-reference:$core" "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1 -or (Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) {
        throw "Unsupported external value type was accepted or changed an output: $invalid"
    }
}
Write-Output 'Invalid readonly writes and unsupported value types rejected; previous outputs preserved.'
exit 0
