param(
    [Parameter(Mandatory)][string]$Output,
    [ValidateRange(1, 100000)][int]$Methods = 200
)
$builder = [System.Text.StringBuilder]::new()
$fullOutput = [System.IO.Path]::GetFullPath($Output)
[System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($fullOutput)) | Out-Null
[void]$builder.Append("class Generated {`n")
for ($i = 0; $i -lt $Methods; $i++) {
    [void]$builder.Append("    static int Method$i(int a, int b) { return (a + b) * 10 - 3; }`n")
}
[void]$builder.Append("}`n")
[System.IO.File]::WriteAllText($fullOutput, $builder.ToString(), [System.Text.UTF8Encoding]::new($false))
