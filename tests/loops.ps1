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
$work = Join-Path (Join-Path $root 'benchmarks/results/loops') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/loops/Program.cs'
$output = Join-Path $work 'Loops.dll'
$manifest = Join-Path $work 'manifest.txt'
[IO.File]::WriteAllLines($manifest, @($source))
& $binary csc $source "-out:$output"
if ($LASTEXITCODE -ne 0) { throw 'Loop fixture failed to compile' }
$oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'
foreach ($method in @('ExecuteOnce', 'ContinueTestsCondition', 'BreakSkipsCondition', 'ContinueAndBreakThroughFinally', 'LoopInsideFinally', 'ScopedSiblings', 'NestedScopedSiblings', 'EarlierSiblingScope', 'NestedLoops', 'SingleAndEmptyBody')) {
    $result = & dotnet $oracle $manifest $output 'LoopFixture.Runner' $method 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Runtime mismatch for $method`: $result" }
    Write-Output $result.Trim()
}
$diagnosticOracle = Join-Path $root 'tests/AccessibilityOracle/bin/Release/net10.0/AccessibilityOracle.dll'
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'class C { void F() { do {} while (1); } }',
    'class C { void F() { do { bool local = false; } while (local); } }',
    'class C { void F() { do int local = 1; while (false); } }',
    'class C { void F() { int x = 1; do { int x = 2; } while (false); } }',
    'class C { void F(int x) { do { int x = 2; } while (false); } }',
    'class C { void F() { do { int x = 1; int x = 2; } while (false); } }',
    'class C { void F() { do { int x = 2; } while (false); int x = 1; } }',
    'class C { void F() { if (true) { do { int x = 2; } while (false); } int x = 1; } }',
    'class C { void F() { do { int x = 1; if (true) { int x = 2; } } while (false); } }',
    'class C { void F() { do { if (true) { int x = 1; } int y = x; } while (false); } }',
    'class C { void F() { do { for (int x = 0; x < 1; x++) {} int x = 1; } while (false); } }',
    'class C { void F() { do {} while (false) } }',
    'class C { void F() { do { try {} finally { continue; } } while (false); } }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    & dotnet $diagnosticOracle --probe $bad $output reject
    if ($LASTEXITCODE -ne 0) { throw "Roslyn accepted invalid loop: $invalid" }
    $null = & $binary csc $bad "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid loop was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Failed compilation changed previous output' }
}
Write-Output 'Invalid loops rejected by Roslyn and compiler233; previous output preserved.'
exit 0
