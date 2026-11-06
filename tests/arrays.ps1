$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$build = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Compiler build failed: $build" }
$build = & dotnet build (Join-Path $root 'tests/CompileOracle/CompileOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $build" }
$work = Join-Path (Join-Path $root 'benchmarks/results/arrays') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/arrays/Program.cs'
$output = Join-Path $work 'Arrays.dll'
& $binary csc $source "-out:$output"
if ($LASTEXITCODE -ne 0) { throw 'Array fixture failed to compile' }
$manifest = Join-Path $work 'manifest.txt'
[IO.File]::WriteAllLines($manifest, @($source))
$oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'
foreach ($method in @('FloatRead', 'FloatEach', 'BoolRead', 'BoolEach', 'IntEach', 'ReferenceRead', 'ObjectRead', 'ReferenceEach', 'Bounds', 'WriteBounds', 'NullArray')) {
    $result = & dotnet $oracle $manifest $output 'ArrayFixture.Runner' $method 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Runtime mismatch for $method`: $result" }
    if ($method -in @('Bounds', 'WriteBounds') -and $result -notmatch 'exception:System.IndexOutOfRangeException') { throw "Expected bounds exception: $result" }
    if ($method -eq 'NullArray' -and $result -notmatch 'exception:System.NullReferenceException') { throw "Expected null exception: $result" }
    Write-Output $result.Trim()
}
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'class C { void F(bool[] a) { a[0] = 1; } }',
    'class C { void F(float[] a) { a[true] = 1.5f; } }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid array assignment was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Failed compilation changed previous output' }
}
Write-Output 'Invalid array assignments rejected and previous output preserved.'
exit 0
