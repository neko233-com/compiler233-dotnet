$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$output = & (Join-Path $root 'benchmarks/equivalent-compile.ps1') -Iterations 10 -Warmup 1 -ExtraMethods 3 | Out-String
if ($LASTEXITCODE -ne 0) { throw "Equivalent benchmark failed: $output" }
$paths = @($output -split "\r?\n" | Where-Object { $_ -match 'report\.json\s*$' -and (Test-Path -LiteralPath $_.Trim() -PathType Leaf) })
if ($paths.Count -ne 1) { throw "Could not locate the benchmark report: $output" }
$report = Get-Content -LiteralPath $paths[0].Trim() -Raw | ConvertFrom-Json
if ($report.schema_version -ne 1 -or $report.architecture -ne 'X64' -or $report.iterations -ne 10 -or $report.generated_methods -ne 3 -or
    $report.generated_methods_executed -ne 3 -or $report.generated_expected_result -ne 18) {
    throw 'The report does not describe the requested native benchmark.'
}
if ($report.sources.Count -ne 2 -or $report.validation.Count -ne 12 -or $report.limitations.Count -lt 4) {
    throw 'The report is missing input, runtime validation or limitations.'
}
foreach ($source in $report.sources) {
    if ((Get-FileHash -LiteralPath $source.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $source.sha256) {
        throw 'A recorded source hash does not match the input.'
    }
}
if ((Get-FileHash -LiteralPath $report.reference.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $report.reference.sha256) {
    throw 'The recorded reference hash does not match the input.'
}
if ((Get-FileHash -LiteralPath $report.compiler233.executable -Algorithm SHA256).Hash.ToLowerInvariant() -ne $report.compiler233.sha256 -or
    (Get-FileHash -LiteralPath $report.roslyn.executable -Algorithm SHA256).Hash.ToLowerInvariant() -ne $report.roslyn.executable_sha256 -or
    (Get-FileHash -LiteralPath (Join-Path $root 'benchmarks/equivalent-compile.ps1') -Algorithm SHA256).Hash.ToLowerInvariant() -ne $report.benchmark_script_sha256) {
    throw 'A recorded executable or benchmark script hash does not match.'
}
foreach ($assembly in $report.roslyn.assemblies) {
    if ((Get-FileHash -LiteralPath $assembly.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $assembly.sha256) {
        throw 'A recorded Roslyn assembly hash does not match.'
    }
}
$left = [string]::Join([char]0, [string[]]$report.compiler233.arguments[1..($report.compiler233.arguments.Count - 1)])
$right = [string]::Join([char]0, [string[]]$report.roslyn.arguments[1..($report.roslyn.arguments.Count - 1)])
if ($left -ne $right) { throw 'The compilers did not receive the same common arguments.' }
foreach ($kind in @('compiler233', 'roslyn')) {
    $result = $report.$kind
    if ($result.samples.Count -ne 10 -or @($result.samples | Where-Object {
        $_.milliseconds -le 0 -or $_.peak_working_set_bytes -le 0 -or $_.peak_commit_bytes -le 0
    }).Count -ne 0) {
        throw "$kind is missing timing or memory samples."
    }
    foreach ($metric in @(
        @('milliseconds', 'wall_percentiles_ms'),
        @('peak_working_set_bytes', 'peak_working_set_percentiles_bytes'),
        @('peak_commit_bytes', 'peak_commit_percentiles_bytes')
    )) {
        $ordered = @($result.samples | ForEach-Object { $_.($metric[0]) } | Sort-Object)
        foreach ($percentile in @(@('p50', 4), @('p95', 9), @('p99', 9))) {
            if ([Math]::Abs($result.($metric[1]).($percentile[0]) - $ordered[$percentile[1]]) -gt 0.0000001) {
                throw "$kind percentile $($percentile[0]) for $($metric[0]) does not match the samples."
            }
        }
    }
}
# Compare the post-exit OS high-water counter with an independently observed
# live working set from a process that touches a known allocation.
$info = [Diagnostics.ProcessStartInfo]::new()
$info.FileName = (Get-Process -Id $PID).Path
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
foreach ($argument in @(
    '-NoProfile',
    '-Command',
    '$data = [byte[]]::new(16777216); for ($i = 0; $i -lt $data.Length; $i += 4096) { $data[$i] = 1 }; Start-Sleep -Milliseconds 250'
)) { $info.ArgumentList.Add($argument) }
$process = [Diagnostics.Process]::Start($info)
if (-not $process) { throw 'Could not start the memory counter probe.' }
try {
    $handle = $process.Handle
    $observed = 0L
    while (-not $process.HasExited) {
        try {
            $process.Refresh()
            if (-not $process.HasExited) { $observed = [Math]::Max($observed, $process.WorkingSet64) }
        } catch [InvalidOperationException] {}
        [Threading.Thread]::Sleep(5)
    }
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) { throw 'The memory counter probe failed.' }
    $peak = [Compiler233Bench.Win32MemoryV1]::Read($handle)
    if ($observed -le 0 -or $peak[0] -lt $observed -or $peak[1] -le 0) {
        throw 'The post-exit memory counters are inconsistent with the live process.'
    }
} finally {
    $process.Dispose()
}
Write-Output 'Equivalent compiler benchmark smoke test passed; results are scoped to the validated synthetic input.'
exit 0
