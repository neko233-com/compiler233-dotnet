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
    'enum E { [System.Obsolete] A }'
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
[IO.File]::WriteAllText($bad, 'public class Consumer { public static EnumFixture.Direction Echo(EnumFixture.Direction value) { return value; } }')
$null = & $binary csc $bad "-reference:$reference" "-out:$output" 2>&1
if ($LASTEXITCODE -ne 1) { throw 'Unsupported external enum was accepted as a reference type.' }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or
    (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $referenceBefore) {
    throw 'Unsupported external enum changed a previous output'
}
Write-Output 'Unsupported external enum rejected; previous outputs preserved.'
$valid = Join-Path $work 'InternalVisibility.cs'
$validOutput = Join-Path $work 'InternalVisibility.dll'
[IO.File]::WriteAllText($valid, 'enum Hidden { A = 4 } internal class InternalClass { public static Hidden Value() { return Hidden.A; } } public class PublicClass { internal Hidden Field; private protected Hidden Access() { return Hidden.A; } public static int Run() { return (int)InternalClass.Value(); } }')
& $binary csc $valid "-out:$validOutput"
if ($LASTEXITCODE -ne 0) { throw 'Legal internal enum visibility was rejected.' }
& dotnet $enumOracle --invoke $validOutput 'PublicClass' 'Run' '4'
if ($LASTEXITCODE -ne 0) { throw 'Legal internal enum visibility failed at runtime.' }
exit 0
