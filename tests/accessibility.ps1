$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$build = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Compiler build failed: $build" }
$build = & dotnet build (Join-Path $root 'tests/AccessibilityOracle/AccessibilityOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $build" }
$work = Join-Path (Join-Path $root 'benchmarks/results/accessibility') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/accessibility/Api.cs'
$output = Join-Path $work 'Api.dll'
$reference = Join-Path (Join-Path $work 'ref') 'Api.dll'
& $binary csc $source "-out:$output" "-refout:$reference"
if ($LASTEXITCODE -ne 0) { throw 'Accessibility fixture failed to compile' }
$oracle = Join-Path $root 'tests/AccessibilityOracle/bin/Release/net10.0/AccessibilityOracle.dll'
& dotnet $oracle $source $output
if ($LASTEXITCODE -ne 0) { throw 'Metadata visibility differs from Roslyn' }
$consumer = Join-Path $work 'Consumer.cs'
$consumerOutput = Join-Path $work 'Consumer.dll'
foreach ($library in @($output, $reference)) {
    $valid = 'class Client { public static int F(AccessFixture.Api api) { return api.PublicField + api.Value; } }'
    [IO.File]::WriteAllText($consumer, $valid)
    & dotnet $oracle --probe $consumer $library accept
    if ($LASTEXITCODE -ne 0) { throw 'Roslyn rejected the public API' }
    & $binary csc $consumer "-r:$library" "-out:$consumerOutput"
    if ($LASTEXITCODE -ne 0) { throw 'Compiler rejected the public API' }
    & dotnet $oracle --execute $output $consumerOutput
    if ($LASTEXITCODE -ne 0) { throw 'Public consumer failed to run against the implementation' }
    $derived = 'class Derived : AccessFixture.Api { public int F() { return base.ProtectedMethod(); } }'
    [IO.File]::WriteAllText($consumer, $derived)
    & dotnet $oracle --probe $consumer $library accept
    if ($LASTEXITCODE -ne 0) { throw 'Roslyn rejected derived protected access' }
    & $binary csc $consumer "-r:$library" "-out:$consumerOutput"
    if ($LASTEXITCODE -ne 0) { throw 'Compiler rejected derived protected access' }
    $before = (Get-FileHash -Algorithm SHA256 $consumerOutput).Hash
    foreach ($invalid in @(
        'class Client { static int F(AccessFixture.Api api) { return api.PrivateField; } }',
        'class Client { static int F(AccessFixture.Api api) { return api.InternalField; } }',
        'class Client { static int F(AccessFixture.Api api) { return api.ProtectedField; } }',
        'class Client { static int F(AccessFixture.Api api) { return api.PrivateMethod(); } }',
        'class Client { static int F(AccessFixture.Api api) { return api.StaticMethod(); } }',
        'class Client { static int F(AccessFixture.Api api) { return api.StaticField; } }',
        'class Client { static int F() { return AccessFixture.Api.PublicMethod(); } }',
        'class Client { static int F(AccessFixture.Api api) { return api.Write; } }',
        'class Client { static void F(AccessFixture.Api api) { api.Value = 1; } }',
        'class Client { static int F(AccessFixture.InternalType api) { return api.Field; } }'
    )) {
        [IO.File]::WriteAllText($consumer, $invalid)
        & dotnet $oracle --probe $consumer $library reject
        if ($LASTEXITCODE -ne 0) { throw "Roslyn did not reject the external access: $invalid" }
        $null = & $binary csc $consumer "-r:$library" "-out:$consumerOutput" 2>&1
        if ($LASTEXITCODE -ne 1) { throw "Compiler accepted external inaccessible member: $invalid" }
        if ((Get-FileHash -Algorithm SHA256 $consumerOutput).Hash -ne $before) { throw 'Failed compilation changed previous output' }
    }
}
foreach ($invalid in @(
    'class A { int x; } class B { static int F(A a) { return a.x; } }',
    'class A { int M() { return 1; } } class B { static int F(A a) { return a.M(); } }',
    'class A { private A() {} } class B { static A F() { return new A(); } }',
    'class A { public int P { get; private set; } } class B { static void F(A a) { a.P = 1; } }',
    'class A { private static int M() { return 1; } } class B { static int F() { return A.M(); } }',
    'class A { int x; } class B { static A F() { return new A { x = 1 }; } }',
    'class A { protected int M() { return 1; } } class B : A { int F(A a) { return a.M(); } }',
    'class A { public public int x; }',
    'class A { public int P { get; public set; } }',
    'class A { public int P { private get; private set; } }',
    'private class A {}'
)) {
    [IO.File]::WriteAllText($consumer, $invalid)
    & dotnet $oracle --probe $consumer $output reject
    if ($LASTEXITCODE -ne 0) { throw "Roslyn did not reject the local access: $invalid" }
    $null = & $binary csc $consumer "-out:$consumerOutput" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Compiler accepted local inaccessible member: $invalid" }
}
foreach ($valid in @(
    'class A { int x; public static int F(A a) { a.x = 1; return a.x; } }',
    'class A { public int P { get; private set; } public static int F(A a) { a.P = 2; return a.P; } }',
    'class A { internal int x; } class B { static int F(A a) { return a.x; } }',
    'class A { protected int M() { return 1; } } class B : A { int F() { return this.M(); } }'
)) {
    [IO.File]::WriteAllText($consumer, $valid)
    & dotnet $oracle --probe $consumer $output accept
    if ($LASTEXITCODE -ne 0) { throw "Roslyn rejected valid local access: $valid" }
    $null = & $binary csc $consumer "-out:$consumerOutput" 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Compiler rejected valid local access: $valid" }
}
Write-Output 'Metadata and real downstream execution verified; inaccessible members and invalid modifiers rejected.'
exit 0
