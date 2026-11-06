$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$build = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Compiler build failed: $build" }
$project = Join-Path $root 'tests/CompileOracle/CompileOracle.csproj'
$build = & dotnet build $project -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $build" }
$work = Join-Path (Join-Path $root 'benchmarks/results/expression') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/expression/Program.cs'
$output = Join-Path $work 'Expression.dll'
& $binary csc $source "-out:$output"
if ($LASTEXITCODE -ne 0) { throw 'Expression fixture did not compile' }
$manifest = Join-Path $work 'manifest.txt'
[IO.File]::WriteAllLines($manifest, @($source))
$oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'
$result = & dotnet $oracle $manifest $output 'ExpressionFixture.Runner' 'Run' 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or $result -notmatch 'Roslyn=42, compiler233=42') { throw "Runtime mismatch: $result" }
Write-Output $result.Trim()
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'class C { void F() => 1; }',
    'class C { void F() => int x = 1; }',
    'class C { int F() => "wrong"; }',
    'class C { void P => 1; }',
    'interface I { int F() => 1; }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid or unsupported source was accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Failed compilation changed previous output' }
}
Write-Output 'Invalid expression bodies rejected and previous output preserved.'
exit 0
