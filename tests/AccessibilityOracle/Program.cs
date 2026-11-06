using System.Reflection;
using System.Runtime.Loader;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

var references = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator)
    .Select(path => MetadataReference.CreateFromFile(path)).ToArray<MetadataReference>();
if (args.Length == 4 && args[0] == "--probe") {
    var probeReferences = references.Concat([MetadataReference.CreateFromFile(args[2])]);
    var probe = CSharpCompilation.Create("AccessProbe", [CSharpSyntaxTree.ParseText(File.ReadAllText(args[1]))], probeReferences,
        new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
    using var target = new MemoryStream();
    var result = probe.Emit(target);
    bool expectedSuccess = args[3] == "accept";
    if (result.Success != expectedSuccess) throw new Exception($"Unexpected Roslyn probe result: {string.Join("\n", result.Diagnostics)}");
    return 0;
}
if (args.Length == 3 && args[0] == "--execute") {
    string implementation = Path.GetFullPath(args[1]);
    var context = new AssemblyLoadContext("consumer", true);
    context.Resolving += (loader, name) => name.Name == Path.GetFileNameWithoutExtension(implementation)
        ? loader.LoadFromAssemblyPath(implementation) : null;
    var consumer = context.LoadFromAssemblyPath(Path.GetFullPath(args[2]));
    var api = context.LoadFromAssemblyPath(implementation);
    var type = api.GetType("AccessFixture.Api")!;
    var instance = Activator.CreateInstance(type)!;
    type.GetField("PublicField")!.SetValue(instance, 9);
    var result = consumer.GetType("Client")!.GetMethod("F")!.Invoke(null, [instance]);
    if (!Equals(result, 9)) throw new Exception($"Unexpected downstream result: {result}");
    Console.WriteLine("Downstream compiler233 consumer executed against implementation: 9.");
    return 0;
}
if (args.Length != 2) throw new ArgumentException("usage: AccessibilityOracle <source.cs> <implementation.dll> | --probe <source.cs> <reference.dll> <accept|reject>");
var compilation = CSharpCompilation.Create("AccessRoslyn", [CSharpSyntaxTree.ParseText(File.ReadAllText(args[0]))], references,
    new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
using var bytes = new MemoryStream();
var emit = compilation.Emit(bytes);
if (!emit.Success) throw new Exception(string.Join("\n", emit.Diagnostics));
bytes.Position = 0;
var expected = new AssemblyLoadContext("roslyn", true).LoadFromStream(bytes);
var actual = new AssemblyLoadContext("compiler233", true).LoadFromAssemblyPath(Path.GetFullPath(args[1]));
static SortedDictionary<string, int> Accessibility(Assembly assembly)
{
    const BindingFlags flags = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.Static | BindingFlags.DeclaredOnly;
    var result = new SortedDictionary<string, int>(StringComparer.Ordinal);
    foreach (var type in assembly.GetTypes()) {
        string prefix = type.FullName!;
        result[$"T {prefix}"] = (int)(type.Attributes & TypeAttributes.VisibilityMask);
        foreach (var field in type.GetFields(flags)) result[$"F {prefix}.{field.Name}"] = (int)(field.Attributes & FieldAttributes.FieldAccessMask);
        foreach (var method in type.GetMethods(flags).Cast<MethodBase>().Concat(type.GetConstructors(flags))) {
            string parameters = string.Join(",", method.GetParameters().Select(p => p.ParameterType.FullName));
            result[$"M {prefix}.{method.Name}({parameters})"] = (int)(method.Attributes & MethodAttributes.MemberAccessMask);
        }
    }
    return result;
}
var a = Accessibility(expected);
var b = Accessibility(actual);
if (!a.SequenceEqual(b)) {
    foreach (string key in a.Keys.Union(b.Keys).Order(StringComparer.Ordinal)) {
        int? x = a.TryGetValue(key, out int av) ? av : null;
        int? y = b.TryGetValue(key, out int bv) ? bv : null;
        if (x != y) Console.Error.WriteLine($"{key}: Roslyn={x} compiler233={y}");
    }
    return 1;
}
Console.WriteLine($"Compared {a.Count} type, field, constructor, method, and accessor visibility entries with Roslyn.");
return 0;
