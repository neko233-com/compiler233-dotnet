$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
& cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Compiler build failed' }
& dotnet build (Join-Path $root 'tests/EnumOracle/EnumOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw 'Runtime probe build failed' }
$work = Join-Path (Join-Path $root 'benchmarks/results/statics') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/statics/Program.cs'
$output = Join-Path $work 'Statics.dll'
$reference = Join-Path (Join-Path $work 'ref') 'Statics.dll'
& $binary csc $source "-out:$output" "-refout:$reference"
if ($LASTEXITCODE -ne 0) { throw 'Static fixture failed to compile' }
$oracle = Join-Path $root 'tests/EnumOracle/bin/Release/net10.0/EnumOracle.dll'
& dotnet $oracle --static $output $reference
if ($LASTEXITCODE -ne 0) { throw 'Static metadata or execution validation failed' }
$consumerSource = Join-Path $work 'StaticConsumer.cs'
$consumerOutput = Join-Path $work 'StaticConsumer.dll'
[IO.File]::WriteAllText($consumerSource, 'using StaticFixture; public static class StaticConsumer { public static int Run() { return Operations.Run() + Plain.Add(3); } }')
& $binary csc $consumerSource "-reference:$reference" "-out:$consumerOutput"
if ($LASTEXITCODE -ne 0) { throw 'Static reference consumer failed to compile' }
& dotnet $oracle --invoke $consumerOutput 'StaticConsumer' 'Run' '1241'
if ($LASTEXITCODE -ne 0) { throw 'Static reference consumer failed to execute' }
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$referenceBefore = (Get-FileHash -Algorithm SHA256 $reference).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'static class S { int Field; }',
    'static class S { int Property { get; set; } }',
    'static class S { void Method() { } }',
    'static class S { public S() { } }',
    'static class S { public static S() { } }',
    'static class S { static S(int x) { } }',
    'static class S { static S() : base() { } }',
    'static class S { static S() { } static S() { } }',
    'static class S { async static S() { } }',
    'static class S { extern static S() { } }',
    'static class S { static virtual void M() { } }',
    'static abstract class S { }',
    'static sealed class S { }',
    'static interface S { }',
    'interface I { I() { } }',
    'static static class S { }',
    'class B { } static class S : B { }',
    'static class S { } class C : S { }',
    'static class S { } class C { S Field; }',
    'static class S { } class C { S[] Field; }',
    'static class S { } class C { S M() { return null; } }',
    'static class S { } class C { void M(S value) { } }',
    'static class S { } class C { void M() { S value = null; } }',
    'static class S { } class C { void M() { new S(); } }',
    'class C { static Different() { } }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-out:$output" "-refout:$reference" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid static code was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or
        (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $referenceBefore) {
        throw 'Invalid static compilation changed a previous output'
    }
}
$externalBad = 'class C { StaticFixture.Operations Field; }'
[IO.File]::WriteAllText($bad, $externalBad)
$diagnostics = & $binary csc $bad "-reference:$reference" "-out:$output" 2>&1
if ($LASTEXITCODE -ne 1 -or (($diagnostics | Out-String) -notmatch 'static type .* cannot be used as a value type')) {
    throw "External static field type was not rejected for the expected reason: $diagnostics"
}
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or
    (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $referenceBefore) {
    throw 'Invalid external static compilation changed a previous output'
}
Write-Output 'Invalid static class and constructor declarations rejected; previous outputs preserved.'
exit 0
