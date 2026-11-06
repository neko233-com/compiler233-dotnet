$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$name = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$name"
$build = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Compiler build failed: $build" }
$work = Join-Path (Join-Path $root 'benchmarks/results/output-safety') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $work 'Input.cs'
[System.IO.File]::WriteAllText($source, 'class Input { static int F() { return 1; } }')
$sourceHash = (Get-FileHash -Algorithm SHA256 $source).Hash
$out = Join-Path $work 'out.dll'
function Assert-Rejected([string[]] $arguments) {
    $result = & $binary @arguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 1 -or $result -notmatch 'CD2330022') { throw "Path conflict was not rejected: $result" }
}
Assert-Rejected @('csc', $source, "-out:$source")
Assert-Rejected @('csc', $source, "-out:$(Join-Path $work './Input.cs')")
Assert-Rejected @('csc', $source, "-out:$out", "-report:$source")
Assert-Rejected @('csc', $source, "-out:$out", "-refout:$source")
if ((Get-FileHash -Algorithm SHA256 $source).Hash -ne $sourceHash) { throw 'Source was overwritten' }
Assert-Rejected @('csc', $source, "-out:$out", "-report:$out")
if (Test-Path $out) { throw 'Conflicting output/report unexpectedly created an artifact' }
$reference = Join-Path $work 'reference.dll'
[System.IO.File]::WriteAllText($reference, 'sentinel')
$referenceHash = (Get-FileHash -Algorithm SHA256 $reference).Hash
Assert-Rejected @('csc', $source, "-r:$reference", "-out:$reference")
Assert-Rejected @('csc', $source, "-r:$reference", "-out:$out", "-report:$reference")
if ((Get-FileHash -Algorithm SHA256 $reference).Hash -ne $referenceHash) { throw 'Reference was overwritten' }
$response = Join-Path $work 'arguments.rsp'
[System.IO.File]::WriteAllText($response, "`"$source`"`n")
$responseHash = (Get-FileHash -Algorithm SHA256 $response).Hash
Assert-Rejected @('csc', "@$response", "-out:$response")
Assert-Rejected @('csc', "@$response", "-out:$out", "-report:$response")
if ((Get-FileHash -Algorithm SHA256 $response).Hash -ne $responseHash) { throw 'Response file was overwritten' }
foreach ($option in @('-checked+', '-debug:portable', '-deterministic+', '-warnaserror+', '-analyzer:missing.dll', '-errorlog:errors.json', '-doc:api.xml')) {
    $result = & $binary csc $source "-out:$out" $option 2>&1 | Out-String
    if ($LASTEXITCODE -ne 1 -or $result -notmatch 'CD2330002') { throw "Unsupported option $option was silently accepted: $result" }
    if (Test-Path $out) { throw "Unsupported option $option produced an artifact" }
}
Write-Output 'Output, report and input path collision checks passed.'
