using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis;

if ((args.Length != 3 && (args.Length != 4 || args[3] != "compile")) || !int.TryParse(args[1], out int iterations) || iterations <= 0 || !int.TryParse(args[2], out int warmup) || warmup < 0)
{
    Console.Error.WriteLine("usage: RoslynBench <file.cs> <positive iterations> <non-negative warmup> [compile]");
    return 2;
}
byte[] bytes = File.ReadAllBytes(args[0]);
string source = new UTF8Encoding(false, true).GetString(bytes);
var options = CSharpParseOptions.Default;
if (args.Length == 4)
{
    var references = new[] { MetadataReference.CreateFromFile(typeof(object).Assembly.Location) };
    var compilationOptions = new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary, optimizationLevel: OptimizationLevel.Release);
    MemoryStream Compile()
    {
        var tree = CSharpSyntaxTree.ParseText(source, options);
        var compilation = CSharpCompilation.Create("Benchmark", [tree], references, compilationOptions);
        var result = new MemoryStream();
        var emitted = compilation.Emit(result);
        if (!emitted.Success) throw new InvalidOperationException(string.Join("\n", emitted.Diagnostics));
        return result;
    }
    using var probe = Compile();
    for (int i = 0; i < warmup; i++) using (Compile()) { }
    var timings = new long[iterations];
    for (int i = 0; i < iterations; i++)
    {
        long start = Stopwatch.GetTimestamp();
        using var assembly = Compile();
        GC.KeepAlive(assembly);
        timings[i] = (long)Stopwatch.GetElapsedTime(start).TotalNanoseconds;
    }
    Array.Sort(timings);
    long P(int p) => timings[((timings.Length * p + 99) / 100) - 1];
    Console.WriteLine(JsonSerializer.Serialize(new {
        engine = "Roslyn " + typeof(CSharpSyntaxTree).Assembly.GetName().Version,
        stage = "parse, bind, emit", source_sha256 = Convert.ToHexStringLower(SHA256.HashData(bytes)),
        bytes = bytes.Length, iterations, warmup, has_parse_issues = false, output_bytes = probe.Length,
        p50_ns = P(50), p95_ns = P(95), samples_ns = timings
    }));
    return 0;
}
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
