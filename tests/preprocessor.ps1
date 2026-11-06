$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$work = Join-Path $root 'benchmarks/results/preprocessor'
New-Item -ItemType Directory -Force $work | Out-Null
$build = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Compiler build failed: $build" }
$build = & dotnet build (Join-Path $root 'tests/CompileOracle/CompileOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $build" }
$oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'
$sources = @((Join-Path $root 'tests/fixtures/preprocessor/First.cs'), (Join-Path $root 'tests/fixtures/preprocessor/Second.cs'))
$manifest = Join-Path $work 'manifest.txt'
[System.IO.File]::WriteAllLines($manifest, $sources, [System.Text.UTF8Encoding]::new($false))
$output = Join-Path $work 'Preprocessor.dll'
foreach ($case in @(
    @{ defines = ''; expected = 7 },
    @{ defines = 'FLAG'; expected = 29 },
    @{ defines = 'FLAG,SECOND'; expected = 11 }
)) {
    $arguments = @('csc', "-out:$output") + $sources
    if ($case.defines) { $arguments += "-define:$($case.defines)" }
    $result = & $binary @arguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Compilation failed for '$($case.defines)': $result" }
    $result = & dotnet $oracle $manifest $output 'Demo.Program' 'Run' $case.defines 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0 -or $result -notmatch "compiler233=$($case.expected)\b") { throw "Conditional result mismatch for '$($case.defines)': $result" }
    Write-Output $result.Trim()
}
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$errorSource = Join-Path $work 'error.cs'
[System.IO.File]::WriteAllText($errorSource, "#if FLAG`n#error stop`n#endif`nclass C {}")
$result = & $binary csc "-out:$output" '-define:FLAG' $errorSource 2>&1 | Out-String
if ($LASTEXITCODE -ne 1 -or $result -notmatch 'CD2330010') { throw "Active #error was not rejected: $result" }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Failed compilation overwrote the previous DLL' }
Write-Output 'Preprocessor runtime and output-preservation checks passed.'
exit 0
