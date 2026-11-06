$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
& cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Compiler build failed' }
& dotnet build (Join-Path $root 'tests/EnumOracle/EnumOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw 'Runtime probe build failed' }
$work = Join-Path (Join-Path $root 'benchmarks/results/structs') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$output = Join-Path $work 'StructApi.dll'
$reference = Join-Path (Join-Path $work 'ref') 'StructApi.dll'
& $binary csc (Join-Path $root 'tests/fixtures/structs/Alias.cs') (Join-Path $root 'tests/fixtures/structs/Values.cs') "-out:$output" "-refout:$reference"
if ($LASTEXITCODE -ne 0) { throw 'Struct fixture failed to compile' }
$oracle = Join-Path $root 'tests/EnumOracle/bin/Release/net10.0/EnumOracle.dll'
& dotnet $oracle --structs $output $reference
if ($LASTEXITCODE -ne 0) { throw 'Struct metadata, runtime, or downstream validation failed' }
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$referenceBefore = (Get-FileHash -Algorithm SHA256 $reference).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'readonly struct S { public int X; }',
    '[System.Serializable] struct S { public int X; }',
    'struct S { private int X; }',
    'struct S { public float X; }',
    'struct S { public int X = 1; }',
    'struct S { public int X; public S(int x) { X = x; } }',
    'struct S { public int X; public int Read() { return X; } }',
    'struct S : System.IDisposable { public int X; }',
    'struct S { public int X; } class C { static int F() { S x; return x.X; } }',
    'struct S { public int X; } class C { static int F() { S x = x; return x.X; } }',
    'struct S { public int X; } class C { static object F() { S x = default(S); return x; } }',
    'struct S { public int X; } class C { static S[] F() { return new S[1]; } }',
    'struct S { public int X; } class C { static int F() { S x = default(S); x.X++; return x.X; } }',
    'struct S { public int X; } class C { S x; void F() { x.X = 1; } }',
    'struct S { public int X; } class C { static S x; static void F() { C.x.X = 1; } }',
    'struct S { public int X; } public class C { public S Field; }',
    'struct S { public int X; } class C : S { }',
    'struct S { public int X; } interface I : S { }',
    'struct S { public int X; } struct S { public int Y; }',
    'using A; namespace A { public enum S { V = 0 } } namespace B { public struct S { public int X; } class C { S F(S x) { return x; } } }',
    'namespace Hidden { public struct S { public int X; } } namespace Other { public class C { public static S F(S x) { return x; } } }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-out:$output" "-refout:$reference" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Unsupported struct code was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or
        (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $referenceBefore) {
        throw 'Invalid struct compilation changed a previous output'
    }
}
Write-Output 'Unsupported struct forms rejected; previous outputs preserved.'
exit 0
