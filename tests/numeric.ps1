$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$build = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Compiler build failed: $build" }
$build = & dotnet build (Join-Path $root 'tests/CompileOracle/CompileOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $build" }
$work = Join-Path (Join-Path $root 'benchmarks/results/numeric') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/numeric/Program.cs'
$output = Join-Path $work 'Numeric.dll'
& $binary csc $source "-out:$output"
if ($LASTEXITCODE -ne 0) { throw 'Numeric fixture failed to compile' }
$manifest = Join-Path $work 'manifest.txt'
[IO.File]::WriteAllLines($manifest, @($source))
$oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'
foreach ($method in @('NanLessEqual', 'NanGreaterEqual', 'NormalCompare', 'Negation', 'ReturnInt', 'Promotion', 'Fields', 'InitialField', 'MixedLeft', 'MixedRight', 'Rounding', 'Evaluation', 'Conditional', 'MixedCompare')) {
    $result = & dotnet $oracle $manifest $output 'NumericFixture.Runner' $method 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Runtime mismatch for $method`: $result" }
    Write-Output $result.Trim()
}
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'class C { int F(float n) => n; }',
    'class C { float F(bool n) => n; }',
    'class C { float F() => 1 + true; }',
    'class C { void F() { float n = 1.0f; n &= 1; } }',
    'class C { float F() => 1.0f ?? 1; }',
    'class C { bool F(int x) => x == null; }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid numeric conversion was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Failed compilation changed previous output' }
}
Write-Output 'Invalid numeric conversions rejected and previous output preserved.'
exit 0
