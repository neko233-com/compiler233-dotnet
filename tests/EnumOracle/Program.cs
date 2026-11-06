using System.Reflection;
using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;
using System.Runtime.Loader;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

static Assembly LoadAssembly(string path)
{
    string assemblyPath = Path.GetFullPath(path);
    var loader = new AssemblyLoadContext("runtime-probe", true);
    loader.Resolving += (context, name) => {
        string dependency = Path.Combine(Path.GetDirectoryName(assemblyPath)!, name.Name + ".dll");
        return File.Exists(dependency) ? context.LoadFromAssemblyPath(dependency) : null;
    };
    return loader.LoadFromAssemblyPath(assemblyPath);
}
if (args.Length == 2 && args[0] == "--make-framework-bridge")
{
    var bridgeReferences = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator)
        .Select(path => MetadataReference.CreateFromFile(path));
    const string source = "public static class FrameworkBridge { public static System.DayOfWeek Echo(System.DayOfWeek value) => value; }";
    var compilation = CSharpCompilation.Create("FrameworkBridge", [CSharpSyntaxTree.ParseText(source)], bridgeReferences,
        new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
    using var stream = File.Create(args[1]);
    var result = compilation.Emit(stream);
    if (!result.Success) throw new Exception("Framework bridge build failed: " + string.Join("\n", result.Diagnostics));
    stream.Position = 0;
    using var image = new PEReader(stream, PEStreamOptions.LeaveOpen);
    var metadata = image.GetMetadataReader();
    bool strongReference = metadata.AssemblyReferences
        .Select(metadata.GetAssemblyReference)
        .Any(reference => metadata.GetString(reference.Name) == "System.Private.CoreLib" &&
            metadata.GetBlobBytes(reference.PublicKeyOrToken).Length == 8);
    if (!strongReference) throw new Exception("Framework bridge did not emit the expected strong-name token reference");
    return;
}
if (args.Length == 5 && args[0] == "--invoke")
{
    var assembly = LoadAssembly(args[1]);
    object? result = assembly.GetType(args[2], true)!.GetMethod(args[3])!.Invoke(null, null);
    if (!Equals(result, int.Parse(args[4]))) throw new Exception($"Unexpected runtime result: {result}");
    Console.WriteLine($"{args[2]}.{args[3]} returned {result}.");
    return;
}
if (args.Length == 4 && args[0] == "--external")
{
    var externalConsumerType = LoadAssembly(args[1]).GetType("ExternalConsumer", true)!;
    var signatures = new[] {
        ("Echo", "EnumFixture.Direction"), ("EchoAlias", "EnumFixture.Direction"), ("EchoSignedByte", "EnumFixture.SignedByte"),
        ("EchoUnsignedByte", "EnumFixture.UnsignedByte"), ("EchoSignedShort", "EnumFixture.SignedShort"),
        ("EchoUnsignedShort", "EnumFixture.UnsignedShort"), ("EchoSignedInt", "EnumFixture.SignedInt"),
        ("EchoUnsignedInt", "EnumFixture.UnsignedInt"), ("EchoSignedLong", "EnumFixture.SignedLong"),
        ("EchoUnsignedLong", "EnumFixture.UnsignedLong")
    };
    foreach (var (methodName, enumName) in signatures)
    {
        var method = externalConsumerType.GetMethod(methodName)!;
        if (!method.ReturnType.IsEnum || method.ReturnType.FullName != enumName ||
            method.GetParameters().Single().ParameterType != method.ReturnType ||
            method.ReturnType.Assembly.GetName().Name != "EnumApi")
            throw new Exception($"Invalid external enum signature: {methodName}");
    }
    var array = externalConsumerType.GetMethod("EchoArray")!;
    if (!array.ReturnType.IsArray || array.ReturnType.GetElementType()?.FullName != "EnumFixture.Direction" ||
        array.GetParameters().Single().ParameterType != array.ReturnType)
        throw new Exception("Invalid external enum array signature");
    var framework = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator)
        .Select(path => MetadataReference.CreateFromFile(path));
    var references = framework.Concat([MetadataReference.CreateFromFile(args[2]), MetadataReference.CreateFromFile(args[3])]);
    var externalReferenceCompilation = CSharpCompilation.Create("ExternalReferenceProbe", references: references,
        options: new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
    var referenceConsumer = externalReferenceCompilation.GetTypeByMetadataName("ExternalConsumer")
        ?? throw new Exception("External consumer reference type missing");
    foreach (var (methodName, enumName) in signatures)
    {
        var method = referenceConsumer.GetMembers(methodName).OfType<IMethodSymbol>().Single();
        if (method.ReturnType.TypeKind != TypeKind.Enum || method.ReturnType.ToDisplayString() != enumName ||
            method.ReturnType.ContainingAssembly.Name != "EnumApi" ||
            !SymbolEqualityComparer.Default.Equals(method.ReturnType, method.Parameters.Single().Type))
            throw new Exception($"Invalid external enum reference signature: {methodName}");
    }
    object? result = externalConsumerType.GetMethod("Run")!.Invoke(null, null);
    if (!Equals(result, 257)) throw new Exception($"External consumer returned {result}, expected 257");
    Console.WriteLine("External enum signatures, reference metadata, array operations and cross-assembly execution passed (257).");
    return;
}
if (args.Length != 2) throw new ArgumentException("usage: EnumOracle <implementation.dll> <reference.dll> | --invoke <assembly.dll> <type> <method> <expected-int>");
string implementation = Path.GetFullPath(args[0]);
string reference = Path.GetFullPath(args[1]);
var platform = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!)
    .Split(Path.PathSeparator)
    .Select(path => MetadataReference.CreateFromFile(path))
    .ToArray<MetadataReference>();
var context = new AssemblyLoadContext("implementation", true);
var actual = context.LoadFromAssemblyPath(implementation);

var specs = new (string Name, Type Underlying, bool Public, (string Name, object Value)[] Members)[] {
    ("EnumFixture.Direction", typeof(int), true, [("Negative", -2), ("Zero", 0), ("Positive", 2), ("Alias", 2), ("Next", 3)]),
    ("EnumFixture.SignedByte", typeof(sbyte), true, [("Minimum", (sbyte)-128), ("Zero", (sbyte)0), ("Maximum", (sbyte)127)]),
    ("EnumFixture.UnsignedByte", typeof(byte), true, [("Zero", (byte)0), ("Maximum", (byte)255)]),
    ("EnumFixture.SignedShort", typeof(short), true, [("Minimum", (short)-32768), ("Maximum", (short)32767)]),
    ("EnumFixture.UnsignedShort", typeof(ushort), true, [("Zero", (ushort)0), ("Maximum", (ushort)65535)]),
    ("EnumFixture.SignedInt", typeof(int), true, [("Minimum", int.MinValue), ("Maximum", int.MaxValue)]),
    ("EnumFixture.UnsignedInt", typeof(uint), true, [("Zero", 0U), ("One", 1U), ("Two", 2U), ("Maximum", uint.MaxValue)]),
    ("EnumFixture.SignedLong", typeof(long), true, [("Minimum", long.MinValue), ("Zero", 0L), ("Maximum", long.MaxValue)]),
    ("EnumFixture.UnsignedLong", typeof(ulong), true, [("Zero", 0UL), ("Maximum", ulong.MaxValue)]),
    ("EnumFixture.Empty", typeof(int), true, []),
    ("EnumFixture.InternalCode", typeof(int), false, [("One", 1)]),
    ("EnumFixture.Bits", typeof(int), true, [("Read", 1), ("Write", 2), ("Both", 3), ("Future", 8), ("AfterFuture", 9), ("Last", 8), ("Minus", -1), ("Mask", 2), ("Precedence", 7), ("Shift", -2)]),
    ("EnumFixture.ByteFlags", typeof(byte), true, [("High", (byte)128), ("Low", (byte)1), ("Both", (byte)129), ("Invert", (byte)255), ("Arithmetic", (byte)255)]),
    ("EnumOne.Shared", typeof(int), true, [("Value", 1)]),
    ("EnumTwo.Shared", typeof(int), true, [("Value", 2)])
};
var referenceCompilation = CSharpCompilation.Create("ReferenceProbe",
    references: platform.Concat([MetadataReference.CreateFromFile(reference)]),
    options: new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
var underlyingTypes = new Dictionary<Type, SpecialType> {
    [typeof(sbyte)] = SpecialType.System_SByte, [typeof(byte)] = SpecialType.System_Byte,
    [typeof(short)] = SpecialType.System_Int16, [typeof(ushort)] = SpecialType.System_UInt16,
    [typeof(int)] = SpecialType.System_Int32, [typeof(uint)] = SpecialType.System_UInt32,
    [typeof(long)] = SpecialType.System_Int64, [typeof(ulong)] = SpecialType.System_UInt64
};

foreach (var spec in specs)
{
    var type = actual.GetType(spec.Name, throwOnError: true)!;
    if (!type.IsEnum || !type.IsSealed || type.IsPublic != spec.Public || Enum.GetUnderlyingType(type) != spec.Underlying)
        throw new Exception($"Invalid enum type: {spec.Name}");
    if (type.IsDefined(typeof(FlagsAttribute), false) != (spec.Name == "EnumFixture.Bits"))
        throw new Exception($"Invalid FlagsAttribute metadata: {spec.Name}");
    var valueField = type.GetField("value__", BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)
        ?? throw new Exception($"{spec.Name} has no value__ field");
    if (valueField.FieldType != spec.Underlying ||
        valueField.Attributes != (FieldAttributes.Public | FieldAttributes.SpecialName | FieldAttributes.RTSpecialName))
        throw new Exception($"Invalid value__ metadata: {spec.Name}");
    var fields = type.GetFields(BindingFlags.Public | BindingFlags.Static);
    if (fields.Length != spec.Members.Length) throw new Exception($"Wrong member count: {spec.Name}");
    foreach (var member in spec.Members)
    {
        var field = type.GetField(member.Name, BindingFlags.Public | BindingFlags.Static)
            ?? throw new Exception($"Missing member: {spec.Name}.{member.Name}");
        if (field.FieldType != type || !Equals(field.GetRawConstantValue(), member.Value) ||
            field.Attributes != (FieldAttributes.Public | FieldAttributes.Static | FieldAttributes.Literal | FieldAttributes.HasDefault))
            throw new Exception($"Invalid member metadata: {spec.Name}.{member.Name}");
    }
    var symbol = referenceCompilation.GetTypeByMetadataName(spec.Name)
        ?? throw new Exception($"Reference type missing: {spec.Name}");
    if (symbol.TypeKind != TypeKind.Enum || symbol.EnumUnderlyingType?.SpecialType != underlyingTypes[spec.Underlying] ||
        symbol.DeclaredAccessibility != (spec.Public ? Accessibility.Public : Accessibility.Internal))
        throw new Exception($"Invalid reference enum type: {spec.Name}");
    if (symbol.GetAttributes().Any(attribute => attribute.AttributeClass?.ToDisplayString() == "System.FlagsAttribute") != (spec.Name == "EnumFixture.Bits"))
        throw new Exception($"Invalid reference FlagsAttribute metadata: {spec.Name}");
    var symbolFields = symbol.GetMembers().OfType<IFieldSymbol>().Where(field => field.HasConstantValue).ToArray();
    if (symbolFields.Length != spec.Members.Length) throw new Exception($"Wrong reference member count: {spec.Name}");
    foreach (var member in spec.Members)
    {
        var field = symbolFields.SingleOrDefault(field => field.Name == member.Name)
            ?? throw new Exception($"Missing reference member: {spec.Name}.{member.Name}");
        if (!SymbolEqualityComparer.Default.Equals(field.Type, symbol) || !Equals(field.ConstantValue, member.Value))
            throw new Exception($"Invalid reference member: {spec.Name}.{member.Name}");
    }
}

void CheckMethod(string owner, string name, string result, string parameter)
{
    var method = actual.GetType(owner, true)!.GetMethod(name) ?? throw new Exception($"Missing {owner}.{name}");
    if (method.ReturnType.FullName != result || method.GetParameters().Single().ParameterType.FullName != parameter)
        throw new Exception($"Invalid method signature: {owner}.{name}");
}
CheckMethod("EnumFixture.Api", "Echo", "EnumFixture.Direction", "EnumFixture.Direction");
CheckMethod("EnumFixture.Api", "EchoArray", "EnumFixture.Direction[]", "EnumFixture.Direction[]");
CheckMethod("EnumFixture.Api", "Cast", "EnumFixture.Direction", "System.Int32");
CheckMethod("EnumOne.Holder", "Echo", "EnumOne.Shared", "EnumOne.Shared");
CheckMethod("EnumTwo.OtherHolder", "Echo", "EnumTwo.Shared", "EnumTwo.Shared");
CheckMethod("EnumImported.ImportedHolder", "Echo", "EnumOne.Shared", "EnumOne.Shared");
CheckMethod("EnumAliased.AliasHolder", "Echo", "EnumTwo.Shared", "EnumTwo.Shared");
var api = actual.GetType("EnumFixture.Api", true)!;
if (api.GetField("Current")!.FieldType.FullName != "EnumFixture.Direction") throw new Exception("Invalid Current field type");
if (api.GetField("ZeroField")!.FieldType.FullName != "EnumFixture.UnsignedLong" ||
    Convert.ToUInt64(api.GetField("ZeroField")!.GetValue(null)) != 0) throw new Exception("Invalid zero field");
foreach (var test in new[] {
    ("EnumFixture.Api", "Run", 2), ("EnumFixture.Api", "RoundTrip", -2),
    ("EnumFixture.Api", "Arrays", 9), ("EnumFixture.Api", "ZeroConversions", 4), ("EnumFixture.Api", "FlagConstants", 143), ("EnumFixture.Api", "FlagOperations", 42), ("EnumFixture.Api", "FlagSwitch", 19), ("EnumFixture.Api", "IncrementEnums", 44), ("EnumFixture.Api", "IncrementExceptions", 2), ("EnumFixture.Api", "EnumArithmetic", 45),
    ("EnumFixture.Api", "SwitchEnums", 43), ("EnumFixture.Api", "SwitchAll", 36), ("EnumFixture.Api", "SwitchUnsignedDense", 15), ("EnumFixture.Api", "SwitchNoDefault", 13),
    ("EnumOne.Holder", "Run", 1), ("EnumTwo.OtherHolder", "Run", 2),
    ("EnumImported.ImportedHolder", "Run", 1), ("EnumAliased.AliasHolder", "Run", 2)
})
{
    object? result = actual.GetType(test.Item1, true)!.GetMethod(test.Item2)!.Invoke(null, null);
    if (!Equals(result, test.Item3)) throw new Exception($"{test.Item1}.{test.Item2} returned {result}, expected {test.Item3}");
}

const string consumerSource = """
    using EnumFixture;
    public static class Downstream {
        public static int Run() {
            Api.Current = Direction.Positive;
            return (int)Api.Echo(Api.Current) + (int)Direction.Negative + (int)UnsignedByte.Maximum
                + (int)EnumOne.Holder.Echo(EnumOne.Shared.Value) + (int)EnumTwo.OtherHolder.Echo(EnumTwo.Shared.Value);
        }
    }
    """;
var consumerCompilation = CSharpCompilation.Create("Downstream", [CSharpSyntaxTree.ParseText(consumerSource)],
    platform.Concat([MetadataReference.CreateFromFile(reference)]),
    new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
using var consumerBytes = new MemoryStream();
var consumerEmit = consumerCompilation.Emit(consumerBytes);
if (!consumerEmit.Success) throw new Exception("Downstream rejected reference: " + string.Join("\n", consumerEmit.Diagnostics));
consumerBytes.Position = 0;
var consumerContext = new AssemblyLoadContext("downstream", true);
consumerContext.Resolving += (loader, name) => name.Name == AssemblyName.GetAssemblyName(implementation).Name
    ? loader.LoadFromAssemblyPath(implementation) : null;
var consumer = consumerContext.LoadFromStream(consumerBytes);
var downstreamResult = consumer.GetType("Downstream", true)!.GetMethod("Run")!.Invoke(null, null);
if (!Equals(downstreamResult, 258)) throw new Exception($"Downstream returned {downstreamResult}, expected 258");
Console.WriteLine($"Validated {specs.Length} enum types, metadata, reference symbols, runtime behavior and downstream execution (258).");
