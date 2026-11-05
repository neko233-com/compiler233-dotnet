using System.Reflection;
using System.Runtime.Loader;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

// usage: CompileOracle <manifest.txt> <compiler-output.dll> <typeFullName> <method>
// Compiles the same sources with Roslyn, runs both implementations and compares
// the returned value. A mismatch means the emitted IL does not behave like csc.
if (args.Length != 4) throw new ArgumentException("usage: CompileOracle <manifest.txt> <assembly.dll> <type> <method>");

var manifestPath = args[0];
var assemblyPath = args[1];
var typeName = args[2];
var methodName = args[3];

var sources = File.ReadAllLines(manifestPath)
    .Select(line => line.Trim())
    .Where(line => line.Length > 0)
    .Select(path => CSharpSyntaxTree.ParseText(File.ReadAllText(path), new CSharpParseOptions(LanguageVersion.CSharp9), path: path))
    .ToArray();

var references = ((string?)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES") ?? string.Empty)
    .Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
    .Where(path => path.EndsWith(".dll", StringComparison.OrdinalIgnoreCase))
    .Select(path => MetadataReference.CreateFromFile(path))
    .ToArray<MetadataReference>();

var compilation = CSharpCompilation.Create(
    "RoslynReference",
    sources,
    references,
    new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary, allowUnsafe: true));

using var stream = new MemoryStream();
var emit = compilation.Emit(stream);
if (!emit.Success)
{
    Console.Error.WriteLine("Roslyn rejected the sources:");
    foreach (var diagnostic in emit.Diagnostics.Where(item => item.Severity == DiagnosticSeverity.Error))
    {
        Console.Error.WriteLine("  " + diagnostic);
    }
    return 2;
}
stream.Position = 0;

var expected = Invoke(
    new AssemblyLoadContext("roslyn", true).LoadFromStream(stream),
    typeName,
    methodName);
var actual = Invoke(
    LoadWithDependencies(assemblyPath),
    typeName,
    methodName);

Console.WriteLine($"{typeName}.{methodName}(): Roslyn={expected}, compiler233={actual}");
if (expected != actual)
{
    Console.Error.WriteLine("Result mismatch");
    return 1;
}
return 0;

static Assembly LoadWithDependencies(string path)
{
    var full = Path.GetFullPath(path);
    var directory = Path.GetDirectoryName(full)!;
    var context = new AssemblyLoadContext("compiler233", true);
    context.Resolving += (loader, name) =>
    {
        var candidate = Path.Combine(directory, name.Name + ".dll");
        return File.Exists(candidate) ? loader.LoadFromAssemblyPath(candidate) : null;
    };
    return context.LoadFromAssemblyPath(full);
}

static string Invoke(Assembly assembly, string typeName, string methodName)
{
    var type = assembly.GetType(typeName, throwOnError: true)!;
    var method = type.GetMethod(methodName, BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic)
        ?? throw new MissingMethodException(typeName, methodName);
    try
    {
        var value = method.Invoke(null, null);
        return value is null ? "<null>" : value.ToString() ?? "<empty>";
    }
    catch (TargetInvocationException error)
    {
        return "exception:" + error.InnerException?.GetType().FullName;
    }
}
