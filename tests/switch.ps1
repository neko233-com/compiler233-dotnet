$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
& cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Compiler build failed' }
& dotnet build (Join-Path $root 'tests/CompileOracle/CompileOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw 'Runtime oracle build failed' }
& dotnet build (Join-Path $root 'tests/AccessibilityOracle/AccessibilityOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw 'Diagnostic oracle build failed' }
$work = Join-Path (Join-Path $root 'benchmarks/results/switch') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/switch/Program.cs'
$output = Join-Path $work 'Switch.dll'
$manifest = Join-Path $work 'manifest.txt'
[IO.File]::WriteAllLines($manifest, @($source))
& $binary csc $source "-out:$output"
if ($LASTEXITCODE -ne 0) { throw 'Switch fixture failed to compile' }
$oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'
foreach ($method in @('IntegerCases', 'StringCases', 'EscapedStringCases', 'EscapedExpression', 'BooleanCases', 'RadixCases', 'DenseCases', 'DenseExtremeCases', 'DenseMinCases', 'DenseWithFilter', 'NestedSwitchCases', 'ConstantExpression', 'EmptySwitchEvaluatesOnce', 'LoopBreakAndContinue', 'NestedLoopBreak', 'NestedSwitchContinue', 'BreakThroughFinally', 'SwitchInsideFinally', 'ContinueThroughFinally', 'BlockScopes', 'NestedDo')) {
    $result = & dotnet $oracle $manifest $output 'SwitchFixture.Runner' $method 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Runtime mismatch for $method`: $result" }
    Write-Output $result.Trim()
}
$diagnosticOracle = Join-Path $root 'tests/AccessibilityOracle/bin/Release/net10.0/AccessibilityOracle.dll'
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'class C { void F(int x) { switch (x) { case 1: x++; case 2: break; } } }',
    'class C { void F(int x) { switch (x) { case 1: break; case 1: break; } } }',
    'class C { void F(int x) { switch (x) { default: break; default: break; } } }',
    'class C { void F(int x) { switch (x) { case true: break; } } }',
    'class C { void F(int x) { switch (x) { case 1: continue; } } }',
    'class C { void F(int x) { switch (x) { case 1: } } }',
    'class C { int F(int x) { switch (x) { case 1: break; return 1; default: return 2; } } }',
    'class C { void F(string x) { switch (x) { case "\u0061": break; case "a": break; } } }',
    'class C { string F() { return "\q"; } }',
    'class C { string F() { return "\u12"; } }',
    'class C { string F() { return "\x"; } }',
    'class C { string F() { return "\U00110000"; } }',
    'class C { void F() { while (true) { try {} finally { switch (1) { case 1: continue; } } } } }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    & dotnet $diagnosticOracle --probe $bad $output reject
    if ($LASTEXITCODE -ne 0) { throw "Roslyn accepted invalid switch: $invalid" }
    $null = & $binary csc $bad "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid switch was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Failed compilation changed previous output' }
}
foreach ($unsupported in @(
    'class C { void F(int x) { switch (x) { case 1: int local = 2; break; } } }',
    'class C { string F() { return "\uD800"; } }'
)) {
    [IO.File]::WriteAllText($bad, $unsupported)
    & dotnet $diagnosticOracle --probe $bad $output accept
    if ($LASTEXITCODE -ne 0) { throw "Roslyn rejected expected supported syntax: $unsupported" }
    $null = & $binary csc $bad "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Unsupported switch syntax was accepted: $unsupported" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Unsupported compilation changed previous output' }
}
Write-Output 'Invalid and unsupported switch cases rejected; previous output preserved.'
exit 0
