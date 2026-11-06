$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
& cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Compiler build failed' }
foreach ($project in @('EnumOracle')) {
    & dotnet build (Join-Path $root "tests/$project/$project.csproj") -c Release --nologo -v quiet
    if ($LASTEXITCODE -ne 0) { throw "$project build failed" }
}
$work = Join-Path (Join-Path $root 'benchmarks/results/enums') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/enums/Api.cs'
$output = Join-Path $work 'EnumApi.dll'
$reference = Join-Path (Join-Path $work 'ref') 'EnumApi.dll'
& $binary csc $source "-out:$output" "-refout:$reference"
if ($LASTEXITCODE -ne 0) { throw 'Enum fixture failed to compile' }
$enumOracle = Join-Path $root 'tests/EnumOracle/bin/Release/net10.0/EnumOracle.dll'
& dotnet $enumOracle $output $reference
if ($LASTEXITCODE -ne 0) { throw 'Enum metadata or downstream runtime validation failed' }
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$referenceBefore = (Get-FileHash -Algorithm SHA256 $reference).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'enum E : bool { A }',
    'enum E : char { A }',
    'enum E : byte { A = 256 }',
    'enum E : sbyte { A = -129 }',
    'enum E : uint { A = -1 }',
    'enum E { A, A }',
    'static enum E { A }',
    'abstract enum E { A }',
    'sealed enum E { A }',
    'namespace Collision { enum E { A } class E { } }',
    'namespace One { enum E { A } } namespace Two { class C { E M(E value) { return value; } } }',
    'namespace One { enum E { A } } namespace Two { enum E { A } } namespace Three { using One; using Two; class C { E M(E value) { return value; } } }',
    'enum E { A = 2147483647, B }',
    'enum E { A = B, B }',
    'enum E { A = B, B = A }',
    'enum E { A = 2147483647 + 1 }',
    'enum E { A = 1 / 0 }',
    'enum E { A = Missing }',
    'enum E { A = 0, B = 0 } class C { int M(E value) { switch(value) { case E.A: return 1; case E.B: return 2; default: return 3; } } }',
    'enum E { A = 0 } class C { int M(E value) { switch(value) { case E.A: return 1; case 0: return 2; default: return 3; } } }',
    'enum E { A } enum F { A } class C { int M(E value) { switch(value) { case F.A: return 1; default: return 2; } } }',
    'enum E { A } class C { int M(E value) { switch(value) { case 1: return 1; default: return 2; } } }',
    'enum E : byte { A } class C { int M(E value) { switch(value) { case (E)256: return 1; default: return 2; } } }',
    'enum E { A = 1, B = 2, Both = 3 } class C { int M(E value) { switch(value) { case E.A | E.B: return 1; case E.Both: return 2; default: return 3; } } }',
    'enum E : uint { Max = 4294967295U, Zero = 0 } class C { int M() { return (int)(E.Max | E.Zero); } }',
    'enum E : uint { Zero = 0 } class C { int M() { return (int)~E.Zero; } }',
    'enum E { A } enum F { A } class C { E M(E x, F y) { return x | y; } }',
    'enum E { A } class C { E M(E x, int y) { return x | y; } }',
    'class C { float M() { return 1f & 2f; } }',
    'class C { int M() { return 0U | 0; } }',
    'class C { int M() { return ~0U; } }',
    'using System; class FlagsAttribute { } [Flags] enum E { A }',
    'namespace Other { public class FlagsAttribute { } } namespace N { using System; using Other; [Flags] enum E { A } }',
    'using System; using Flags = System.ObsoleteAttribute; [Flags] enum E { A }',
    'using System = Other; using Flags = System.FlagsAttribute; [Flags] enum E { A }',
    'class C { int M() { return default(void); } }',
    'class C { int M() { return default(Missing); } }',
    'class C { int M() { return default; } }',
    'class C { bool M() { return default(int); } }',
    'class C { void M() { default(int); } }',
    'class C { void M() { default(int) + 1; } }',
    'enum E { A } enum F { A } class C { E M() { return default(F); } }',
    'enum E { A } enum F { A } class C { int M(E value) { switch (value) { case default(F): return 1; default: return 0; } } }',
    'class C { int M(int value) { switch (value) { case default(int): return 1; case 0: return 2; default: return 0; } } }',
    'enum E : byte { Max = 255 } class C { E M() { return E.Max + 1; } }',
    'enum E { Max = 2147483647 } class C { E M() { return E.Max + 1; } }',
    'enum E : long { Max = 9223372036854775807L } class C { E M() { return E.Max + 1; } }',
    'enum E { Min = -2147483648 } class C { int M() { return 0 - E.Min; } }',
    'enum E { Min = -2147483648, Max = 2147483647 } class C { int M() { return E.Max - E.Min; } }',
    'enum E : byte { A } class C { E M(E value, int delta) { return value + delta; } }',
    'enum E { A } class C { E M(E value) { return value + (2147483647 + 1); } }',
    'enum E { A = 1, B = 2 } class C { int M(E value) { switch(value) { case E.A + 1: return 1; case E.B: return 2; default: return 3; } } }',
    'enum E { A } class C { E M() { return E.A++; } }',
    'class C { string M(string value) { return value++; } }',
    'enum E { A } class C { string M() { return E.A.ToString(null); } }',
    'enum E { A } class C { E M(string value) { return (E)value; } }',
    'class C { public int P { get; } int M() { return this.P++; } }',
    'class C { int M() { string[] values = new string[1]; values[0]++; return 0; } }',
    'enum E { A } enum E { B }',
    'enum E { A } class C { int F(E value) { return value; } }',
    'enum E { A } enum F { A } class C { E M() { return F.A; } }',
    'enum Hidden { A } public class C { public Hidden M() { return Hidden.A; } }',
    'enum Hidden { A } public class C { protected Hidden M() { return Hidden.A; } }',
    'enum Hidden { A } public class C { public Hidden[] Field; }',
    'enum Hidden { A } public class C { public void M(Hidden value) { } }',
    'enum Hidden { A } public class C { public Hidden Property { get; set; } }',
    'enum E { A } class C { E M(int value) { return value; } }',
    'enum E { A } class C { E M() { return 1; } }',
    'enum E : uint { Max = 4294967295U } class C { int M() { return (int)E.Max; } }',
    'enum E : byte { Max = 255 } enum F : sbyte { Zero = 0 } class C { F M() { return (F)E.Max; } }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-out:$output" "-refout:$reference" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid enum code was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or
        (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $referenceBefore) {
        throw 'Invalid enum compilation changed a previous output'
    }
}
Write-Output 'Invalid enum code rejected; previous outputs preserved.'
foreach ($unsupported in @(
    '[System.Obsolete] enum E { A }',
    'enum E { [System.Obsolete] A }',
    '[Flags] enum E { A }'
)) {
    [IO.File]::WriteAllText($bad, $unsupported)
    $null = & $binary csc $bad "-out:$output" "-refout:$reference" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Unsupported enum attribute was silently accepted: $unsupported" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or
        (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $referenceBefore) {
        throw 'Unsupported enum attribute changed a previous output'
    }
}
Write-Output 'Unsupported enum attributes rejected; previous outputs preserved.'
$consumerSource = Join-Path $root 'tests/fixtures/enums/Consumer.cs'
$consumerOutput = Join-Path $work 'ExternalConsumer.dll'
$consumerReference = Join-Path (Join-Path $work 'consumer-ref') 'ExternalConsumer.dll'
& $binary csc $consumerSource "-reference:$reference" "-out:$consumerOutput" "-refout:$consumerReference"
if ($LASTEXITCODE -ne 0) { throw 'External enum consumer failed to compile.' }
& dotnet $enumOracle --external $consumerOutput $consumerReference $reference
if ($LASTEXITCODE -ne 0) { throw 'External enum consumer failed to execute.' }
foreach ($inaccessible in @(
    'class C { EnumFixture.InternalCode Field; }',
    'class C { int M() { return (int)EnumFixture.InternalCode.One; } }'
)) {
    [IO.File]::WriteAllText($bad, $inaccessible)
    $null = & $binary csc $bad "-reference:$reference" "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Inaccessible external enum was accepted: $inaccessible" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Inaccessible enum changed a previous output.' }
}
$otherSource = Join-Path $work 'OtherEnum.cs'
$otherOutput = Join-Path $work 'OtherEnum.dll'
[IO.File]::WriteAllText($otherSource, 'namespace EnumFixture { public enum Direction { Value = 7 } }')
& $binary csc $otherSource "-out:$otherOutput"
if ($LASTEXITCODE -ne 0) { throw 'Second enum assembly failed to compile.' }
[IO.File]::WriteAllText($bad, 'class C { EnumFixture.Direction M(EnumFixture.Direction value) { return value; } }')
$null = & $binary csc $bad "-reference:$reference" "-reference:$otherOutput" "-out:$output" 2>&1
if ($LASTEXITCODE -ne 1) { throw 'Ambiguous referenced enum was accepted.' }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Ambiguous enum changed a previous output.' }
$localSource = Join-Path $work 'LocalEnum.cs'
$localOutput = Join-Path $work 'LocalEnum.dll'
[IO.File]::WriteAllText($localSource, 'namespace EnumFixture { public enum Direction { Value = 7 } public class Local { public static int Run() { return (int)Direction.Value; } } }')
& $binary csc $localSource "-reference:$reference" "-out:$localOutput"
if ($LASTEXITCODE -ne 0) { throw 'Locally declared enum did not take precedence.' }
& dotnet $enumOracle --invoke $localOutput 'EnumFixture.Local' 'Run' '7'
if ($LASTEXITCODE -ne 0) { throw 'Locally declared enum failed at runtime.' }
[IO.File]::WriteAllText($bad, 'namespace EnumFixture { public enum Direction { Value = 7 } public static class Local { public static int Run() { return (int)Api.Echo(Direction.Value); } } }')
$null = & $binary csc $bad "-reference:$reference" "-out:$output" 2>&1
if ($LASTEXITCODE -ne 1) { throw 'Distinct local and external enum types were treated as identical.' }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Enum identity error changed a previous output.' }
$firstSource = Join-Path $work 'FirstIdentity.cs'
$firstOutput = Join-Path $work 'FirstIdentity.dll'
$secondSource = Join-Path $work 'SecondIdentity.cs'
$secondOutput = Join-Path $work 'SecondIdentity.dll'
[IO.File]::WriteAllText($firstSource, 'namespace Shared { public enum E { One = 1 } }')
[IO.File]::WriteAllText($secondSource, 'namespace Shared { public enum E { One = 7 } }')
& $binary csc $firstSource "-out:$firstOutput"
if ($LASTEXITCODE -ne 0) { throw 'First identity assembly failed.' }
& $binary csc $secondSource "-out:$secondOutput"
if ($LASTEXITCODE -ne 0) { throw 'Second identity assembly failed.' }
$bridgeSource = Join-Path $work 'IdentityBridge.cs'
$bridgeOutput = Join-Path $work 'IdentityBridge.dll'
[IO.File]::WriteAllText($bridgeSource, 'public static class IdentityBridge { public static Shared.E Echo(Shared.E value) { return value; } }')
& $binary csc $bridgeSource "-reference:$firstOutput" "-out:$bridgeOutput"
if ($LASTEXITCODE -ne 0) { throw 'Identity bridge failed to compile.' }
$identitySource = Join-Path $work 'IdentityConsumer.cs'
$identityOutput = Join-Path $work 'IdentityConsumer.dll'
[IO.File]::WriteAllText($identitySource, 'public static class IdentityConsumer { public static int Run() { return (int)IdentityBridge.Echo(Shared.E.One); } }')
& $binary csc $identitySource "-reference:$bridgeOutput" "-reference:$firstOutput" "-out:$identityOutput"
if ($LASTEXITCODE -ne 0) { throw 'Matching enum assembly identity was rejected.' }
& dotnet $enumOracle --invoke $identityOutput 'IdentityConsumer' 'Run' '1'
if ($LASTEXITCODE -ne 0) { throw 'Matching enum assembly identity failed at runtime.' }
$null = & $binary csc $identitySource "-reference:$bridgeOutput" "-reference:$secondOutput" "-out:$output" 2>&1
if ($LASTEXITCODE -ne 1) { throw 'Distinct external enum assembly identities were treated as identical.' }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'External enum identity error changed a previous output.' }
Write-Output 'External enum accessibility, ambiguity and local declaration checks passed.'
$runtime = & dotnet --list-runtimes | Where-Object { $_ -match '^Microsoft\.NETCore\.App ' } | Select-Object -Last 1
if ($runtime -notmatch '^Microsoft\.NETCore\.App (\S+) \[(.+)\]$') { throw 'Could not locate the .NET runtime.' }
$core = Join-Path (Join-Path $Matches[2] $Matches[1]) 'System.Private.CoreLib.dll'
$systemSource = Join-Path $work 'SystemEnumConsumer.cs'
$systemOutput = Join-Path $work 'SystemEnumConsumer.dll'
[IO.File]::WriteAllText($systemSource, 'public static class SystemEnumConsumer { public static System.DayOfWeek Echo(System.DayOfWeek value) { return value; } public static int Run() { System.DayOfWeek[] values = new System.DayOfWeek[1]; values[0] = System.DayOfWeek.Friday; return (int)Echo(values[0]); } }')
& $binary csc $systemSource "-reference:$core" "-out:$systemOutput"
if ($LASTEXITCODE -ne 0) { throw 'Framework enum consumer failed to compile.' }
& dotnet $enumOracle --invoke $systemOutput 'SystemEnumConsumer' 'Run' '5'
if ($LASTEXITCODE -ne 0) { throw 'Framework enum consumer failed at runtime.' }
$frameworkBridge = Join-Path $work 'FrameworkBridge.dll'
& dotnet $enumOracle --make-framework-bridge $frameworkBridge
if ($LASTEXITCODE -ne 0) { throw 'Framework bridge could not be built.' }
$bridgeConsumerSource = Join-Path $work 'FrameworkBridgeConsumer.cs'
$bridgeConsumerOutput = Join-Path $work 'FrameworkBridgeConsumer.dll'
[IO.File]::WriteAllText($bridgeConsumerSource, 'public static class FrameworkBridgeConsumer { public static int Run() { return (int)FrameworkBridge.Echo(System.DayOfWeek.Friday); } }')
& $binary csc $bridgeConsumerSource "-reference:$frameworkBridge" "-reference:$core" "-out:$bridgeConsumerOutput"
if ($LASTEXITCODE -ne 0) { throw 'Framework enum method signature failed to compile.' }
& dotnet $enumOracle --invoke $bridgeConsumerOutput 'FrameworkBridgeConsumer' 'Run' '5'
if ($LASTEXITCODE -ne 0) { throw 'Framework enum method signature failed at runtime.' }
$baseSource = Join-Path $work 'EnumBaseConsumer.cs'
$baseOutput = Join-Path $work 'EnumBaseConsumer.dll'
[IO.File]::WriteAllText($baseSource, 'public static class EnumBaseConsumer { public static System.Enum BoxEnum(System.DayOfWeek value) { return value; } public static System.ValueType BoxValue(System.DayOfWeek value) { return value; } public static System.DayOfWeek FromEnum(System.Enum value) { return (System.DayOfWeek)value; } public static System.DayOfWeek FromValue(System.ValueType value) { return (System.DayOfWeek)value; } public static int Run() { System.Enum e = BoxEnum(System.DayOfWeek.Friday); System.ValueType v = BoxValue(System.DayOfWeek.Friday); if (e.ToString() != "Friday") return -1; if (FromEnum(e) != System.DayOfWeek.Friday || FromValue(v) != System.DayOfWeek.Friday) return -2; System.Enum mask = System.DayOfWeek.Monday; if (!System.DayOfWeek.Friday.HasFlag(mask)) return -3; System.Enum explicitValue = (System.Enum)System.DayOfWeek.Friday; return (int)FromEnum(explicitValue); } }')
& $binary csc $baseSource "-reference:$core" "-out:$baseOutput"
if ($LASTEXITCODE -ne 0) { throw 'Enum base boxing consumer failed to compile.' }
& dotnet $enumOracle --invoke $baseOutput 'EnumBaseConsumer' 'Run' '5'
if ($LASTEXITCODE -ne 0) { throw 'Enum base boxing consumer failed at runtime.' }
$fakeSource = Join-Path $work 'FakeCorlib.cs'
$fakeOutput = Join-Path $work 'FakeCorlib.dll'
[IO.File]::WriteAllText($fakeSource, 'namespace System { public class Enum { } public class FlagsAttribute { } }')
& $binary csc $fakeSource "-out:$fakeOutput"
if ($LASTEXITCODE -ne 0) { throw 'Fake base type fixture failed to compile.' }
[IO.File]::WriteAllText($bad, 'enum E { A } class C { public System.Enum Box(E value) { return value; } }')
$null = & $binary csc $bad "-reference:$fakeOutput" "-out:$output" 2>&1
if ($LASTEXITCODE -ne 1) { throw 'An unrelated System.Enum type accepted enum boxing.' }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Invalid base conversion changed a previous output.' }
[IO.File]::WriteAllText($bad, '[System.Flags] enum E { A }')
$null = & $binary csc $bad "-reference:$fakeOutput" "-out:$output" 2>&1
if ($LASTEXITCODE -ne 1) { throw 'An unrelated System.FlagsAttribute was silently replaced by the framework attribute.' }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Invalid attribute resolution changed a previous output.' }
[IO.File]::WriteAllText($bad, 'using Flags = System.FlagsAttribute; [Flags] enum E { A }')
$null = & $binary csc $bad "-reference:$fakeOutput" "-out:$output" 2>&1
if ($LASTEXITCODE -ne 1) { throw 'A Flags alias silently replaced an unrelated attribute with the framework attribute.' }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Invalid attribute alias changed a previous output.' }
$valid = Join-Path $work 'InternalVisibility.cs'
$validOutput = Join-Path $work 'InternalVisibility.dll'
[IO.File]::WriteAllText($valid, 'enum Hidden { A = 4 } internal class InternalClass { public static Hidden Value() { return Hidden.A; } } public class PublicClass { internal Hidden Field; private protected Hidden Access() { return Hidden.A; } public static int Run() { return (int)InternalClass.Value(); } }')
& $binary csc $valid "-out:$validOutput"
if ($LASTEXITCODE -ne 0) { throw 'Legal internal enum visibility was rejected.' }
& dotnet $enumOracle --invoke $validOutput 'PublicClass' 'Run' '4'
if ($LASTEXITCODE -ne 0) { throw 'Legal internal enum visibility failed at runtime.' }
exit 0
