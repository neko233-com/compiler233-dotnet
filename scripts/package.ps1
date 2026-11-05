param(
    [string] $Version = '',
    [string] $Target = ''
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

$metadata = & cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
$package = $metadata.packages | Where-Object { $_.name -eq 'compiler233-dotnet' }
if (-not $package) { throw 'compiler233-dotnet package not found in cargo metadata' }
if (-not $Version) { $Version = $package.version }

$hostTarget = (& rustc -vV | Select-String -Pattern '^host: ').ToString().Split(' ')[1]
if (-not $Target) { $Target = $hostTarget }
if ($Target -eq 'x86_64-apple-darwin') { throw 'macOS Intel (x86_64-apple-darwin) is not supported' }

$platform = switch -Wildcard ($Target) {
    'x86_64-pc-windows-*' { 'win-x64' }
    'aarch64-pc-windows-*' { 'win-arm64' }
    'x86_64-unknown-linux-gnu' { 'linux-x64' }
    'aarch64-unknown-linux-gnu' { 'linux-arm64' }
    'aarch64-apple-darwin' { 'macos-arm64' }
    default { throw "unsupported release target: $Target" }
}

$buildArguments = @('build', '--release', '--locked')
if ($Target -ne $hostTarget) { $buildArguments += @('--target', $Target) }
& cargo @buildArguments
if ($LASTEXITCODE -ne 0) { throw "cargo build failed for $Target" }

$binaryName = if ($Target -like '*windows*') { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$releaseDirectory = if ($Target -ne $hostTarget) { Join-Path $root "target/$Target/release" } else { Join-Path $root 'target/release' }
$binary = Join-Path $releaseDirectory $binaryName
if (-not (Test-Path $binary)) { throw "missing build output: $binary" }

$name = "compiler233-dotnet-$Version-$platform"
$artifacts = Join-Path $root 'artifacts'
$stagingRoot = Join-Path $artifacts 'staging'
$staging = Join-Path $stagingRoot $name
if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Force $staging | Out-Null
Copy-Item $binary $staging
foreach ($document in @('README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md')) {
    Copy-Item (Join-Path $root $document) $staging
}

New-Item -ItemType Directory -Force $artifacts | Out-Null
$extension = if ($IsWindows) { '.zip' } else { '.tar.gz' }
$archive = Join-Path $artifacts ($name + $extension)
if (Test-Path $archive) { Remove-Item $archive -Force }
if ($IsWindows) {
    Compress-Archive -Path $staging -DestinationPath $archive
} else {
    & tar -czf $archive -C $stagingRoot $name
    if ($LASTEXITCODE -ne 0) { throw "tar failed for $Target" }
}

$hash = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
$checksum = $archive + '.sha256'
[System.IO.File]::WriteAllText($checksum, "$hash  $(Split-Path $archive -Leaf)`n", [System.Text.UTF8Encoding]::new($false))

Write-Output $archive
Write-Output $checksum
