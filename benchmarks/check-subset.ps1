$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$binaryName = if ($IsWindows) { 'compiler233-dotnet.exe' } else { 'compiler233-dotnet' }
$binary = Join-Path $root "target/release/$binaryName"
$roslyn = Join-Path $root 'benchmarks/RoslynBench/bin/Release/net10.0/RoslynBench.dll'
if (!(Test-Path $binary) -or !(Test-Path $roslyn)) { throw 'Run benchmarks/run.ps1 first to build both binaries.' }
$cases = @(
    @{ name = 'empty'; source = ''; accepted = $true; roslynIssues = $false },
    @{ name = 'class'; source = 'class C {}'; accepted = $true; roslynIssues = $false },
    @{ name = 'expressions'; source = 'class C { static int F(int a, int b) { return (a + 2) * b - 1; } }'; accepted = $true; roslynIssues = $false },
    @{ name = 'comments'; source = "/* test */ class C { // test`n static int F() { return 1; } }"; accepted = $true; roslynIssues = $false },
    @{ name = 'missing-expression'; source = 'class C { static int F() { return 1 + ; } }'; accepted = $false; roslynIssues = $true },
    @{ name = 'missing-brace'; source = 'class C { static int F() { return 1; }'; accepted = $false; roslynIssues = $true },
    @{ name = 'reserved-name'; source = 'class __arglist {}'; accepted = $false; roslynIssues = $true },
    @{ name = 'large-integer'; source = 'class C { static int F() { return 18446744073709551616; } }'; accepted = $false; roslynIssues = $true },
    @{ name = 'unsupported-public'; source = 'public class C {}'; accepted = $false; roslynIssues = $false },
    @{ name = 'unsupported-unicode'; source = 'class C { static int F() { return 1; } } // 中文'; accepted = $false; roslynIssues = $false }
)
$tempFile = [System.IO.Path]::GetTempFileName()
try {
    foreach ($case in $cases) {
        [System.IO.File]::WriteAllText($tempFile, $case.source, [System.Text.UTF8Encoding]::new($false))
        $null = & $binary check-subset $tempFile 2>&1
        $actualAccepted = $LASTEXITCODE -eq 0
        $reference = & dotnet $roslyn $tempFile 1 0 | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw "Roslyn process failed for $($case.name)" }
        if ($actualAccepted -ne $case.accepted -or $reference.has_parse_issues -ne $case.roslynIssues) {
            throw "Unexpected result for $($case.name): subset=$actualAccepted Roslyn issues=$($reference.has_parse_issues)"
        }
    }
    Write-Output "Subset differential smoke tests passed: $($cases.Count) cases."
} finally { Remove-Item -LiteralPath $tempFile -Force }
