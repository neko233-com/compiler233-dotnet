using System.Reflection;
using System.Reflection.Metadata;
using System.Reflection.PortableExecutable;
using System.Reflection.Emit;
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
static HashSet<ushort> ReadOpcodes(MethodInfo method)
{
    var definitions = typeof(OpCodes).GetFields(BindingFlags.Public | BindingFlags.Static)
        .Select(field => (OpCode)field.GetValue(null)!)
        .GroupBy(opcode => unchecked((ushort)opcode.Value))
        .ToDictionary(group => group.Key, group => group.First());
    byte[] bytes = method.GetMethodBody()?.GetILAsByteArray() ?? throw new Exception("Method body missing");
    var result = new HashSet<ushort>();
    int position = 0;
    while (position < bytes.Length)
    {
        ushort code = bytes[position++];
        if (code == 0xfe) code = (ushort)(0xfe00 | bytes[position++]);
        if (!definitions.TryGetValue(code, out var opcode)) throw new Exception($"Unknown IL opcode {code:X4}");
        result.Add(code);
        position += opcode.OperandType switch {
            OperandType.InlineNone => 0,
            OperandType.ShortInlineBrTarget or OperandType.ShortInlineI or OperandType.ShortInlineVar => 1,
            OperandType.InlineVar => 2,
            OperandType.InlineI or OperandType.InlineBrTarget or OperandType.InlineField or OperandType.InlineMethod or
                OperandType.InlineSig or OperandType.InlineString or OperandType.InlineTok or OperandType.InlineType or OperandType.ShortInlineR => 4,
            OperandType.InlineI8 or OperandType.InlineR => 8,
            OperandType.InlineSwitch => 4 + checked(BitConverter.ToInt32(bytes, position) * 4),
            _ => throw new Exception($"Unsupported IL operand {opcode.OperandType}")
        };
        if (position > bytes.Length) throw new Exception("Invalid IL operand length");
    }
    return result;
}
if (args.Length == 3 && args[0] == "--structs")
{
    foreach (string path in args[1..])
    {
        using var stream = File.OpenRead(path);
        using var pe = new PEReader(stream);
        var reader = pe.GetMetadataReader();
        var point = reader.TypeDefinitions.Select(reader.GetTypeDefinition)
            .Single(type => reader.GetString(type.Namespace) == "StructFixture" && reader.GetString(type.Name) == "Point");
        if (!point.Attributes.HasFlag(TypeAttributes.Public | TypeAttributes.Sealed) ||
            (point.Attributes & TypeAttributes.LayoutMask) != TypeAttributes.SequentialLayout ||
            point.BaseType.Kind != HandleKind.TypeReference)
            throw new Exception($"Incorrect struct type metadata in {path}");
        var parent = reader.GetTypeReference((TypeReferenceHandle)point.BaseType);
        if (reader.GetString(parent.Namespace) != "System" || reader.GetString(parent.Name) != "ValueType")
            throw new Exception($"Struct base type is not System.ValueType in {path}");
        var fields = point.GetFields().Select(reader.GetFieldDefinition).ToArray();
        if (fields.Length != 2 || !fields.Select(field => reader.GetString(field.Name)).SequenceEqual(["X", "Y"]) ||
            fields.Any(field => field.Attributes != FieldAttributes.Public ||
                !reader.GetBlobBytes(field.Signature).SequenceEqual(new byte[] { 0x06, 0x08 })) ||
            point.GetMethods().Select(reader.GetMethodDefinition).Any(method => reader.GetString(method.Name) == ".ctor"))
            throw new Exception($"Incorrect struct fields or synthetic constructor in {path}");
    }
    var assembly = LoadAssembly(args[1]);
    var type = assembly.GetType("StructFixture.Point", true)!;
    var runner = assembly.GetType("StructFixture.Runner", true)!;
    if (!type.IsValueType || type.IsEnum || !type.IsLayoutSequential ||
        System.Runtime.InteropServices.Marshal.SizeOf(type) != 8 ||
        runner.GetMethod("Make")!.ReturnType != type ||
        runner.GetMethod("ChangeCopy")!.GetParameters().Single().ParameterType != type ||
        !Equals(runner.GetMethod("Run")!.Invoke(null, null), 12797) ||
        !Equals(runner.GetMethod("ContainerCopy")!.Invoke(null, null), 384) ||
        !Equals(assembly.GetType("StructConsumer.AliasRunner", true)!.GetMethod("Run")!.Invoke(null, null), 17))
        throw new Exception("Struct layout, signatures, copying or runtime result was incorrect");
    if (!ReadOpcodes(runner.GetMethod("Make")!).Contains(unchecked((ushort)OpCodes.Initobj.Value)) ||
        !ReadOpcodes(runner.GetMethod("ChangeCopy")!).Contains(unchecked((ushort)OpCodes.Ldarga.Value)))
        throw new Exception("Struct initialization or parameter address instruction missing");
    var framework = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator)
        .Select(path => MetadataReference.CreateFromFile(path));
    const string source = "using StructFixture; public static class StructDownstream { public static int Run() { Point p = Runner.Make(); Point q = p; q.X = 8; if (Runner.ChangeCopy(p) != 9 || p.X != 3) return -1; return p.X * 100 + q.X * 10 + p.Y; } }";
    var structCompilation = CSharpCompilation.Create("StructDownstream", [CSharpSyntaxTree.ParseText(source)],
        framework.Concat([MetadataReference.CreateFromFile(args[2])]),
        new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary));
    using var structBytes = new MemoryStream();
    var emitted = structCompilation.Emit(structBytes);
    if (!emitted.Success) throw new Exception("Downstream rejected struct reference: " + string.Join("\n", emitted.Diagnostics));
    structBytes.Position = 0;
    var structContext = new AssemblyLoadContext("struct-downstream", true);
    string structImplementation = Path.GetFullPath(args[1]);
    structContext.Resolving += (loader, name) => name.Name == AssemblyName.GetAssemblyName(structImplementation).Name
        ? loader.LoadFromAssemblyPath(structImplementation) : null;
    var structConsumer = structContext.LoadFromStream(structBytes);
    if (!Equals(structConsumer.GetType("StructDownstream", true)!.GetMethod("Run")!.Invoke(null, null), 384))
        throw new Exception("Struct downstream copy semantics failed");
    Console.WriteLine("Struct metadata, reference, layout, value copies, parameter mutation and downstream execution passed.");
    return;
}
if (args.Length == 2 && args[0] == "--constant-consumer")
{
    using var stream = File.OpenRead(args[1]);
    using var pe = new PEReader(stream);
    var reader = pe.GetMetadataReader();
    if (reader.AssemblyReferences.Select(reader.GetAssemblyReference).Any(item => reader.GetString(item.Name) == "Constants"))
        throw new Exception("Consumer retained an unnecessary assembly reference for inlined constants");
    var constantConsumer = LoadAssembly(args[1]).GetType("ConstantConsumer", true)!.GetMethod("Run")!;
    if (!Equals(constantConsumer.Invoke(null, null), 42)) throw new Exception("Constant consumer failed");
    Console.WriteLine("ConstantConsumer.Run returned 42 without a constants assembly reference.");
    return;
}
if (args.Length == 3 && args[0] == "--constants")
{
    foreach (string path in args[1..])
    {
        using var stream = File.OpenRead(path);
        using var pe = new PEReader(stream);
        var reader = pe.GetMetadataReader();
        var values = reader.TypeDefinitions.Select(reader.GetTypeDefinition)
            .Single(type => reader.GetString(type.Namespace) == "ConstantFixture" && reader.GetString(type.Name) == "Values");
        foreach (var (name, code) in new[] {
            ("Answer", ConstantTypeCode.Int32), ("Negative", ConstantTypeCode.Int32), ("Zero", ConstantTypeCode.Int32),
            ("Max", ConstantTypeCode.Int32), ("Enabled", ConstantTypeCode.Boolean), ("Disabled", ConstantTypeCode.Boolean),
            ("Text", ConstantTypeCode.String), ("Empty", ConstantTypeCode.String),
            ("Missing", ConstantTypeCode.NullReference), ("NullObject", ConstantTypeCode.NullReference)
        })
        {
            var field = values.GetFields().Select(reader.GetFieldDefinition)
                .Single(field => reader.GetString(field.Name) == name);
            if (!field.Attributes.HasFlag(FieldAttributes.Static | FieldAttributes.Literal | FieldAttributes.HasDefault) ||
                reader.GetConstant(field.GetDefaultValue()).TypeCode != code)
                throw new Exception($"Incorrect constant metadata: {name} in {path}");
        }
        var only = reader.TypeDefinitions.Select(reader.GetTypeDefinition)
            .Single(type => reader.GetString(type.Namespace) == "ConstantFixture" && reader.GetString(type.Name) == "OnlyConstants");
        if (only.GetMethods().Select(reader.GetMethodDefinition).Any(method => reader.GetString(method.Name) == ".cctor"))
            throw new Exception("Literal-only class unexpectedly has a type initializer");
    }
    var assembly = LoadAssembly(args[1]);
    var type = assembly.GetType("ConstantFixture.Values", true)!;
    foreach (var (name, value) in new (string, object?)[] {
        ("Answer", 42), ("Negative", -10), ("Zero", 0), ("Max", int.MaxValue), ("Enabled", true), ("Disabled", false),
        ("Text", "常量✓"), ("Empty", ""), ("Missing", null), ("NullObject", null)
    })
        if (!Equals(type.GetField(name)!.GetRawConstantValue(), value)) throw new Exception($"Incorrect literal value: {name}");
    var runner = assembly.GetType("ConstantFixture.Runner", true)!.GetMethod("Run")!;
    if (!Equals(runner.Invoke(null, null), 42)) throw new Exception("Constant fixture returned an unexpected result");
    if (ReadOpcodes(runner).Contains(unchecked((ushort)OpCodes.Ldsfld.Value))) throw new Exception("Literal field was not inlined");
    Console.WriteLine("Constant field metadata, inlining, switches and runtime passed (42).");
    return;
}
if (args.Length == 3 && args[0] == "--readonly")
{
    foreach (string path in args[1..])
    {
        using var stream = File.OpenRead(path);
        using var pe = new PEReader(stream);
        var reader = pe.GetMetadataReader();
        var holder = reader.TypeDefinitions.Select(reader.GetTypeDefinition)
            .Single(type => reader.GetString(type.Namespace) == "ReadonlyFixture" && reader.GetString(type.Name) == "Holder");
        foreach (var (name, isStatic) in new[] {
            ("A", false), ("B", false), ("C", false), ("Numbers", false), ("Pick", false), ("Global", true), ("Inline", true)
        })
        {
            var field = holder.GetFields().Select(reader.GetFieldDefinition)
                .Single(field => reader.GetString(field.Name) == name);
            if (!field.Attributes.HasFlag(FieldAttributes.InitOnly) || field.Attributes.HasFlag(FieldAttributes.Static) != isStatic)
                throw new Exception($"Incorrect readonly field metadata: {name} in {path}");
        }
        var trigger = holder.GetFields().Select(reader.GetFieldDefinition)
            .Single(field => reader.GetString(field.Name) == "Trigger");
        if (trigger.Attributes.HasFlag(FieldAttributes.InitOnly)) throw new Exception("Mutable field marked readonly");
    }
    var assembly = LoadAssembly(args[1]);
    object? result = assembly.GetType("ReadonlyFixture.Holder", true)!.GetMethod("Run")!.Invoke(null, null);
    if (!Equals(result, 39)) throw new Exception($"Readonly fixture returned {result}");
    Console.WriteLine("Readonly field metadata, constructor writes and reference mutation passed (39).");
    return;
}
if (args.Length == 3 && args[0] == "--static")
{
    foreach (string path in args[1..])
    {
        using var stream = File.OpenRead(path);
        using var pe = new PEReader(stream);
        var reader = pe.GetMetadataReader();
        foreach (var (name, explicitCtor) in new[] {
            ("Operations", true), ("Plain", false), ("NoFields", true), ("ExpressionBody", true), ("Failing", true), ("FieldAccess", false), ("OrderTracker", false), ("LateStore", true), ("EarlyRead", true), ("Runner", false)
        })
        {
            var type = reader.TypeDefinitions.Select(reader.GetTypeDefinition)
                .Single(definition => reader.GetString(definition.Namespace) == "StaticFixture" && reader.GetString(definition.Name) == name);
            if (!type.Attributes.HasFlag(TypeAttributes.Abstract) || !type.Attributes.HasFlag(TypeAttributes.Sealed) ||
                type.Attributes.HasFlag(TypeAttributes.BeforeFieldInit) == explicitCtor)
                throw new Exception($"Incorrect static class attributes: {name} in {path}");
            var methods = type.GetMethods().Select(reader.GetMethodDefinition).ToArray();
            if (methods.Any(method => reader.GetString(method.Name) == ".ctor") ||
                methods.Any(method => !method.Attributes.HasFlag(MethodAttributes.Static)))
                throw new Exception($"Static class has instance members: {name} in {path}");
            if (path == args[1] && explicitCtor && methods.Count(method => reader.GetString(method.Name) == ".cctor") != 1)
                throw new Exception($"Static constructor missing: {name}");
        }
        var ordinary = reader.TypeDefinitions.Select(reader.GetTypeDefinition)
            .Single(definition => reader.GetString(definition.Namespace) == "StaticFixture" && reader.GetString(definition.Name) == "Ordinary");
        if (ordinary.Attributes.HasFlag(TypeAttributes.BeforeFieldInit)) throw new Exception("Explicit constructor did not remove BeforeFieldInit");
        var marker = reader.TypeDefinitions.Select(reader.GetTypeDefinition)
            .Single(definition => reader.GetString(definition.Namespace) == "StaticFixture" && reader.GetString(definition.Name) == "Marker");
        if (!marker.Attributes.HasFlag(TypeAttributes.Interface) || marker.Attributes.HasFlag(TypeAttributes.BeforeFieldInit))
            throw new Exception("Interface with a static constructor has incorrect attributes");
    }
    var assembly = LoadAssembly(args[1]);
    object? result = assembly.GetType("StaticFixture.Runner", true)!.GetMethod("Run")!.Invoke(null, null);
    if (!Equals(result, 49)) throw new Exception($"Static initialization returned {result}");
    System.Runtime.CompilerServices.RuntimeHelpers.RunClassConstructor(assembly.GetType("StaticFixture.Marker", true)!.TypeHandle);
    var failing = assembly.GetType("StaticFixture.Failing", true)!.GetMethod("Read")!;
    for (int attempt = 0; attempt < 2; attempt++)
    {
        try { failing.Invoke(null, null); throw new Exception("Failed static initialization unexpectedly succeeded"); }
        catch (TargetInvocationException exception) when (exception.InnerException is TypeInitializationException { InnerException: InvalidOperationException }) { }
    }
    object? steps = assembly.GetType("StaticFixture.Ordinary", true)!.GetField("Steps")!.GetValue(null);
    if (!Equals(steps, 175)) throw new Exception($"Static constructors did not run once each: {steps}");
    Console.WriteLine("Static class metadata, initialization order, exception caching and reference image passed.");
    return;
}
if (args.Length == 2 && args[0] == "--static-framework")
{
    using var stream = File.OpenRead(args[1]);
    using var pe = new PEReader(stream);
    var reader = pe.GetMetadataReader();
    var frameworkReference = reader.AssemblyReferences.Select(reader.GetAssemblyReference)
        .Single(item => reader.GetString(item.Name) == "System.Private.CoreLib");
    byte[] keyOrToken = reader.GetBlobBytes(frameworkReference.PublicKeyOrToken);
    byte[] token = frameworkReference.Flags.HasFlag(AssemblyFlags.PublicKey)
        ? System.Security.Cryptography.SHA1.HashData(keyOrToken)[^8..].Reverse().ToArray()
        : keyOrToken;
    if (!token.SequenceEqual(Convert.FromHexString("7CEC85D7BEA7798E")) ||
        frameworkReference.Version != typeof(string).Assembly.GetName().Version)
        throw new Exception("Framework field assembly reference has an incorrect public-key token");
    Console.WriteLine("Framework static field assembly identity passed.");
    return;
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
    ("EnumFixture.GlobalFlags", typeof(int), true, [("A", 1), ("B", 2)]),
    ("EnumFlagAlias.AliasedFlags", typeof(int), true, [("A", 1)]),
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
    if (type.IsDefined(typeof(FlagsAttribute), false) != (spec.Name is "EnumFixture.Bits" or "EnumFixture.GlobalFlags" or "EnumFlagAlias.AliasedFlags"))
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
    if (symbol.GetAttributes().Any(attribute => attribute.AttributeClass?.ToDisplayString() == "System.FlagsAttribute") != (spec.Name is "EnumFixture.Bits" or "EnumFixture.GlobalFlags" or "EnumFlagAlias.AliasedFlags"))
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
var flagOpcodes = ReadOpcodes(api.GetMethod("HasFlagFast")!);
if (!flagOpcodes.Contains(unchecked((ushort)OpCodes.And.Value)) || !flagOpcodes.Contains(unchecked((ushort)OpCodes.Ceq.Value)) ||
    flagOpcodes.Contains(unchecked((ushort)OpCodes.Box.Value)) || flagOpcodes.Contains(unchecked((ushort)OpCodes.Callvirt.Value)))
    throw new Exception("Same-type HasFlag did not use the allocation-free bitwise path");
if (api.GetField("Current")!.FieldType.FullName != "EnumFixture.Direction") throw new Exception("Invalid Current field type");
if (api.GetField("ZeroField")!.FieldType.FullName != "EnumFixture.UnsignedLong" ||
    Convert.ToUInt64(api.GetField("ZeroField")!.GetValue(null)) != 0) throw new Exception("Invalid zero field");
var directionType = actual.GetType("EnumFixture.Direction", true)!;
var directionValue = Enum.ToObject(directionType, 2);
foreach (string methodName in new[] { "Box", "BoxExplicit" })
{
    object? boxed = api.GetMethod(methodName)!.Invoke(null, [directionValue]);
    if (boxed?.GetType() != directionType || Convert.ToInt32(boxed) != 2)
        throw new Exception($"{methodName} did not preserve the enum type and value");
}
var unbox = api.GetMethod("Unbox")!;
foreach (var (value, exceptionType) in new[] {
    ((object?)null, typeof(NullReferenceException)),
    (Enum.ToObject(actual.GetType("EnumFixture.UnsignedByte", true)!, 2), typeof(InvalidCastException))
})
{
    try { unbox.Invoke(null, [value]); throw new Exception("Invalid unbox unexpectedly succeeded"); }
    catch (TargetInvocationException exception) when (exception.InnerException?.GetType() == exceptionType) { }
}
var formatDirection = api.GetMethod("FormatDirection")!;
if (!Equals(formatDirection.Invoke(null, [null]), "Positive")) throw new Exception("Null format was not treated as general format");
try { formatDirection.Invoke(null, ["invalid"]); throw new Exception("Invalid enum format unexpectedly succeeded"); }
catch (TargetInvocationException exception) when (exception.InnerException is FormatException) { }
var bitsValue = Enum.ToObject(actual.GetType("EnumFixture.Bits", true)!, 1);
try { api.GetMethod("HasNull")!.Invoke(null, [bitsValue]); throw new Exception("HasFlag(null) unexpectedly succeeded"); }
catch (TargetInvocationException exception) when (exception.InnerException is ArgumentNullException) { }
try { api.GetMethod("HasOther")!.Invoke(null, [bitsValue, directionValue]); throw new Exception("HasFlag with another enum type unexpectedly succeeded"); }
catch (TargetInvocationException exception) when (exception.InnerException is ArgumentException) { }
try { api.GetMethod("CompareOther")!.Invoke(null, [bitsValue, directionValue]); throw new Exception("CompareTo with another enum type unexpectedly succeeded"); }
catch (TargetInvocationException exception) when (exception.InnerException is ArgumentException) { }
foreach (var test in new[] {
    ("EnumFixture.Api", "Run", 2), ("EnumFixture.Api", "RoundTrip", -2), ("EnumFixture.Api", "Defaults", 48),
    ("EnumFixture.Api", "Arrays", 9), ("EnumFixture.Api", "ZeroConversions", 4), ("EnumFixture.Api", "FlagConstants", 143), ("EnumFixture.Api", "FlagOperations", 42), ("EnumFixture.Api", "FlagSwitch", 19), ("EnumFixture.Api", "IncrementEnums", 44), ("EnumFixture.Api", "IncrementExceptions", 2), ("EnumFixture.Api", "EnumArithmetic", 45), ("EnumFixture.Api", "EnumBoxing", 46), ("EnumFixture.Api", "EnumMethods", 47),
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
