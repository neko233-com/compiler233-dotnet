$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$build = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Compiler build failed: $build" }
$build = & dotnet build (Join-Path $root 'tests/CompileOracle/CompileOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $build" }
$work = Join-Path (Join-Path $root 'benchmarks/results/assignments') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/assignments/Program.cs'
$output = Join-Path $work 'Assignments.dll'
& $binary csc $source "-out:$output"
if ($LASTEXITCODE -ne 0) { throw 'Assignment fixture failed to compile' }
$manifest = Join-Path $work 'manifest.txt'
[IO.File]::WriteAllLines($manifest, @($source))
$oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'
foreach ($method in @('Prefix', 'Postfix', 'AssignmentExpression', 'StringAssignment', 'FloatAssignment', 'ShadowLocal', 'ShadowParameter', 'PropertyPrefix', 'PropertyPostfix', 'PropertyAssignment', 'PropertyShadow', 'PropertyCompound', 'PropertyEffects', 'AssignmentEffects', 'LocalIncrement', 'ParameterIncrement', 'StaticProperty')) {
    $result = & dotnet $oracle $manifest $output 'AssignmentFixture.Runner' $method 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Runtime mismatch for $method`: $result" }
    Write-Output $result.Trim()
}
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$bad = Join-Path $work 'Bad.cs'
foreach ($invalid in @(
    'class C { int P { get; } void F() { P = 1; } }',
    'class C { int P { set; } int F() { return P++; } }',
    'class C { int P { get; set; } static void F() { P = 1; } }',
    'class C { int P { get; set; } static int F() { return P++; } }'
)) {
    [IO.File]::WriteAllText($bad, $invalid)
    $null = & $binary csc $bad "-out:$output" 2>&1
    if ($LASTEXITCODE -ne 1) { throw "Invalid property access accepted: $invalid" }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Failed compilation changed previous output' }
}
Write-Output 'Invalid property access rejected and previous output preserved.'
exit 0
