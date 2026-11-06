$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$build = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Compiler build failed: $build" }
$build = & dotnet build (Join-Path $root 'tests/CompileOracle/CompileOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $build" }
$work = Join-Path (Join-Path $root 'benchmarks/results/exceptions') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/exceptions/Program.cs'
$output = Join-Path $work 'Exceptions.dll'
$reference = Join-Path (Join-Path $work 'ref') 'Exceptions.dll'
& $binary csc $source "-out:$output" "-refout:$reference"
if ($LASTEXITCODE -ne 0) { throw 'Exception fixture failed to compile' }
$manifest = Join-Path $work 'manifest.txt'
[IO.File]::WriteAllLines($manifest, @($source))
$oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'
foreach ($method in @('Catch', 'Multiple', 'GeneralCatch', 'ReturnFinally', 'RethrowFinally', 'LoopLeave', 'LoopInsideTry', 'Custom', 'CatchVariable', 'FinallyOnUnhandled', 'AllPathsReturn', 'FinallyThrows', 'VoidFinally', 'StringFinally', 'BoolFinally', 'NestedRethrow', 'FilterBeforeFinally', 'FilterTrue', 'FilterTypeMismatch', 'FilterThrows', 'FilterMutation', 'FilterGeneral', 'FilterMultiple', 'FilterTrueBeforeFinally', 'FilterWithFinallyAndRethrow', 'FilterGeneralBeforeTyped', 'FilterCustomException')) {
    $result = & dotnet $oracle $manifest $output 'ExceptionFixture.Runner' $method 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Runtime mismatch for $method`: $result" }
    Write-Output $result.Trim()
}
$consumer = Join-Path $work 'Consumer.cs'
$consumerOutput = Join-Path $work 'Consumer.dll'
[IO.File]::WriteAllText($consumer, 'public static class Consumer { public static int Run() { return ExceptionFixture.Runner.Catch(); } }')
& $binary csc $consumer "-r:$reference" "-out:$consumerOutput"
if ($LASTEXITCODE -ne 0) { throw 'Exception API reference consumer failed to compile' }
$consumerManifest = Join-Path $work 'consumer-manifest.txt'
[IO.File]::WriteAllLines($consumerManifest, @($source, $consumer))
$consumerResult = & dotnet $oracle $consumerManifest $consumerOutput 'Consumer' 'Run' 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or $consumerResult -notmatch 'Roslyn=7, compiler233=7') { throw "Exception API consumer failed: $consumerResult" }
Write-Output $consumerResult.Trim()
$build = & dotnet build (Join-Path $root 'tests/AccessibilityOracle/AccessibilityOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Diagnostic oracle build failed: $build" }
$diagnosticOracle = Join-Path $root 'tests/AccessibilityOracle/bin/Release/net10.0/AccessibilityOracle.dll'
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'class C { void F() { throw; } }',
    'class C { void F() { throw 1; } }',
    'class C { void F() { try {} finally { return; } } }',
    'class C { void F() { try {} catch (int) {} } }',
    'using System; class C { void F() { try {} catch (Exception e) when (1) {} } }',
    'using System; class C { void F() { try {} catch (Exception) {} catch (ArgumentException) {} } }',
    'class C { int F() { try { int hidden = 1; } finally {} return hidden; } }',
    'class C { void F() { try { int hidden = 1; } finally { int value = hidden; } } }',
    'using System; class C { Exception F() { try {} catch (Exception error) {} return error; } }',
    'class C { void F() { try {} } }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    & dotnet $diagnosticOracle --probe $bad $output reject
    if ($LASTEXITCODE -ne 0) { throw "Roslyn accepted invalid exception code: $invalid" }
    $null = & $binary csc $bad "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid exception code was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Failed compilation changed previous output' }
}
Write-Output 'Invalid exception code rejected by Roslyn and compiler233; previous output preserved.'
exit 0
