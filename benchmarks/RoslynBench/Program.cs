using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Microsoft.CodeAnalysis.CSharp;

if (args.Length != 3 || !int.TryParse(args[1], out int iterations) || iterations <= 0 || !int.TryParse(args[2], out int warmup) || warmup < 0)
{
    Console.Error.WriteLine("usage: RoslynBench <file.cs> <positive iterations> <non-negative warmup>");
    return 2;
}
byte[] bytes = File.ReadAllBytes(args[0]);
string source = new UTF8Encoding(false, true).GetString(bytes);
var options = CSharpParseOptions.Default;
bool hasError = CSharpSyntaxTree.ParseText(source, options).GetRoot().ContainsDiagnostics;
for (int i = 0; i < warmup; i++) _ = CSharpSyntaxTree.ParseText(source, options).GetRoot().ContainsDiagnostics;
var samples = new long[iterations];
for (int i = 0; i < iterations; i++)
{
    long start = Stopwatch.GetTimestamp();
    var root = CSharpSyntaxTree.ParseText(source, options).GetRoot();
    bool diagnostic = root.ContainsDiagnostics;
    GC.KeepAlive(root);
    samples[i] = (long)Stopwatch.GetElapsedTime(start).TotalNanoseconds;
    GC.KeepAlive(diagnostic);
}
Array.Sort(samples);
long Percentile(int p) => samples[((samples.Length * p + 99) / 100) - 1];
Console.WriteLine(JsonSerializer.Serialize(new {
    engine = "Roslyn " + typeof(CSharpSyntaxTree).Assembly.GetName().Version,
    source_sha256 = Convert.ToHexStringLower(SHA256.HashData(bytes)),
    bytes = bytes.Length, iterations, warmup, has_parse_issues = hasError,
    p50_ns = Percentile(50), p95_ns = Percentile(95), samples_ns = samples
}));
return 0;
