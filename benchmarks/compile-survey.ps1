param(
    [string] $Corpus = 'D:\Code\Poko-Projects\tuanjie-project-sf\Assets',
    [string] $References = 'D:\Code\Poko-Projects\tuanjie-project-sf\Library\ScriptAssemblies',
    [int] $MaxFiles = 100
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binary = Join-Path $root 'target/release/compiler233-dotnet.exe'
$work = Join-Path $root 'benchmarks/results/survey'
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Force $work | Out-Null

$refs = @(Get-ChildItem $References -Filter *.dll | ForEach-Object { "-r:$($_.FullName)" })
$files = @(Get-ChildItem $Corpus -Recurse -File -Filter *.cs |
    Where-Object { $_.FullName -notmatch '\\(bin|obj|Library|Temp)\\' } |
    Sort-Object FullName | Select-Object -First $MaxFiles)

$ok = 0
$failures = @{}
$elapsed = [System.Diagnostics.Stopwatch]::StartNew()
$index = 0
foreach ($file in $files) {
    $index++
    $output = Join-Path $work "out-$index.dll"
    $result = & $binary csc $file @refs "-out:$output" 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { $ok++; continue }
    $message = ($result -split "`n" | Where-Object { $_ -match 'error CD233|error CS' } | Select-Object -First 1)
    if (-not $message) { $message = ($result -split "`n" | Select-Object -First 1) }
    $message = $message -replace '^.*?error (CD233\d+|CS\d+): ', ''
    $message = $message -replace "'[^']*'", "'X'"
    $message = $message.Trim()
    if ($failures.ContainsKey($message)) { $failures[$message]++ } else { $failures[$message] = 1 }
}
$elapsed.Stop()
Write-Output ("survey: {0} files | compiled {1} | failed {2} | {3:N1}s" -f $files.Count, $ok, ($files.Count - $ok), $elapsed.Elapsed.TotalSeconds)
$failures.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 15 | ForEach-Object { "  {0,4}  {1}" -f $_.Value, $_.Key }
