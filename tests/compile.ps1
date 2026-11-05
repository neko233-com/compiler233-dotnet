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

# External assembly reference: compile a library, then consume it with -r.
$externalDir = Join-Path $work 'external'
New-Item -ItemType Directory -Force $externalDir | Out-Null
$libSource = Join-Path $root 'tests/fixtures/external/Lib.cs'
$runnerSource = Join-Path $root 'tests/fixtures/external/Runner.cs'
$libDll = Join-Path $externalDir 'Demo.External.dll'
& $binary csc $libSource "-out:$libDll"
if ($LASTEXITCODE -ne 0) { throw 'library for external reference failed to compile' }
$runnerDll = Join-Path $externalDir 'Runner.dll'
& $binary csc $runnerSource "-r:$libDll" "-out:$runnerDll"
if ($LASTEXITCODE -ne 0) { throw 'consumer of the external reference failed to compile' }
$externalManifest = Join-Path $externalDir 'manifest.txt'
[System.IO.File]::WriteAllLines($externalManifest, @($libSource, $runnerSource), [System.Text.UTF8Encoding]::new($false))
$external = & dotnet $oracle $externalManifest $runnerDll 'Demo.ExternalRunner' 'Run' 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "external reference comparison failed: $external" }
if ($external -notmatch 'compiler233=42') { throw "unexpected external result: $external" }
Write-Output $external.Trim()

# Real framework assembly: qualified type name resolved through -reference.
$runtimeLine = (& dotnet --list-runtimes | Where-Object { $_ -match '^Microsoft\.NETCore\.App ' } | Select-Object -Last 1)
if ($runtimeLine -and $runtimeLine -match '^Microsoft\.NETCore\.App (\S+) \[(.+)\]$') {
    $runtimeVersion = $Matches[1]
    $runtimeRoot = $Matches[2]
    $regexDll = Join-Path (Join-Path $runtimeRoot $runtimeVersion) 'System.Text.RegularExpressions.dll'
    if (Test-Path $regexDll) {
        $frameworkSource = Join-Path $root 'tests/fixtures/external/Framework.cs'
        $frameworkOutput = Join-Path $work 'Framework.dll'
        & $binary csc $frameworkSource "-r:$regexDll" "-out:$frameworkOutput"
        if ($LASTEXITCODE -ne 0) { throw 'framework reference compilation failed' }
        $frameworkManifest = Join-Path $work 'framework-manifest.txt'
        [System.IO.File]::WriteAllLines($frameworkManifest, @($frameworkSource), [System.Text.UTF8Encoding]::new($false))
        $frameworkResult = & dotnet $oracle $frameworkManifest $frameworkOutput 'Demo.FrameworkRunner' 'Run' 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) { throw "framework reference comparison failed: $frameworkResult" }
        Write-Output $frameworkResult.Trim()
    }
}
# Generic method call: a Roslyn-built library is referenced with -r.
$genericProject = Join-Path $root 'tests/fixtures/generic/GenericLib.csproj'
$genericBuild = & dotnet build $genericProject -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "generic library build failed: $genericBuild" }
$genericLib = Join-Path $root 'tests/fixtures/generic/bin/Release/net8.0/GenericLib.dll'
$genericRunner = Join-Path $root 'tests/fixtures/generic/Runner.cs'
$genericOutput = Join-Path $work 'GenericRunner.dll'
& $binary csc $genericRunner "-r:$genericLib" "-out:$genericOutput"
if ($LASTEXITCODE -ne 0) { throw 'generic method call failed to compile' }
# The runtime probe resolves referenced assemblies from the output directory.
Copy-Item $genericLib $work -Force
$genericManifest = Join-Path $work 'generic-manifest.txt'
[System.IO.File]::WriteAllLines($genericManifest, @((Join-Path $root 'tests/fixtures/generic/Lib.cs'), $genericRunner), [System.Text.UTF8Encoding]::new($false))
$genericResult = & dotnet $oracle $genericManifest $genericOutput 'Demo.GenericRunner' 'Run' 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "generic call comparison failed: $genericResult" }
Write-Output $genericResult.Trim()
Write-Output 'Compile and runtime differential checks passed.'
