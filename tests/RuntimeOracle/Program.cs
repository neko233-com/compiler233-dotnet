using System.Reflection;
using System.Runtime.Loader;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

if (args.Length != 2) throw new ArgumentException("usage: RuntimeOracle <source.cs> <compiler-output.dll|--diagnose>");
var syntax = CSharpSyntaxTree.ParseText(File.ReadAllText(args[0]));
var compilation = CSharpCompilation.Create("RoslynOracle", [syntax],
    [MetadataReference.CreateFromFile(typeof(object).Assembly.Location)],
    new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
using var stream = new MemoryStream();
var emit = compilation.Emit(stream);
if (args[1] == "--diagnose") return emit.Success ? 0 : 1;
if (!emit.Success) throw new Exception(string.Join("\n", emit.Diagnostics));
stream.Position = 0;
var reference = new AssemblyLoadContext("reference", true).LoadFromStream(stream);
var actual = new AssemblyLoadContext("actual", true).LoadFromAssemblyPath(Path.GetFullPath(args[1]));

bool generated = syntax.GetRoot().ToString().Contains("class Generated", StringComparison.Ordinal);
string className = generated ? "Generated" : "Arithmetic";
string Run(Assembly assembly, string method, params object[] arguments)
{
    var type = assembly.GetType(className, throwOnError: true)!;
    if (type.IsPublic || type.GetConstructor(Type.EmptyTypes) is null) throw new Exception("type or default constructor visibility mismatch");
    var candidate = type.GetMethods(BindingFlags.Static | BindingFlags.NonPublic).Single(m => m.Name == method && m.GetParameters().Length == arguments.Length);
    if (!candidate.IsPrivate || candidate.ReturnType != typeof(int)) throw new Exception("method metadata mismatch");
    try { return "value:" + candidate.Invoke(null, arguments); }
    catch (TargetInvocationException error) { return "exception:" + error.InnerException!.GetType().FullName; }
}

var cases = generated ? Enumerable.Range(0, 200).Select(i => ($"Method{i}", new object[] { i, 5 })).ToArray() : new (string Name, object[] Args)[] {
    ("Add", [2, 3]), ("Add", [int.MaxValue, 1]), ("Scale", [2, 3]),
    ("Divide", [-7, 2]), ("Divide", [5, -1]), ("Divide", [int.MinValue, -2]), ("DivByConstantZero", [5]), ("Constant", []),
    ("Overload", [2]), ("Overload", [2, 5]),
};
foreach (var test in cases)
{
    var expected = Run(reference, test.Item1, test.Item2);
    var observed = Run(actual, test.Item1, test.Item2);
    if (expected != observed) throw new Exception($"{test.Item1}: Roslyn={expected}, compiler233={observed}");
    if (!generated) Console.WriteLine($"{test.Item1}: {observed}");
}
if (generated) Console.WriteLine($"Generated corpus: {cases.Length} runtime results match Roslyn.");
return 0;
