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
[IO.File]::WriteAllText($consumerSource, 'using StaticFixture; using Values = StaticFixture.FieldAccess; public static class StaticConsumer { public static int Run() { Values.Number = 10; Values.Number += 3; int previous = Values.Number++; if (previous != 13 || StaticFixture.FieldAccess.Number != 14) return -1; Values.Code = FieldCode.A; Values.Code |= FieldCode.B; if (Values.Code != FieldCode.Both) return -2; Values.Text = "ok"; Values.Enabled = true; Values.Rate = 2f; if (Values.Text != "ok" || !Values.Enabled || Values.Rate != 2f || Values.Value != null) return -3; return Operations.Run() + Plain.Add(3) + Values.Number; } }')
& $binary csc $consumerSource "-reference:$reference" "-out:$consumerOutput"
if ($LASTEXITCODE -ne 0) { throw 'Static reference consumer failed to compile' }
& dotnet $oracle --invoke $consumerOutput 'StaticConsumer' 'Run' '1255'
if ($LASTEXITCODE -ne 0) { throw 'Static reference consumer failed to execute' }
& dotnet build (Join-Path $root 'tests/StaticForeign/StaticForeign.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw 'Foreign static field fixture failed to build' }
$foreign = Join-Path $work 'StaticForeign.dll'
Copy-Item -LiteralPath (Join-Path $root 'tests/StaticForeign/bin/Release/net10.0/StaticForeign.dll') -Destination $foreign
$foreignSource = Join-Path $work 'ForeignConsumer.cs'
$foreignOutput = Join-Path $work 'ForeignConsumer.dll'
[IO.File]::WriteAllText($foreignSource, 'using ForeignFieldFixture; public static class ForeignConsumer { public static int Run() { Values.Counter = 3; Values.Counter += 4; int old = Values.Counter++; Values.Mode = Status.One; Values.Mode |= Status.Two; Values.Rate = 2f; Values.Text = "ok"; Values.Ready = true; Values.Numbers = new int[1]; Values.Numbers[0] = 9; Values.Codes = new Status[1]; Values.Codes[0] = Status.One; if (old != 7 || Values.Counter != 8 || Values.Mode != Status.Both) return -1; if (Values.Rate != 2f || Values.Text != "ok" || !Values.Ready || Values.Value != null || Values.Numbers[0] != 9 || Values.Codes[0] != Status.One) return -2; return Values.Counter + Values.ReadOnly; } }')
& $binary csc $foreignSource "-reference:$foreign" "-out:$foreignOutput"
if ($LASTEXITCODE -ne 0) { throw 'Foreign static field consumer failed to compile' }
& dotnet $oracle --invoke $foreignOutput 'ForeignConsumer' 'Run' '14'
if ($LASTEXITCODE -ne 0) { throw 'Foreign static field consumer failed to execute' }
$runtime = & dotnet --list-runtimes | Where-Object { $_ -match '^Microsoft\.NETCore\.App ' } | Select-Object -Last 1
if ($runtime -notmatch '^Microsoft\.NETCore\.App (\S+) \[(.+)\]$') { throw 'No .NET runtime found' }
$core = Join-Path (Join-Path $Matches[2] $Matches[1]) 'System.Private.CoreLib.dll'
$frameworkSource = Join-Path $work 'FrameworkField.cs'
$frameworkOutput = Join-Path $work 'FrameworkField.dll'
[IO.File]::WriteAllText($frameworkSource, 'public static class FrameworkField { public static int Run() { if (System.String.Empty == "") return 1; return 0; } }')
& $binary csc $frameworkSource "-reference:$core" "-out:$frameworkOutput"
if ($LASTEXITCODE -ne 0) { throw 'Framework static field consumer failed to compile' }
& dotnet $oracle --invoke $frameworkOutput 'FrameworkField' 'Run' '1'
if ($LASTEXITCODE -ne 0) { throw 'Framework static field consumer failed to execute' }
& dotnet $oracle --static-framework $frameworkOutput
if ($LASTEXITCODE -ne 0) { throw 'Framework static field assembly identity was incorrect' }
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
    'class S { public int Field; } class C { int M() { return S.Field; } }',
    'class S { private static int Field; } class C { int M() { return S.Field; } }',
    'class S { public const int Field = 1; }',
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
foreach ($invalid in @(
    'using ForeignFieldFixture; class C { void M() { Values.ReadOnly = 1; } }',
    'using ForeignFieldFixture; class C { void M() { Values.ReadOnly++; } }',
    'using ForeignFieldFixture; class C { int M() { return Values.Literal; } }',
    'using ForeignFieldFixture; class C { int M() { return Values.Hidden; } }',
    'using ForeignFieldFixture; class C { int M() { return Instance.Number; } }',
    'class C { int M() { return Values.Counter; } }',
    'using ForeignFieldFixture; using OtherForeign; class C { int M() { return Values.Counter; } }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-reference:$foreign" "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid foreign field access was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Invalid field access changed a previous output.' }
}
Write-Output 'Invalid static declarations and field accesses rejected; previous outputs preserved.'
exit 0
