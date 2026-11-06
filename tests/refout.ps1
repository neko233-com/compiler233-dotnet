$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$build = & cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw "Compiler build failed: $build" }
$build = & dotnet build (Join-Path $root 'tests/ReferenceOracle/ReferenceOracle.csproj') -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Reference oracle build failed: $build" }
$work = Join-Path (Join-Path $root 'benchmarks/results/refout') ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $work | Out-Null
$source = Join-Path $root 'tests/fixtures/refout/Api.cs'
$output = Join-Path $work 'Api.dll'
$reference = Join-Path (Join-Path $work 'ref') 'Api.dll'
& $binary csc $source "-out:$output" "-refout:$reference"
if ($LASTEXITCODE -ne 0) { throw 'Compilation with -refout failed' }
$consumerSource = Join-Path $root 'tests/fixtures/refout/Consumer.cs'
$consumerOutput = Join-Path $work 'RustConsumer.dll'
& $binary csc $consumerSource "-r:$reference" "-out:$consumerOutput"
if ($LASTEXITCODE -ne 0) { throw 'Compiler failed to consume its own reference assembly' }
$oracle = Join-Path $root 'tests/ReferenceOracle/bin/Release/net10.0/ReferenceOracle.dll'
& dotnet $oracle $output $reference $source $consumerOutput
if ($LASTEXITCODE -ne 0) { throw 'Reference assembly verification failed' }
$foreignProject = Join-Path $root 'tests/fixtures/refout/ForeignLib.csproj'
$foreignBuild = & dotnet build $foreignProject -c Release --nologo -v quiet
if ($LASTEXITCODE -ne 0) { throw "Foreign fixture build failed: $foreignBuild" }
$foreign = Join-Path $root 'tests/fixtures/refout/bin/Release/net10.0/ForeignLib.dll'
$badConsumer = Join-Path $work 'BadConsumer.cs'
$consumerHash = (Get-FileHash -Algorithm SHA256 $consumerOutput).Hash
foreach ($invalidConsumer in @(
    @{ source = 'class C { static void M(Foreign.Locked x) { x.Value = 1; } }'; expected = 'no accessible instance setter' },
    @{ source = 'class C { static void M(Foreign.MethodOnly x) { x.Value = 1; } }'; expected = 'no accessible instance setter' },
    @{ source = 'class C { static void M(Demo.Counter x) { x.Value = "wrong"; } }'; expected = "assignment to 'Value' expects Int" }
)) {
    [System.IO.File]::WriteAllText($badConsumer, $invalidConsumer.source)
    $result = & $binary csc $badConsumer "-r:$foreign" "-r:$reference" "-out:$consumerOutput" 2>&1 | Out-String
    if ($LASTEXITCODE -ne 1 -or -not $result.Contains($invalidConsumer.expected)) { throw "Invalid property was not rejected as expected: $result" }
    if ((Get-FileHash -Algorithm SHA256 $consumerOutput).Hash -ne $consumerHash) { throw 'Invalid consumer modified the previous output' }
}
$before = (Get-FileHash -Algorithm SHA256 $output).Hash
$refBefore = (Get-FileHash -Algorithm SHA256 $reference).Hash
$invalid = Join-Path $work 'Invalid.cs'
[System.IO.File]::WriteAllText($invalid, 'class C { static int F() { return missing; } }')
$null = & $binary csc $invalid "-out:$output" "-refout:$reference" 2>&1
if ($LASTEXITCODE -ne 1) { throw 'Invalid source unexpectedly compiled' }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $refBefore) { throw 'Invalid source modified an artifact' }
$directory = Join-Path $work 'not-a-file'
New-Item -ItemType Directory $directory | Out-Null
$null = & $binary csc $source "-out:$output" "-refout:$directory" 2>&1
if ($LASTEXITCODE -ne 1) { throw 'Invalid reference output unexpectedly succeeded' }
if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before) { throw 'Reference output failure modified the implementation assembly' }
if ($IsWindows) {
    # Block replacement after both temporary artifacts have been written.
    $locked = [System.IO.File]::Open($reference, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::Read)
    try {
        $null = & $binary csc $source "-out:$output" "-refout:$reference" 2>&1
        if ($LASTEXITCODE -ne 1) { throw 'Locked reference output unexpectedly succeeded' }
        $newOutput = Join-Path $work 'New.dll'
        $null = & $binary csc $source "-out:$newOutput" "-refout:$reference" 2>&1
        if ($LASTEXITCODE -ne 1 -or (Test-Path $newOutput)) { throw 'Failed reference replacement left a new implementation output' }
    } finally { $locked.Dispose() }
    if ((Get-FileHash -Algorithm SHA256 $output).Hash -ne $before -or (Get-FileHash -Algorithm SHA256 $reference).Hash -ne $refBefore) { throw 'Failed second replacement did not restore both outputs' }
}
Write-Output 'Reference output failure preservation checks passed.'
exit 0
