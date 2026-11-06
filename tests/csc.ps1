$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$buildOutput = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Rust build failed: $buildOutput" }

$work = Join-Path $root 'benchmarks/results/csc'
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
$spacedDir = Join-Path $work 'proof dir'
New-Item -ItemType Directory -Force $spacedDir | Out-Null

function Write-Text([string] $path, [string[]] $lines) {
    [System.IO.File]::WriteAllLines($path, $lines, [System.Text.UTF8Encoding]::new($false))
}

function Invoke-Csc([string[]] $arguments) {
    $output = & $binary @arguments 2>&1 | Out-String
    return [pscustomobject]@{ ExitCode = $LASTEXITCODE; Output = $output }
}

function Assert-True([bool] $condition, [string] $message) {
    if (-not $condition) { throw $message }
}

$fixture = Join-Path $work 'Arithmetic.cs'
Copy-Item (Join-Path $root 'tests/fixtures/Arithmetic.cs') $fixture -Force
$second = Join-Path $work 'Second.cs'
Write-Text $second @('class Second { static int F() { return 2; } }')

# Nested response files, quotes around paths with spaces and comments.
$output = Join-Path $spacedDir 'patch.dll'
$refout = Join-Path $spacedDir 'patch.ref.dll'
$inner = Join-Path $work 'inner.rsp'
Write-Text $inner @(
    '# inner response file',
    '-nologo',
    '-langversion:9.0',
    '-optimize+',
    ('"' + ($fixture.Replace('\', '/')) + '"')
)
$outer = Join-Path $work 'outer.rsp'
Write-Text $outer @(
    '-target:library',
    '-define:HOTRELOAD233_PATCH',
    ('-out:"' + ($output.Replace('\', '/')) + '"'),
    ('"@' + ($inner.Replace('\', '/')) + '"')
)
$reportPath = Join-Path $work 'success-report.json'
$result = Invoke-Csc @("@$outer", "-report:$reportPath")
Assert-True ($result.ExitCode -eq 0) "csc invocation failed: $($result.Output)"
Assert-True (Test-Path $output) 'csc invocation did not write the assembly'
$report = Get-Content $reportPath -Raw | ConvertFrom-Json
Assert-True ($report.outcome -eq 'success') 'report outcome is not success'
Assert-True ($report.output_bytes -gt 0) 'report did not record output bytes'
Assert-True ($report.define -contains 'HOTRELOAD233_PATCH') 'report did not record the define'
Assert-True ($report.ignored_options -contains 'nologo') 'report did not record ignored options'
Assert-True ($report.ignored_options -contains 'optimize') 'report did not record the optimize modifier'

# A reference output is a separate metadata-only assembly.
$result = Invoke-Csc @($fixture, "-out:$output", "-refout:$refout")
Assert-True ($result.ExitCode -eq 0) "reference output failed: $($result.Output)"
Assert-True (Test-Path $refout) 'reference output was not created'

# A failed compilation must keep the previously produced assembly.
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$invalid = Join-Path $work 'Invalid.cs'
Write-Text $invalid @('class A { static int F() { return missing; } }')
$result = Invoke-Csc @("-target:library", "-out:$output", $invalid)
Assert-True ($result.ExitCode -eq 1) 'invalid source was accepted'
Assert-True ((Get-FileHash -Algorithm SHA256 $output).Hash -eq $before) 'failed compilation replaced the output'
Assert-True ($result.Output -match 'CD2330011') 'semantic error did not use a compiler233 diagnostic code'

# Syntax errors carry a location in csc form.
$broken = Join-Path $work 'Broken.cs'
Write-Text $broken @('class A {', '    void M( { }', '}')
$result = Invoke-Csc @($broken)
Assert-True ($result.ExitCode -eq 1) 'broken source was accepted'
Assert-True ($result.Output -match 'Broken\.cs\(\d+,\d+\): error CD2330010') "syntax diagnostic is not csc shaped: $($result.Output)"

# Multi-file requests compile into a single assembly.
$multiReport = Join-Path $work 'multi-report.json'
$multiOutput = Join-Path $work 'Multi.dll'
$result = Invoke-Csc @('-target:library', "-out:$multiOutput", $fixture, $second, "-report:$multiReport")
Assert-True ($result.ExitCode -eq 0) "multi-file request failed: $($result.Output)"
Assert-True (Test-Path $multiOutput) 'multi-file request produced no assembly'
$multi = Get-Content $multiReport -Raw | ConvertFrom-Json
Assert-True ($multi.source_count -eq 2) 'report did not record both sources'
Assert-True ($multi.outcome -eq 'success') 'report outcome is not success'

# Contract errors: csc codes for csc concepts, compiler233 codes for missing features.
$result = Invoke-Csc @('-frobnicate:yes', $fixture)
Assert-True ($result.ExitCode -eq 1) 'unknown option was accepted'
Assert-True ($result.Output -match 'CS2007') 'unknown option did not use CS2007'

$result = Invoke-Csc @('-target:library', (Join-Path $work 'Missing.cs'))
Assert-True ($result.ExitCode -eq 1) 'missing source was accepted'
Assert-True ($result.Output -match 'CS2001') 'missing source did not use CS2001'

$result = Invoke-Csc @('-target:exe', $fixture)
Assert-True ($result.ExitCode -eq 1) 'executable target was accepted'
Assert-True ($result.Output -match 'CD2330002') 'executable target did not report the unsupported case'

$result = Invoke-Csc @('-langversion:banana', $fixture)
Assert-True ($result.ExitCode -eq 1) 'invalid language version was accepted'
Assert-True ($result.Output -match 'CS1617') 'invalid language version did not use CS1617'

$result = Invoke-Csc @("@$(Join-Path $work 'absent.rsp')")
Assert-True ($result.ExitCode -eq 1) 'absent response file was accepted'
Assert-True ($result.Output -match 'CD2330007') 'absent response file did not report the missing file'

$result = Invoke-Csc @('-help')
Assert-True ($result.ExitCode -eq 0) 'help did not exit successfully'
Assert-True ($result.Output -match 'csc') 'help did not describe the csc front end'

$result = Invoke-Csc @('version')
Assert-True ($result.ExitCode -eq 0) 'version did not exit successfully'
Assert-True ($result.Output -match 'compiler233-dotnet \d+\.\d+\.\d+') "version output is not a version: $($result.Output)"

Write-Output 'csc front end contract checks passed.'
