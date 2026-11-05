using System.Text.Json;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

// usage: DifferentialOracle <manifest.txt> <output.json>
//        [--langversion:9.0] [--define:A;B] [--reference:<dll|dir>]...
if (args.Length < 2) throw new ArgumentException("usage: DifferentialOracle <manifest.txt> <output.json> [--langversion:<v>] [--define:<syms>] [--reference:<dll|dir>]");

var manifestPath = args[0];
var outputPath = args[1];
var languageVersion = LanguageVersion.CSharp9;
var defines = Array.Empty<string>();
var referenceArguments = new List<string>();
var syntaxOnly = false;
for (var index = 2; index < args.Length; index++)
{
    var argument = args[index];
    if (argument.Equals("--syntax-only", StringComparison.OrdinalIgnoreCase))
    {
        syntaxOnly = true;
    }
    else if (argument.StartsWith("--langversion:", StringComparison.OrdinalIgnoreCase))
    {
        languageVersion = ParseLanguageVersion(argument["--langversion:".Length..]);
    }
    else if (argument.StartsWith("--define:", StringComparison.OrdinalIgnoreCase))
    {
        defines = argument["--define:".Length..].Split([';', ','], StringSplitOptions.RemoveEmptyEntries);
    }
    else if (argument.StartsWith("--reference:", StringComparison.OrdinalIgnoreCase))
    {
        referenceArguments.Add(argument["--reference:".Length..]);
    }
    else
    {
        throw new ArgumentException("unknown oracle argument: " + argument);
    }
}

var referencePaths = new List<string>();
var trusted = (AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES") as string ?? string.Empty)
    .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
    .Where(path => path.EndsWith(".dll", StringComparison.OrdinalIgnoreCase));
referencePaths.AddRange(trusted);
foreach (var candidate in referenceArguments)
{
    if (Directory.Exists(candidate))
    {
        referencePaths.AddRange(Directory.GetFiles(candidate, "*.dll", SearchOption.TopDirectoryOnly).OrderBy(path => path, StringComparer.Ordinal));
    }
    else if (File.Exists(candidate))
    {
        referencePaths.Add(candidate);
    }
    else
    {
        throw new FileNotFoundException("reference not found: " + candidate, candidate);
    }
}

var references = new List<MetadataReference>();
var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
foreach (var path in referencePaths)
{
    var full = Path.GetFullPath(path);
    if (!seen.Add(full)) continue;
    try { references.Add(MetadataReference.CreateFromFile(full)); }
    catch { /* native or unreadable images are not managed references */ }
}

var parseOptions = new CSharpParseOptions(languageVersion, preprocessorSymbols: defines);
var compilationOptions = new CSharpCompilationOptions(
    OutputKind.DynamicallyLinkedLibrary,
    allowUnsafe: true,
    warningLevel: 4,
    concurrentBuild: true);

var files = new List<object>();
foreach (var rawLine in File.ReadLines(manifestPath))
{
    var file = rawLine.Trim();
    if (file.Length == 0) continue;
    files.Add(Analyze(file));
}

var report = new
{
    tool = "roslyn-oracle",
    roslynVersion = typeof(CSharpCompilation).Assembly.GetName().Version?.ToString(),
    languageVersion = languageVersion.ToDisplayString(),
    define = defines,
    syntaxOnly,
    referenceCount = references.Count,
    files,
};
File.WriteAllText(outputPath, JsonSerializer.Serialize(report, new JsonSerializerOptions { WriteIndented = true }));
return 0;

object Analyze(string file)
{
    try
    {
        var text = File.ReadAllText(file);
        var tree = CSharpSyntaxTree.ParseText(text, parseOptions, path: file);
        // Syntax-only mode is the fair comparison for a front end that does not
        // bind symbols yet; semantic mode is the fair comparison for csc.
        var diagnostics = syntaxOnly
            ? tree.GetDiagnostics()
            : CSharpCompilation.Create("RoslynOracleProbe", [tree], references, compilationOptions).GetDiagnostics();
        diagnostics = diagnostics
            .Where(diagnostic => diagnostic.Severity is DiagnosticSeverity.Error or DiagnosticSeverity.Warning)
            .OrderBy(diagnostic => diagnostic.Location.GetLineSpan().StartLinePosition.Line)
            .ThenBy(diagnostic => diagnostic.Location.GetLineSpan().StartLinePosition.Character)
            .ThenBy(diagnostic => diagnostic.Id, StringComparer.Ordinal)
            .ToList();
        var errors = diagnostics.Where(diagnostic => diagnostic.Severity == DiagnosticSeverity.Error).ToList();
        return new
        {
            path = file,
            accepted = errors.Count == 0,
            errorCount = errors.Count,
            warningCount = diagnostics.Count() - errors.Count,
            diagnostics = diagnostics.Take(50).Select(diagnostic =>
            {
                var span = diagnostic.Location.GetLineSpan().StartLinePosition;
                return new
                {
                    severity = diagnostic.Severity == DiagnosticSeverity.Error ? "error" : "warning",
                    code = diagnostic.Id,
                    line = span.Line + 1,
                    column = span.Character + 1,
                    message = Shorten(diagnostic.GetMessage()),
                };
            }).ToList(),
        };
    }
    catch (Exception exception)
    {
        return new
        {
            path = file,
            accepted = false,
            errorCount = 0,
            warningCount = 0,
            analysisError = exception.GetType().Name + ": " + Shorten(exception.Message),
            diagnostics = Array.Empty<object>(),
        };
    }
}

static string Shorten(string? message)
{
    var value = message ?? string.Empty;
    return value.Length <= 300 ? value : value[..300] + "…";
}

static LanguageVersion ParseLanguageVersion(string value) => value.Trim().ToLowerInvariant() switch
{
    "7.3" or "7.3.0" => LanguageVersion.CSharp7_3,
    "8" or "8.0" => LanguageVersion.CSharp8,
    "9" or "9.0" => LanguageVersion.CSharp9,
    "10" or "10.0" => LanguageVersion.CSharp10,
    "11" or "11.0" => LanguageVersion.CSharp11,
    "12" or "12.0" => LanguageVersion.CSharp12,
    "13" or "13.0" => LanguageVersion.CSharp13,
    "latest" => LanguageVersion.Latest,
    "latestmajor" => LanguageVersion.LatestMajor,
    "preview" => LanguageVersion.Preview,
    _ => throw new ArgumentException("unsupported language version for the oracle: " + value),
};
