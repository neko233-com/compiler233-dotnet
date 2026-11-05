$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$buildOutput = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Rust build failed: $buildOutput" }

$oracleProject = Join-Path $root 'tests/CompileOracle/CompileOracle.csproj'
$oracleBuild = & dotnet build $oracleProject -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $oracleBuild" }
$oracle = Join-Path $root 'tests/CompileOracle/bin/Release/net10.0/CompileOracle.dll'

$work = Join-Path $root 'benchmarks/results/compile'
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Force $work | Out-Null

$sources = @(Get-ChildItem (Join-Path $root 'tests/fixtures/compile') -Filter *.cs | Sort-Object Name | ForEach-Object { $_.FullName })
if ($sources.Count -eq 0) { throw 'no compile fixtures found' }

$output = Join-Path $work 'Program.dll'
$result = & $binary csc @sources "-out:$output"
if ($LASTEXITCODE -ne 0) { throw "compiler failed on the fixture: $result" }
if (-not (Test-Path $output)) { throw 'compiler produced no assembly' }

$manifest = Join-Path $work 'manifest.txt'
[System.IO.File]::WriteAllLines($manifest, $sources, [System.Text.UTF8Encoding]::new($false))

$comparison = & dotnet $oracle $manifest $output 'Demo.Program' 'Run' 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "runtime comparison failed: $comparison" }
if ($comparison -notmatch 'Roslyn=(\d+), compiler233=(\d+)') { throw "unexpected oracle output: $comparison" }
if ($Matches[1] -ne $Matches[2]) { throw "result mismatch: $comparison" }
Write-Output $comparison.Trim()

# A failed compilation keeps the previous assembly and reports a located error.
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$undefined = Join-Path $work 'Undefined.cs'
[System.IO.File]::WriteAllText($undefined, "class Broken { static int F() { return missing; } }")
$failure = & $binary csc $undefined "-out:$output" 2>&1 | Out-String
if ($LASTEXITCODE -ne 1) { throw 'undefined identifier was accepted' }
if ($failure -notmatch 'CD2330011') { throw "semantic error did not use CD2330011: $failure" }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'failed compilation replaced the output' }

$syntax = Join-Path $work 'Syntax.cs'
[System.IO.File]::WriteAllText($syntax, "class Broken { static int F() { return 1; }")
$failure = & $binary csc $syntax "-out:$output" 2>&1 | Out-String
if ($LASTEXITCODE -ne 1) { throw 'unbalanced source was accepted' }
if ($failure -notmatch 'CD2330010') { throw "syntax error did not use CD2330010: $failure" }

Write-Output 'Compile and runtime differential checks passed.'
