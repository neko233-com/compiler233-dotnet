$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$caseRoot = Join-Path (Join-Path $root 'benchmarks/results/differential-contract') ([guid]::NewGuid().ToString('N'))
$output = Join-Path $caseRoot 'output'
New-Item -ItemType Directory -Force $output | Out-Null
$sentinel = Join-Path $output 'keep.txt'
[System.IO.File]::WriteAllText($sentinel, 'preserve')
$shell = [System.Environment]::ProcessPath
$script = Join-Path $root 'tests/differential.ps1'
$result = & $shell -NoProfile -File $script -OutputDirectory $output 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "Passing differential gate returned failure: $result" }
if ((Get-Content -LiteralPath $sentinel -Raw) -ne 'preserve') { throw 'Differential gate removed an unrelated output file' }
$report = Get-Content -LiteralPath (Join-Path $output 'differential.json') -Raw | ConvertFrom-Json
if ($report.falseAccept -ne 0 -or $report.reject -lt 1) { throw 'Differential fixture classification was not exercised' }
$missing = Join-Path $caseRoot 'missing'
$result = & $shell -NoProfile -File $script -Corpus $missing -OutputDirectory $output 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { throw "Missing corpus was reported as success: $result" }
if ((Get-Content -LiteralPath $sentinel -Raw) -ne 'preserve') { throw 'Failed differential run removed an unrelated output file' }
Write-Output 'Differential gate exit status and output preservation checks passed.'
exit 0
