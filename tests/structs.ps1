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
    'struct S { protected int X; }',
    'struct S { private int X; } class C { static int F() { S x = default(S); return x.X; } }',
    'struct S { public S Child; }',
    'enum E { Value } public struct S { public E Field; }',
    'struct S { public int X = 1; }',
    'struct S { public int X; public S(int x) { X = x; } }',
    'struct S { public int X; public int Read() { return X; } }',
    'struct S : System.IDisposable { public int X; }',
    'struct S { public int X; } class C { static int F() { S x; return x.X; } }',
    'struct S { public int X; } class C { static int F() { S x = x; return x.X; } }',
    'struct S { public int X; } class C { static object F() { S x = default(S); return x; } }',
    'struct S { public int X; } class C { S F() { return new S(1); } }',
    'struct S { public int X; } class C { S F() { return new S { X = 1, X = 2 }; } }',
    'struct S { public int X; } class C { S F() { return new S { Missing = 1 }; } }',
    'struct S { private int X; } class C { S F() { return new S { X = 1 }; } }',
    'struct S { public int X; } class C { S F() { return new S { X = "wrong" }; } }',
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
$foreign = Join-Path $work 'StructCollisionReference.dll'
& dotnet $oracle --make-struct-collision $foreign
if ($LASTEXITCODE -ne 0) { throw 'Collision reference build failed' }
[IO.File]::WriteAllText($bad, 'using StructLibrary; namespace StructLibrary { public struct Same { public int X; } } namespace StructCollision { public class C { public static Same Echo(Same value) { return value; } } }')
$null = & $binary csc $bad "-r:$foreign" "-out:$output" "-refout:$reference" 2>&1
if ($LASTEXITCODE -ne 1 -or (Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or
    (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $referenceBefore) {
    throw 'Referenced class collision was accepted or changed an output'
}
$qualified = Join-Path $work 'Qualified.cs'
$qualifiedOutput = Join-Path $work 'Qualified.dll'
[IO.File]::WriteAllText($qualified, 'using Alias = StructLibrary.Same; namespace StructLibrary { public struct Same { public int X; } } namespace StructCollision { public class C { public static int Qualified() { StructLibrary.Same x = default(StructLibrary.Same); x.X = 8; return x.X; } public static int Aliased() { Alias x = default(Alias); x.X = 9; return x.X; } } }')
& $binary csc $qualified "-r:$foreign" "-out:$qualifiedOutput"
if ($LASTEXITCODE -ne 0) { throw 'Qualified or aliased struct should remain resolvable' }
& dotnet $oracle --invoke $qualifiedOutput 'StructCollision.C' 'Qualified' '8'
if ($LASTEXITCODE -ne 0) { throw 'Qualified struct runtime failed' }
& dotnet $oracle --invoke $qualifiedOutput 'StructCollision.C' 'Aliased' '9'
if ($LASTEXITCODE -ne 0) { throw 'Aliased struct runtime failed' }
Write-Output 'Unsupported struct forms rejected; previous outputs preserved.'
exit 0
