$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$build = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Compiler build failed: $build" }
$build = & dotnet build (Join-Path $root 'tests/CompileOracle/CompileOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $build" }
$work = Join-Path (Join-Path $root 'benchmarks/results/primitives') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/primitives/Program.cs'
$output = Join-Path $work 'Primitive.dll'
& $binary csc $source "-out:$output"
if ($LASTEXITCODE -ne 0) { throw 'Primitive fixture failed to compile' }
$manifest = Join-Path $work 'manifest.txt'
[IO.File]::WriteAllLines($manifest, @($source))
$oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'
foreach ($method in @('Once', 'Integer', 'NullFormat', 'Single', 'Boolean', 'IntegerHash', 'StringHash', 'StringValue', 'NullString', 'NullObject')) {
    $result = & dotnet $oracle $manifest $output 'PrimitiveFixture.Runner' $method 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Runtime mismatch for $method`: $result" }
    if ($method -eq 'Once' -and $result -notmatch 'Roslyn=1, compiler233=1') { throw "Receiver evaluated more than once: $result" }
    if ($method -in @('NullString', 'NullObject') -and $result -notmatch 'exception:System.NullReferenceException') { throw "Expected null exception: $result" }
    Write-Output $result.Trim()
}
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'class C { string F(int x) => x.ToString(1); }',
    'class C { string F(int x) => x.ToString(null); }',
    'class C { string F(bool x) => x.ToString("X"); }',
    'class C { string F(int x) => x.ToString<int>(); }',
    'class C { int F(int x) => x.GetHashCode(1); }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid primitive call was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Failed compilation changed previous output' }
}
Write-Output 'Invalid primitive calls rejected and previous output preserved.'
exit 0
