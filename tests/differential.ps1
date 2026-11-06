param(
    [string[]] $Corpus = @(),
    [string] $OutputDirectory = '',
    [ValidateSet('frontend', 'compile')] [string] $Mode = 'frontend',
    [string] $LangVersion = '9.0',
    [string[]] $Define = @(),
    [string[]] $References = @(),
    [int] $MaxFiles = 0,
    [int] $MaxFalseAccepts = 0,
    [switch] $KeepWork
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
if ($Corpus.Count -eq 0) { $Corpus = @(Join-Path $root 'tests/fixtures/differential') }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $root 'benchmarks/results/differential' }

$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$buildOutput = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Rust build failed: $buildOutput" }

$oracleProject = Join-Path $root 'tests/DifferentialOracle/DifferentialOracle.csproj'
$oracleBuild = & dotnet build $oracleProject -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Oracle build failed: $oracleBuild" }
$oracle = Join-Path $root 'tests/DifferentialOracle/bin/Release/net10.0/DifferentialOracle.dll'

$files = [System.Collections.Generic.List[string]]::new()
foreach ($entry in $Corpus) {
    $resolved = Resolve-Path $entry
    if (Test-Path $resolved -PathType Leaf) {
        $files.Add($resolved.Path)
        continue
    }
    Get-ChildItem $resolved -Recurse -File -Filter *.cs |
        Where-Object { $_.FullName -notmatch '[\\/](bin|obj|Library|Temp|\.git)[\\/]' } |
        ForEach-Object { $files.Add($_.FullName) }
}
$unique = @($files | Sort-Object -Unique)
if ($MaxFiles -gt 0 -and $unique.Count -gt $MaxFiles) { $unique = $unique[0..($MaxFiles - 1)] }
if ($unique.Count -eq 0) { throw 'no C# files found in the corpus' }

New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$outputRoot = (Resolve-Path -LiteralPath $OutputDirectory).Path
$work = Join-Path $outputRoot ('.work-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$utf8 = [System.Text.UTF8Encoding]::new($false)

$manifest = Join-Path $work 'manifest.txt'
[System.IO.File]::WriteAllLines($manifest, $unique, $utf8)
$roslynReportPath = Join-Path $OutputDirectory 'roslyn.json'
$oracleArguments = @($manifest, $roslynReportPath, "--langversion:$LangVersion")
if ($Mode -eq 'frontend') { $oracleArguments += '--syntax-only' }
if ($Define.Count -gt 0) { $oracleArguments += "--define:$($Define -join ';')" }
foreach ($reference in $References) { $oracleArguments += "--reference:$reference" }
& dotnet $oracle @oracleArguments
if ($LASTEXITCODE -ne 0) { throw 'Roslyn oracle failed' }

$roslynReport = Get-Content $roslynReportPath -Raw | ConvertFrom-Json
$roslynByPath = @{}
foreach ($item in $roslynReport.files) { $roslynByPath[$item.path.ToLowerInvariant()] = $item }

$oursByPath = @{}
if ($Mode -eq 'frontend') {
    $frontendReportPath = Join-Path $work 'frontend.json'
    $frontendArguments = @('frontend', $manifest, "-report:$frontendReportPath")
    foreach ($symbol in $Define) { $frontendArguments += "-define:$symbol" }
    $frontendOutput = & $binary @frontendArguments 2>&1 | Out-String
    if ($LASTEXITCODE -notin @(0, 1)) { throw "frontend failed with exit code $LASTEXITCODE`: $frontendOutput" }
    if (-not (Test-Path $frontendReportPath)) { throw "frontend run produced no report: $frontendOutput" }
    $frontendReport = Get-Content $frontendReportPath -Raw | ConvertFrom-Json
    foreach ($item in $frontendReport.files) { $oursByPath[$item.path.ToLowerInvariant()] = $item }
}
else {
    $outputPath = Join-Path $work 'out.dll'
    $reportPath = Join-Path $work 'report.json'
    for ($index = 0; $index -lt $unique.Count; $index++) {
        $file = $unique[$index]
        Remove-Item $reportPath -Force -ErrorAction SilentlyContinue
        $arguments = @('csc', $file, "-out:$outputPath", "-report:$reportPath", "-langversion:$LangVersion")
        foreach ($symbol in $Define) { $arguments += "-define:$symbol" }
        $compilerOutput = & $binary @arguments 2>&1 | Out-String
        $exitCode = $LASTEXITCODE
        if ($exitCode -notin @(0, 1)) { throw "compiler failed unexpectedly for $file with exit code $exitCode`: $compilerOutput" }
        $compilerReport = $null
        if (Test-Path $reportPath) { $compilerReport = Get-Content $reportPath -Raw | ConvertFrom-Json }
        if ($exitCode -eq 0 -and $null -eq $compilerReport) { throw "compiler reported success without a report: $file" }
        $firstError = $null
        if ($null -ne $compilerReport) {
            $first = @($compilerReport.diagnostics | Where-Object { $_.severity -eq 'error' } | Select-Object -First 1)
            if ($first.Count -gt 0) { $firstError = $first[0] }
        }
        $oursByPath[$file.ToLowerInvariant()] = [pscustomobject]@{
            path = $file
            ok = ($exitCode -eq 0)
            error = $firstError
            output = if ($exitCode -eq 0) { '' } else { $compilerOutput.Trim() }
        }
    }
    Remove-Item $outputPath -Force -ErrorAction SilentlyContinue
}

$results = [System.Collections.Generic.List[object]]::new()
$counts = @{ accept = 0; reject = 0; false_reject = 0; false_accept = 0 }
$roslynAccepted = 0
$compilerAccepted = 0
foreach ($file in $unique) {
    $key = $file.ToLowerInvariant()
    $oracleRecord = $roslynByPath[$key]
    if ($null -eq $oracleRecord) { throw "oracle did not analyze $file" }
    $ours = $oursByPath[$key]
    if ($null -eq $ours) { throw "compiler did not analyze $file" }

    $roslynOk = [bool]$oracleRecord.accepted
    $oursOk = [bool]$ours.ok
    $classification = if ($oursOk -and $roslynOk) { 'accept' }
        elseif ($oursOk) { 'false_accept' }
        elseif ($roslynOk) { 'false_reject' }
        else { 'reject' }
    $counts[$classification]++
    if ($roslynOk) { $roslynAccepted++ }
    if ($oursOk) { $compilerAccepted++ }

    $roslynFirst = @($oracleRecord.diagnostics | Where-Object { $_.severity -eq 'error' } | Select-Object -First 1)
    $results.Add([pscustomobject]@{
        path = $file
        classification = $classification
        roslynAccepted = $roslynOk
        compilerAccepted = $oursOk
        roslynFirstError = if ($roslynFirst.Count -gt 0) { $roslynFirst[0] } else { $null }
        compilerFirstError = $ours.error
        compilerOutput = $ours.output
    })
}

$summary = [pscustomobject]@{
    tool = 'compiler233-dotnet-differential'
    mode = $Mode
    languageVersion = $LangVersion
    define = $Define
    referenceCount = $References.Count
    roslynVersion = $roslynReport.roslynVersion
    roslynSyntaxOnly = [bool]$roslynReport.syntaxOnly
    fileCount = $unique.Count
    accept = $counts.accept
    reject = $counts.reject
    falseReject = $counts.false_reject
    falseAccept = $counts.false_accept
    roslynAccepted = $roslynAccepted
    compilerAccepted = $compilerAccepted
    compilerAcceptRate = [math]::Round($compilerAccepted / $unique.Count, 4)
    roslynAcceptRate = [math]::Round($roslynAccepted / $unique.Count, 4)
    files = $results
}
$summaryPath = Join-Path $OutputDirectory 'differential.json'
[System.IO.File]::WriteAllText($summaryPath, ($summary | ConvertTo-Json -Depth 8), $utf8)

Write-Output ("Differential [{0}]: {1} files | accept {2} | reject {3} | false_reject {4} | false_accept {5}" -f `
    $Mode, $unique.Count, $counts.accept, $counts.reject, $counts.false_reject, $counts.false_accept)
Write-Output ("Roslyn accepted {0}/{1}; compiler233 accepted {2}/{1}" -f $roslynAccepted, $unique.Count, $compilerAccepted)
Write-Output "Details: $summaryPath"

if (-not $KeepWork) {
    $resolvedWork = (Resolve-Path -LiteralPath $work).Path
    $comparison = if ($IsWindows) { [StringComparison]::OrdinalIgnoreCase } else { [StringComparison]::Ordinal }
    $prefix = if ([IO.Path]::EndsInDirectorySeparator($outputRoot)) { $outputRoot } else { $outputRoot + [IO.Path]::DirectorySeparatorChar }
    if (-not $resolvedWork.StartsWith($prefix, $comparison)) { throw 'Work directory escaped output directory' }
    Remove-Item -LiteralPath $resolvedWork -Recurse -Force
}

$falseAccepts = @($results | Where-Object { $_.classification -eq 'false_accept' })
if ($falseAccepts.Count -gt 0) {
    Write-Output 'False accepts (compiler233 accepted what Roslyn rejected):'
    $falseAccepts | Select-Object -First 10 | ForEach-Object { Write-Output ("  {0}" -f $_.path) }
}
if ($falseAccepts.Count -gt $MaxFalseAccepts) {
    throw "false accepts exceeded the limit: $($falseAccepts.Count) > $MaxFalseAccepts"
}
exit 0
