using System.Reflection;
using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;
using System.Runtime.Loader;
using System.Text;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

if (args.Length != 4) throw new ArgumentException("usage: ReferenceOracle <implementation.dll> <reference.dll> <source.cs> <consumer.dll>");
string implementation = Path.GetFullPath(args[0]);
string reference = Path.GetFullPath(args[1]);
byte[] marker = Encoding.Unicode.GetBytes("IMPLEMENTATION_ONLY_4A27");
if (File.ReadAllBytes(implementation).AsSpan().IndexOf(marker) < 0) throw new Exception("Implementation fixture does not contain the marker");
if (File.ReadAllBytes(reference).AsSpan().IndexOf(marker) >= 0) throw new Exception("Reference PE leaks implementation-only strings");

using (var implementationPe = new PEReader(File.OpenRead(implementation)))
using (var referencePe = new PEReader(File.OpenRead(reference)))
{
    var impl = implementationPe.GetMetadataReader();
    var metadata = referencePe.GetMetadataReader();
    if (impl.GetString(impl.GetAssemblyDefinition().Name) != metadata.GetString(metadata.GetAssemblyDefinition().Name))
        throw new Exception("Reference and implementation assembly identities differ");
    bool isReference = metadata.GetAssemblyDefinition().GetCustomAttributes().Any(handle => {
        var constructor = metadata.GetCustomAttribute(handle).Constructor;
        if (constructor.Kind != HandleKind.MemberReference) return false;
        var parent = metadata.GetMemberReference((MemberReferenceHandle)constructor).Parent;
        if (parent.Kind != HandleKind.TypeReference) return false;
        var type = metadata.GetTypeReference((TypeReferenceHandle)parent);
        return metadata.GetString(type.Namespace) == "System.Runtime.CompilerServices" && metadata.GetString(type.Name) == "ReferenceAssemblyAttribute";
    });
    if (!isReference) throw new Exception("ReferenceAssemblyAttribute is missing");
    int bodies = 0;
    foreach (var handle in metadata.MethodDefinitions)
    {
        var method = metadata.GetMethodDefinition(handle);
        if (method.RelativeVirtualAddress == 0) continue;
        byte[] il = referencePe.GetMethodBody(method.RelativeVirtualAddress).GetILBytes()!;
        if (!il.SequenceEqual(new byte[] { 0x14, 0x7a })) throw new Exception("A method contains an implementation body");
        bodies++;
    }
    if (bodies == 0) throw new Exception("Expected reference method stubs");
    if (metadata.FieldDefinitions.Any(handle => (metadata.GetFieldDefinition(handle).Attributes & FieldAttributes.FieldAccessMask) == FieldAttributes.Private))
        throw new Exception("Reference class retains a private field");
    foreach (var typeHandle in metadata.TypeDefinitions)
    {
        var type = metadata.GetTypeDefinition(typeHandle);
        if (metadata.GetString(type.Name) != "Counter") continue;
        var properties = type.GetProperties().Select(handle => metadata.GetString(metadata.GetPropertyDefinition(handle).Name)).ToArray();
        if (!properties.Contains("Value")) throw new Exception("Value property is missing from reference metadata");
    }
}

try
{
    new AssemblyLoadContext("cannot-execute", true).LoadFromAssemblyPath(reference);
    throw new Exception("Reference assembly unexpectedly loaded for execution");
}
catch (BadImageFormatException) { }

var platform = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator)
    .Select(path => MetadataReference.CreateFromFile(path)).ToList<MetadataReference>();
var referenceMetadata = MetadataReference.CreateFromFile(reference);
var apiSource = CSharpSyntaxTree.ParseText(File.ReadAllText(args[2]));
var apiCompilation = CSharpCompilation.Create("ApiSource", [apiSource], platform,
    new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
platform.Add(referenceMetadata);
var source = CSharpSyntaxTree.ParseText("using Demo; public static class Consumer { public static int Run() { var c = new Counter(); c.Values = new int[] { 21 }; c.Value = c.Values[0]; Counter.Offset = 1; ICounter i = c; return i.Twice(c.Value) + Counter.Offset; } }");
var compilation = CSharpCompilation.Create("Consumer", [source], platform,
    new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
var referenceSymbols = (IAssemblySymbol)compilation.GetAssemblyOrModuleSymbol(referenceMetadata)!;
static string[] PublicMembers(INamedTypeSymbol type) => type.GetMembers()
    .Where(member => member.DeclaredAccessibility == Accessibility.Public)
    .Where(member => member is not IMethodSymbol method || method.MethodKind is not (MethodKind.PropertyGet or MethodKind.PropertySet))
    .Select(member => member.ToDisplayString(SymbolDisplayFormat.CSharpErrorMessageFormat))
    .Order(StringComparer.Ordinal).ToArray();
foreach (string name in new[] { "Demo.Counter", "Demo.ICounter" })
{
    var expected = PublicMembers(apiCompilation.GetTypeByMetadataName(name)!);
    var observed = PublicMembers(referenceSymbols.GetTypeByMetadataName(name)!);
    if (!expected.SequenceEqual(observed)) throw new Exception($"Public API mismatch for {name}: Roslyn=[{string.Join(", ", expected)}], reference=[{string.Join(", ", observed)}]");
}
using var result = new MemoryStream();
var emitted = compilation.Emit(result);
if (!emitted.Success) throw new Exception(string.Join("\n", emitted.Diagnostics));
result.Position = 0;
var context = new AssemblyLoadContext("consumer", true);
context.Resolving += (loader, name) => name.Name == Path.GetFileNameWithoutExtension(implementation)
    ? loader.LoadFromAssemblyPath(implementation) : null;
var consumer = context.LoadFromStream(result);
var actual = consumer.GetType("Consumer")!.GetMethod("Run")!.Invoke(null, null);
if (!Equals(actual, 43)) throw new Exception($"Unexpected consumer result: {actual}");
var actualApi = context.Assemblies.Single(assembly => assembly.GetName().Name == Path.GetFileNameWithoutExtension(implementation));
var instance = Activator.CreateInstance(actualApi.GetType("Demo.Counter")!)!;
var rustConsumer = context.LoadFromAssemblyPath(Path.GetFullPath(args[3]));
var rustActual = rustConsumer.GetType("Client.Runner")!.GetMethod("Run")!.Invoke(null, [instance]);
if (!Equals(rustActual, 42)) throw new Exception($"Unexpected compiler233 consumer result: {rustActual}");
Console.WriteLine("Reference marker, IL stubs, Roslyn public API and both consumer runtime results verified (43, 42).");
