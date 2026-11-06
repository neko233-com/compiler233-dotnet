//! A real C# to CIL pipeline for a deliberately bounded language core.
//!
//! Unlike [`crate::syntax`], which only validates syntax, this module builds an
//! abstract syntax tree, resolves names against a two-pass symbol table and
//! emits executable IL. Anything outside the supported core is rejected with a
//! located diagnostic instead of being approximated.
use std::{collections::HashMap, rc::Rc};

use hao::{
    Module,
    dotnet::entries::Version,
    dotnet::{
        entries::{
            Constant, CustomAttribute, Entry, Field, InterfaceImpl, Method, MethodDefOrRef,
            MethodSpec, Param, ResolutionScope, TypeDef, TypeDefOrRef, TypeSpec,
        },
        il::{EmitLabel, Emitter, Instruction, MemberToken, MethodBody, OpCode, Operand},
        md::streams::tables_stream::{FieldFlags, MethodFlags, TypeAttributes},
        module::{ModuleKind, TargetRuntime},
        signature::{ElementType, PrimitiveType},
        signature::{FieldSig, MethodSig, TypeSig},
    },
};

use crate::emitter::TrackedEmitter;
use crate::syntax::{self, SyntaxError};

#[derive(Debug, Clone)]
pub struct CompileError {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
    /// True for lexer/parser errors (syntax), false for semantic or emit errors.
    pub syntax: bool,
}

impl CompileError {
    fn new(file: &str, source: &str, offset: usize, message: impl Into<String>) -> Self {
        let (line, column) = line_column(source, offset);
        CompileError {
            file: file.to_string(),
            line,
            column,
            message: message.into(),
            syntax: false,
        }
    }
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}({},{}): {}",
            self.file, self.line, self.column, self.message
        )
    }
}

fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let end = offset.min(source.len());
    let prefix = &source[..end];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = match prefix.rfind('\n') {
        Some(index) => prefix.len() - index,
        None => prefix.len() + 1,
    };
    (line, column)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ty {
    Void,
    Object,
    Int,
    Float,
    Bool,
    String,
    Null,
    Enum(String, EnumBase),
    Struct(String),
    Class(String),
    Array(Box<Ty>),
    Generic(String, Vec<Ty>),
    Named(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnumBase {
    I1,
    U1,
    I2,
    U2,
    I4,
    U4,
    I8,
    U8,
}

impl EnumBase {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "sbyte" | "System.SByte" => Some(Self::I1),
            "byte" | "System.Byte" => Some(Self::U1),
            "short" | "System.Int16" => Some(Self::I2),
            "ushort" | "System.UInt16" => Some(Self::U2),
            "int" | "System.Int32" => Some(Self::I4),
            "uint" | "System.UInt32" => Some(Self::U4),
            "long" | "System.Int64" => Some(Self::I8),
            "ulong" | "System.UInt64" => Some(Self::U8),
            _ => None,
        }
    }
    fn bounds(self) -> (i128, i128) {
        match self {
            Self::I1 => (i8::MIN as i128, i8::MAX as i128),
            Self::U1 => (0, u8::MAX as i128),
            Self::I2 => (i16::MIN as i128, i16::MAX as i128),
            Self::U2 => (0, u16::MAX as i128),
            Self::I4 => (i32::MIN as i128, i32::MAX as i128),
            Self::U4 => (0, u32::MAX as i128),
            Self::I8 => (i64::MIN as i128, i64::MAX as i128),
            Self::U8 => (0, u64::MAX as i128),
        }
    }
    fn primitive(self) -> PrimitiveType {
        match self {
            Self::I1 => PrimitiveType::I1,
            Self::U1 => PrimitiveType::U1,
            Self::I2 => PrimitiveType::I2,
            Self::U2 => PrimitiveType::U2,
            Self::I4 => PrimitiveType::I4,
            Self::U4 => PrimitiveType::U4,
            Self::I8 => PrimitiveType::I8,
            Self::U8 => PrimitiveType::U8,
        }
    }
    fn element_type(self) -> ElementType {
        match self {
            Self::I1 => ElementType::I1,
            Self::U1 => ElementType::U1,
            Self::I2 => ElementType::I2,
            Self::U2 => ElementType::U2,
            Self::I4 => ElementType::I4,
            Self::U4 => ElementType::U4,
            Self::I8 => ElementType::I8,
            Self::U8 => ElementType::U8,
        }
    }
    fn width(self) -> usize {
        match self {
            Self::I1 | Self::U1 => 1,
            Self::I2 | Self::U2 => 2,
            Self::I4 | Self::U4 => 4,
            Self::I8 | Self::U8 => 8,
        }
    }
    fn conversion(self) -> OpCode {
        match self {
            Self::I1 => OpCode::Conv_I1,
            Self::U1 => OpCode::Conv_U1,
            Self::I2 => OpCode::Conv_I2,
            Self::U2 => OpCode::Conv_U2,
            Self::I4 => OpCode::Conv_I4,
            Self::U4 => OpCode::Conv_U4,
            Self::I8 => OpCode::Conv_I8,
            Self::U8 => OpCode::Conv_U8,
        }
    }
    fn is_unsigned(self) -> bool {
        matches!(self, Self::U1 | Self::U2 | Self::U4 | Self::U8)
    }

    fn decode_bits(self, value: u64) -> i128 {
        match self {
            Self::I1 => value as u8 as i8 as i128,
            Self::U1 => value as u8 as i128,
            Self::I2 => value as u16 as i16 as i128,
            Self::U2 => value as u16 as i128,
            Self::I4 => value as u32 as i32 as i128,
            Self::U4 => value as u32 as i128,
            Self::I8 => value as i64 as i128,
            Self::U8 => value as i128,
        }
    }
}

#[derive(Debug, Clone)]
struct EnumInfo {
    file: String,
    offset: usize,
    namespace: String,
    name: String,
    is_public: bool,
    is_flags: bool,
    flags_name: Option<String>,
    flags_imports: Vec<String>,
    is_external: bool,
    external_identity: Option<ExternalEnumIdentity>,
    base: EnumBase,
    members: Vec<(String, i128)>,
    values: HashMap<String, i128>,
}

#[derive(Debug, Clone)]
struct ExternalEnumIdentity {
    definition: Entry<TypeDef>,
    assembly: String,
    version: Version,
    culture: String,
    public_key: Vec<u8>,
}

fn strong_name_token(public_key: &[u8]) -> [u8; 8] {
    use sha1::Digest;
    let digest = sha1::Sha1::digest(public_key);
    let mut token = [0; 8];
    for (byte, source) in token.iter_mut().zip(digest[12..].iter().rev()) {
        *byte = *source;
    }
    token
}

impl EnumInfo {
    fn full_name(&self) -> String {
        if self.namespace.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.namespace, self.name)
        }
    }

    fn matches_type_reference(&self, reference: &TypeDefOrRef, allow_local: bool) -> bool {
        let Some(identity) = &self.external_identity else {
            return allow_local && matches!(reference, TypeDefOrRef::TypeDef(_));
        };
        match reference {
            TypeDefOrRef::TypeDef(entry) => identity.definition.ptr_eq(entry),
            TypeDefOrRef::TypeRef(entry) => {
                let reference = entry.get();
                let Some(ResolutionScope::AssemblyRef(assembly)) = &reference.scope else {
                    return false;
                };
                let assembly = assembly.get();
                let key_matches =
                    if assembly.public_key_or_token.len() == 8 && !identity.public_key.is_empty() {
                        assembly.public_key_or_token.as_slice()
                            == strong_name_token(&identity.public_key)
                    } else {
                        assembly.public_key_or_token == identity.public_key
                    };
                assembly.name.eq_ignore_ascii_case(&identity.assembly)
                    && assembly.version == identity.version
                    && assembly.culture.eq_ignore_ascii_case(&identity.culture)
                    && key_matches
            }
            TypeDefOrRef::TypeSpec(_) => false,
        }
    }
}

#[derive(Debug)]
struct Program {
    types: Vec<TypeDecl>,
    enums: EnumValues,
}

#[derive(Debug)]
struct TypeDecl {
    file: String,
    offset: usize,
    namespace: String,
    enum_imports: EnumImports,
    name: String,
    base: Option<String>,
    interfaces: Vec<String>,
    is_interface: bool,
    is_struct: bool,
    is_static: bool,
    is_public: bool,
    static_ctor: Option<Block>,
    fields: Vec<FieldDecl>,
    methods: Vec<MethodDecl>,
    properties: Vec<PropertyDecl>,
}

#[derive(Debug)]
struct FieldDecl {
    name: String,
    ty: Ty,
    is_static: bool,
    is_readonly: bool,
    is_const: bool,
    visibility: Visibility,
    init: Option<Expr>,
}

#[derive(Debug)]
struct PropertyDecl {
    name: String,
    ty: Ty,
    is_static: bool,
    visibility: Visibility,
    getter_visibility: Option<Visibility>,
    setter_visibility: Option<Visibility>,
    has_getter: bool,
    has_setter: bool,
    getter: Option<Block>,
    setter: Option<Block>,
}

#[derive(Debug)]
struct MethodDecl {
    name: String,
    is_static: bool,
    visibility: Visibility,
    ret: Ty,
    params: Vec<(Ty, String)>,
    body: Option<Block>,
    is_ctor: bool,
    virtual_kind: VirtualKind,
    base_args: Option<Vec<Expr>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Public,
    Private,
    Internal,
    Protected,
    ProtectedInternal,
    PrivateProtected,
}

impl Visibility {
    fn from_bits(bits: u8, default: Self) -> Option<Self> {
        match bits {
            0 => Some(default),
            1 => Some(Self::Public),
            2 => Some(Self::Private),
            4 => Some(Self::Internal),
            8 => Some(Self::Protected),
            12 => Some(Self::ProtectedInternal),
            10 => Some(Self::PrivateProtected),
            _ => None,
        }
    }

    fn more_restrictive_than(self, parent: Self) -> bool {
        match parent {
            Self::Public => self != Self::Public,
            Self::ProtectedInternal => matches!(
                self,
                Self::Protected | Self::Internal | Self::PrivateProtected | Self::Private
            ),
            Self::Protected | Self::Internal => {
                matches!(self, Self::PrivateProtected | Self::Private)
            }
            Self::PrivateProtected => self == Self::Private,
            Self::Private => false,
        }
    }

    fn method_flags(self) -> MethodFlags {
        match self {
            Self::Public => MethodFlags::Public,
            Self::Private => MethodFlags::Private,
            Self::Internal => MethodFlags::Assem,
            Self::Protected => MethodFlags::Family,
            Self::ProtectedInternal => MethodFlags::FamORAssem,
            Self::PrivateProtected => MethodFlags::FamANDAssem,
        }
    }

    fn field_flags(self) -> FieldFlags {
        match self {
            Self::Public => FieldFlags::Public,
            Self::Private => FieldFlags::Private,
            Self::Internal => FieldFlags::Assembly,
            Self::Protected => FieldFlags::Family,
            Self::ProtectedInternal => FieldFlags::FamORAssem,
            Self::PrivateProtected => FieldFlags::FamANDAssem,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VirtualKind {
    None,
    Virtual,
    Override,
}

#[derive(Debug)]
struct Block {
    statements: Vec<Stmt>,
}

#[derive(Debug)]
struct CatchClause {
    ty: Option<Ty>,
    name: Option<String>,
    filter: Option<Expr>,
    body: Block,
}

#[derive(Debug)]
struct SwitchSection {
    labels: Vec<SwitchLabel>,
    body: Block,
}

#[derive(Debug)]
enum SwitchLabel {
    Case(Expr),
    Default,
}

fn block_terminates(block: &Block) -> bool {
    block
        .statements
        .last()
        .is_some_and(|statement| match statement {
            Stmt::Return(_) | Stmt::Throw(_) => true,
            Stmt::Block(block) => block_terminates(block),
            Stmt::If {
                then_branch,
                else_branch: Some(otherwise),
                ..
            } => block_terminates(then_branch) && block_terminates(otherwise),
            Stmt::Try {
                body,
                catches,
                finally,
            } => {
                finally.as_ref().is_some_and(block_terminates)
                    || (block_terminates(body)
                        && catches.iter().all(|clause| block_terminates(&clause.body)))
            }
            Stmt::Switch { sections, .. } => {
                sections.iter().any(|section| {
                    section
                        .labels
                        .iter()
                        .any(|label| matches!(label, SwitchLabel::Default))
                }) && sections.iter().all(|section| {
                    block_terminates(&section.body) && !escapes_switch(&section.body, 0, 0)
                })
            }
            _ => false,
        })
}

fn escapes_switch(block: &Block, nested_breakables: usize, nested_loops: usize) -> bool {
    block.statements.iter().any(|statement| match statement {
        Stmt::Break => nested_breakables == 0,
        Stmt::Continue => nested_loops == 0,
        Stmt::Block(body) => escapes_switch(body, nested_breakables, nested_loops),
        Stmt::If {
            then_branch,
            else_branch,
            ..
        } => {
            escapes_switch(then_branch, nested_breakables, nested_loops)
                || else_branch
                    .as_ref()
                    .is_some_and(|body| escapes_switch(body, nested_breakables, nested_loops))
        }
        Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::For { body, .. }
        | Stmt::ForEach { body, .. } => {
            escapes_switch(body, nested_breakables + 1, nested_loops + 1)
        }
        Stmt::Switch { sections, .. } => sections
            .iter()
            .any(|section| escapes_switch(&section.body, nested_breakables + 1, nested_loops)),
        Stmt::Try {
            body,
            catches,
            finally,
        } => {
            escapes_switch(body, nested_breakables, nested_loops)
                || catches
                    .iter()
                    .any(|clause| escapes_switch(&clause.body, nested_breakables, nested_loops))
                || finally
                    .as_ref()
                    .is_some_and(|body| escapes_switch(body, nested_breakables, nested_loops))
        }
        _ => false,
    })
}

fn switch_section_exits(block: &Block) -> bool {
    block
        .statements
        .last()
        .is_some_and(|statement| match statement {
            Stmt::Break | Stmt::Continue | Stmt::Return(_) | Stmt::Throw(_) => true,
            Stmt::Block(block) => switch_section_exits(block),
            Stmt::If {
                then_branch,
                else_branch: Some(otherwise),
                ..
            } => switch_section_exits(then_branch) && switch_section_exits(otherwise),
            Stmt::Try {
                body,
                catches,
                finally,
            } => {
                finally.as_ref().is_some_and(switch_section_exits)
                    || (switch_section_exits(body)
                        && catches
                            .iter()
                            .all(|clause| switch_section_exits(&clause.body)))
            }
            Stmt::Switch { .. } => block_terminates(block),
            _ => false,
        })
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SwitchConstant {
    Int(i32),
    Enum(i128),
    Bool(bool),
    String(String),
    Null,
}

fn switch_constant(value: &Expr) -> Option<SwitchConstant> {
    match value {
        Expr::Int(value, _) => Some(SwitchConstant::Int(*value)),
        Expr::Negate(value) => match value.as_ref() {
            Expr::Int(value, _) => value.checked_neg().map(SwitchConstant::Int),
            _ => None,
        },
        Expr::Bool(value) => Some(SwitchConstant::Bool(*value)),
        Expr::Str(value) => Some(SwitchConstant::String(value.clone())),
        Expr::Null => Some(SwitchConstant::Null),
        Expr::Default(Ty::Int) => Some(SwitchConstant::Int(0)),
        Expr::Default(Ty::Bool) => Some(SwitchConstant::Bool(false)),
        Expr::Default(Ty::String) => Some(SwitchConstant::Null),
        _ => None,
    }
}

#[derive(Debug)]
enum Stmt {
    Local {
        ty: Ty,
        name: String,
        init: Option<Expr>,
    },
    Assign {
        name: String,
        value: Expr,
    },
    Expression(Expr),
    Return(Option<Expr>),
    Throw(Option<Expr>),
    Try {
        body: Block,
        catches: Vec<CatchClause>,
        finally: Option<Block>,
    },
    If {
        condition: Expr,
        then_branch: Block,
        else_branch: Option<Block>,
    },
    While {
        condition: Expr,
        body: Block,
    },
    DoWhile {
        body: Block,
        condition: Expr,
    },
    Switch {
        value: Expr,
        sections: Vec<SwitchSection>,
    },
    For {
        init: Option<Box<Stmt>>,
        condition: Option<Expr>,
        step: Option<Expr>,
        body: Block,
    },
    Empty,
    Block(Block),
    AssignOp {
        name: String,
        op: String,
        value: Expr,
    },
    Break,
    Continue,
    AssignMember {
        target: Expr,
        name: String,
        value: Expr,
    },
    AssignMemberOp {
        target: Expr,
        name: String,
        op: String,
        value: Expr,
    },
    AssignIndex {
        target: Expr,
        index: Expr,
        value: Expr,
    },
    ForEach {
        ty: Ty,
        name: String,
        collection: Expr,
        body: Block,
    },
}

fn contains_do(block: &Block) -> bool {
    block.statements.iter().any(|statement| match statement {
        Stmt::DoWhile { .. } => true,
        Stmt::While { body, .. }
        | Stmt::For { body, .. }
        | Stmt::ForEach { body, .. }
        | Stmt::Block(body) => contains_do(body),
        Stmt::If {
            then_branch,
            else_branch,
            ..
        } => contains_do(then_branch) || else_branch.as_ref().is_some_and(contains_do),
        Stmt::Try {
            body,
            catches,
            finally,
        } => {
            contains_do(body)
                || catches.iter().any(|clause| contains_do(&clause.body))
                || finally.as_ref().is_some_and(contains_do)
        }
        Stmt::Switch { sections, .. } => sections.iter().any(|section| contains_do(&section.body)),
        _ => false,
    })
}

// Validate declaration spaces around do-while bodies before emission. A C# local
// conflicts with a declaration in an enclosing scope even when that declaration
// occurs later in the source, so checking only the locals emitted so far is not
// enough. Other declaration-space rules remain part of the broader binder work.
fn validate_do_scopes<'a>(
    block: &'a Block,
    params: &[(String, Ty)],
    ancestors: &mut Vec<Vec<&'a str>>,
    inside_do: bool,
) -> Result<(), String> {
    fn conflicts(name: &str, params: &[(String, Ty)], scopes: &[Vec<&str>]) -> bool {
        params.iter().any(|(param, _)| param == name)
            || scopes.iter().any(|scope| scope.contains(&name))
    }
    let names: Vec<&str> = block
        .statements
        .iter()
        .filter_map(|statement| match statement {
            Stmt::Local { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    if inside_do {
        let mut seen = std::collections::HashSet::new();
        for name in &names {
            if !seen.insert(*name) || conflicts(name, params, ancestors) {
                return Err(format!(
                    "local variable '{name}' conflicts with an existing variable"
                ));
            }
        }
    }
    ancestors.push(names);
    for statement in &block.statements {
        match statement {
            Stmt::DoWhile { body, .. } => validate_do_scopes(body, params, ancestors, true)?,
            Stmt::While { body, .. } | Stmt::Block(body) => {
                validate_do_scopes(body, params, ancestors, inside_do)?;
            }
            Stmt::If {
                then_branch,
                else_branch,
                ..
            } => {
                validate_do_scopes(then_branch, params, ancestors, inside_do)?;
                if let Some(branch) = else_branch {
                    validate_do_scopes(branch, params, ancestors, inside_do)?;
                }
            }
            Stmt::For { init, body, .. } => {
                let name = init.as_deref().and_then(|init| match init {
                    Stmt::Local { name, .. } => Some(name.as_str()),
                    _ => None,
                });
                if let Some(name) = name {
                    if inside_do && conflicts(name, params, ancestors) {
                        return Err(format!(
                            "local variable '{name}' conflicts with an existing variable"
                        ));
                    }
                    ancestors.push(vec![name]);
                }
                validate_do_scopes(body, params, ancestors, inside_do)?;
                if name.is_some() {
                    ancestors.pop();
                }
            }
            Stmt::ForEach { name, body, .. } => {
                if inside_do && conflicts(name, params, ancestors) {
                    return Err(format!(
                        "local variable '{name}' conflicts with an existing variable"
                    ));
                }
                ancestors.push(vec![name.as_str()]);
                validate_do_scopes(body, params, ancestors, inside_do)?;
                ancestors.pop();
            }
            Stmt::Try {
                body,
                catches,
                finally,
            } => {
                validate_do_scopes(body, params, ancestors, inside_do)?;
                for clause in catches {
                    if let Some(name) = &clause.name {
                        if inside_do && conflicts(name, params, ancestors) {
                            return Err(format!(
                                "local variable '{name}' conflicts with an existing variable"
                            ));
                        }
                        ancestors.push(vec![name.as_str()]);
                    }
                    validate_do_scopes(&clause.body, params, ancestors, inside_do)?;
                    if clause.name.is_some() {
                        ancestors.pop();
                    }
                }
                if let Some(body) = finally {
                    validate_do_scopes(body, params, ancestors, inside_do)?;
                }
            }
            Stmt::Switch { sections, .. } => {
                for section in sections {
                    validate_do_scopes(&section.body, params, ancestors, inside_do)?;
                }
            }
            _ => {}
        }
    }
    ancestors.pop();
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    BitAnd,
    BitOr,
    BitXor,
    And,
    Or,
}

#[derive(Debug)]
enum Expr {
    Int(i32, bool),
    Float(f32),
    Null,
    Bool(bool),
    Str(String),
    Default(Ty),
    Name(String),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    Negate(Box<Expr>),
    Not(Box<Expr>),
    Complement(Box<Expr>),
    Coalesce(Box<Expr>, Box<Expr>),
    Conditional {
        condition: Box<Expr>,
        when_true: Box<Expr>,
        when_false: Box<Expr>,
    },
    As(Box<Expr>, Ty),
    Cast(Ty, Box<Expr>),
    Call {
        receiver: Option<Box<Expr>>,
        name: String,
        generic_args: Vec<Ty>,
        args: Vec<Expr>,
    },
    Member {
        target: Box<Expr>,
        name: String,
    },
    New {
        ty: Ty,
        args: Vec<Expr>,
        fields: Vec<(String, Expr)>,
    },
    NewArray {
        element: Ty,
        size: Box<Expr>,
    },
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
    },
    Assign {
        name: String,
        value: Box<Expr>,
    },
    Increment {
        name: String,
        delta: i32,
        prefix: bool,
    },
    IncrementMember {
        target: Box<Expr>,
        name: String,
        delta: i32,
        prefix: bool,
    },
    IncrementIndex {
        target: Box<Expr>,
        index: Box<Expr>,
        delta: i32,
        prefix: bool,
    },
}

pub fn compile(
    sources: &[(String, String)],
    assembly_name: &str,
    references: &[String],
) -> Result<Vec<u8>, CompileError> {
    let externals = Externals::load(references, sources)?;
    let mut program = Program {
        types: Vec::new(),
        enums: externals.enum_values(),
    };
    // Enums and struct types must be known regardless of declaration order.
    let mut structs = HashMap::new();
    let empty_structs = HashMap::new();
    for (file, text) in sources {
        let (types, enums) = parse_file(file, text, &program.enums, &empty_structs)?;
        program.enums.extend(enums);
        for decl in types.into_iter().filter(|decl| decl.is_struct) {
            let full_name = if decl.namespace.is_empty() {
                decl.name.clone()
            } else {
                format!("{}.{}", decl.namespace, decl.name)
            };
            structs.insert(full_name, decl.is_public);
        }
    }
    for (file, text) in sources {
        let (types, _) = parse_file(file, text, &program.enums, &structs)?;
        program.types.extend(types);
    }
    emit(&program, assembly_name, sources, &externals)
}

/// External assemblies referenced with `-reference:`, indexed by type name.
struct Externals {
    modules: Vec<Module>,
    types: HashMap<String, (usize, Entry<TypeDef>)>,
    /// Short type name to candidate full names, for sources that rely on `using`.
    by_short_name: HashMap<String, Vec<String>>,
    /// Declaring type full name to methods, because loaded TypeDef rows do not
    /// carry their method list.
    methods: HashMap<String, Vec<(usize, Entry<Method>)>>,
    fields: HashMap<String, Vec<(usize, Entry<Field>)>>,
}

impl Externals {
    fn load(paths: &[String], sources: &[(String, String)]) -> Result<Self, CompileError> {
        let mut modules = Vec::new();
        let mut types = HashMap::new();
        let mut by_short_name: HashMap<String, Vec<String>> = HashMap::new();
        let mut methods: HashMap<String, Vec<(usize, Entry<Method>)>> = HashMap::new();
        let mut fields: HashMap<String, Vec<(usize, Entry<Field>)>> = HashMap::new();
        for path in paths {
            let module = Module::from_path_no_resolve(path).map_err(|error| CompileError {
                file: sources
                    .first()
                    .map(|(file, _)| file.clone())
                    .unwrap_or_default(),
                line: 1,
                column: 1,
                message: format!("cannot read reference '{path}': {error}"),
                syntax: false,
            })?;
            let index = modules.len();
            for entry in module.all_types() {
                let full_name = entry.get().full_name();
                let short = full_name
                    .rsplit('.')
                    .next()
                    .unwrap_or(&full_name)
                    .to_string();
                by_short_name
                    .entry(short)
                    .or_default()
                    .push(full_name.clone());
                types.entry(full_name).or_insert((index, entry.clone()));
            }
            for field in module.all_fields() {
                if let Some(declaring) = field.get().declaring_type.upgrade() {
                    fields
                        .entry(declaring.get().full_name())
                        .or_default()
                        .push((index, field.clone()));
                }
            }
            for method in module.all_methods() {
                if let Some(declaring) = method.get().declaring_type.upgrade() {
                    methods
                        .entry(declaring.get().full_name())
                        .or_default()
                        .push((index, method.clone()));
                }
            }
            modules.push(module);
        }
        Ok(Externals {
            modules,
            types,
            by_short_name,
            methods,
            fields,
        })
    }

    fn enum_values(&self) -> EnumValues {
        let mut result = HashMap::new();
        for (full_name, (module_index, entry)) in &self.types {
            let definition = entry.get();
            if !definition.is_visible()
                || definition.declaring_type.upgrade().is_some()
                || !definition
                    .extends
                    .as_ref()
                    .is_some_and(|base| base.full_name() == "System.Enum")
                || self
                    .by_short_name
                    .get(&definition.name)
                    .is_some_and(|candidates| {
                        candidates
                            .iter()
                            .filter(|candidate| *candidate == full_name)
                            .count()
                            != 1
                    })
            {
                continue;
            }
            let Some(assembly) = &self.modules[*module_index].assembly else {
                continue;
            };
            let assembly = assembly.get();
            let identity = ExternalEnumIdentity {
                definition: entry.clone(),
                assembly: assembly.name.clone(),
                version: assembly.version,
                culture: assembly.culture.clone(),
                public_key: assembly.public_key.clone(),
            };
            let Some(fields) = self.fields.get(full_name) else {
                continue;
            };
            let base = fields.iter().find_map(|(_, field)| {
                let field = field.get();
                if field.name != "value__" {
                    return None;
                }
                match &field.signature.ty {
                    TypeSig::Primitive(PrimitiveType::I1) => Some(EnumBase::I1),
                    TypeSig::Primitive(PrimitiveType::U1) => Some(EnumBase::U1),
                    TypeSig::Primitive(PrimitiveType::I2) => Some(EnumBase::I2),
                    TypeSig::Primitive(PrimitiveType::U2) => Some(EnumBase::U2),
                    TypeSig::Primitive(PrimitiveType::I4) => Some(EnumBase::I4),
                    TypeSig::Primitive(PrimitiveType::U4) => Some(EnumBase::U4),
                    TypeSig::Primitive(PrimitiveType::I8) => Some(EnumBase::I8),
                    TypeSig::Primitive(PrimitiveType::U8) => Some(EnumBase::U8),
                    _ => None,
                }
            });
            let Some(base) = base else { continue };
            let mut values = HashMap::new();
            let mut members = Vec::new();
            let mut valid = true;
            for (_, field) in fields {
                let field = field.get();
                if !field
                    .flags
                    .contains(FieldFlags::Static | FieldFlags::Literal)
                    || field.flags & FieldFlags::FieldAccessMask != FieldFlags::Public
                {
                    continue;
                }
                let Some(constant) = &field.constant else {
                    valid = false;
                    break;
                };
                if constant.element_type != base.element_type()
                    || constant.value.len() != base.width()
                {
                    valid = false;
                    break;
                }
                let Some(raw) = constant.as_i64() else {
                    valid = false;
                    break;
                };
                let value = if base == EnumBase::U8 {
                    raw as u64 as i128
                } else {
                    raw as i128
                };
                if values.insert(field.name.clone(), value).is_some() {
                    valid = false;
                    break;
                }
                members.push((field.name.clone(), value));
            }
            if valid {
                result.insert(
                    full_name.clone(),
                    Rc::new(EnumInfo {
                        file: String::new(),
                        offset: 0,
                        namespace: definition.namespace.clone(),
                        name: definition.name.clone(),
                        is_public: true,
                        is_flags: false,
                        flags_name: None,
                        flags_imports: Vec::new(),
                        is_external: true,
                        external_identity: Some(identity),
                        base,
                        members,
                        values,
                    }),
                );
            }
        }
        result
    }

    /// Finds the open generic definition `Name\`arity` in the reference set.
    fn find_generic(&self, name: &str, arity: usize) -> Option<&(usize, Entry<TypeDef>)> {
        let short_name = name.rsplit('.').next().unwrap_or(name);
        let wanted = format!("{short_name}`{arity}");
        self.types
            .iter()
            .find(|(full_name, _)| full_name.rsplit('.').next() == Some(wanted.as_str()))
            .map(|(_, entry)| entry)
    }

    fn find(&self, full_name: &str) -> Option<&(usize, Entry<TypeDef>)> {
        if let Some(entry) = self.types.get(full_name) {
            return Some(entry);
        }
        let short = full_name.rsplit('.').next().unwrap_or(full_name);
        let candidates = self.by_short_name.get(short)?;
        if candidates.len() == 1 {
            return self.types.get(&candidates[0]);
        }
        None
    }

    fn method(
        &self,
        type_full_name: &str,
        name: &str,
        arity: usize,
    ) -> Option<(usize, Entry<Method>)> {
        let (_, entry) = self.find(type_full_name)?;
        let full_name = entry.get().full_name();

        let candidates = self.methods.get(&full_name)?;
        let found = candidates.iter().find(|(_, method)| {
            let method = method.get();
            method.name == name && method.signature.params.len() == arity
        })?;
        Some((found.0, found.1.clone()))
    }

    fn field_in_chain(&self, type_full_name: &str, name: &str) -> Option<(usize, Entry<Field>)> {
        let mut current = Some(type_full_name.to_string());
        let mut depth = 0usize;
        while let Some(type_name) = current {
            if depth > 12 {
                return None;
            }
            depth += 1;
            let entry = self.find(&type_name).map(|(_, entry)| entry.clone())?;
            let full_name = entry.get().full_name();
            if let Some(found) = self.fields.get(&full_name).and_then(|candidates| {
                candidates
                    .iter()
                    .find(|(_, field)| field.get().name == name)
            }) {
                return Some((found.0, found.1.clone()));
            }
            let guard = entry.get();
            let base = guard.extends.as_ref().and_then(|extends| match extends {
                TypeDefOrRef::TypeDef(def) => Some(def.get().full_name()),
                TypeDefOrRef::TypeRef(reference) => Some(reference.get().full_name()),
                TypeDefOrRef::TypeSpec(_) => None,
            });
            drop(guard);
            current = base;
        }
        None
    }

    /// Walks the `extends` chain of a referenced type so inherited methods and
    /// constructors resolve the way the runtime would see them.
    fn method_in_chain(
        &self,
        type_full_name: &str,
        name: &str,
        arity: usize,
    ) -> Option<(usize, Entry<Method>)> {
        let mut current = Some(type_full_name.to_string());
        let mut depth = 0usize;
        while let Some(type_name) = current {
            if depth > 12 {
                return None;
            }
            depth += 1;
            if let Some(found) = self.method(&type_name, name, arity) {
                return Some(found);
            }
            let entry = self.find(&type_name)?.1.clone();
            let guard = entry.get();
            let base = guard.extends.as_ref().and_then(|extends| match extends {
                TypeDefOrRef::TypeDef(def) => Some(def.get().full_name()),
                TypeDefOrRef::TypeRef(reference) => Some(reference.get().full_name()),
                TypeDefOrRef::TypeSpec(_) => None,
            });
            drop(guard);
            current = base;
        }
        None
    }
}

// ---------------------------------------------------------------- parsing

struct Parser<'a> {
    enums: EnumValues,
    structs: &'a HashMap<String, bool>,
    current_namespace: String,
    root_imports: EnumImports,
    scope_imports: EnumImports,
    file: &'a str,
    source: &'a str,
    tokens: Vec<syntax::Token>,
    position: usize,
}

type EnumValues = HashMap<String, Rc<EnumInfo>>;

#[derive(Debug, Clone, Default)]
struct EnumImports {
    namespaces: Vec<String>,
    aliases: HashMap<String, String>,
}

fn find_enum<'a>(
    enums: &'a EnumValues,
    name: &str,
    namespace: &str,
    imports: &EnumImports,
) -> Option<&'a EnumInfo> {
    let mut parts = name.splitn(2, '.');
    let root = parts.next()?;
    if let Some(alias) = imports.aliases.get(root) {
        let expanded = match parts.next() {
            Some(rest) => format!("{alias}.{rest}"),
            None => alias.clone(),
        };
        return enums.get(&expanded).map(Rc::as_ref);
    }
    let mut scope = namespace;
    while !scope.is_empty() {
        if let Some(found) = enums.get(&format!("{scope}.{name}")) {
            return Some(found.as_ref());
        }
        scope = scope.rsplit_once('.').map_or("", |(parent, _)| parent);
    }
    if let Some(found) = enums.get(name) {
        return Some(found.as_ref());
    }
    let mut matches = imports
        .namespaces
        .iter()
        .filter_map(|import| enums.get(&format!("{import}.{name}")).map(Rc::as_ref));
    let first = matches.next()?;
    if matches.any(|other| other.full_name() != first.full_name()) {
        None
    } else {
        Some(first)
    }
}

fn find_struct<'a>(
    structs: &'a HashMap<String, bool>,
    name: &str,
    namespace: &str,
    imports: &EnumImports,
) -> Option<&'a str> {
    let mut parts = name.splitn(2, '.');
    let root = parts.next()?;
    if let Some(alias) = imports.aliases.get(root) {
        let expanded = match parts.next() {
            Some(rest) => format!("{alias}.{rest}"),
            None => alias.clone(),
        };
        return structs
            .get_key_value(&expanded)
            .map(|(name, _)| name.as_str());
    }
    let mut scope = namespace;
    while !scope.is_empty() {
        if let Some((found, _)) = structs.get_key_value(&format!("{scope}.{name}")) {
            return Some(found);
        }
        scope = scope.rsplit_once('.').map_or("", |(parent, _)| parent);
    }
    if let Some((found, _)) = structs.get_key_value(name) {
        return Some(found);
    }
    let mut matches = imports.namespaces.iter().filter_map(|import| {
        structs
            .get_key_value(&format!("{import}.{name}"))
            .map(|(name, _)| name.as_str())
    });
    let first = matches.next()?;
    if matches.any(|other| other != first) {
        None
    } else {
        Some(first)
    }
}

#[derive(Debug, Clone, Copy)]
enum IntegerLiteralType {
    I32,
    U32,
    I64,
    U64,
}

#[derive(Debug)]
enum EnumConstantExpr {
    Int(i32),
    Name(String),
    Negate(Box<EnumConstantExpr>),
    Complement(Box<EnumConstantExpr>),
    Binary(EnumConstantOp, Box<EnumConstantExpr>, Box<EnumConstantExpr>),
}

#[derive(Debug, Clone, Copy)]
enum EnumConstantOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    And,
    Or,
    Xor,
    ShiftLeft,
    ShiftRight,
}

#[derive(Debug)]
enum EnumInitializer {
    Literal(i128),
    Expression(EnumConstantExpr),
    Implicit,
}

#[derive(Debug)]
struct PendingEnumMember {
    name: String,
    initializer: EnumInitializer,
}

fn parse_file(
    file: &str,
    source: &str,
    enums: &EnumValues,
    structs: &HashMap<String, bool>,
) -> Result<(Vec<TypeDecl>, EnumValues), CompileError> {
    let tokens = syntax::lex(source).map_err(|error: SyntaxError| {
        CompileError::new(file, source, error.offset, error.message)
    })?;
    let mut parser = Parser {
        enums: enums.clone(),
        structs,
        current_namespace: String::new(),
        root_imports: EnumImports::default(),
        scope_imports: EnumImports::default(),
        file,
        source,
        tokens,
        position: 0,
    };
    let types = parser.parse_unit()?;
    let enums = std::mem::take(&mut parser.enums);
    Ok((types, enums))
}

impl<'a> Parser<'a> {
    fn token(&self) -> syntax::Token {
        self.tokens[self.position.min(self.tokens.len() - 1)]
    }

    fn text(&self) -> &'a str {
        let token = self.token();
        &self.source[token.start..token.end]
    }

    fn peek(&self, offset: usize) -> Option<&'a str> {
        let index = self.position + offset;
        if index >= self.tokens.len() {
            return None;
        }
        let token = self.tokens[index];
        Some(&self.source[token.start..token.end])
    }

    fn bump(&mut self) {
        if self.position + 1 < self.tokens.len() {
            self.position += 1;
        }
    }

    fn at_end(&self) -> bool {
        self.token().kind == syntax::TokenKind::EndOfFile
    }

    fn check(&self, text: &str) -> bool {
        self.text() == text
    }

    fn eat(&mut self, text: &str) -> bool {
        if self.check(text) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error(&self, message: impl Into<String>) -> CompileError {
        let mut error = CompileError::new(self.file, self.source, self.token().start, message);
        error.syntax = true;
        error
    }

    fn expect(&mut self, text: &str) -> Result<(), CompileError> {
        if self.eat(text) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{text}', found '{}'", self.text())))
        }
    }

    fn identifier(&mut self) -> Result<String, CompileError> {
        if self.token().kind == syntax::TokenKind::Identifier {
            let name = self.text().to_string();
            self.bump();
            Ok(name)
        } else {
            Err(self.error(format!("expected an identifier, found '{}'", self.text())))
        }
    }

    fn skip_attribute_lists(&mut self) -> Result<(), CompileError> {
        while self.check("[") {
            let mut depth = 0usize;
            loop {
                if self.at_end() {
                    return Err(self.error("unterminated attribute"));
                }
                if self.check("[") {
                    depth += 1;
                } else if self.check("]") {
                    depth -= 1;
                }
                self.bump();
                if depth == 0 {
                    break;
                }
            }
        }
        Ok(())
    }

    fn parse_unit(&mut self) -> Result<Vec<TypeDecl>, CompileError> {
        let mut types = Vec::new();
        let mut namespace = String::new();
        let mut type_access = None;
        let mut type_modifiers = Vec::new();
        let mut type_attributes = String::new();
        while !self.at_end() {
            let attribute_start = self.position;
            self.skip_attribute_lists()?;
            for token in &self.tokens[attribute_start..self.position] {
                type_attributes.push_str(&self.source[token.start..token.end]);
            }
            if self.eat(";") {
                continue;
            }
            let text = self.text().to_string();
            match text.as_str() {
                "using" => {
                    self.bump();
                    let mut parts = Vec::new();
                    while !self.at_end() && !self.check(";") {
                        parts.push(self.text().to_string());
                        self.bump();
                    }
                    self.expect(";")?;
                    let imports = if namespace.is_empty() {
                        &mut self.root_imports
                    } else {
                        &mut self.scope_imports
                    };
                    if parts.first().is_some_and(|part| part != "static") {
                        if parts.get(1).is_some_and(|part| part == "=") {
                            if parts.len() > 2 {
                                imports
                                    .aliases
                                    .insert(parts[0].clone(), parts[2..].join(""));
                            }
                        } else {
                            imports.namespaces.push(parts.join(""));
                        }
                    }
                }
                "namespace" => {
                    self.bump();
                    namespace = self.parse_qualified_name()?;
                    self.current_namespace.clone_from(&namespace);
                    self.scope_imports = EnumImports::default();
                    if self.eat(";") {
                        continue;
                    }
                    self.expect("{")?;
                }
                "public" | "internal" => {
                    if type_access.replace(text == "public").is_some() {
                        return Err(self.error("duplicate type accessibility"));
                    }
                    self.bump();
                }
                "private" | "protected" => {
                    return Err(self.error("top-level types cannot be private or protected"));
                }
                "static" | "sealed" | "abstract" | "partial" | "unsafe" | "readonly" => {
                    type_modifiers.push(text);
                    self.bump();
                }
                "enum" => {
                    if !type_modifiers.is_empty() {
                        return Err(self.error("invalid enum modifier"));
                    }
                    let flags_name = match type_attributes.as_str() {
                        "" => None,
                        "[System.Flags]"
                        | "[System.FlagsAttribute]"
                        | "[System.Flags()]"
                        | "[System.FlagsAttribute()]" => {
                            if self.enum_imports().aliases.contains_key("System") {
                                return Err(self.error(
                                    "System alias prevents resolving the framework Flags attribute",
                                ));
                            }
                            Some("System.Flags".to_string())
                        }
                        "[global::System.Flags]"
                        | "[global::System.FlagsAttribute]"
                        | "[global::System.Flags()]"
                        | "[global::System.FlagsAttribute()]" => Some("System.Flags".to_string()),
                        "[Flags]" | "[Flags()]" | "[FlagsAttribute]" | "[FlagsAttribute()]" => {
                            let spelling = if type_attributes.starts_with("[FlagsAttribute") {
                                "FlagsAttribute"
                            } else {
                                "Flags"
                            };
                            let imports = self.enum_imports();
                            if let Some(alias) = imports.aliases.get(spelling) {
                                if !matches!(
                                    alias.as_str(),
                                    "System.FlagsAttribute" | "global::System.FlagsAttribute"
                                ) {
                                    return Err(self.error(
                                        "Flags alias does not name the framework attribute",
                                    ));
                                }
                                if alias == "System.FlagsAttribute"
                                    && imports.aliases.contains_key("System")
                                {
                                    return Err(self.error(
                                        "System alias prevents resolving the framework Flags attribute",
                                    ));
                                }
                            } else if !imports.namespaces.iter().any(|name| name == "System")
                                && namespace != "System"
                                && !namespace.starts_with("System.")
                            {
                                return Err(self.error("Flags attribute is not in scope"));
                            }
                            Some(spelling.to_string())
                        }
                        _ => return Err(self.error("enum attribute is not supported")),
                    };
                    type_attributes.clear();
                    self.parse_enum_declaration(
                        &namespace,
                        type_access.take().unwrap_or(false),
                        flags_name,
                    )?;
                }
                "class" | "struct" | "interface" | "record" => {
                    if text == "record" {
                        return Err(
                            self.error(format!("'{text}' declarations are not supported yet"))
                        );
                    }
                    if text == "struct"
                        && (!type_modifiers.is_empty() || !type_attributes.is_empty())
                    {
                        return Err(
                            self.error("struct attributes and modifiers are not supported yet")
                        );
                    }
                    let is_static = type_modifiers.iter().any(|modifier| modifier == "static");
                    if is_static
                        && (text != "class"
                            || type_modifiers
                                .iter()
                                .filter(|modifier| *modifier == "static")
                                .count()
                                != 1
                            || type_modifiers.iter().any(|modifier| {
                                matches!(modifier.as_str(), "abstract" | "sealed" | "readonly")
                            }))
                    {
                        return Err(self.error("invalid static type declaration"));
                    }
                    type_modifiers.clear();
                    type_attributes.clear();
                    types.push(self.parse_type(
                        &namespace,
                        type_access.take().unwrap_or(false),
                        is_static,
                    )?);
                    if self.check("}") && !namespace.is_empty() {
                        self.bump();
                        namespace.clear();
                        self.current_namespace.clear();
                        self.scope_imports = EnumImports::default();
                    }
                }
                "}" => {
                    if namespace.is_empty() {
                        return Err(self.error("unexpected '}'"));
                    }
                    self.bump();
                    namespace.clear();
                    self.current_namespace.clear();
                    self.scope_imports = EnumImports::default();
                }
                _ => return Err(self.error(format!("unexpected top-level token '{text}'"))),
            }
        }
        Ok(types)
    }

    fn parse_new_type(&mut self) -> Result<Ty, CompileError> {
        let name = self.parse_qualified_name()?;
        if self.check("<") {
            self.bump();
            let mut args = Vec::new();
            loop {
                args.push(self.parse_type_reference()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(">")?;
            return Ok(Ty::Generic(name, args));
        }
        Ok(match name.as_str() {
            "int" => Ty::Int,
            "float" => Ty::Float,
            "bool" => Ty::Bool,
            "string" | "String" => Ty::String,
            "object" => Ty::Object,
            other => self.named_type(other)?,
        })
    }

    fn enum_info(&self, name: &str) -> Option<&EnumInfo> {
        find_enum(
            &self.enums,
            name,
            &self.current_namespace,
            &self.enum_imports(),
        )
    }

    fn struct_name(&self, name: &str) -> Option<&str> {
        if self.structs.is_empty() {
            return None;
        }
        find_struct(
            self.structs,
            name,
            &self.current_namespace,
            &self.enum_imports(),
        )
    }

    fn named_type(&self, name: &str) -> Result<Ty, CompileError> {
        match (self.enum_info(name), self.struct_name(name)) {
            (Some(_), Some(_)) => Err(self.error(format!(
                "type name '{name}' resolves to both an enum and a struct"
            ))),
            (Some(info), None) => Ok(Ty::Enum(info.full_name(), info.base)),
            (None, Some(full_name)) => Ok(Ty::Struct(full_name.to_string())),
            (None, None) => Ok(Ty::Class(name.to_string())),
        }
    }

    fn enum_imports(&self) -> EnumImports {
        let mut imports = self.root_imports.clone();
        imports
            .namespaces
            .extend(self.scope_imports.namespaces.iter().cloned());
        imports.aliases.extend(self.scope_imports.aliases.clone());
        imports
    }

    fn parse_enum_literal(&mut self, base: EnumBase) -> Result<i128, CompileError> {
        let negative = self.eat("-");
        if !negative {
            self.eat("+");
        }
        if self.token().kind != syntax::TokenKind::Number {
            return Err(self.error("enum initializer must be a supported integral constant"));
        }
        let text = self.text().replace('_', "");
        self.bump();
        let suffix_start = text.trim_end_matches(['u', 'U', 'l', 'L']).len();
        let digits = &text[..suffix_start];
        let suffix = text[suffix_start..].to_ascii_lowercase();
        if !matches!(suffix.as_str(), "" | "u" | "l" | "ul" | "lu") {
            return Err(self.error("invalid integral literal suffix"));
        }
        let (radix, digits) = if let Some(rest) = digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X"))
        {
            (16, rest)
        } else if let Some(rest) = digits
            .strip_prefix("0b")
            .or_else(|| digits.strip_prefix("0B"))
        {
            (2, rest)
        } else {
            (10, digits)
        };
        let magnitude = u64::from_str_radix(digits, radix)
            .map_err(|_| self.error("enum initializer is not a supported integral literal"))?;
        let mut kind = match suffix.as_str() {
            "" if magnitude <= i32::MAX as u64 => IntegerLiteralType::I32,
            "" if magnitude <= u32::MAX as u64 => IntegerLiteralType::U32,
            "" if magnitude <= i64::MAX as u64 => IntegerLiteralType::I64,
            "" => IntegerLiteralType::U64,
            "u" if magnitude <= u32::MAX as u64 => IntegerLiteralType::U32,
            "u" => IntegerLiteralType::U64,
            "l" if magnitude <= i64::MAX as u64 => IntegerLiteralType::I64,
            "l" | "ul" | "lu" => IntegerLiteralType::U64,
            _ => unreachable!(),
        };
        let value = if negative {
            if magnitude == (1u64 << 31) && suffix.is_empty() {
                kind = IntegerLiteralType::I32;
            } else if magnitude == (1u64 << 63) && matches!(suffix.as_str(), "" | "l") {
                kind = IntegerLiteralType::I64;
            } else {
                kind = match kind {
                    IntegerLiteralType::I32 => IntegerLiteralType::I32,
                    IntegerLiteralType::U32 | IntegerLiteralType::I64 => IntegerLiteralType::I64,
                    IntegerLiteralType::U64 => {
                        return Err(self.error("cannot negate an unsigned long constant"));
                    }
                };
            }
            -(magnitude as i128)
        } else {
            magnitude as i128
        };
        let (min, max) = base.bounds();
        if value < min || value > max {
            return Err(self.error("enum initializer is outside the underlying type range"));
        }
        let implicit = match kind {
            IntegerLiteralType::I32 => true,
            IntegerLiteralType::U32 => matches!(base, EnumBase::U4 | EnumBase::I8 | EnumBase::U8),
            IntegerLiteralType::I64 => base == EnumBase::I8 || (base == EnumBase::U8 && value >= 0),
            IntegerLiteralType::U64 => base == EnumBase::U8,
        };
        if !implicit {
            return Err(
                self.error("enum initializer has no implicit conversion to its underlying type")
            );
        }
        Ok(value)
    }

    fn parse_enum_constant_unary(&mut self) -> Result<EnumConstantExpr, CompileError> {
        if self.check("-")
            && self
                .tokens
                .get(self.position + 1)
                .is_some_and(|token| token.kind == syntax::TokenKind::Number)
        {
            return Ok(EnumConstantExpr::Int(
                self.parse_enum_literal(EnumBase::I4)? as i32,
            ));
        }
        if self.eat("-") {
            return Ok(EnumConstantExpr::Negate(Box::new(
                self.parse_enum_constant_unary()?,
            )));
        }
        if self.eat("+") {
            return self.parse_enum_constant_unary();
        }
        if self.eat("~") {
            return Ok(EnumConstantExpr::Complement(Box::new(
                self.parse_enum_constant_unary()?,
            )));
        }
        if self.eat("(") {
            let value = self.parse_enum_constant_expression(1)?;
            self.expect(")")?;
            return Ok(value);
        }
        if self.token().kind == syntax::TokenKind::Number {
            return Ok(EnumConstantExpr::Int(
                self.parse_enum_literal(EnumBase::I4)? as i32,
            ));
        }
        if self.token().kind == syntax::TokenKind::Identifier {
            return Ok(EnumConstantExpr::Name(self.identifier()?));
        }
        Err(self.error("unsupported enum constant expression"))
    }

    fn parse_enum_constant_expression(
        &mut self,
        minimum: u8,
    ) -> Result<EnumConstantExpr, CompileError> {
        let mut left = self.parse_enum_constant_unary()?;
        loop {
            let operator = match self.text() {
                "|" => Some((EnumConstantOp::Or, 1, 1)),
                "^" => Some((EnumConstantOp::Xor, 2, 1)),
                "&" => Some((EnumConstantOp::And, 3, 1)),
                "<<" => Some((EnumConstantOp::ShiftLeft, 4, 1)),
                ">" if self.peek(1) == Some(">")
                    && self.tokens[self.position].end == self.tokens[self.position + 1].start =>
                {
                    Some((EnumConstantOp::ShiftRight, 4, 2))
                }
                "+" => Some((EnumConstantOp::Add, 5, 1)),
                "-" => Some((EnumConstantOp::Subtract, 5, 1)),
                "*" => Some((EnumConstantOp::Multiply, 6, 1)),
                "/" => Some((EnumConstantOp::Divide, 6, 1)),
                "%" => Some((EnumConstantOp::Remainder, 6, 1)),
                _ => None,
            };
            let Some((operator, precedence, length)) = operator else {
                break;
            };
            if precedence < minimum {
                break;
            }
            for _ in 0..length {
                self.bump();
            }
            let right = self.parse_enum_constant_expression(precedence + 1)?;
            left = EnumConstantExpr::Binary(operator, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn evaluate_enum_expression(
        &self,
        expression: &EnumConstantExpr,
        members: &[PendingEnumMember],
        names: &HashMap<String, usize>,
        base: EnumBase,
        values: &mut [Option<i128>],
        visiting: &mut [bool],
    ) -> Result<i32, CompileError> {
        let evaluate =
            |expression: &EnumConstantExpr, values: &mut [Option<i128>], visiting: &mut [bool]| {
                self.evaluate_enum_expression(expression, members, names, base, values, visiting)
            };
        let value = match expression {
            EnumConstantExpr::Int(value) => return Ok(*value),
            EnumConstantExpr::Name(name) => {
                let index = *names
                    .get(name)
                    .ok_or_else(|| self.error(format!("unknown enum member '{name}'")))?;
                let value =
                    self.evaluate_enum_member(index, members, names, base, values, visiting)?;
                return i32::try_from(value).map_err(|_| {
                    self.error("enum constant expression exceeds supported int range")
                });
            }
            EnumConstantExpr::Negate(inner) => evaluate(inner, values, visiting)?.checked_neg(),
            EnumConstantExpr::Complement(inner) => Some(!evaluate(inner, values, visiting)?),
            EnumConstantExpr::Binary(operator, left, right) => {
                let left = evaluate(left, values, visiting)?;
                let right = evaluate(right, values, visiting)?;
                match operator {
                    EnumConstantOp::Add => left.checked_add(right),
                    EnumConstantOp::Subtract => left.checked_sub(right),
                    EnumConstantOp::Multiply => left.checked_mul(right),
                    EnumConstantOp::Divide => left.checked_div(right),
                    EnumConstantOp::Remainder => left.checked_rem(right),
                    EnumConstantOp::And => Some(left & right),
                    EnumConstantOp::Or => Some(left | right),
                    EnumConstantOp::Xor => Some(left ^ right),
                    EnumConstantOp::ShiftLeft => Some(left.wrapping_shl((right as u32) & 31)),
                    EnumConstantOp::ShiftRight => Some(left >> ((right as u32) & 31)),
                }
            }
        };
        value.ok_or_else(|| self.error("invalid or overflowing enum constant expression"))
    }

    fn evaluate_enum_member(
        &self,
        index: usize,
        members: &[PendingEnumMember],
        names: &HashMap<String, usize>,
        base: EnumBase,
        values: &mut [Option<i128>],
        visiting: &mut [bool],
    ) -> Result<i128, CompileError> {
        if let Some(value) = values[index] {
            return Ok(value);
        }
        if visiting[index] {
            return Err(self.error(format!("circular enum constant '{}'", members[index].name)));
        }
        visiting[index] = true;
        let value = match &members[index].initializer {
            EnumInitializer::Literal(value) => *value,
            EnumInitializer::Implicit if index == 0 => 0,
            EnumInitializer::Implicit => self
                .evaluate_enum_member(index - 1, members, names, base, values, visiting)?
                .checked_add(1)
                .ok_or_else(|| self.error("enum auto-increment overflow"))?,
            EnumInitializer::Expression(EnumConstantExpr::Name(name)) => {
                let target = *names
                    .get(name)
                    .ok_or_else(|| self.error(format!("unknown enum member '{name}'")))?;
                self.evaluate_enum_member(target, members, names, base, values, visiting)?
            }
            EnumInitializer::Expression(expression) => {
                if matches!(base, EnumBase::U4 | EnumBase::I8 | EnumBase::U8) {
                    return Err(self.error(
                        "enum constant arithmetic for this underlying type is not supported",
                    ));
                }
                self.evaluate_enum_expression(expression, members, names, base, values, visiting)?
                    as i128
            }
        };
        let (min, max) = base.bounds();
        if value < min || value > max {
            return Err(self.error("enum constant is outside the underlying type range"));
        }
        visiting[index] = false;
        values[index] = Some(value);
        Ok(value)
    }

    fn parse_enum_declaration(
        &mut self,
        namespace: &str,
        is_public: bool,
        flags_name: Option<String>,
    ) -> Result<(), CompileError> {
        let offset = self.token().start;
        self.bump(); // enum
        let name = self.identifier()?;
        let full_name = if namespace.is_empty() {
            name.clone()
        } else {
            format!("{namespace}.{name}")
        };
        if let Some(existing) = self.enums.get(&full_name)
            && !existing.is_external
            && (existing.file != self.file || existing.offset != offset)
        {
            return Err(self.error(format!("duplicate enum type '{name}'")));
        }
        let base = if self.eat(":") {
            let underlying = self.parse_qualified_name()?;
            EnumBase::parse(&underlying)
                .ok_or_else(|| self.error(format!("invalid enum underlying type '{underlying}'")))?
        } else {
            EnumBase::I4
        };
        self.expect("{")?;
        let mut names = HashMap::new();
        let mut pending = Vec::new();
        while !self.check("}") && !self.at_end() {
            let attribute_start = self.position;
            self.skip_attribute_lists()?;
            if self.position != attribute_start {
                return Err(self.error("enum member attributes are not supported"));
            }
            let member = self.identifier()?;
            if names.insert(member.clone(), pending.len()).is_some() {
                return Err(self.error(format!("duplicate enum member '{member}'")));
            }
            let initializer = if self.eat("=") {
                let numeric_offset = usize::from(self.check("+") || self.check("-"));
                if self
                    .tokens
                    .get(self.position + numeric_offset)
                    .is_some_and(|token| token.kind == syntax::TokenKind::Number)
                    && matches!(self.peek(numeric_offset + 1), Some(",") | Some("}"))
                {
                    EnumInitializer::Literal(self.parse_enum_literal(base)?)
                } else {
                    EnumInitializer::Expression(self.parse_enum_constant_expression(1)?)
                }
            } else {
                EnumInitializer::Implicit
            };
            pending.push(PendingEnumMember {
                name: member,
                initializer,
            });
            if !self.eat(",") {
                break;
            }
        }
        self.eat(",");
        self.expect("}")?;
        let mut resolved = vec![None; pending.len()];
        let mut visiting = vec![false; pending.len()];
        for index in 0..pending.len() {
            self.evaluate_enum_member(index, &pending, &names, base, &mut resolved, &mut visiting)?;
        }
        let members: Vec<(String, i128)> = pending
            .into_iter()
            .zip(resolved)
            .map(|(member, value)| (member.name, value.expect("enum member was resolved")))
            .collect();
        let values = members.iter().cloned().collect();
        let imports = self.enum_imports();
        let mut flags_imports = imports.namespaces;
        if flags_name
            .as_deref()
            .is_some_and(|name| imports.aliases.contains_key(name))
        {
            flags_imports.push("System".to_string());
        }
        self.enums.insert(
            full_name,
            Rc::new(EnumInfo {
                file: self.file.to_string(),
                offset,
                namespace: namespace.to_string(),
                name,
                is_public,
                is_flags: flags_name.is_some(),
                flags_name,
                flags_imports,
                is_external: false,
                external_identity: None,
                base,
                members,
                values,
            }),
        );
        Ok(())
    }

    fn parse_qualified_name(&mut self) -> Result<String, CompileError> {
        let mut name = self.identifier_like()?;
        while self.eat(".") {
            name.push('.');
            name.push_str(&self.identifier_like()?);
        }
        Ok(name)
    }

    fn identifier_like(&mut self) -> Result<String, CompileError> {
        let text = self.text().to_string();
        if self.token().kind == syntax::TokenKind::Identifier
            || self.token().kind == syntax::TokenKind::Keyword
        {
            self.bump();
            Ok(text)
        } else {
            Err(self.error(format!("expected a name, found '{text}'")))
        }
    }

    fn parse_type(
        &mut self,
        namespace: &str,
        is_public: bool,
        is_static: bool,
    ) -> Result<TypeDecl, CompileError> {
        let class_offset = self.token().start;
        let is_interface = self.check("interface");
        let is_struct = self.check("struct");
        self.bump(); // class / struct / interface / record
        if self.check("record") {
            self.bump();
        }
        let name = self.identifier()?;
        let mut base = None;
        let mut interfaces = Vec::new();
        if self.eat(":") {
            if is_struct {
                return Err(self.error("struct interfaces are not supported yet"));
            }
            if is_static {
                return Err(self.error("static classes cannot declare base types or interfaces"));
            }
            // The first entry is the base class, the rest are interfaces.
            loop {
                let entry = self.parse_qualified_name()?;
                if self.check("<") {
                    self.skip_balanced("<", ">")?;
                }
                if base.is_none() {
                    base = Some(entry);
                } else {
                    interfaces.push(entry);
                }
                if !self.eat(",") {
                    break;
                }
            }
        }
        if is_interface {
            // Interfaces have no base class; every listed type is a base interface.
            interfaces.extend(base.take());
        }
        let mut decl = TypeDecl {
            file: self.file.to_string(),
            offset: class_offset,
            namespace: namespace.to_string(),
            enum_imports: self.enum_imports(),
            name,
            base,
            interfaces,
            is_interface,
            is_struct,
            is_static,
            is_public,
            static_ctor: None,
            fields: Vec::new(),
            methods: Vec::new(),
            properties: Vec::new(),
        };
        self.expect("{")?;
        while !self.check("}") && !self.at_end() {
            if is_struct {
                self.expect("public")?;
                self.expect("int")?;
                let name = self.identifier()?;
                self.expect(";")?;
                decl.fields.push(FieldDecl {
                    name,
                    ty: Ty::Int,
                    is_static: false,
                    is_readonly: false,
                    is_const: false,
                    visibility: Visibility::Public,
                    init: None,
                });
            } else {
                self.parse_member(&mut decl)?;
            }
        }
        self.expect("}")?;
        Ok(decl)
    }

    fn skip_balanced(&mut self, open: &str, close: &str) -> Result<(), CompileError> {
        let mut depth = 0usize;
        loop {
            if self.at_end() {
                return Err(self.error(format!("unterminated '{open}'")));
            }
            if self.check(open) {
                depth += 1;
            } else if self.check(close) {
                depth -= 1;
            }
            self.bump();
            if depth == 0 {
                return Ok(());
            }
        }
    }

    fn parse_member(&mut self, decl: &mut TypeDecl) -> Result<(), CompileError> {
        self.skip_attribute_lists()?;
        let mut is_static = false;
        let mut is_readonly = false;
        let mut is_const = false;
        let mut access_bits = 0;
        let mut virtual_kind = VirtualKind::None;
        let mut invalid_constructor_modifier = false;
        loop {
            match self.text() {
                "public" | "private" | "internal" | "protected" => {
                    let bit = match self.text() {
                        "public" => 1,
                        "private" => 2,
                        "internal" => 4,
                        _ => 8,
                    };
                    if access_bits & bit != 0 {
                        return Err(self.error("duplicate member accessibility"));
                    }
                    access_bits |= bit;
                    self.bump();
                }
                "static" => {
                    if is_static {
                        return Err(self.error("duplicate static modifier"));
                    }
                    is_static = true;
                    self.bump();
                }
                "virtual" => {
                    virtual_kind = VirtualKind::Virtual;
                    self.bump();
                }
                "override" => {
                    virtual_kind = VirtualKind::Override;
                    self.bump();
                }
                "readonly" => {
                    if is_readonly {
                        return Err(self.error("duplicate readonly modifier"));
                    }
                    is_readonly = true;
                    self.bump();
                }
                "const" => {
                    if is_const {
                        return Err(self.error("duplicate const modifier"));
                    }
                    is_const = true;
                    self.bump();
                }
                "sealed" | "unsafe" | "extern" | "async" | "partial" => {
                    if self.text() != "unsafe" {
                        invalid_constructor_modifier = true;
                    }
                    self.bump();
                }
                _ => break,
            }
        }
        if is_const {
            if is_static || is_readonly {
                return Err(self.error("const fields cannot be static or readonly"));
            }
            is_static = true;
        }
        let visibility = Visibility::from_bits(
            access_bits,
            if decl.is_interface {
                Visibility::Public
            } else {
                Visibility::Private
            },
        )
        .ok_or_else(|| self.error("invalid member accessibility"))?;
        if decl.is_interface && visibility != Visibility::Public {
            return Err(self.error("non-public interface members are not supported"));
        }
        if self.check("}") || self.at_end() {
            return Ok(());
        }

        // Constructor: `Name(...)`.
        if self.token().kind == syntax::TokenKind::Identifier && self.peek(1) == Some("(") {
            let name = self.identifier()?;
            if name != decl.name {
                return Err(self.error("constructor name must match its declaring type"));
            }
            if invalid_constructor_modifier
                || is_readonly
                || is_const
                || virtual_kind != VirtualKind::None
            {
                return Err(self.error("invalid constructor modifier"));
            }
            if decl.is_interface && !is_static {
                return Err(self.error("interfaces cannot declare instance constructors"));
            }
            if is_static {
                if access_bits != 0 {
                    return Err(
                        self.error("static constructors cannot have access or virtual modifiers")
                    );
                }
                if decl.static_ctor.is_some() {
                    return Err(self.error("duplicate static constructor"));
                }
                self.expect("(")?;
                self.expect(")")?;
                if self.check(":") {
                    return Err(self.error("static constructors cannot have initializers"));
                }
                let body = if self.eat("=>") {
                    self.parse_expression_body(true)?
                } else {
                    self.parse_block()?
                };
                decl.static_ctor = Some(body);
                return Ok(());
            }
            if decl.is_static {
                return Err(self.error("static classes cannot declare instance constructors"));
            }
            let params = self.parse_parameter_list()?;
            let mut base_args = None;
            if self.eat(":") {
                // `base(...)` and `this(...)` share the argument list here;
                // initializer chaining is only partially modelled.
                if self.check("base") || self.check("this") {
                    self.bump();
                }
                self.expect("(")?;
                let mut args = Vec::new();
                if !self.check(")") {
                    loop {
                        args.push(self.parse_expression()?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                }
                self.expect(")")?;
                base_args = Some(args);
            }
            let body = if self.eat("=>") {
                self.parse_expression_body(true)?
            } else {
                self.parse_block()?
            };
            decl.methods.push(MethodDecl {
                name,
                is_static: false,
                visibility,
                ret: Ty::Void,
                params,
                body: Some(body),
                is_ctor: true,
                virtual_kind: VirtualKind::None,
                base_args,
            });
            return Ok(());
        }

        if decl.is_static && !is_static {
            return Err(self.error("static classes cannot declare instance members"));
        }
        if is_static && virtual_kind != VirtualKind::None {
            return Err(self.error("static members cannot be virtual or override"));
        }

        let ty = self.parse_type_reference()?;
        let name = self.identifier()?;
        if (is_readonly || is_const) && (self.check("=>") || self.check("{") || self.check("(")) {
            return Err(self.error("readonly and const are supported only on fields"));
        }
        if self.eat("=>") {
            if decl.is_interface || ty == Ty::Void {
                return Err(self.error("expression-bodied property is not supported here"));
            }
            let getter = self.parse_expression_body(false)?;
            decl.properties.push(PropertyDecl {
                name,
                ty,
                is_static,
                visibility,
                getter_visibility: None,
                setter_visibility: None,
                has_getter: true,
                has_setter: false,
                getter: Some(getter),
                setter: None,
            });
            return Ok(());
        }
        if self.check("{") {
            // Property with accessors: `int Value { get; set; }`.
            self.bump();
            let mut getter = None;
            let mut setter = None;
            let mut has_getter = false;
            let mut has_setter = false;
            let mut getter_visibility = None;
            let mut setter_visibility = None;
            while !self.check("}") && !self.at_end() {
                self.skip_attribute_lists()?;
                let mut accessor_bits = 0;
                while matches!(self.text(), "public" | "private" | "internal" | "protected") {
                    let bit = match self.text() {
                        "public" => 1,
                        "private" => 2,
                        "internal" => 4,
                        _ => 8,
                    };
                    if accessor_bits & bit != 0 {
                        return Err(self.error("duplicate accessor accessibility"));
                    }
                    accessor_bits |= bit;
                    self.bump();
                }
                let accessor_visibility = if accessor_bits == 0 {
                    None
                } else {
                    let access = Visibility::from_bits(accessor_bits, visibility)
                        .ok_or_else(|| self.error("invalid accessor accessibility"))?;
                    if !access.more_restrictive_than(visibility) {
                        return Err(
                            self.error("accessor must be more restrictive than its property")
                        );
                    }
                    Some(access)
                };
                let accessor = self.identifier_like()?;
                let body = if self.eat(";") {
                    None
                } else if self.eat("=>") {
                    if decl.is_interface {
                        return Err(self.error("interface accessor bodies are not supported"));
                    }
                    Some(self.parse_expression_body(accessor != "get")?)
                } else {
                    Some(self.parse_block()?)
                };
                match accessor.as_str() {
                    "get" => {
                        if has_getter {
                            return Err(self.error("duplicate get accessor"));
                        }
                        has_getter = true;
                        getter_visibility = accessor_visibility;
                        getter = body;
                    }
                    "set" => {
                        if has_setter {
                            return Err(self.error("duplicate set accessor"));
                        }
                        has_setter = true;
                        setter_visibility = accessor_visibility;
                        setter = body;
                    }
                    other => {
                        return Err(self.error(format!("unsupported accessor '{other}'")));
                    }
                }
            }
            self.expect("}")?;
            if (getter_visibility.is_some() || setter_visibility.is_some())
                && (!has_getter
                    || !has_setter
                    || (getter_visibility.is_some() && setter_visibility.is_some()))
            {
                return Err(self.error(
                    "accessor accessibility requires both accessors and may occur only once",
                ));
            }
            decl.properties.push(PropertyDecl {
                name,
                ty,
                is_static,
                visibility,
                getter_visibility,
                setter_visibility,
                has_getter,
                has_setter,
                getter,
                setter,
            });
            return Ok(());
        }
        if self.check("(") {
            let params = self.parse_parameter_list()?;
            let body = if self.check("{") {
                Some(self.parse_block()?)
            } else if self.eat("=>") {
                if decl.is_interface {
                    return Err(self.error("interface method bodies are not supported"));
                }
                Some(self.parse_expression_body(ty == Ty::Void)?)
            } else {
                self.expect(";")?;
                None
            };
            decl.methods.push(MethodDecl {
                name,
                is_static,
                visibility,
                ret: ty,
                params,
                body,
                is_ctor: false,
                virtual_kind,
                base_args: None,
            });
            return Ok(());
        }

        // Field declarators, with optional initializers.
        if (is_readonly || is_const)
            && (invalid_constructor_modifier || virtual_kind != VirtualKind::None)
        {
            return Err(self.error("invalid field modifier"));
        }
        if decl.is_interface && !is_static {
            return Err(self.error("interfaces cannot declare instance fields"));
        }
        fn valid_field_type(ty: &Ty) -> bool {
            match ty {
                Ty::Void | Ty::Null => false,
                Ty::Array(element) => valid_field_type(element),
                Ty::Generic(_, arguments) => arguments.iter().all(valid_field_type),
                _ => true,
            }
        }
        if !valid_field_type(&ty) {
            return Err(self.error("invalid field type"));
        }
        let mut current_name = name;
        loop {
            let init = if self.eat("=") {
                Some(self.parse_expression()?)
            } else {
                None
            };
            if is_const && init.is_none() {
                return Err(self.error("constant fields require an initializer"));
            }
            decl.fields.push(FieldDecl {
                name: current_name.clone(),
                ty: ty.clone(),
                is_static,
                is_readonly,
                is_const,
                visibility,
                init,
            });
            if !self.eat(",") {
                break;
            }
            current_name = self.identifier()?;
        }
        self.expect(";")
    }

    fn parse_expression_body(&mut self, is_void: bool) -> Result<Block, CompileError> {
        let statement = if is_void {
            let statement = self.parse_simple_statement()?;
            match &statement {
                Stmt::Assign { .. }
                | Stmt::AssignOp { .. }
                | Stmt::AssignMember { .. }
                | Stmt::AssignMemberOp { .. }
                | Stmt::AssignIndex { .. }
                | Stmt::Expression(
                    Expr::Call { .. }
                    | Expr::New { .. }
                    | Expr::Increment { .. }
                    | Expr::Assign { .. },
                ) => statement,
                _ => {
                    return Err(
                        self.error("expression-bodied void member requires a statement expression")
                    );
                }
            }
        } else {
            let expression = self.parse_expression()?;
            self.expect(";")?;
            Stmt::Return(Some(expression))
        };
        Ok(Block {
            statements: vec![statement],
        })
    }

    fn parse_type_reference(&mut self) -> Result<Ty, CompileError> {
        let name = self.parse_qualified_name()?;
        let mut ty = match name.as_str() {
            "void" => Ty::Void,
            "int" => Ty::Int,
            "float" => Ty::Float,
            "bool" => Ty::Bool,
            "string" | "String" => Ty::String,
            "object" => Ty::Object,
            other => self.named_type(other)?,
        };
        if self.check("<") {
            if matches!(ty, Ty::Struct(_)) {
                return Err(self.error("generic struct declarations are not supported yet"));
            }
            self.bump();
            let mut args = Vec::new();
            loop {
                args.push(self.parse_type_reference()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(">")?;
            ty = Ty::Generic(name.clone(), args);
        }
        while self.eat("[") {
            if matches!(ty, Ty::Struct(_)) {
                return Err(self.error("arrays of struct values are not supported yet"));
            }
            self.expect("]")?;
            ty = Ty::Array(Box::new(ty.clone()));
        }
        if self.eat("?") {
            // Nullable annotations on reference types do not change the emitted
            // signature; Nullable<T> for value types is rejected.
            if !matches!(
                ty,
                Ty::Class(_) | Ty::String | Ty::Array(_) | Ty::Generic(_, _)
            ) {
                return Err(self.error("nullable value types are not supported yet"));
            }
        }
        Ok(ty)
    }

    fn parse_parameter_list(&mut self) -> Result<Vec<(Ty, String)>, CompileError> {
        self.expect("(")?;
        let mut params = Vec::new();
        if !self.check(")") {
            loop {
                let ty = self.parse_type_reference()?;
                let name = self.identifier()?;
                if self.eat("=") {
                    return Err(self.error("optional parameter defaults are not supported yet"));
                }
                params.push((ty, name));
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.expect(")")?;
        Ok(params)
    }

    fn parse_block(&mut self) -> Result<Block, CompileError> {
        self.expect("{")?;
        let mut statements = Vec::new();
        while !self.check("}") && !self.at_end() {
            statements.push(self.parse_statement()?);
        }
        self.expect("}")?;
        Ok(Block { statements })
    }

    fn parse_statement(&mut self) -> Result<Stmt, CompileError> {
        if self.check("{") {
            let block = self.parse_block()?;
            return Ok(Stmt::Block(block));
        }
        if self.eat(";") {
            return Ok(Stmt::Empty);
        }
        match self.text() {
            "if" => {
                self.bump();
                self.expect("(")?;
                let condition = self.parse_expression()?;
                self.expect(")")?;
                let then_branch = self.parse_statement_block()?;
                let else_branch = if self.eat("else") {
                    Some(self.parse_statement_block()?)
                } else {
                    None
                };
                return Ok(Stmt::If {
                    condition,
                    then_branch,
                    else_branch,
                });
            }
            "while" => {
                self.bump();
                self.expect("(")?;
                let condition = self.parse_expression()?;
                self.expect(")")?;
                let body = self.parse_statement_block()?;
                return Ok(Stmt::While { condition, body });
            }
            "do" => {
                self.bump();
                let is_block = self.check("{");
                let body = self.parse_statement_block()?;
                if !is_block && matches!(body.statements.first(), Some(Stmt::Local { .. })) {
                    return Err(self.error("a local declaration cannot be an embedded statement"));
                }
                self.expect("while")?;
                self.expect("(")?;
                let condition = self.parse_expression()?;
                self.expect(")")?;
                self.expect(";")?;
                return Ok(Stmt::DoWhile { body, condition });
            }
            "switch" => {
                self.bump();
                self.expect("(")?;
                let value = self.parse_expression()?;
                self.expect(")")?;
                self.expect("{")?;
                let mut sections = Vec::new();
                while !self.check("}") && !self.at_end() {
                    let mut labels = Vec::new();
                    loop {
                        if self.eat("case") {
                            let value = self.parse_expression()?;
                            self.expect(":")?;
                            labels.push(SwitchLabel::Case(value));
                        } else if self.eat("default") {
                            self.expect(":")?;
                            labels.push(SwitchLabel::Default);
                        } else {
                            break;
                        }
                    }
                    if labels.is_empty() {
                        return Err(self.error("a switch section requires a case or default label"));
                    }
                    let mut statements = Vec::new();
                    while !self.check("case")
                        && !self.check("default")
                        && !self.check("}")
                        && !self.at_end()
                    {
                        statements.push(self.parse_statement()?);
                    }
                    sections.push(SwitchSection {
                        labels,
                        body: Block { statements },
                    });
                }
                self.expect("}")?;
                return Ok(Stmt::Switch { value, sections });
            }
            "for" => {
                self.bump();
                self.expect("(")?;
                let init = if self.check(";") {
                    self.bump();
                    None
                } else {
                    Some(Box::new(self.parse_simple_statement()?))
                };
                let condition = if self.check(";") {
                    None
                } else {
                    Some(self.parse_expression()?)
                };
                self.expect(";")?;
                let step = if self.check(")") {
                    None
                } else {
                    Some(self.parse_expression()?)
                };
                self.expect(")")?;
                let body = self.parse_statement_block()?;
                return Ok(Stmt::For {
                    init,
                    condition,
                    step,
                    body,
                });
            }
            "return" => {
                self.bump();
                if self.eat(";") {
                    return Ok(Stmt::Return(None));
                }
                let value = self.parse_expression()?;
                self.expect(";")?;
                return Ok(Stmt::Return(Some(value)));
            }
            "throw" => {
                self.bump();
                if self.eat(";") {
                    return Ok(Stmt::Throw(None));
                }
                let value = self.parse_expression()?;
                self.expect(";")?;
                return Ok(Stmt::Throw(Some(value)));
            }
            "try" => {
                self.bump();
                let body = self.parse_block()?;
                let mut catches = Vec::new();
                while self.eat("catch") {
                    if catches.last().is_some_and(|clause: &CatchClause| {
                        clause.ty.is_none() && clause.filter.is_none()
                    }) {
                        return Err(self.error("a general catch must be the last catch clause"));
                    }
                    let (ty, name) = if self.eat("(") {
                        let ty = self.parse_type_reference()?;
                        let name = if self.check(")") {
                            None
                        } else {
                            Some(self.identifier()?)
                        };
                        self.expect(")")?;
                        (Some(ty), name)
                    } else {
                        (None, None)
                    };
                    let filter = if self.eat("when") {
                        self.expect("(")?;
                        let condition = self.parse_expression()?;
                        self.expect(")")?;
                        Some(condition)
                    } else {
                        None
                    };
                    let body = self.parse_block()?;
                    catches.push(CatchClause {
                        ty,
                        name,
                        filter,
                        body,
                    });
                }
                let finally = if self.eat("finally") {
                    Some(self.parse_block()?)
                } else {
                    None
                };
                if catches.is_empty() && finally.is_none() {
                    return Err(self.error("try requires a catch or finally clause"));
                }
                return Ok(Stmt::Try {
                    body,
                    catches,
                    finally,
                });
            }
            "break" => {
                self.bump();
                self.expect(";")?;
                return Ok(Stmt::Break);
            }
            "continue" => {
                self.bump();
                self.expect(";")?;
                return Ok(Stmt::Continue);
            }
            "foreach" => {
                self.bump();
                self.expect("(")?;
                let ty = self.parse_type_reference()?;
                let name = self.identifier()?;
                self.expect("in")?;
                let collection = self.parse_expression()?;
                self.expect(")")?;
                let body = self.parse_statement_block()?;
                return Ok(Stmt::ForEach {
                    ty,
                    name,
                    collection,
                    body,
                });
            }
            "case" | "default" | "goto" | "lock" | "using" | "yield" | "checked" | "unchecked"
            | "fixed" | "unsafe" => {
                return Err(self.error(format!(
                    "'{}' statements are not supported by the bounded core yet",
                    self.text()
                )));
            }
            _ => {}
        }
        self.parse_simple_statement()
    }

    fn parse_statement_block(&mut self) -> Result<Block, CompileError> {
        if self.check("{") {
            return self.parse_block();
        }
        let statement = self.parse_statement()?;
        Ok(Block {
            statements: vec![statement],
        })
    }

    fn parse_simple_statement(&mut self) -> Result<Stmt, CompileError> {
        // Local declaration: `Type name = ...;` (also `var`).
        if self.looks_like_local_declaration() {
            let ty = self.parse_type_reference()?;
            let name = self.identifier()?;
            let init = if self.eat("=") {
                Some(self.parse_expression()?)
            } else {
                None
            };
            self.expect(";")?;
            return Ok(Stmt::Local { ty, name, init });
        }
        // Compound assignment: `name += expr;`
        if self.token().kind == syntax::TokenKind::Identifier
            && matches!(
                self.peek(1),
                Some("+=")
                    | Some("-=")
                    | Some("*=")
                    | Some("/=")
                    | Some("%=")
                    | Some("&=")
                    | Some("|=")
                    | Some("^=")
            )
        {
            let name = self.identifier()?;
            let op = self.text().to_string();
            self.bump();
            let value = self.parse_expression()?;
            self.expect(";")?;
            return Ok(Stmt::AssignOp { name, op, value });
        }
        // Assignment: `name = expr;`
        if self.token().kind == syntax::TokenKind::Identifier && self.peek(1) == Some("=") {
            let name = self.identifier()?;
            self.expect("=")?;
            let value = self.parse_expression()?;
            self.expect(";")?;
            return Ok(Stmt::Assign { name, value });
        }
        let expression = self.parse_expression()?;
        if matches!(
            self.text(),
            "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^="
        ) {
            if let Expr::Member { target, name } = expression {
                let op = self.text().to_string();
                self.bump();
                let value = self.parse_expression()?;
                self.expect(";")?;
                return Ok(Stmt::AssignMemberOp {
                    target: *target,
                    name,
                    op,
                    value,
                });
            }
            return Err(self.error("compound assignment requires a member target"));
        }
        if self.check("=") {
            if let Expr::Index { target, index } = expression {
                self.bump();
                let value = self.parse_expression()?;
                self.expect(";")?;
                return Ok(Stmt::AssignIndex {
                    target: *target,
                    index: *index,
                    value,
                });
            }
            if let Expr::Member { target, name } = expression {
                self.bump();
                let value = self.parse_expression()?;
                self.expect(";")?;
                return Ok(Stmt::AssignMember {
                    target: *target,
                    name,
                    value,
                });
            }
            return Err(self.error("only member access can be assigned here"));
        }
        self.expect(";")?;
        if !matches!(
            expression,
            Expr::Call { .. }
                | Expr::New { .. }
                | Expr::Increment { .. }
                | Expr::IncrementMember { .. }
                | Expr::IncrementIndex { .. }
                | Expr::Assign { .. }
        ) {
            return Err(self.error("expression is not a valid statement"));
        }
        Ok(Stmt::Expression(expression))
    }

    fn looks_like_local_declaration(&mut self) -> bool {
        let saved = self.position;
        let result = (|| -> Result<bool, CompileError> {
            self.parse_type_reference()?;
            Ok(self.token().kind == syntax::TokenKind::Identifier
                && matches!(self.peek(1), Some("=") | Some(";")))
        })()
        .unwrap_or(false);
        self.position = saved;
        result
    }

    fn parse_expression(&mut self) -> Result<Expr, CompileError> {
        if self.token().kind == syntax::TokenKind::Identifier && self.peek(1) == Some("=") {
            let name = self.identifier()?;
            self.expect("=")?;
            let value = self.parse_expression()?;
            return Ok(Expr::Assign {
                name,
                value: Box::new(value),
            });
        }
        let mut left = self.parse_coalesce()?;
        while self.check("??") {
            self.bump();
            let right = self.parse_coalesce()?;
            left = Expr::Coalesce(Box::new(left), Box::new(right));
        }
        if self.eat("?") {
            let when_true = self.parse_expression()?;
            self.expect(":")?;
            let when_false = self.parse_expression()?;
            return Ok(Expr::Conditional {
                condition: Box::new(left),
                when_true: Box::new(when_true),
                when_false: Box::new(when_false),
            });
        }
        Ok(left)
    }

    fn parse_coalesce(&mut self) -> Result<Expr, CompileError> {
        self.parse_logical_or()
    }

    fn parse_logical_or(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_logical_and()?;
        while self.check("||") {
            self.bump();
            let right = self.parse_logical_and()?;
            left = Expr::Binary(BinaryOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_logical_and(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_bitwise_or()?;
        while self.check("&&") {
            self.bump();
            let right = self.parse_bitwise_or()?;
            left = Expr::Binary(BinaryOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_bitwise_or(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_bitwise_xor()?;
        while self.eat("|") {
            left = Expr::Binary(
                BinaryOp::BitOr,
                Box::new(left),
                Box::new(self.parse_bitwise_xor()?),
            );
        }
        Ok(left)
    }

    fn parse_bitwise_xor(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_bitwise_and()?;
        while self.eat("^") {
            left = Expr::Binary(
                BinaryOp::BitXor,
                Box::new(left),
                Box::new(self.parse_bitwise_and()?),
            );
        }
        Ok(left)
    }

    fn parse_bitwise_and(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_equality()?;
        while self.eat("&") {
            left = Expr::Binary(
                BinaryOp::BitAnd,
                Box::new(left),
                Box::new(self.parse_equality()?),
            );
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_relational()?;
        loop {
            let op = match self.text() {
                "==" => BinaryOp::Equal,
                "!=" => BinaryOp::NotEqual,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.parse_relational()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn parse_relational(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_additive()?;
        loop {
            if self.check("as") {
                self.bump();
                let ty = self.parse_type_reference()?;
                left = Expr::As(Box::new(left), ty);
                continue;
            }
            let op = match self.text() {
                "<" => BinaryOp::Less,
                "<=" => BinaryOp::LessOrEqual,
                ">" => BinaryOp::Greater,
                ">=" => BinaryOp::GreaterOrEqual,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.parse_additive()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn parse_additive(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.text() {
                "+" => BinaryOp::Add,
                "-" => BinaryOp::Subtract,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.parse_multiplicative()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, CompileError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.text() {
                "*" => BinaryOp::Multiply,
                "/" => BinaryOp::Divide,
                "%" => BinaryOp::Remainder,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.parse_unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, CompileError> {
        if self.check("++") || self.check("--") {
            let delta = if self.check("++") { 1 } else { -1 };
            self.bump();
            return match self.parse_unary()? {
                Expr::Name(name) => Ok(Expr::Increment {
                    name,
                    delta,
                    prefix: true,
                }),
                Expr::Member { target, name } => Ok(Expr::IncrementMember {
                    target,
                    name,
                    delta,
                    prefix: true,
                }),
                Expr::Index { target, index } => Ok(Expr::IncrementIndex {
                    target,
                    index,
                    delta,
                    prefix: true,
                }),
                _ => Err(self.error("increment requires an assignable variable or member")),
            };
        }
        match self.text() {
            "-" => {
                self.bump();
                // This decimal literal is a special C# int constant when it
                // is the direct operand of unary minus.
                if self.token().kind == syntax::TokenKind::Number
                    && self.text().replace('_', "") == "2147483648"
                {
                    self.bump();
                    return Ok(Expr::Int(i32::MIN, true));
                }
                Ok(Expr::Negate(Box::new(self.parse_unary()?)))
            }
            "!" => {
                self.bump();
                Ok(Expr::Not(Box::new(self.parse_unary()?)))
            }
            "~" => {
                self.bump();
                Ok(Expr::Complement(Box::new(self.parse_unary()?)))
            }
            "(" => {
                if self.looks_like_cast() {
                    self.bump();
                    let target = self.parse_type_reference()?;
                    self.expect(")")?;
                    let operand = self.parse_unary()?;
                    return Ok(Expr::Cast(target, Box::new(operand)));
                }
                self.bump();
                let inner = self.parse_expression()?;
                self.expect(")")?;
                Ok(inner)
            }
            _ => self.parse_postfix(),
        }
    }

    fn looks_like_cast(&mut self) -> bool {
        let saved = self.position;
        let ok = (|| -> bool {
            if !self.check("(") {
                return false;
            }
            self.bump();
            if self.parse_type_reference().is_err() || !self.check(")") {
                return false;
            }
            self.bump();
            matches!(
                self.token().kind,
                syntax::TokenKind::Identifier
                    | syntax::TokenKind::Number
                    | syntax::TokenKind::String
                    | syntax::TokenKind::Char
            ) || matches!(
                self.text(),
                "(" | "!" | "~" | "-" | "+" | "new" | "this" | "base" | "typeof" | "default"
            )
        })();
        self.position = saved;
        ok
    }

    fn parse_postfix(&mut self) -> Result<Expr, CompileError> {
        let mut expression = self.parse_primary()?;
        loop {
            if self.check(".") {
                self.bump();
                let name = self.identifier()?;
                let generic_args = self.parse_generic_arguments()?;
                if self.check("(") {
                    let args = self.parse_arguments()?;
                    expression = Expr::Call {
                        receiver: Some(Box::new(expression)),
                        name,
                        generic_args,
                        args,
                    };
                } else {
                    expression = Expr::Member {
                        target: Box::new(expression),
                        name,
                    };
                }
                continue;
            }
            if self.check("[") {
                self.bump();
                let index = self.parse_expression()?;
                self.expect("]")?;
                expression = Expr::Index {
                    target: Box::new(expression),
                    index: Box::new(index),
                };
                continue;
            }
            if self.check("<") && self.looks_like_generic_call() {
                let generic_args = self.parse_generic_arguments()?;
                let Expr::Name(name) = expression else {
                    return Err(self.error("generic arguments require a method name"));
                };
                let args = self.parse_arguments()?;
                expression = Expr::Call {
                    receiver: None,
                    name,
                    generic_args,
                    args,
                };
                continue;
            }
            if self.check("(") {
                let args = self.parse_arguments()?;
                match expression {
                    Expr::Name(name) => {
                        expression = Expr::Call {
                            receiver: None,
                            name,
                            generic_args: Vec::new(),
                            args,
                        };
                    }
                    _ => return Err(self.error("only a named method can be called")),
                }
                continue;
            }
            if self.check("++") || self.check("--") {
                let delta = if self.check("++") { 1 } else { -1 };
                self.bump();
                return match expression {
                    Expr::Name(name) => Ok(Expr::Increment {
                        name,
                        delta,
                        prefix: false,
                    }),
                    Expr::Member { target, name } => Ok(Expr::IncrementMember {
                        target,
                        name,
                        delta,
                        prefix: false,
                    }),
                    Expr::Index { target, index } => Ok(Expr::IncrementIndex {
                        target,
                        index,
                        delta,
                        prefix: false,
                    }),
                    _ => Err(self.error("increment requires an assignable variable or member")),
                };
            }
            return Ok(expression);
        }
    }
    fn parse_generic_arguments(&mut self) -> Result<Vec<Ty>, CompileError> {
        if !self.check("<") {
            return Ok(Vec::new());
        }
        self.bump();
        let mut types = Vec::new();
        loop {
            types.push(self.parse_type_reference()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(">")?;
        Ok(types)
    }

    fn parse_arguments(&mut self) -> Result<Vec<Expr>, CompileError> {
        self.expect("(")?;
        let mut args = Vec::new();
        if !self.check(")") {
            loop {
                args.push(self.parse_expression()?);
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.expect(")")?;
        Ok(args)
    }

    fn looks_like_generic_call(&mut self) -> bool {
        let saved = self.position;
        let ok = self.parse_generic_arguments().is_ok() && self.check("(");
        self.position = saved;
        ok
    }

    fn parse_primary(&mut self) -> Result<Expr, CompileError> {
        if self.eat("default") {
            self.expect("(")?;
            let ty = self.parse_type_reference()?;
            self.expect(")")?;
            return Ok(Expr::Default(ty));
        }
        if self.check("new") {
            self.bump();
            let element = self.parse_new_type()?;
            if self.eat("[") {
                if matches!(element, Ty::Struct(_)) {
                    return Err(self.error("arrays of struct values are not supported yet"));
                }
                let size = self.parse_expression()?;
                self.expect("]")?;
                return Ok(Expr::NewArray {
                    element,
                    size: Box::new(size),
                });
            }
            if !matches!(element, Ty::Class(_) | Ty::Generic(_, _) | Ty::Struct(_)) {
                return Err(self.error("only classes can be constructed with 'new Type(...)'"));
            }
            let args = if self.check("{") {
                Vec::new()
            } else {
                self.parse_arguments()?
            };
            let mut fields = Vec::new();
            if self.check("{") {
                self.bump();
                while !self.check("}") && !self.at_end() {
                    let name = self.identifier()?;
                    self.expect("=")?;
                    let value = self.parse_expression()?;
                    fields.push((name, value));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("}")?;
            }
            return Ok(Expr::New {
                ty: element,
                args,
                fields,
            });
        }
        if self.check("this") {
            self.bump();
            return Ok(Expr::Name("this".to_string()));
        }
        if self.check("base") {
            self.bump();
            return Ok(Expr::Name("base".to_string()));
        }
        let token = self.token();
        match token.kind {
            syntax::TokenKind::Number => {
                let text = self.text().to_string();
                self.bump();
                let cleaned = text.replace('_', "");
                let radix_integer = matches!(cleaned.get(..2), Some("0x" | "0X" | "0b" | "0B"));
                let is_float = !radix_integer
                    && (cleaned.contains('.')
                        || cleaned.contains('e')
                        || cleaned.contains('E')
                        || cleaned.ends_with(['f', 'F', 'd', 'D', 'm', 'M']));
                if is_float {
                    let numeric = cleaned.trim_end_matches(['f', 'F', 'd', 'D', 'm', 'M']);
                    let value = numeric.parse::<f32>().map_err(|_| {
                        self.error(format!("float literal '{text}' is not supported"))
                    })?;
                    return Ok(Expr::Float(value));
                }
                let trimmed = cleaned.trim_end_matches(['u', 'U', 'l', 'L']);
                let suffix = cleaned[trimmed.len()..].to_ascii_lowercase();
                if !matches!(suffix.as_str(), "" | "u" | "l" | "ul" | "lu") {
                    return Err(self.error("invalid integer literal suffix"));
                }
                let value = if let Some(hex) = trimmed
                    .strip_prefix("0x")
                    .or_else(|| trimmed.strip_prefix("0X"))
                {
                    i32::from_str_radix(hex, 16)
                } else if let Some(bin) = trimmed
                    .strip_prefix("0b")
                    .or_else(|| trimmed.strip_prefix("0B"))
                {
                    i32::from_str_radix(bin, 2)
                } else {
                    trimmed.parse::<i32>()
                }
                .map_err(|_| self.error(format!("integer literal '{text}' is not supported")))?;
                Ok(Expr::Int(value, suffix.is_empty()))
            }
            syntax::TokenKind::String => {
                let raw = self.text();
                self.bump();
                Ok(Expr::Str(unquote(raw)?))
            }
            syntax::TokenKind::Identifier => {
                let text = self.text().to_string();
                match text.as_str() {
                    "true" => {
                        self.bump();
                        Ok(Expr::Bool(true))
                    }
                    "false" => {
                        self.bump();
                        Ok(Expr::Bool(false))
                    }
                    _ => {
                        self.bump();
                        Ok(Expr::Name(text))
                    }
                }
            }
            syntax::TokenKind::Keyword => match self.text() {
                "null" => {
                    self.bump();
                    Ok(Expr::Null)
                }
                "true" => {
                    self.bump();
                    Ok(Expr::Bool(true))
                }
                "false" => {
                    self.bump();
                    Ok(Expr::Bool(false))
                }
                other
                    if matches!(
                        other,
                        "string"
                            | "int"
                            | "bool"
                            | "float"
                            | "double"
                            | "long"
                            | "short"
                            | "byte"
                            | "object"
                            | "char"
                            | "uint"
                            | "ulong"
                            | "ushort"
                            | "sbyte"
                            | "decimal"
                    ) =>
                {
                    // Predefined type used as a receiver: `string.Empty`.
                    self.bump();
                    Ok(Expr::Name(other.to_string()))
                }
                other => Err(self.error(format!("'{other}' is not supported in expressions"))),
            },
            _ => Err(self.error(format!(
                "unexpected token '{}' in an expression",
                self.text()
            ))),
        }
    }
}

fn unquote(raw: &str) -> Result<String, CompileError> {
    fn error(message: &str) -> CompileError {
        CompileError {
            file: String::new(),
            line: 1,
            column: 1,
            message: message.into(),
            syntax: true,
        }
    }
    fn hex(
        characters: &mut std::iter::Peekable<std::str::Chars<'_>>,
        minimum: usize,
        maximum: usize,
    ) -> Result<u32, CompileError> {
        let mut value = 0u32;
        let mut count = 0;
        while count < maximum {
            let Some(digit) = characters
                .peek()
                .and_then(|character| character.to_digit(16))
            else {
                break;
            };
            characters.next();
            value = (value << 4) | digit;
            count += 1;
        }
        if count < minimum {
            return Err(error("invalid hexadecimal string escape"));
        }
        Ok(value)
    }
    let body = raw
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| error("unsupported string literal"))?;
    if !body.contains('\\') {
        return Ok(body.to_string());
    }
    let mut value = Vec::with_capacity(body.len());
    let mut characters = body.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\\' {
            value.extend_from_slice(character.encode_utf16(&mut [0; 2]));
            continue;
        }
        let Some(escaped) = characters.next() else {
            return Err(error("unterminated escape sequence"));
        };
        match escaped {
            'u' => value.push(hex(&mut characters, 4, 4)? as u16),
            'x' => value.push(hex(&mut characters, 1, 4)? as u16),
            'U' => {
                let scalar = hex(&mut characters, 8, 8)?;
                if scalar <= 0xffff {
                    value.push(scalar as u16);
                } else {
                    let scalar = char::from_u32(scalar)
                        .ok_or_else(|| error("invalid Unicode scalar in string escape"))?;
                    value.extend_from_slice(scalar.encode_utf16(&mut [0; 2]));
                }
            }
            _ => {
                let decoded = match escaped {
                    '0' => 0,
                    'a' => 7,
                    'b' => 8,
                    'f' => 12,
                    'n' => 10,
                    'r' => 13,
                    't' => 9,
                    'v' => 11,
                    '\\' => 92,
                    '\'' => 39,
                    '"' => 34,
                    _ => return Err(error("invalid string escape sequence")),
                };
                value.push(decoded);
            }
        }
    }
    String::from_utf16(&value)
        .map_err(|_| error("unpaired UTF-16 surrogate string literals are not supported yet"))
}

// ---------------------------------------------------------------- emission

// ---------------------------------------------------------------- emission

struct FieldSymbol {
    ty: Ty,
    is_static: bool,
    entry: Entry<Field>,
}

#[derive(Clone)]
struct MethodSymbol {
    name: String,
    is_static: bool,
    ret: Ty,
    params: Vec<Ty>,
    entry: Entry<Method>,
}

#[derive(Clone)]
struct PropertySymbol {
    ty: Ty,
    is_static: bool,
    getter: Option<Entry<Method>>,
    setter: Option<Entry<Method>>,
}

#[derive(Default)]
struct TypeSymbol {
    base: Option<String>,
    interfaces: Vec<String>,
    fields: HashMap<String, FieldSymbol>,
    methods: Vec<MethodSymbol>,
    ctors: Vec<MethodSymbol>,
    properties: HashMap<String, PropertySymbol>,
}

fn type_error(
    decl: &TypeDecl,
    sources: &[(String, String)],
    message: impl Into<String>,
) -> CompileError {
    let source = sources
        .iter()
        .find(|(file, _)| file == &decl.file)
        .map(|(_, text)| text.as_str())
        .unwrap_or("");
    CompileError::new(&decl.file, source, decl.offset, message)
}

fn enum_error(
    decl: &EnumInfo,
    sources: &[(String, String)],
    message: impl Into<String>,
) -> CompileError {
    let source = sources
        .iter()
        .find(|(file, _)| file == &decl.file)
        .map(|(_, text)| text.as_str())
        .unwrap_or("");
    CompileError::new(&decl.file, source, decl.offset, message)
}

fn program_error(
    program: &Program,
    sources: &[(String, String)],
    message: impl Into<String>,
) -> CompileError {
    let message = message.into();
    if let Some(decl) = program.types.first() {
        type_error(decl, sources, message)
    } else if let Some(decl) = program
        .enums
        .values()
        .filter(|entry| !entry.is_external)
        .min_by_key(|entry| (&entry.file, entry.offset))
    {
        enum_error(decl, sources, message)
    } else {
        CompileError::new(
            sources.first().map(|(file, _)| file.as_str()).unwrap_or(""),
            "",
            0,
            message,
        )
    }
}

fn is_reference(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::String | Ty::Class(_) | Ty::Array(_) | Ty::Generic(_, _)
    )
}

fn core_exception_name(name: &str) -> Option<&'static str> {
    match name {
        "Exception" | "System.Exception" => Some("System.Exception"),
        "SystemException" | "System.SystemException" => Some("System.SystemException"),
        "ArithmeticException" | "System.ArithmeticException" => Some("System.ArithmeticException"),
        "DivideByZeroException" | "System.DivideByZeroException" => {
            Some("System.DivideByZeroException")
        }
        "OverflowException" | "System.OverflowException" => Some("System.OverflowException"),
        "InvalidOperationException" | "System.InvalidOperationException" => {
            Some("System.InvalidOperationException")
        }
        "NullReferenceException" | "System.NullReferenceException" => {
            Some("System.NullReferenceException")
        }
        "IndexOutOfRangeException" | "System.IndexOutOfRangeException" => {
            Some("System.IndexOutOfRangeException")
        }
        "ArgumentException" | "System.ArgumentException" => Some("System.ArgumentException"),
        "ArgumentNullException" | "System.ArgumentNullException" => {
            Some("System.ArgumentNullException")
        }
        "ApplicationException" | "System.ApplicationException" => {
            Some("System.ApplicationException")
        }
        _ => None,
    }
}

fn core_exception_base(name: &str) -> Option<&'static str> {
    match core_exception_name(name)? {
        "System.SystemException" | "System.ApplicationException" => Some("System.Exception"),
        "System.ArithmeticException"
        | "System.InvalidOperationException"
        | "System.NullReferenceException"
        | "System.IndexOutOfRangeException"
        | "System.ArgumentException" => Some("System.SystemException"),
        "System.DivideByZeroException" | "System.OverflowException" => {
            Some("System.ArithmeticException")
        }
        "System.ArgumentNullException" => Some("System.ArgumentException"),
        _ => None,
    }
}

/// Select the CLR array instruction for the physical element representation.
fn array_element_opcode(ty: &Ty, store: bool) -> Option<OpCode> {
    match (ty, store) {
        (Ty::Int, false) => Some(OpCode::Ldelem_I4),
        (Ty::Int, true) => Some(OpCode::Stelem_I4),
        (Ty::Float, false) => Some(OpCode::Ldelem_R4),
        (Ty::Float, true) => Some(OpCode::Stelem_R4),
        (Ty::Bool, false) => Some(OpCode::Ldelem_U1),
        (Ty::Bool, true) => Some(OpCode::Stelem_I1),
        (Ty::Enum(_, EnumBase::I1), false) => Some(OpCode::Ldelem_I1),
        (Ty::Enum(_, EnumBase::U1), false) => Some(OpCode::Ldelem_U1),
        (Ty::Enum(_, EnumBase::I1 | EnumBase::U1), true) => Some(OpCode::Stelem_I1),
        (Ty::Enum(_, EnumBase::I2), false) => Some(OpCode::Ldelem_I2),
        (Ty::Enum(_, EnumBase::U2), false) => Some(OpCode::Ldelem_U2),
        (Ty::Enum(_, EnumBase::I2 | EnumBase::U2), true) => Some(OpCode::Stelem_I2),
        (Ty::Enum(_, EnumBase::I4), false) => Some(OpCode::Ldelem_I4),
        (Ty::Enum(_, EnumBase::U4), false) => Some(OpCode::Ldelem_U4),
        (Ty::Enum(_, EnumBase::I4 | EnumBase::U4), true) => Some(OpCode::Stelem_I4),
        (Ty::Enum(_, EnumBase::I8 | EnumBase::U8), false) => Some(OpCode::Ldelem_I8),
        (Ty::Enum(_, EnumBase::I8 | EnumBase::U8), true) => Some(OpCode::Stelem_I8),
        (Ty::Object | Ty::String | Ty::Class(_) | Ty::Array(_) | Ty::Generic(_, _), false) => {
            Some(OpCode::Ldelem_Ref)
        }
        (Ty::Object | Ty::String | Ty::Class(_) | Ty::Array(_) | Ty::Generic(_, _), true) => {
            Some(OpCode::Stelem_Ref)
        }
        _ => None,
    }
}

fn external_ty(sig: &TypeSig, enums: &EnumValues, allow_local: bool) -> Ty {
    match sig {
        TypeSig::Primitive(PrimitiveType::I4) => Ty::Int,
        TypeSig::Primitive(PrimitiveType::R4) => Ty::Float,
        TypeSig::Primitive(PrimitiveType::Boolean) => Ty::Bool,
        TypeSig::Primitive(PrimitiveType::String) => Ty::String,
        TypeSig::Primitive(PrimitiveType::Object) => Ty::Object,
        TypeSig::Primitive(PrimitiveType::Void) => Ty::Void,
        TypeSig::Class(TypeDefOrRef::TypeDef(entry)) => Ty::Class(entry.get().full_name()),
        TypeSig::Class(TypeDefOrRef::TypeRef(entry)) => Ty::Class(entry.get().full_name()),
        TypeSig::ValueType(reference) => enums
            .get(&reference.full_name())
            .filter(|info| info.matches_type_reference(reference, allow_local))
            .map(|info| Ty::Enum(info.full_name(), info.base))
            .unwrap_or_else(|| Ty::Named(format!("{sig:?}"))),
        TypeSig::SzArray(inner) => Ty::Array(Box::new(external_ty(inner, enums, allow_local))),
        other => Ty::Named(format!("{other:?}")),
    }
}

/// Replaces `MVar` placeholders with the supplied instantiation.
fn substitute(sig: &TypeSig, instantiation: &[TypeSig], enums: &EnumValues) -> Ty {
    match sig {
        TypeSig::Var(index) | TypeSig::MVar(index) => instantiation
            .get(*index as usize)
            .map(|sig| external_ty(sig, enums, true))
            .unwrap_or(Ty::Named("generic argument".to_string())),
        TypeSig::SzArray(inner) => Ty::Array(Box::new(substitute(inner, instantiation, enums))),
        other => external_ty(other, enums, false),
    }
}

fn is_static_type(entry: &Entry<TypeDef>) -> bool {
    let flags = entry.get().flags;
    flags.contains(TypeAttributes::Abstract | TypeAttributes::Sealed)
        && !flags.contains(TypeAttributes::Interface)
}

fn resolve_type(
    ty: &Ty,
    types: &HashMap<String, Entry<TypeDef>>,
    externals: &Externals,
    module: &mut Module,
) -> Result<TypeSig, String> {
    Ok(match ty {
        Ty::Void => TypeSig::VOID,
        Ty::Int => TypeSig::INT32,
        Ty::Object => TypeSig::OBJECT,
        Ty::Float => TypeSig::Primitive(PrimitiveType::R4),
        Ty::Null => return Err("null has no type of its own".to_string()),
        Ty::Bool => TypeSig::BOOL,
        Ty::String => TypeSig::STRING,
        Ty::Struct(name) => {
            let entry = types
                .get(name)
                .ok_or_else(|| format!("unknown struct '{name}'"))?;
            TypeSig::ValueType(TypeDefOrRef::TypeDef(entry.clone()))
        }
        Ty::Enum(name, _) => {
            if let Some(entry) = types.get(name) {
                TypeSig::ValueType(TypeDefOrRef::TypeDef(entry.clone()))
            } else {
                let (index, entry) = externals
                    .types
                    .get(name)
                    .ok_or_else(|| format!("unknown enum type '{name}'"))?;
                if !entry.get().is_visible() {
                    return Err(format!("referenced enum type '{name}' is inaccessible"));
                }
                TypeSig::ValueType(
                    module
                        .import_foreign_type(
                            &TypeDefOrRef::TypeDef(entry.clone()),
                            &externals.modules[*index],
                        )
                        .map_err(|error| error.to_string())?,
                )
            }
        }
        Ty::Class(name) => match types.get(name) {
            Some(entry) if is_static_type(entry) => {
                return Err(format!(
                    "static type '{name}' cannot be used as a value type"
                ));
            }
            Some(entry) => {
                if entry
                    .get()
                    .extends
                    .as_ref()
                    .is_some_and(|base| base.full_name() == "System.ValueType")
                {
                    return Err(format!("struct type '{name}' is not in scope"));
                }
                TypeSig::Class(TypeDefOrRef::TypeDef(entry.clone()))
            }
            None if core_exception_name(name).is_some() && externals.find(name).is_none() => {
                TypeSig::Class(
                    module
                        .corlib_type(core_exception_name(name).unwrap())
                        .map_err(|error| error.to_string())?,
                )
            }
            None => match externals.find(name) {
                Some((index, entry)) => {
                    if !entry.get().is_visible() {
                        return Err(format!("referenced type '{name}' is inaccessible"));
                    }
                    if is_static_type(entry) {
                        return Err(format!(
                            "static type '{name}' cannot be used as a value type"
                        ));
                    }
                    if entry
                        .get()
                        .extends
                        .as_ref()
                        .is_some_and(|base| base.full_name() == "System.Enum")
                    {
                        return Err(format!(
                            "referenced enum type '{name}' is not supported yet"
                        ));
                    }
                    if entry
                        .get()
                        .extends
                        .as_ref()
                        .is_some_and(|base| base.full_name() == "System.ValueType")
                        && !(entry.get().full_name() == "System.Enum"
                            && trusted_corlib(externals, *index))
                    {
                        return Err(format!(
                            "referenced value type '{name}' is not supported yet"
                        ));
                    }
                    let imported = module
                        .import_foreign_type(
                            &TypeDefOrRef::TypeDef(entry.clone()),
                            &externals.modules[*index],
                        )
                        .map_err(|error| error.to_string())?;
                    TypeSig::Class(imported)
                }
                None => return Err(format!("unknown type '{name}'")),
            },
        },
        Ty::Array(element) => TypeSig::sz_array(resolve_type(element, types, externals, module)?),
        Ty::Generic(name, args) => {
            if args.iter().any(|arg| matches!(arg, Ty::Struct(_))) {
                return Err("struct generic arguments are not supported yet".to_string());
            }
            let Some((index, entry)) = externals.find_generic(name, args.len()) else {
                return Err(format!("unknown generic type '{name}'"));
            };
            if !entry.get().is_visible() {
                return Err(format!("referenced type '{name}' is inaccessible"));
            }
            if entry
                .get()
                .extends
                .as_ref()
                .is_some_and(|base| base.full_name() == "System.ValueType")
            {
                return Err(format!("generic value type '{name}' is not supported yet"));
            }
            let open = module
                .import_foreign_type(
                    &TypeDefOrRef::TypeDef(entry.clone()),
                    &externals.modules[*index],
                )
                .map_err(|error| error.to_string())?;
            let mut signatures = Vec::with_capacity(args.len());
            for argument in args {
                signatures.push(resolve_type(argument, types, externals, module)?);
            }
            TypeSig::GenericInst {
                generic: Box::new(TypeSig::Class(open)),
                args: signatures,
            }
        }
        Ty::Named(name) => return Err(format!("type '{name}' is not supported yet")),
    })
}

fn less_visible_enum<'a>(ty: &'a Ty, enums: &EnumValues) -> Option<&'a str> {
    match ty {
        Ty::Enum(name, _) => enums
            .get(name)
            .filter(|info| !info.is_public)
            .map(|_| name.as_str()),
        Ty::Array(element) => less_visible_enum(element, enums),
        Ty::Generic(_, arguments) => arguments
            .iter()
            .find_map(|argument| less_visible_enum(argument, enums)),
        _ => None,
    }
}

fn source_field_constant(field: &FieldDecl) -> Result<Constant, String> {
    let expression = field
        .init
        .as_ref()
        .ok_or("constant field has no initializer")?;
    let null = || Constant {
        element_type: ElementType::Class,
        value: vec![0; 4],
    };
    match (&field.ty, expression) {
        (Ty::Int, expression) => FnEmitter::constant_int(expression)
            .map(|value| Constant {
                element_type: ElementType::I4,
                value: value.to_le_bytes().to_vec(),
            })
            .ok_or_else(|| "int constant initializer is unsupported or overflows".to_string()),
        (Ty::Bool, Expr::Bool(value)) => Ok(Constant {
            element_type: ElementType::Boolean,
            value: vec![u8::from(*value)],
        }),
        (Ty::Bool, Expr::Default(Ty::Bool)) => Ok(Constant {
            element_type: ElementType::Boolean,
            value: vec![0],
        }),
        (Ty::String, Expr::Str(value)) => Ok(Constant {
            element_type: ElementType::String,
            value: value.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        }),
        (Ty::String | Ty::Object, Expr::Null) => Ok(null()),
        (Ty::String, Expr::Default(Ty::String)) | (Ty::Object, Expr::Default(Ty::Object)) => {
            Ok(null())
        }
        _ => Err("constant field type or initializer is not supported yet".to_string()),
    }
}

fn trusted_corlib(externals: &Externals, index: usize) -> bool {
    let Some(assembly) = externals.modules[index].assembly.as_ref() else {
        return false;
    };
    let assembly = assembly.get();
    let expected: &[u8] = if assembly.name.eq_ignore_ascii_case("System.Private.CoreLib") {
        &[0x7c, 0xec, 0x85, 0xd7, 0xbe, 0xa7, 0x79, 0x8e]
    } else if assembly.name.eq_ignore_ascii_case("mscorlib") {
        &[0xb7, 0x7a, 0x5c, 0x56, 0x19, 0x34, 0xe0, 0x89]
    } else {
        return false;
    };
    !assembly.public_key.is_empty() && strong_name_token(&assembly.public_key) == expected
}

fn validate_enum_flags(
    program: &Program,
    externals: &Externals,
    sources: &[(String, String)],
) -> Result<(), CompileError> {
    for info in program.enums.values().filter(|entry| !entry.is_external) {
        let Some(name) = info.flags_name.as_deref() else {
            continue;
        };
        let qualified = name == "System.Flags";
        let short = if qualified { "Flags" } else { name };
        let suffixed = format!("{short}Attribute");
        let in_scope = |namespace: &str| {
            if qualified {
                return namespace == "System";
            }
            namespace.is_empty()
                || namespace == info.namespace
                || (!namespace.is_empty() && info.namespace.starts_with(&format!("{namespace}.")))
                || info.flags_imports.iter().any(|import| import == namespace)
        };
        let conflicts_local_type = program
            .types
            .iter()
            .any(|ty| (ty.name == short || ty.name == suffixed) && in_scope(&ty.namespace));
        let conflicts_local_enum = program.enums.values().any(|other| {
            !other.is_external
                && (other.name == short || other.name == suffixed)
                && in_scope(&other.namespace)
        });
        if conflicts_local_type || conflicts_local_enum {
            return Err(enum_error(
                info,
                sources,
                "Flags attribute name conflicts with a local type",
            ));
        }
        for candidate in [short, suffixed.as_str()] {
            if let Some(names) = externals.by_short_name.get(candidate) {
                let mut system_flags = 0;
                for full_name in names {
                    let namespace = full_name
                        .rsplit_once('.')
                        .map_or("", |(namespace, _)| namespace);
                    if !in_scope(namespace) {
                        continue;
                    }
                    if full_name == "System.FlagsAttribute" {
                        system_flags += 1;
                        if let Some((index, _)) = externals.types.get(full_name)
                            && trusted_corlib(externals, *index)
                            && system_flags == 1
                        {
                            continue;
                        }
                    }
                    return Err(enum_error(
                        info,
                        sources,
                        "Flags attribute name is ambiguous or is not the framework attribute",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_enum_api_accessibility(
    program: &Program,
    sources: &[(String, String)],
) -> Result<(), CompileError> {
    for decl in &program.types {
        if !decl.is_public {
            continue;
        }
        let visible = |visibility| {
            matches!(
                visibility,
                Visibility::Public | Visibility::Protected | Visibility::ProtectedInternal
            )
        };
        let check = |ty: &Ty, description: &str| {
            if let Some(name) = less_visible_enum(ty, &program.enums) {
                Err(type_error(
                    decl,
                    sources,
                    format!("enum '{name}' is less accessible than {description}"),
                ))
            } else if let Ty::Struct(name) = ty
                && program.types.iter().any(|candidate| {
                    candidate.is_struct
                        && !candidate.is_public
                        && (if candidate.namespace.is_empty() {
                            candidate.name.clone()
                        } else {
                            format!("{}.{}", candidate.namespace, candidate.name)
                        }) == *name
                })
            {
                Err(type_error(
                    decl,
                    sources,
                    format!("struct '{name}' is less accessible than {description}"),
                ))
            } else {
                Ok(())
            }
        };
        for field in &decl.fields {
            if visible(field.visibility) {
                check(&field.ty, &format!("field '{}.{}'", decl.name, field.name))?;
            }
        }
        for property in &decl.properties {
            if visible(property.visibility) {
                check(
                    &property.ty,
                    &format!("property '{}.{}'", decl.name, property.name),
                )?;
            }
        }
        for method in &decl.methods {
            if visible(method.visibility) {
                let description = format!("method '{}.{}'", decl.name, method.name);
                check(&method.ret, &description)?;
                for (parameter, _) in &method.params {
                    check(parameter, &description)?;
                }
            }
        }
    }
    Ok(())
}

fn emit(
    program: &Program,
    assembly_name: &str,
    sources: &[(String, String)],
    externals: &Externals,
) -> Result<Vec<u8>, CompileError> {
    validate_enum_flags(program, externals, sources)?;
    validate_enum_api_accessibility(program, sources)?;
    for decl in &program.types {
        let mut names = std::collections::HashSet::new();
        for field in &decl.fields {
            if field.name == decl.name
                || !names.insert(&field.name)
                || decl
                    .properties
                    .iter()
                    .any(|property| property.name == field.name)
                || decl
                    .methods
                    .iter()
                    .any(|method| !method.is_ctor && method.name == field.name)
            {
                return Err(type_error(
                    decl,
                    sources,
                    format!("field '{}' conflicts with another member", field.name),
                ));
            }
        }
    }
    if program.types.is_empty() && program.enums.values().all(|entry| entry.is_external) {
        return Err(CompileError {
            file: sources
                .first()
                .map(|(file, _)| file.clone())
                .unwrap_or_default(),
            line: 1,
            column: 1,
            message: "no type declaration was found".into(),
            syntax: false,
        });
    }
    let mut module = Module::new(assembly_name, ModuleKind::Dll, TargetRuntime::NetCore(10));
    let object = module
        .corlib_type("System.Object")
        .map_err(|error| program_error(program, sources, error.to_string()))?;
    let value_type = if program.types.iter().any(|decl| decl.is_struct) {
        Some(
            module
                .corlib_type("System.ValueType")
                .map_err(|error| program_error(program, sources, error.to_string()))?,
        )
    } else {
        None
    };

    // Pass 0: declare every type so fields and signatures can reference types
    // that are declared later in the compilation.
    let mut types: HashMap<String, Entry<TypeDef>> = HashMap::new();
    let mut enums: Vec<&EnumInfo> = program
        .enums
        .values()
        .filter(|entry| !entry.is_external)
        .map(Rc::as_ref)
        .collect();
    enums
        .sort_by(|left, right| (&left.namespace, &left.name).cmp(&(&right.namespace, &right.name)));
    for decl in &enums {
        let base = module
            .corlib_type("System.Enum")
            .map_err(|error| enum_error(decl, sources, error.to_string()))?;
        let flags = (if decl.is_public {
            TypeAttributes::Public
        } else {
            TypeAttributes::NotPublic
        }) | TypeAttributes::Sealed;
        let entry = module.add_type(TypeDef::new(flags, &decl.namespace, &decl.name, Some(base)));
        if decl.is_flags {
            let constructor = module
                .corlib_attribute("System.FlagsAttribute")
                .map_err(|error| enum_error(decl, sources, error.to_string()))?;
            entry
                .get_mut()
                .custom_attributes
                .push(CustomAttribute::without_arguments(constructor));
        }
        types.insert(decl.full_name(), entry);
    }
    for decl in &program.types {
        let full_name = if decl.namespace.is_empty() {
            decl.name.clone()
        } else {
            format!("{}.{}", decl.namespace, decl.name)
        };
        if types.contains_key(&decl.name) || types.contains_key(&full_name) {
            return Err(type_error(
                decl,
                sources,
                format!("duplicate type '{}'", decl.name),
            ));
        }
        let entry = module.add_type(TypeDef::new(
            (if decl.is_public {
                TypeAttributes::Public
            } else {
                TypeAttributes::NotPublic
            }) | if decl.is_interface {
                TypeAttributes::Interface | TypeAttributes::Abstract
            } else if decl.is_struct {
                TypeAttributes::Sealed
                    | TypeAttributes::SequentialLayout
                    | TypeAttributes::BeforeFieldInit
            } else {
                (if decl.is_static {
                    TypeAttributes::Abstract | TypeAttributes::Sealed
                } else {
                    TypeAttributes::empty()
                }) | if decl.static_ctor.is_none() {
                    TypeAttributes::BeforeFieldInit
                } else {
                    TypeAttributes::empty()
                }
            },
            &decl.namespace,
            &decl.name,
            if decl.is_interface {
                None
            } else if decl.is_struct {
                value_type.clone()
            } else {
                Some(object.clone())
            },
        ));
        types.insert(decl.name.clone(), entry.clone());
        if decl.is_struct {
            types.insert(full_name, entry);
        }
    }
    for decl in &enums {
        let entry = types
            .get(&decl.full_name())
            .expect("enum type was declared")
            .clone();
        TypeDef::add_field(
            &entry,
            Field::new(
                FieldFlags::Public | FieldFlags::SpecialName | FieldFlags::RTSpecialName,
                "value__",
                FieldSig {
                    ty: TypeSig::Primitive(decl.base.primitive()),
                },
            ),
        );
        for (name, value) in &decl.members {
            let mut field = Field::new(
                FieldFlags::Public
                    | FieldFlags::Static
                    | FieldFlags::Literal
                    | FieldFlags::HasDefault,
                name,
                FieldSig {
                    ty: TypeSig::ValueType(TypeDefOrRef::TypeDef(entry.clone())),
                },
            );
            field.constant = Some(Constant {
                element_type: decl.base.element_type(),
                value: (*value as u64).to_le_bytes()[..decl.base.width()].to_vec(),
            });
            TypeDef::add_field(&entry, field);
        }
    }

    // A type listed first in the base list may still be an interface
    // (`class X : IFoo`), which has no extends entry.
    let local_interfaces: std::collections::HashSet<String> = program
        .types
        .iter()
        .filter(|decl| decl.is_interface)
        .map(|decl| decl.name.clone())
        .collect();
    let is_interface_name = |name: &str| -> bool {
        let short = name.rsplit('.').next().unwrap_or(name);
        if local_interfaces.contains(short) {
            return true;
        }
        externals
            .find(name)
            .is_some_and(|(_, entry)| entry.get().flags.contains(TypeAttributes::Interface))
    };
    let effective: Vec<(Option<String>, Vec<String>)> = program
        .types
        .iter()
        .map(|decl| {
            let mut base = decl.base.clone();
            let mut interfaces = decl.interfaces.clone();
            if let Some(name) = &base
                && is_interface_name(name)
            {
                interfaces.push(name.clone());
                base = None;
            }
            (base, interfaces)
        })
        .collect();

    // Pass 0.5: resolve base types now that every local type exists.
    for (index, decl) in program.types.iter().enumerate() {
        let Some(base_name) = &effective[index].0 else {
            continue;
        };
        let Some(entry) = types.get(&decl.name).cloned() else {
            continue;
        };
        let resolved = if let Some(local) = types.get(base_name) {
            if is_static_type(local) {
                return Err(type_error(
                    decl,
                    sources,
                    format!("cannot derive from static type '{base_name}'"),
                ));
            }
            if local
                .get()
                .extends
                .as_ref()
                .is_some_and(|base| base.full_name() == "System.ValueType")
            {
                return Err(type_error(
                    decl,
                    sources,
                    format!("cannot derive from struct '{base_name}'"),
                ));
            }
            TypeDefOrRef::TypeDef(local.clone())
        } else if let Some((index, foreign)) = externals.find(base_name) {
            if is_static_type(foreign) {
                return Err(type_error(
                    decl,
                    sources,
                    format!("cannot derive from static type '{base_name}'"),
                ));
            }
            module
                .import_foreign_type(
                    &TypeDefOrRef::TypeDef(foreign.clone()),
                    &externals.modules[*index],
                )
                .map_err(|error| type_error(decl, sources, error.to_string()))?
        } else if let Some(name) = core_exception_name(base_name) {
            module
                .corlib_type(name)
                .map_err(|error| type_error(decl, sources, error.to_string()))?
        } else {
            return Err(type_error(
                decl,
                sources,
                format!("unknown base type '{base_name}'"),
            ));
        };
        entry.get_mut().extends = Some(resolved);
    }

    // Pass 0.6: record implemented interfaces.
    for (index, decl) in program.types.iter().enumerate() {
        if effective[index].1.is_empty() {
            continue;
        }
        let Some(entry) = types.get(&decl.name).cloned() else {
            continue;
        };
        for interface_name in &effective[index].1 {
            let resolved = if let Some(local) = types.get(interface_name) {
                if is_static_type(local) {
                    return Err(type_error(
                        decl,
                        sources,
                        format!("static type '{interface_name}' cannot be an interface"),
                    ));
                }
                if local
                    .get()
                    .extends
                    .as_ref()
                    .is_some_and(|base| base.full_name() == "System.ValueType")
                {
                    return Err(type_error(
                        decl,
                        sources,
                        format!("struct '{interface_name}' cannot be an interface"),
                    ));
                }
                TypeDefOrRef::TypeDef(local.clone())
            } else if let Some((index, foreign)) = externals.find(interface_name) {
                if is_static_type(foreign) {
                    return Err(type_error(
                        decl,
                        sources,
                        format!("static type '{interface_name}' cannot be an interface"),
                    ));
                }
                module
                    .import_foreign_type(
                        &TypeDefOrRef::TypeDef(foreign.clone()),
                        &externals.modules[*index],
                    )
                    .map_err(|error| type_error(decl, sources, error.to_string()))?
            } else {
                return Err(type_error(
                    decl,
                    sources,
                    format!("unknown interface '{interface_name}'"),
                ));
            };
            entry
                .get_mut()
                .interfaces
                .push(InterfaceImpl::new(resolved));
        }
    }

    // Pass 1: fields, method shells and constructors.
    let mut symbols: HashMap<String, TypeSymbol> = HashMap::new();
    let mut jobs: Vec<(usize, Entry<Method>, Job)> = Vec::new();
    for (index, decl) in program.types.iter().enumerate() {
        let ty = types[&decl.name].clone();
        let mut symbol = TypeSymbol {
            base: effective[index].0.clone(),
            interfaces: effective[index].1.clone(),
            ..TypeSymbol::default()
        };

        for field in &decl.fields {
            let signature = resolve_type(&field.ty, &types, externals, &mut module)
                .map_err(|message| type_error(decl, sources, message))?;
            let flags = field.visibility.field_flags()
                | if field.is_const {
                    FieldFlags::Literal | FieldFlags::HasDefault
                } else {
                    FieldFlags::empty()
                }
                | if field.is_readonly {
                    FieldFlags::InitOnly
                } else {
                    FieldFlags::empty()
                }
                | if field.is_static {
                    FieldFlags::Static
                } else {
                    FieldFlags::empty()
                };
            let mut definition = Field::new(flags, &field.name, FieldSig { ty: signature });
            if field.is_const {
                definition.constant = Some(
                    source_field_constant(field)
                        .map_err(|message| type_error(decl, sources, message))?,
                );
            }
            let entry = TypeDef::add_field(&ty, definition);
            symbol.fields.insert(
                field.name.clone(),
                FieldSymbol {
                    ty: field.ty.clone(),
                    is_static: field.is_static,
                    entry,
                },
            );
        }

        for (method_index, method) in decl.methods.iter().enumerate() {
            let params: Vec<TypeSig> = method
                .params
                .iter()
                .map(|(ty, _)| resolve_type(ty, &types, externals, &mut module))
                .collect::<Result<Vec<_>, String>>()
                .map_err(|message| type_error(decl, sources, message))?;
            let return_signature = resolve_type(&method.ret, &types, externals, &mut module)
                .map_err(|message| type_error(decl, sources, message))?;
            let virtual_flags = if decl.is_interface {
                MethodFlags::Public
                    | MethodFlags::Virtual
                    | MethodFlags::NewSlot
                    | MethodFlags::Abstract
            } else {
                let mut flags = match method.virtual_kind {
                    VirtualKind::Virtual => MethodFlags::Virtual | MethodFlags::NewSlot,
                    VirtualKind::Override => MethodFlags::Virtual | MethodFlags::ReuseSlot,
                    VirtualKind::None => MethodFlags::empty(),
                };
                if !effective[index].1.is_empty()
                    && !method.is_static
                    && !method.is_ctor
                    && method.visibility == Visibility::Public
                    && method.virtual_kind == VirtualKind::None
                {
                    // Implicit interface implementations must be virtual and final.
                    flags =
                        flags | MethodFlags::Virtual | MethodFlags::NewSlot | MethodFlags::Final;
                }
                flags
            };
            let (flags, name, signature) = if method.is_ctor {
                (
                    method.visibility.method_flags()
                        | MethodFlags::HideBySig
                        | MethodFlags::SpecialName
                        | MethodFlags::RTSpecialName,
                    ".ctor".to_string(),
                    MethodSig::new_instance(TypeSig::VOID, params),
                )
            } else if method.is_static {
                (
                    method.visibility.method_flags() | MethodFlags::Static | MethodFlags::HideBySig,
                    method.name.clone(),
                    MethodSig::new_static(return_signature, params),
                )
            } else {
                (
                    method.visibility.method_flags() | MethodFlags::HideBySig | virtual_flags,
                    method.name.clone(),
                    MethodSig::new_instance(return_signature, params),
                )
            };
            let shell = Method::new(flags, name.clone(), signature);
            // Abstract interface methods must not carry a body.
            let shell = if decl.is_interface {
                shell
            } else {
                shell.with_body(Emitter::new().finish())
            };
            let entry = TypeDef::add_method(&ty, shell);
            entry.get_mut().params = method
                .params
                .iter()
                .enumerate()
                .map(|(parameter_index, (_, parameter_name))| {
                    Entry::new(Param::new(
                        parameter_index as u16 + 1,
                        parameter_name.clone(),
                    ))
                })
                .collect();
            let symbol_method = MethodSymbol {
                name: name.clone(),
                is_static: method.is_static && !method.is_ctor,
                ret: method.ret.clone(),
                params: method.params.iter().map(|(ty, _)| ty.clone()).collect(),
                entry: entry.clone(),
            };
            if method.is_ctor {
                symbol.ctors.push(symbol_method);
            } else {
                symbol.methods.push(symbol_method);
            }
            if !decl.is_interface {
                jobs.push((index, entry, Job::Declared(method_index)));
            }
        }

        for (property_index, property) in decl.properties.iter().enumerate() {
            let signature = resolve_type(&property.ty, &types, externals, &mut module)
                .map_err(|message| type_error(decl, sources, message))?;
            let auto_getter = property.has_getter && property.getter.is_none();
            let auto_setter = property.has_setter && property.setter.is_none();
            let backing_name = format!("<{}>k__BackingField", property.name);
            if auto_getter || auto_setter {
                let flags = if property.is_static {
                    FieldFlags::Private | FieldFlags::Static
                } else {
                    FieldFlags::Private
                };
                let entry = TypeDef::add_field(
                    &ty,
                    Field::new(
                        flags,
                        &backing_name,
                        FieldSig {
                            ty: signature.clone(),
                        },
                    ),
                );
                symbol.fields.insert(
                    backing_name,
                    FieldSymbol {
                        ty: property.ty.clone(),
                        is_static: property.is_static,
                        entry,
                    },
                );
            }
            let mut property_symbol = PropertySymbol {
                ty: property.ty.clone(),
                is_static: property.is_static,
                getter: None,
                setter: None,
            };
            let extra = MethodFlags::HideBySig
                | MethodFlags::SpecialName
                | if property.is_static {
                    MethodFlags::Static
                } else {
                    MethodFlags::empty()
                };
            if property.has_getter {
                let signature = if property.is_static {
                    MethodSig::new_static(signature.clone(), vec![])
                } else {
                    MethodSig::new_instance(signature.clone(), vec![])
                };
                let entry = TypeDef::add_method(
                    &ty,
                    Method::new(
                        extra
                            | property
                                .getter_visibility
                                .unwrap_or(property.visibility)
                                .method_flags(),
                        format!("get_{}", property.name),
                        signature,
                    )
                    .with_body(Emitter::new().finish()),
                );
                property_symbol.getter = Some(entry.clone());
                jobs.push((index, entry, Job::Getter(property_index)));
            }
            if property.has_setter {
                let signature = if property.is_static {
                    MethodSig::new_static(TypeSig::VOID, vec![signature.clone()])
                } else {
                    MethodSig::new_instance(TypeSig::VOID, vec![signature.clone()])
                };
                let entry = TypeDef::add_method(
                    &ty,
                    Method::new(
                        extra
                            | property
                                .setter_visibility
                                .unwrap_or(property.visibility)
                                .method_flags(),
                        format!("set_{}", property.name),
                        signature,
                    )
                    .with_body(Emitter::new().finish()),
                );
                entry.get_mut().params = vec![Entry::new(Param::new(1, "value"))];
                property_symbol.setter = Some(entry.clone());
                jobs.push((index, entry, Job::Setter(property_index)));
            }
            ty.get_mut()
                .properties
                .push(Entry::new(hao::dotnet::entries::Property {
                    name: property.name.clone(),
                    signature: hao::dotnet::signature::PropertySig {
                        has_this: !property.is_static,
                        ty: signature.clone(),
                        params: Vec::new(),
                    },
                    getter: property_symbol.getter.clone(),
                    setter: property_symbol.setter.clone(),
                    ..Default::default()
                }));
            symbol
                .properties
                .insert(property.name.clone(), property_symbol);
        }

        if !decl.is_interface && !decl.is_struct && !decl.is_static && symbol.ctors.is_empty() {
            let flags = MethodFlags::Public
                | MethodFlags::HideBySig
                | MethodFlags::SpecialName
                | MethodFlags::RTSpecialName;
            let entry = TypeDef::add_method(
                &ty,
                Method::new(
                    flags,
                    ".ctor",
                    MethodSig::new_instance(TypeSig::VOID, vec![]),
                )
                .with_body(Emitter::new().finish()),
            );
            symbol.ctors.push(MethodSymbol {
                name: ".ctor".to_string(),
                is_static: false,
                ret: Ty::Void,
                params: Vec::new(),
                entry: entry.clone(),
            });
            jobs.push((index, entry, Job::DefaultCtor));
        }

        symbols.insert(decl.name.clone(), symbol);
    }

    // An explicit static constructor follows the static field initializers.
    for (index, decl) in program.types.iter().enumerate() {
        if decl.static_ctor.is_none()
            && !decl
                .fields
                .iter()
                .any(|field| field.is_static && !field.is_const && field.init.is_some())
        {
            continue;
        }
        let ty = types[&decl.name].clone();
        let flags = MethodFlags::Private
            | MethodFlags::Static
            | MethodFlags::HideBySig
            | MethodFlags::SpecialName
            | MethodFlags::RTSpecialName;
        let entry = TypeDef::add_method(
            &ty,
            Method::new(
                flags,
                ".cctor",
                MethodSig::new_static(TypeSig::VOID, vec![]),
            )
            .with_body(Emitter::new().finish()),
        );
        jobs.push((index, entry, Job::StaticCtor));
    }

    for (index, entry, job) in jobs {
        let body = emit_body(
            &mut module,
            externals,
            &program.enums,
            effective[index].0.as_ref(),
            &types,
            &symbols,
            &program.types[index],
            &job,
            sources,
        )?;
        entry.edit_body(|slot| *slot = body);
    }

    module.write().map_err(|error| {
        program_error(
            program,
            sources,
            format!("assembly emission failed: {error}"),
        )
    })
}

#[derive(Clone, Copy)]
enum Job {
    Declared(usize),
    DefaultCtor,
    StaticCtor,
    Getter(usize),
    Setter(usize),
}

#[allow(clippy::too_many_arguments)]
fn emit_body(
    module: &mut Module,
    externals: &Externals,
    enums: &EnumValues,
    base: Option<&String>,
    types: &HashMap<String, Entry<TypeDef>>,
    symbols: &HashMap<String, TypeSymbol>,
    decl: &TypeDecl,
    job: &Job,
    sources: &[(String, String)],
) -> Result<MethodBody, CompileError> {
    let source = sources
        .iter()
        .find(|(file, _)| file == &decl.file)
        .map(|(_, text)| text.clone())
        .unwrap_or_default();
    let mut context = FnEmitter {
        module,
        types,
        symbols,
        current: decl.name.clone(),
        namespace: decl.namespace.clone(),
        enum_imports: &decl.enum_imports,
        is_static: true,
        field_write_context: FieldWriteContext::Normal,
        return_ty: Ty::Void,
        externals,
        enums,
        base: base.cloned(),
        locals: HashMap::new(),
        initializing_struct: None,
        int_constant_cache: HashMap::new(),
        params: Vec::new(),
        controls: Vec::new(),
        protected_depth: 0,
        finally_depth: 0,
        catches: Vec::new(),
        return_target: None,
        emitter: TrackedEmitter::new(),
        file: decl.file.clone(),
        source,
        offset: decl.offset,
    };
    match job {
        Job::Declared(method_index) => {
            let method = &decl.methods[*method_index];
            context.is_static = method.is_static;
            context.return_ty = method.ret.clone();
            context.params = method
                .params
                .iter()
                .map(|(ty, name)| (name.clone(), ty.clone()))
                .collect();
            let Some(body) = &method.body else {
                if decl.is_interface {
                    return context
                        .emitter
                        .finish()
                        .map_err(|message| type_error(decl, sources, message));
                }
                return Err(context.error(format!(
                    "method '{}' has no body; extern methods are not supported",
                    method.name
                )));
            };
            if method.is_ctor {
                context.call_base_constructor(method.base_args.as_deref())?;
                for field in &decl.fields {
                    if field.is_static {
                        continue;
                    }
                    if let Some(initializer) = &field.init {
                        context.field_store(field, initializer)?;
                    }
                }
                context.field_write_context = FieldWriteContext::InstanceConstructor;
            }
            context.validate_do_scopes(body)?;
            context.block(body)?;
            if method.is_ctor || method.ret == Ty::Void {
                context.emitter.ret();
            } else {
                if !block_terminates(body) {
                    return Err(context.error(format!(
                        "method '{}' must end with a return statement",
                        method.name
                    )));
                }
            }
        }
        Job::DefaultCtor => {
            context.is_static = false;
            context.call_base_constructor(None)?;
            for field in &decl.fields {
                if field.is_static {
                    continue;
                }
                if let Some(initializer) = &field.init {
                    context.field_store(field, initializer)?;
                }
            }
            context.emitter.ret();
        }
        Job::Getter(property_index) => {
            let property = &decl.properties[*property_index];
            context.is_static = property.is_static;
            context.return_ty = property.ty.clone();
            match &property.getter {
                Some(body) => {
                    context.validate_do_scopes(body)?;
                    context.block(body)?;
                    if !block_terminates(body) {
                        return Err(context.error("a getter must return a value"));
                    }
                }
                None => {
                    let name = format!("<{}>k__BackingField", property.name);
                    context.load(&name)?;
                    context.emitter.ret();
                }
            }
        }
        Job::Setter(property_index) => {
            let property = &decl.properties[*property_index];
            context.is_static = property.is_static;
            context.return_ty = Ty::Void;
            context.params = vec![("value".to_string(), property.ty.clone())];
            match &property.setter {
                Some(body) => {
                    context.validate_do_scopes(body)?;
                    context.block(body)?;
                    context.emitter.ret();
                }
                None => {
                    let name = format!("<{}>k__BackingField", property.name);
                    context.assign(&name, &Expr::Name("value".to_string()))?;
                    context.emitter.ret();
                }
            }
        }
        Job::StaticCtor => {
            context.is_static = true;
            context.field_write_context = FieldWriteContext::StaticConstructor;
            for field in &decl.fields {
                if !field.is_static || field.is_const {
                    continue;
                }
                if let Some(initializer) = &field.init {
                    context.field_store(field, initializer)?;
                }
            }
            if let Some(body) = &decl.static_ctor {
                context.validate_do_scopes(body)?;
                context.block(body)?;
            }
            context.emitter.ret();
        }
    }
    if let Some((target, local)) = context.return_target {
        context.emitter.mark(target);
        if let Some(local) = local {
            context.emitter.ldloc(local);
        }
        context.emitter.ret();
    }
    context
        .emitter
        .finish()
        .map_err(|message| type_error(decl, sources, message))
}

#[derive(Clone, Copy)]
struct ControlTarget {
    end: EmitLabel,
    next: Option<EmitLabel>,
    protected_depth: usize,
    finally_depth: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldWriteContext {
    Normal,
    InstanceConstructor,
    StaticConstructor,
}

struct FnEmitter<'a> {
    module: &'a mut Module,
    types: &'a HashMap<String, Entry<TypeDef>>,
    symbols: &'a HashMap<String, TypeSymbol>,
    current: String,
    namespace: String,
    enum_imports: &'a EnumImports,
    is_static: bool,
    field_write_context: FieldWriteContext,
    return_ty: Ty,
    externals: &'a Externals,
    enums: &'a EnumValues,
    base: Option<String>,
    locals: HashMap<String, (u16, Ty)>,
    initializing_struct: Option<String>,
    int_constant_cache: HashMap<usize, Option<i32>>,
    params: Vec<(String, Ty)>,
    controls: Vec<ControlTarget>,
    protected_depth: usize,
    finally_depth: usize,
    catches: Vec<usize>,
    return_target: Option<(EmitLabel, Option<u16>)>,
    emitter: TrackedEmitter,
    file: String,
    source: String,
    offset: usize,
}

enum IncrementMemberTarget {
    Field(Entry<Field>),
    Property {
        getter: Entry<Method>,
        setter: Entry<Method>,
    },
}

enum StaticFieldValue {
    Location(MemberToken),
    Literal(Constant),
}

enum LiteralValue {
    Int(i32),
    Float(f32),
    Bool(bool),
    String(String),
    Null,
    Enum(i128, EnumBase),
}

impl<'a> FnEmitter<'a> {
    fn error(&self, message: impl Into<String>) -> CompileError {
        CompileError::new(&self.file, &self.source, self.offset, message)
    }

    fn decode_literal(&self, ty: &Ty, constant: &Constant) -> Result<LiteralValue, CompileError> {
        let value = &constant.value;
        let result = match (ty, constant.element_type) {
            (Ty::Int, ElementType::I4) if value.len() == 4 => {
                LiteralValue::Int(i32::from_le_bytes(value.as_slice().try_into().unwrap()))
            }
            (Ty::Float, ElementType::R4) if value.len() == 4 => {
                LiteralValue::Float(f32::from_le_bytes(value.as_slice().try_into().unwrap()))
            }
            (Ty::Bool, ElementType::Boolean) if value.len() == 1 && value[0] <= 1 => {
                LiteralValue::Bool(value[0] != 0)
            }
            (Ty::String, ElementType::String) if value.len().is_multiple_of(2) => {
                LiteralValue::String(
                    constant
                        .as_string()
                        .ok_or_else(|| self.error("constant string contains unsupported UTF-16"))?,
                )
            }
            (Ty::String | Ty::Object | Ty::Class(_), ElementType::Class)
                if value.as_slice() == [0, 0, 0, 0] =>
            {
                LiteralValue::Null
            }
            (Ty::Enum(_, base), element)
                if element == base.element_type() && value.len() == base.width() =>
            {
                let raw = constant
                    .as_i64()
                    .ok_or_else(|| self.error("invalid enum constant"))?;
                let number = if *base == EnumBase::U8 {
                    raw as u64 as i128
                } else {
                    i128::from(raw)
                };
                LiteralValue::Enum(number, *base)
            }
            _ => return Err(self.error("constant metadata type or value is not supported")),
        };
        Ok(result)
    }

    fn emit_literal(&mut self, ty: &Ty, constant: &Constant) -> Result<(), CompileError> {
        match self.decode_literal(ty, constant)? {
            LiteralValue::Int(value) => {
                self.emitter.ldc_i4(value);
            }
            LiteralValue::Float(value) => {
                self.emitter
                    .emit(Instruction::new(OpCode::Ldc_R4, Operand::Float32(value)));
            }
            LiteralValue::Bool(value) => {
                self.emitter.ldc_i4(i32::from(value));
            }
            LiteralValue::String(value) => {
                self.emitter.ldstr(value);
            }
            LiteralValue::Null => {
                self.emitter.ldnull();
            }
            LiteralValue::Enum(value, base) if base.width() == 8 => {
                self.emitter.emit(Instruction::new(
                    OpCode::Ldc_I8,
                    Operand::Int64(value as u64 as i64),
                ));
            }
            LiteralValue::Enum(value, _) => {
                self.emitter.ldc_i4(value as i32);
            }
        }
        Ok(())
    }

    fn literal_reference(
        &mut self,
        expression: &Expr,
    ) -> Result<Option<(Ty, Constant)>, CompileError> {
        match expression {
            Expr::Name(name) => {
                if self.locals.contains_key(name)
                    || self.params.iter().any(|(parameter, _)| parameter == name)
                {
                    return Ok(None);
                }
                let Some(field) = self.field_symbol(&self.current, name) else {
                    return Ok(None);
                };
                let entry = field.entry.get();
                if !entry.flags.contains(FieldFlags::Literal) {
                    return Ok(None);
                }
                Ok(Some((
                    field.ty,
                    entry
                        .constant
                        .clone()
                        .ok_or_else(|| self.error("constant field has no metadata value"))?,
                )))
            }
            Expr::Member { target, name } => match self.static_field(target, name, false)? {
                Some((ty, StaticFieldValue::Literal(constant))) => Ok(Some((ty, constant))),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    fn evaluate_int_constant(&mut self, expression: &Expr) -> Result<Option<i32>, CompileError> {
        let key = std::ptr::from_ref(expression) as usize;
        if let Some(cached) = self.int_constant_cache.get(&key) {
            return Ok(*cached);
        }
        let result = match expression {
            Expr::Int(value, true) => Some(*value),
            Expr::Default(Ty::Int) => Some(0),
            Expr::Member { target, name } if matches!(target.as_ref(), Expr::Name(type_name) if type_name == "int") => {
                match name.as_str() {
                    "MinValue" => Some(i32::MIN),
                    "MaxValue" => Some(i32::MAX),
                    _ => None,
                }
            }
            Expr::Name(_) | Expr::Member { .. } => match self.literal_reference(expression)? {
                Some((ty, constant)) if ty == Ty::Int => {
                    match self.decode_literal(&ty, &constant)? {
                        LiteralValue::Int(value) => Some(value),
                        _ => return Err(self.error("invalid int constant metadata")),
                    }
                }
                _ => None,
            },
            Expr::Negate(inner) => match self.evaluate_int_constant(inner)? {
                Some(value) => Some(
                    value
                        .checked_neg()
                        .ok_or_else(|| self.error("constant integer overflow"))?,
                ),
                None => None,
            },
            Expr::Complement(inner) => self.evaluate_int_constant(inner)?.map(|value| !value),
            Expr::Binary(op, left, right)
                if matches!(
                    op,
                    BinaryOp::Add
                        | BinaryOp::Subtract
                        | BinaryOp::Multiply
                        | BinaryOp::Divide
                        | BinaryOp::Remainder
                        | BinaryOp::BitAnd
                        | BinaryOp::BitOr
                        | BinaryOp::BitXor
                ) =>
            {
                let left = self.evaluate_int_constant(left)?;
                let right = self.evaluate_int_constant(right)?;
                match (left, right) {
                    (Some(left), Some(right)) => Some(
                        match op {
                            BinaryOp::Add => left.checked_add(right),
                            BinaryOp::Subtract => left.checked_sub(right),
                            BinaryOp::Multiply => left.checked_mul(right),
                            BinaryOp::Divide => left.checked_div(right),
                            BinaryOp::Remainder => left.checked_rem(right),
                            BinaryOp::BitAnd => Some(left & right),
                            BinaryOp::BitOr => Some(left | right),
                            BinaryOp::BitXor => Some(left ^ right),
                            _ => unreachable!(),
                        }
                        .ok_or_else(|| {
                            self.error("constant integer overflow or division by zero")
                        })?,
                    ),
                    _ => None,
                }
            }
            _ => None,
        };
        self.int_constant_cache.insert(key, result);
        Ok(result)
    }

    fn has_untracked_enum_constant(&mut self, expression: &Expr) -> Result<bool, CompileError> {
        if self.enum_constant(expression).is_some() {
            return Ok(false);
        }
        match expression {
            Expr::Name(_) | Expr::Member { .. } => Ok(matches!(
                self.literal_reference(expression)?,
                Some((Ty::Enum(_, _), _))
            )),
            Expr::Binary(_, left, right) => Ok(self.has_untracked_enum_constant(left)?
                || self.has_untracked_enum_constant(right)?),
            Expr::Cast(_, inner) | Expr::Negate(inner) | Expr::Complement(inner) => {
                self.has_untracked_enum_constant(inner)
            }
            _ => Ok(false),
        }
    }

    fn require_member_access(
        &self,
        access: u16,
        owner: &str,
        local: bool,
        receiver: Option<&str>,
        is_static: bool,
        name: &str,
    ) -> Result<(), CompileError> {
        let owner_short = owner.rsplit('.').next().unwrap_or(owner);
        let same_type = local && owner_short == self.current;
        if access == 6
            || (local && matches!(access, 3 | 5))
            || (same_type && (1..=6).contains(&access))
        {
            return Ok(());
        }
        let derived = same_type
            || self.assignable(
                &Ty::Class(self.current.clone()),
                &Ty::Class(owner.to_string()),
            );
        let receiver_allowed = is_static
            || receiver.is_none_or(|ty| {
                self.assignable(&Ty::Class(ty.to_string()), &Ty::Class(self.current.clone()))
            });
        let family = derived && receiver_allowed;
        let allowed = match access {
            6 => true,
            1 => same_type,
            3 => local,
            4 => family,
            5 => local || family,
            2 => local && family,
            _ => false,
        };
        if allowed {
            Ok(())
        } else {
            Err(self.error(format!("member '{owner}.{name}' is inaccessible")))
        }
    }

    fn require_method_access(
        &self,
        method: &Entry<Method>,
        local: bool,
        receiver: Option<&str>,
    ) -> Result<(), CompileError> {
        let method = method.get();
        let owner = method
            .declaring_type
            .upgrade()
            .ok_or_else(|| self.error(format!("method '{}' has no declaring type", method.name)))?;
        if !local && !owner.get().is_visible() {
            return Err(self.error(format!(
                "referenced type '{}' is inaccessible",
                owner.get().full_name()
            )));
        }
        self.require_member_access(
            (method.flags & MethodFlags::MemberAccessMask).bits(),
            &owner.get().full_name(),
            local,
            receiver,
            method.flags.contains(MethodFlags::Static),
            &method.name,
        )
    }

    fn require_field_access(
        &self,
        field: &Entry<Field>,
        local: bool,
        receiver: Option<&str>,
    ) -> Result<(), CompileError> {
        let field = field.get();
        let owner = field
            .declaring_type
            .upgrade()
            .ok_or_else(|| self.error(format!("field '{}' has no declaring type", field.name)))?;
        if !local && !owner.get().is_visible() {
            return Err(self.error(format!(
                "referenced type '{}' is inaccessible",
                owner.get().full_name()
            )));
        }
        self.require_member_access(
            (field.flags & FieldFlags::FieldAccessMask).bits(),
            &owner.get().full_name(),
            local,
            receiver,
            field.flags.contains(FieldFlags::Static),
            &field.name,
        )
    }

    fn require_field_writable(
        &self,
        field: &Entry<Field>,
        local: bool,
        this_receiver: bool,
    ) -> Result<(), CompileError> {
        let field = field.get();
        if field.flags.contains(FieldFlags::Literal) {
            return Err(self.error(format!(
                "constant field '{}' cannot be assigned",
                field.name
            )));
        }
        if !field.flags.contains(FieldFlags::InitOnly) {
            return Ok(());
        }
        let same_type = local
            && field.declaring_type.upgrade().is_some_and(|owner| {
                let owner = owner.get();
                owner.name == self.current && owner.namespace == self.namespace
            });
        let allowed = same_type
            && if field.flags.contains(FieldFlags::Static) {
                self.field_write_context == FieldWriteContext::StaticConstructor
            } else {
                this_receiver && self.field_write_context == FieldWriteContext::InstanceConstructor
            };
        if allowed {
            Ok(())
        } else {
            Err(self.error(format!(
                "readonly field '{}' cannot be assigned here",
                field.name
            )))
        }
    }

    fn argument_index(&self, position: usize) -> u16 {
        if self.is_static {
            position as u16
        } else {
            position as u16 + 1
        }
    }

    fn field_symbol(&self, type_name: &str, name: &str) -> Option<FieldSymbol> {
        self.symbols
            .get(type_name)
            .and_then(|symbol| symbol.fields.get(name))
            .map(|field| FieldSymbol {
                ty: field.ty.clone(),
                is_static: field.is_static,
                entry: field.entry.clone(),
            })
    }

    fn shadows_type_root(&self, path: &str) -> bool {
        let root = path.split('.').next().unwrap_or(path);
        self.locals.contains_key(root)
            || self.params.iter().any(|(name, _)| name == root)
            || self.field_symbol(&self.current, root).is_some()
            || self
                .symbols
                .get(&self.current)
                .is_some_and(|symbol| symbol.properties.contains_key(root))
    }

    /// Resolve a type-qualified field without evaluating the type name as a value.
    fn static_field(
        &mut self,
        receiver: &Expr,
        name: &str,
        writing: bool,
    ) -> Result<Option<(Ty, StaticFieldValue)>, CompileError> {
        let Some(path) = Self::type_path(receiver) else {
            return Ok(None);
        };
        if self.shadows_type_root(&path) {
            return Ok(None);
        }
        let mut parts = path.splitn(2, '.');
        let root = parts.next().unwrap_or(&path);
        let alias = self.enum_imports.aliases.get(root);
        let path = if let Some(alias) = alias {
            let alias = alias.strip_prefix("global::").unwrap_or(alias);
            match parts.next() {
                Some(rest) => format!("{alias}.{rest}"),
                None => alias.to_string(),
            }
        } else {
            path
        };
        let lookup = |full: &str| -> Option<(bool, String)> {
            let short = full.rsplit('.').next().unwrap_or(full);
            if let Some(entry) = self.types.get(full).or_else(|| self.types.get(short))
                && entry.get().full_name() == full
            {
                return Some((true, entry.get().name.clone()));
            }
            self.externals
                .types
                .contains_key(full)
                .then(|| (false, full.to_string()))
        };
        let owner = if alias.is_some() {
            lookup(&path)
        } else {
            let mut namespace = self.namespace.as_str();
            let mut found = None;
            while !namespace.is_empty() {
                if let Some(candidate) = lookup(&format!("{namespace}.{path}")) {
                    found = Some(candidate);
                    break;
                }
                namespace = namespace.rsplit_once('.').map_or("", |(parent, _)| parent);
            }
            if found.is_none() {
                found = lookup(&path);
            }
            if found.is_none() {
                for namespace in &self.enum_imports.namespaces {
                    if let Some(candidate) = lookup(&format!("{namespace}.{path}")) {
                        if found
                            .as_ref()
                            .is_some_and(|previous| previous != &candidate)
                        {
                            return Err(self.error(format!("type name '{path}' is ambiguous")));
                        }
                        found = Some(candidate);
                    }
                }
            }
            found
        };
        let Some((local, owner)) = owner else {
            return Ok(None);
        };
        let (ty, value, flags) = if local {
            let Some(field) = self.field_symbol(&owner, name) else {
                return Ok(None);
            };
            self.require_field_access(&field.entry, true, None)?;
            if writing {
                self.require_field_writable(&field.entry, true, true)?;
            }
            let (flags, constant) = {
                let entry = field.entry.get();
                (entry.flags, entry.constant.clone())
            };
            let value = if flags.contains(FieldFlags::Literal) {
                StaticFieldValue::Literal(
                    constant.ok_or_else(|| self.error("constant field has no metadata value"))?,
                )
            } else {
                StaticFieldValue::Location(MemberToken::Field(field.entry))
            };
            (field.ty, value, flags)
        } else {
            let short = owner.rsplit('.').next().unwrap_or(&owner);
            if self
                .externals
                .by_short_name
                .get(short)
                .is_some_and(|names| {
                    names
                        .iter()
                        .filter(|candidate| *candidate == &owner)
                        .count()
                        > 1
                })
            {
                return Err(self.error(format!("referenced type '{owner}' is ambiguous")));
            }
            let Some((index, field)) = self.externals.field_in_chain(&owner, name) else {
                return Ok(None);
            };
            self.require_field_access(&field, false, None)?;
            if writing {
                self.require_field_writable(&field, false, true)?;
            }
            let (flags, signature, constant) = {
                let field = field.get();
                (
                    field.flags,
                    field.signature.ty.clone(),
                    field.constant.clone(),
                )
            };
            let ty = external_ty(&signature, self.enums, false);
            fn supported(ty: &Ty) -> bool {
                match ty {
                    Ty::Int
                    | Ty::Float
                    | Ty::Bool
                    | Ty::String
                    | Ty::Object
                    | Ty::Enum(_, _)
                    | Ty::Class(_) => true,
                    Ty::Array(element) => supported(element),
                    _ => false,
                }
            }
            if !supported(&ty) {
                return Err(self.error(format!("field '{owner}.{name}' has an unsupported type")));
            }
            let value = if flags.contains(FieldFlags::Literal) {
                StaticFieldValue::Literal(
                    constant.ok_or_else(|| self.error("constant field has no metadata value"))?,
                )
            } else {
                let token = self
                    .module
                    .import_foreign_field(&field, &self.externals.modules[index])
                    .map_err(|error| self.error(error.to_string()))?;
                StaticFieldValue::Location(token)
            };
            (ty, value, flags)
        };
        if !flags.contains(FieldFlags::Static) {
            return Err(self.error(format!("'{owner}.{name}' is not static")));
        }
        Ok(Some((ty, value)))
    }

    fn method_symbol(
        &self,
        type_name: &str,
        name: &str,
        arity: usize,
        want_static: bool,
    ) -> Option<MethodSymbol> {
        self.symbols.get(type_name).and_then(|symbol| {
            symbol
                .methods
                .iter()
                .find(|method| {
                    method.name == name
                        && method.params.len() == arity
                        && method.is_static == want_static
                })
                .cloned()
        })
    }

    /// Instance methods are inherited, so search the local base chain too.
    fn inherited_method(&self, type_name: &str, name: &str, arity: usize) -> Option<MethodSymbol> {
        let mut current = Some(type_name.to_string());
        while let Some(owner) = current {
            let short = owner.rsplit('.').next().unwrap_or(&owner).to_string();
            if let Some(method) = self.method_symbol(&short, name, arity, false) {
                return Some(method);
            }
            let base = self
                .symbols
                .get(&short)
                .and_then(|symbol| symbol.base.clone())?;
            current = Some(base);
        }
        None
    }

    fn constructor(&self, type_name: &str, arity: usize) -> Option<MethodSymbol> {
        self.symbols.get(type_name).and_then(|symbol| {
            symbol
                .ctors
                .iter()
                .find(|ctor| ctor.params.len() == arity)
                .cloned()
        })
    }

    fn assignable(&self, actual: &Ty, expected: &Ty) -> bool {
        if actual == expected {
            return true;
        }
        let (Ty::Class(actual_name), Ty::Class(expected_name)) = (actual, expected) else {
            return false;
        };
        if self.implements_interface(actual, expected) {
            return true;
        }
        let mut current = Some(actual_name.clone());
        while let Some(name) = current {
            let short = name.rsplit('.').next().unwrap_or(&name).to_string();
            let Some(symbol) = self.symbols.get(&short) else {
                return false;
            };
            match &symbol.base {
                Some(base) => {
                    if base == expected_name
                        || base.rsplit('.').next() == Some(expected_name.as_str())
                    {
                        return true;
                    }
                    current = Some(base.clone());
                }
                None => return false,
            }
        }
        false
    }

    /// True when `actual` (or one of its bases) lists `expected` as an interface.
    fn implements_interface(&self, actual: &Ty, expected: &Ty) -> bool {
        let (Ty::Class(actual_name), Ty::Class(expected_name)) = (actual, expected) else {
            return false;
        };
        let expected_short = expected_name.rsplit('.').next().unwrap_or(expected_name);
        let mut current = Some(actual_name.clone());
        while let Some(name) = current {
            let short = name.rsplit('.').next().unwrap_or(&name).to_string();
            let Some(symbol) = self.symbols.get(&short) else {
                return false;
            };
            if symbol.interfaces.iter().any(|interface| {
                interface == expected_name || interface.rsplit('.').next() == Some(expected_short)
            }) {
                return true;
            }
            current = symbol.base.clone();
        }
        false
    }

    /// Follows local base classes until reaching a type owned by a reference
    /// assembly, which is the entry point for inherited external members.
    fn external_base_of(&self, type_name: &str) -> Option<String> {
        let mut current = type_name.to_string();
        let mut depth = 0usize;
        loop {
            if depth > 12 {
                return None;
            }
            depth += 1;
            match self.symbols.get(&current) {
                Some(symbol) => current = symbol.base.clone()?,
                None => return Some(current),
            }
        }
    }

    fn require_assignable(&self, actual: &Ty, expected: &Ty, what: &str) -> Result<(), String> {
        if *expected == Ty::Object && is_reference(actual) {
            return Ok(());
        }
        if *actual == Ty::Null
            && matches!(
                expected,
                Ty::Object | Ty::String | Ty::Class(_) | Ty::Array(_)
            )
        {
            return Ok(());
        }
        if self.assignable(actual, expected) {
            Ok(())
        } else {
            Self::require(actual, expected, what)
        }
    }

    /// Convert the value on top of the evaluation stack when C# requires it.
    fn convert_assignable(&mut self, actual: &Ty, expected: &Ty, what: &str) -> Result<(), String> {
        if matches!(actual, Ty::Enum(_, _)) && self.enum_boxing_cost(expected).is_some() {
            let token = self.enum_type_token(actual)?;
            self.emitter
                .emit(Instruction::new(OpCode::Box, Operand::Token(token)));
            Ok(())
        } else if *actual == Ty::Int && *expected == Ty::Float {
            self.emitter.op(OpCode::Conv_R4);
            Ok(())
        } else {
            self.require_assignable(actual, expected, what)
        }
    }

    fn enum_boxing_cost(&self, ty: &Ty) -> Option<u32> {
        let Ty::Class(name) = ty else {
            return (*ty == Ty::Object).then_some(3);
        };
        let cost = match name.as_str() {
            "System.Enum" => 1,
            "System.ValueType" => 2,
            _ => return None,
        };
        let short = name.rsplit('.').next()?;
        if self
            .types
            .get(short)
            .is_some_and(|entry| entry.get().full_name() == *name)
        {
            return None;
        }
        let (index, entry) = self.externals.types.get(name)?;
        if !entry.get().is_visible() {
            return None;
        }
        if !trusted_corlib(self.externals, *index) {
            return None;
        }
        Some(cost)
    }

    fn enum_type_token(&mut self, ty: &Ty) -> Result<MemberToken, String> {
        match resolve_type(ty, self.types, self.externals, &mut *self.module)? {
            TypeSig::ValueType(TypeDefOrRef::TypeDef(entry)) => Ok(MemberToken::TypeDef(entry)),
            TypeSig::ValueType(TypeDefOrRef::TypeRef(entry)) => Ok(MemberToken::TypeRef(entry)),
            TypeSig::ValueType(TypeDefOrRef::TypeSpec(entry)) => Ok(MemberToken::TypeSpec(entry)),
            _ => Err("enum is not a value type".to_string()),
        }
    }

    fn constant_int(expression: &Expr) -> Option<i32> {
        match expression {
            Expr::Int(value, true) => Some(*value),
            Expr::Default(Ty::Int) => Some(0),
            Expr::Member { target, name } if matches!(target.as_ref(), Expr::Name(type_name) if type_name == "int") => {
                match name.as_str() {
                    "MaxValue" => Some(i32::MAX),
                    "MinValue" => Some(i32::MIN),
                    _ => None,
                }
            }
            Expr::Negate(value) => Self::constant_int(value)?.checked_neg(),
            Expr::Complement(value) => Some(!Self::constant_int(value)?),
            Expr::Binary(op, left, right) => {
                let left = Self::constant_int(left)?;
                let right = Self::constant_int(right)?;
                match op {
                    BinaryOp::Add => left.checked_add(right),
                    BinaryOp::Subtract => left.checked_sub(right),
                    BinaryOp::Multiply => left.checked_mul(right),
                    BinaryOp::Divide => left.checked_div(right),
                    BinaryOp::Remainder => left.checked_rem(right),
                    BinaryOp::BitAnd => Some(left & right),
                    BinaryOp::BitOr => Some(left | right),
                    BinaryOp::BitXor => Some(left ^ right),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn is_enum_zero(expression: &Expr) -> bool {
        matches!(expression, Expr::Int(0, _)) || Self::constant_int(expression) == Some(0)
    }

    fn has_non_int_literal(expression: &Expr) -> bool {
        match expression {
            Expr::Int(_, false) => true,
            Expr::Negate(inner) | Expr::Complement(inner) => Self::has_non_int_literal(inner),
            Expr::Binary(_, left, right) => {
                Self::has_non_int_literal(left) || Self::has_non_int_literal(right)
            }
            _ => false,
        }
    }

    fn int_constant_syntax(expression: &Expr) -> bool {
        match expression {
            Expr::Int(_, true) => true,
            Expr::Default(Ty::Int) => true,
            Expr::Member { target, name } => {
                matches!(target.as_ref(), Expr::Name(type_name) if type_name == "int")
                    && matches!(name.as_str(), "MinValue" | "MaxValue")
            }
            Expr::Negate(inner) | Expr::Complement(inner) => Self::int_constant_syntax(inner),
            Expr::Binary(op, left, right) => {
                matches!(
                    op,
                    BinaryOp::Add
                        | BinaryOp::Subtract
                        | BinaryOp::Multiply
                        | BinaryOp::Divide
                        | BinaryOp::Remainder
                        | BinaryOp::BitAnd
                        | BinaryOp::BitOr
                        | BinaryOp::BitXor
                ) && Self::int_constant_syntax(left)
                    && Self::int_constant_syntax(right)
            }
            _ => false,
        }
    }

    fn enum_integer_allowed(expression: &Expr, base: EnumBase) -> bool {
        if Self::has_non_int_literal(expression) {
            return false;
        }
        if Self::int_constant_syntax(expression) && Self::constant_int(expression).is_none() {
            return false;
        }
        match base {
            EnumBase::I4 | EnumBase::I8 => true,
            _ => Self::constant_int(expression).is_some_and(|value| {
                let (min, max) = base.bounds();
                (min..=max).contains(&i128::from(value))
            }),
        }
    }

    fn convert_enum_integer(&mut self, base: EnumBase) {
        if base.width() == 8 {
            self.emitter.op(if base == EnumBase::U8 {
                OpCode::Conv_U8
            } else {
                OpCode::Conv_I8
            });
        }
    }

    fn finish_enum_arithmetic(&mut self, base: EnumBase) {
        if base.width() < 4 {
            self.emitter.op(base.conversion());
        }
    }

    fn convert_expression(
        &mut self,
        actual: &Ty,
        expected: &Ty,
        expression: &Expr,
        what: &str,
    ) -> Result<(), String> {
        if *actual == Ty::Int
            && let Ty::Enum(_, base) = expected
            && Self::is_enum_zero(expression)
        {
            if base.width() == 8 {
                self.emitter.op(base.conversion());
            }
            return Ok(());
        }
        self.convert_assignable(actual, expected, what)
    }

    fn validate_compound(&self, ty: &Ty, op: &str) -> Result<(), CompileError> {
        let valid = match op {
            "+=" | "-=" => matches!(ty, Ty::Int | Ty::Float | Ty::Enum(_, _)),
            "*=" | "/=" | "%=" => matches!(ty, Ty::Int | Ty::Float),
            "&=" | "|=" | "^=" => matches!(ty, Ty::Int | Ty::Bool | Ty::Enum(_, _)),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(self.error(format!("operator '{op}' is not supported for {ty:?}")))
        }
    }

    fn convert_compound(
        &mut self,
        actual: &Ty,
        expected: &Ty,
        expression: &Expr,
        op: &str,
    ) -> Result<(), String> {
        if let Ty::Enum(_, base) = expected
            && matches!(op, "+=" | "-=")
        {
            if *actual != Ty::Int || !Self::enum_integer_allowed(expression, *base) {
                return Err(
                    "integer operand cannot convert to the enum underlying type".to_string()
                );
            }
            self.convert_enum_integer(*base);
            return Ok(());
        }
        self.convert_expression(actual, expected, expression, "compound assignment")
    }

    fn finish_compound(&mut self, ty: &Ty, op: &str) {
        if let Ty::Enum(_, base) = ty
            && matches!(op, "+=" | "-=")
        {
            self.finish_enum_arithmetic(*base);
        }
    }

    fn require(actual: &Ty, expected: &Ty, what: &str) -> Result<(), String> {
        if actual == expected {
            Ok(())
        } else {
            Err(format!(
                "{what} expects {expected:?} but the expression is {actual:?}"
            ))
        }
    }

    fn block(&mut self, block: &Block) -> Result<(), CompileError> {
        for statement in &block.statements {
            self.statement(statement)?;
        }
        Ok(())
    }

    fn validate_do_scopes(&self, block: &Block) -> Result<(), CompileError> {
        if !contains_do(block) {
            return Ok(());
        }
        validate_do_scopes(block, &self.params, &mut Vec::new(), false)
            .map_err(|message| self.error(message))
    }

    fn scoped_block(&mut self, block: &Block) -> Result<(), CompileError> {
        // Nested control-flow statements restore their own scopes; only a
        // declaration directly in this block changes this scope's bindings.
        if !block
            .statements
            .iter()
            .any(|statement| matches!(statement, Stmt::Local { .. }))
        {
            return self.block(block);
        }
        let locals = self.locals.clone();
        let result = self.block(block);
        self.locals = locals;
        result
    }

    fn is_exception_class(&self, name: &str) -> bool {
        let mut current = name.to_string();
        for _ in 0..64 {
            if let Some(local) = self.symbols.get(&current) {
                let Some(base) = &local.base else {
                    return false;
                };
                current = base.clone();
                continue;
            }
            if core_exception_name(&current).is_some() {
                return true;
            }
            let Some((_, external)) = self.externals.find(&current) else {
                return false;
            };
            let next = external.get().extends.as_ref().map(|base| base.full_name());
            let Some(next) = next else { return false };
            current = next;
        }
        false
    }

    fn exception_derives_from(&self, name: &str, ancestor: &str) -> bool {
        let mut current = name.to_string();
        let target = if self.symbols.contains_key(ancestor) {
            ancestor.to_string()
        } else {
            core_exception_name(ancestor)
                .unwrap_or(ancestor)
                .to_string()
        };
        for _ in 0..64 {
            let local = self.symbols.get(&current);
            let normalized = if local.is_some() {
                current.as_str()
            } else {
                core_exception_name(&current).unwrap_or(&current)
            };
            if normalized == target {
                return true;
            }
            if let Some(local) = local {
                let Some(base) = &local.base else {
                    return false;
                };
                current = base.clone();
            } else if let Some(base) = core_exception_base(&current) {
                current = base.to_string();
            } else if let Some((_, external)) = self.externals.find(&current) {
                let Some(base) = external.get().extends.as_ref().map(|base| base.full_name())
                else {
                    return false;
                };
                current = base;
            } else {
                return false;
            }
        }
        false
    }

    fn exception_reference(&mut self, ty: &Ty) -> Result<TypeDefOrRef, CompileError> {
        let Ty::Class(name) = ty else {
            return Err(self.error("catch type must derive from System.Exception"));
        };
        if !self.is_exception_class(name) {
            return Err(self.error(format!("'{name}' does not derive from System.Exception")));
        }
        match resolve_type(ty, self.types, self.externals, &mut *self.module)
            .map_err(|message| self.error(message))?
        {
            TypeSig::Class(reference) => Ok(reference),
            _ => Err(self.error("catch type must be a class")),
        }
    }

    fn switch_statement(
        &mut self,
        value: &Expr,
        sections: &[SwitchSection],
    ) -> Result<(), CompileError> {
        let value_type = self.expr(value)?;
        let signature = match &value_type {
            Ty::Int => TypeSig::INT32,
            Ty::Bool => TypeSig::BOOL,
            Ty::String => TypeSig::STRING,
            Ty::Enum(_, _) => {
                resolve_type(&value_type, self.types, self.externals, &mut *self.module)
                    .map_err(|message| self.error(message))?
            }
            _ => return Err(self.error("switch supports int, bool, string, and enum values")),
        };
        let value_local = self.emitter.declare_local(signature);
        self.emitter.stloc(value_local);
        let end = self.emitter.label();
        let targets: Vec<_> = sections.iter().map(|_| self.emitter.label()).collect();
        let mut default = None;
        let mut constants = std::collections::HashSet::new();
        let mut cases = Vec::new();
        for (section, target) in sections.iter().zip(&targets) {
            if !switch_section_exits(&section.body) {
                return Err(self.error("a switch section cannot fall through"));
            }
            if section
                .body
                .statements
                .iter()
                .any(|statement| matches!(statement, Stmt::Local { .. }))
            {
                return Err(self
                    .error("direct local declarations in switch sections are not supported yet"));
            }
            for label in &section.labels {
                match label {
                    SwitchLabel::Default => {
                        if default.replace(*target).is_some() {
                            return Err(
                                self.error("a switch cannot have more than one default label")
                            );
                        }
                    }
                    SwitchLabel::Case(expression) => {
                        let constant = if matches!(value_type, Ty::Enum(_, _)) {
                            if let Some((ty, literal)) = self.literal_reference(expression)? {
                                if ty != value_type {
                                    None
                                } else {
                                    match self.decode_literal(&ty, &literal)? {
                                        LiteralValue::Enum(value, _) => {
                                            Some(SwitchConstant::Enum(value))
                                        }
                                        _ => None,
                                    }
                                }
                            } else {
                                self.enum_switch_constant(expression, &value_type)
                                    .map(SwitchConstant::Enum)
                            }
                        } else {
                            if let Some(value) = switch_constant(expression) {
                                Some(value)
                            } else if let Some(value) = self.evaluate_int_constant(expression)? {
                                Some(SwitchConstant::Int(value))
                            } else if let Some((ty, literal)) =
                                self.literal_reference(expression)?
                            {
                                match (ty, self.decode_literal(&value_type, &literal)?) {
                                    (Ty::Bool, LiteralValue::Bool(value)) => {
                                        Some(SwitchConstant::Bool(value))
                                    }
                                    (Ty::String, LiteralValue::String(value)) => {
                                        Some(SwitchConstant::String(value))
                                    }
                                    (Ty::String, LiteralValue::Null) => Some(SwitchConstant::Null),
                                    _ => None,
                                }
                            } else {
                                None
                            }
                        };
                        let Some(constant) = constant else {
                            return Err(
                                self.error("unsupported or incompatible switch case constant")
                            );
                        };
                        let compatible = matches!(
                            (&value_type, &constant),
                            (Ty::Int, SwitchConstant::Int(_))
                                | (Ty::Bool, SwitchConstant::Bool(_))
                                | (Ty::Enum(_, _), SwitchConstant::Enum(_))
                                | (Ty::String, SwitchConstant::String(_) | SwitchConstant::Null)
                        );
                        if !compatible {
                            return Err(
                                self.error("case value is incompatible with the switch type")
                            );
                        }
                        if !constants.insert(constant.clone()) {
                            return Err(self.error("duplicate switch case value"));
                        }
                        cases.push((constant, *target));
                    }
                }
            }
        }
        let string_equality = if cases
            .iter()
            .any(|(constant, _)| matches!(constant, SwitchConstant::String(_)))
        {
            Some(
                self.module
                    .import_corlib_method(
                        "System.String",
                        "op_Equality",
                        MethodSig::new_static(
                            TypeSig::BOOL,
                            vec![TypeSig::STRING, TypeSig::STRING],
                        ),
                    )
                    .map_err(|error| self.error(error.to_string()))?,
            )
        } else {
            None
        };
        let dense_type = match &value_type {
            Ty::Int => true,
            Ty::Enum(_, base) => base.width() <= 4,
            _ => false,
        };
        let case_i4 = |constant: &SwitchConstant| match constant {
            SwitchConstant::Int(value) => Some(*value),
            SwitchConstant::Enum(value) => Some(*value as i32),
            _ => None,
        };
        let bounds = if dense_type && cases.len() >= 4 {
            let mut values = cases.iter().filter_map(|(constant, _)| case_i4(constant));
            values.next().map(|first| {
                values.fold((first, first), |(min, max), value| {
                    (min.min(value), max.max(value))
                })
            })
        } else {
            None
        };
        let dense_bounds = bounds.filter(|(min, max)| {
            let width = i64::from(*max) - i64::from(*min) + 1;
            width <= 4096 && (width as usize) <= cases.len().saturating_mul(2)
        });
        if let Some((min, max)) = dense_bounds {
            let width = (i64::from(max) - i64::from(min) + 1) as usize;
            let mut table = vec![default.unwrap_or(end); width];
            for (constant, target) in &cases {
                let value = case_i4(constant).expect("dense switch has 32-bit constants");
                table[(i64::from(value) - i64::from(min)) as usize] = *target;
            }
            self.emitter.ldloc(value_local);
            if min != 0 {
                self.emitter.ldc_i4(min).op(OpCode::Sub);
            }
            self.emitter.switch(table);
        } else {
            for (constant, target) in &cases {
                self.emitter.ldloc(value_local);
                match constant {
                    SwitchConstant::Int(value) => {
                        self.emitter.ldc_i4(*value).op(OpCode::Ceq);
                    }
                    SwitchConstant::Enum(value) => {
                        let Ty::Enum(_, base) = &value_type else {
                            unreachable!()
                        };
                        if base.width() == 8 {
                            self.emitter.emit(Instruction::new(
                                OpCode::Ldc_I8,
                                Operand::Int64(*value as u64 as i64),
                            ));
                        } else {
                            self.emitter.ldc_i4(*value as i32);
                        }
                        self.emitter.op(OpCode::Ceq);
                    }
                    SwitchConstant::Bool(value) => {
                        self.emitter.ldc_i4(i32::from(*value)).op(OpCode::Ceq);
                    }
                    SwitchConstant::String(value) => {
                        self.emitter.ldstr(value.clone()).call(
                            string_equality
                                .as_ref()
                                .expect("string equality is imported")
                                .clone(),
                        );
                    }
                    SwitchConstant::Null => {
                        self.emitter.ldnull().op(OpCode::Ceq);
                    }
                }
                self.emitter.branch(OpCode::Brtrue, *target);
            }
        }
        self.emitter.branch(OpCode::Br, default.unwrap_or(end));
        self.controls.push(ControlTarget {
            end,
            next: None,
            protected_depth: self.protected_depth,
            finally_depth: self.finally_depth,
        });
        for (section, target) in sections.iter().zip(targets) {
            self.emitter.mark(target);
            self.scoped_block(&section.body)?;
        }
        self.controls.pop();
        self.emitter.mark(end);
        Ok(())
    }

    fn try_statement(
        &mut self,
        body: &Block,
        catches: &[CatchClause],
        finally: Option<&Block>,
    ) -> Result<(), CompileError> {
        let mut previous: Vec<&str> = Vec::new();
        for clause in catches {
            if let Some(Ty::Class(name)) = &clause.ty {
                if previous
                    .iter()
                    .any(|base| self.exception_derives_from(name, base))
                {
                    return Err(self.error(format!(
                        "catch for '{name}' is unreachable after a base exception"
                    )));
                }
                if clause.filter.is_none() {
                    previous.push(name);
                }
            }
        }
        let outer_start = self.emitter.label();
        let inner_start = self.emitter.label();
        let inner_end = self.emitter.label();
        let after_catches = self.emitter.label();
        self.emitter.mark(outer_start);
        self.emitter.mark(inner_start);
        self.protected_depth += 1;
        self.scoped_block(body)?;
        self.protected_depth -= 1;
        if !catches.is_empty() {
            self.emitter.branch(OpCode::Leave, after_catches);
        }
        self.emitter.mark(inner_end);
        for clause in catches {
            let reference = match &clause.ty {
                Some(ty) => self.exception_reference(ty)?,
                None => self
                    .module
                    .corlib_type("System.Object")
                    .map_err(|error| self.error(error.to_string()))?,
            };
            let start = self.emitter.label();
            let end = self.emitter.label();
            let binding = if let Some(name) = &clause.name {
                if self.locals.contains_key(name)
                    || self.params.iter().any(|(parameter, _)| parameter == name)
                {
                    return Err(self.error(format!(
                        "catch variable '{name}' conflicts with an existing variable"
                    )));
                }
                let ty = clause.ty.as_ref().expect("named catch has a type").clone();
                let local = self
                    .emitter
                    .declare_local(TypeSig::Class(reference.clone()));
                let previous = self.locals.insert(name.clone(), (local, ty));
                Some((name.clone(), local, previous))
            } else {
                None
            };
            let filter_start = if let Some(filter) = &clause.filter {
                let filter_start = self.emitter.position();
                let filter_end = self.emitter.label();
                if clause.ty.is_some() {
                    let token = match &reference {
                        TypeDefOrRef::TypeDef(entry) => MemberToken::TypeDef(entry.clone()),
                        TypeDefOrRef::TypeRef(entry) => MemberToken::TypeRef(entry.clone()),
                        TypeDefOrRef::TypeSpec(entry) => MemberToken::TypeSpec(entry.clone()),
                    };
                    let matches = self.emitter.label();
                    self.emitter
                        .emit(Instruction::new(OpCode::Isinst, Operand::Token(token)));
                    self.emitter.dup().branch(OpCode::Brtrue, matches);
                    self.emitter.pop().ldc_i4(0).branch(OpCode::Br, filter_end);
                    self.emitter.mark(matches);
                }
                if let Some((_, local, _)) = &binding {
                    self.emitter.stloc(*local);
                } else {
                    self.emitter.pop();
                }
                let actual = self.expr(filter)?;
                self.convert_assignable(&actual, &Ty::Bool, "exception filter")
                    .map_err(|message| self.error(message))?;
                self.emitter.mark(filter_end);
                self.emitter.op(OpCode::Endfilter);
                Some(filter_start)
            } else {
                None
            };
            let handler_position = self.emitter.position();
            self.emitter.mark(start);
            if filter_start.is_some() {
                self.emitter.pop();
            } else if let Some((_, local, _)) = &binding {
                self.emitter.stloc(*local);
            } else {
                self.emitter.pop();
            }
            self.protected_depth += 1;
            self.catches.push(self.finally_depth);
            self.scoped_block(&clause.body)?;
            self.catches.pop();
            self.protected_depth -= 1;
            if let Some((name, _, prior)) = binding {
                if let Some(prior) = prior {
                    self.locals.insert(name, prior);
                } else {
                    self.locals.remove(&name);
                }
            }
            self.emitter.branch(OpCode::Leave, after_catches);
            self.emitter.mark(end);
            if let Some(filter_start) = filter_start {
                self.emitter.filter_handler(
                    inner_start,
                    inner_end,
                    filter_start,
                    start,
                    end,
                    handler_position,
                );
            } else {
                self.emitter
                    .exception_handler(Some(reference), inner_start, inner_end, start, end);
            }
        }
        self.emitter.mark(after_catches);
        if let Some(finally) = finally {
            let after = self.emitter.label();
            let start = self.emitter.label();
            let end = self.emitter.label();
            self.emitter.branch(OpCode::Leave, after);
            self.emitter.mark(start);
            self.finally_depth += 1;
            self.protected_depth += 1;
            self.scoped_block(finally)?;
            self.protected_depth -= 1;
            self.finally_depth -= 1;
            self.emitter.op(OpCode::Endfinally);
            self.emitter.mark(end);
            self.emitter
                .exception_handler(None, outer_start, start, start, end);
            self.emitter.mark(after);
        }
        Ok(())
    }

    fn statement(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        match statement {
            Stmt::Switch { value, sections } => self.switch_statement(value, sections),
            Stmt::Try {
                body,
                catches,
                finally,
            } => self.try_statement(body, catches, finally.as_ref()),
            Stmt::Throw(value) => {
                if let Some(value) = value {
                    let actual = self.expr(value)?;
                    if actual != Ty::Null {
                        let Ty::Class(name) = &actual else {
                            return Err(
                                self.error("thrown expression must derive from System.Exception")
                            );
                        };
                        if !self.is_exception_class(name) {
                            return Err(
                                self.error("thrown expression must derive from System.Exception")
                            );
                        }
                    }
                    self.emitter.op(OpCode::Throw);
                } else {
                    if self
                        .catches
                        .last()
                        .is_none_or(|depth| self.finally_depth > *depth)
                    {
                        return Err(self.error("rethrow is only allowed inside a catch block"));
                    }
                    self.emitter.op(OpCode::Rethrow);
                }
                Ok(())
            }
            Stmt::Block(block) => self.scoped_block(block),
            Stmt::Empty => Ok(()),
            Stmt::Break => {
                let Some(target) = self.controls.last().copied() else {
                    return Err(self.error("'break' outside of a loop or switch"));
                };
                if self.finally_depth > target.finally_depth {
                    return Err(self.error("cannot leave a finally block"));
                }
                self.emitter.branch(
                    if self.protected_depth > target.protected_depth {
                        OpCode::Leave
                    } else {
                        OpCode::Br
                    },
                    target.end,
                );
                Ok(())
            }
            Stmt::Continue => {
                let Some((target, next)) = self
                    .controls
                    .iter()
                    .rev()
                    .find_map(|target| target.next.map(|next| (*target, next)))
                else {
                    return Err(self.error("'continue' outside of a loop"));
                };
                if self.finally_depth > target.finally_depth {
                    return Err(self.error("cannot leave a finally block"));
                }
                self.emitter.branch(
                    if self.protected_depth > target.protected_depth {
                        OpCode::Leave
                    } else {
                        OpCode::Br
                    },
                    next,
                );
                Ok(())
            }
            Stmt::Local { ty, name, init } => {
                if matches!(ty, Ty::Struct(_)) && init.is_none() {
                    return Err(self.error("struct locals require an initializer for now"));
                }
                let signature = resolve_type(ty, self.types, self.externals, &mut *self.module)
                    .map_err(|message| self.error(message))?;
                let index = self.emitter.declare_local(signature);
                self.locals.insert(name.clone(), (index, ty.clone()));
                if let Some(value) = init {
                    if matches!(ty, Ty::Struct(_)) {
                        self.initializing_struct = Some(name.clone());
                    }
                    let actual = self.expr(value)?;
                    self.initializing_struct = None;
                    self.convert_expression(
                        &actual,
                        ty,
                        value,
                        &format!("initializer of '{name}'"),
                    )
                    .map_err(|message| self.error(message))?;
                    self.emitter.stloc(index);
                }
                Ok(())
            }
            Stmt::Assign { name, value } => self.assign(name, value),
            Stmt::AssignOp { name, op, value } => {
                let opcode = match op.as_str() {
                    "+=" => OpCode::Add,
                    "-=" => OpCode::Sub,
                    "*=" => OpCode::Mul,
                    "/=" => OpCode::Div,
                    "%=" => OpCode::Rem,
                    "&=" => OpCode::And,
                    "|=" => OpCode::Or,
                    "^=" => OpCode::Xor,
                    other => return Err(self.error(format!("unsupported operator '{other}'"))),
                };
                let shadowed = self.locals.contains_key(name)
                    || self.params.iter().any(|(parameter, _)| parameter == name);
                let current = self.current.clone();
                if !shadowed
                    && let Some(field) = self.field_symbol(&current, name)
                    && !field.is_static
                {
                    self.require_field_writable(&field.entry, true, true)?;
                    self.validate_compound(&field.ty, op)?;
                    if self.is_static {
                        return Err(self.error(format!(
                            "instance field '{name}' cannot be used from a static method"
                        )));
                    }
                    // Keep the receiver for the final stfld: [this, this, value].
                    self.emitter.ldarg(0);
                    self.emitter.dup();
                    self.emitter.emit(Instruction::new(
                        OpCode::Ldfld,
                        Operand::Token(MemberToken::Field(field.entry.clone())),
                    ));
                    let rhs = self.expr(value)?;
                    self.convert_compound(&rhs, &field.ty, value, op)
                        .map_err(|message| self.error(message))?;
                    self.emitter.op(opcode);
                    self.finish_compound(&field.ty, op);
                    self.emitter.emit(Instruction::new(
                        OpCode::Stfld,
                        Operand::Token(MemberToken::Field(field.entry)),
                    ));
                    return Ok(());
                }
                let current = self.current.clone();
                let property = self
                    .symbols
                    .get(&current)
                    .and_then(|symbol| symbol.properties.get(name))
                    .map(|property| {
                        (
                            property.ty.clone(),
                            property.getter.clone(),
                            property.setter.clone(),
                            property.is_static,
                        )
                    });
                if !shadowed && let Some((ty, getter, setter, is_static)) = property {
                    self.validate_compound(&ty, op)?;
                    if self.is_static && !is_static {
                        return Err(self.error(format!(
                            "instance property '{name}' cannot be used from a static method"
                        )));
                    }
                    let Some(getter) = getter else {
                        return Err(self.error(format!("property '{name}' has no getter")));
                    };
                    let Some(setter) = setter else {
                        return Err(self.error(format!("property '{name}' has no setter")));
                    };
                    if is_static {
                        self.emitter.call(getter);
                    } else {
                        self.emitter.ldarg(0).callvirt(getter);
                    }
                    let rhs = self.expr(value)?;
                    self.convert_compound(&rhs, &ty, value, op)
                        .map_err(|message| self.error(message))?;
                    self.emitter.op(opcode);
                    self.finish_compound(&ty, op);
                    if is_static {
                        self.emitter.call(setter);
                        return Ok(());
                    }
                    let signature =
                        resolve_type(&ty, self.types, self.externals, &mut *self.module)
                            .map_err(|message| self.error(message))?;
                    let temporary = self.emitter.declare_local(signature);
                    self.emitter.stloc(temporary);
                    self.emitter.ldarg(0);
                    self.emitter.ldloc(temporary);
                    self.emitter.callvirt(setter);
                    return Ok(());
                }
                let ty = self.load(name)?;
                self.validate_compound(&ty, op)?;
                let rhs = self.expr(value)?;
                self.convert_compound(&rhs, &ty, value, op)
                    .map_err(|message| self.error(message))?;
                self.emitter.op(opcode);
                self.finish_compound(&ty, op);
                self.store_top(name)?;
                Ok(())
            }
            Stmt::AssignIndex {
                target,
                index,
                value,
            } => {
                let target_type = self.expr(target)?;
                let Ty::Array(element) = target_type else {
                    return Err(self.error("index assignment requires an array"));
                };
                let index_type = self.expr(index)?;
                Self::require(&index_type, &Ty::Int, "array index")
                    .map_err(|message| self.error(message))?;
                let value_type = self.expr(value)?;
                self.convert_expression(&value_type, &element, value, "array element")
                    .map_err(|message| self.error(message))?;
                let opcode = array_element_opcode(&element, true)
                    .ok_or_else(|| self.error(format!("unsupported array element {element:?}")))?;
                self.emitter.op(opcode);
                Ok(())
            }
            Stmt::ForEach {
                ty,
                name,
                collection,
                body,
            } => {
                let collection_type = self.expr(collection)?;
                let Ty::Array(element) = &collection_type else {
                    return Err(self.error("foreach requires an array"));
                };
                if **element != *ty {
                    return Err(self.error(format!(
                        "foreach variable is {ty:?} but elements are {element:?}"
                    )));
                }
                let array_signature = resolve_type(
                    &collection_type,
                    self.types,
                    self.externals,
                    &mut *self.module,
                )
                .map_err(|message| self.error(message))?;
                let array_local = self.emitter.declare_local(array_signature);
                self.emitter.stloc(array_local);
                let index_local = self.emitter.declare_local(TypeSig::INT32);
                self.emitter.ldc_i4(0);
                self.emitter.stloc(index_local);
                let element_signature =
                    resolve_type(ty, self.types, self.externals, &mut *self.module)
                        .map_err(|message| self.error(message))?;
                let element_local = self.emitter.declare_local(element_signature);
                let previous = self
                    .locals
                    .insert(name.clone(), (element_local, ty.clone()));
                let top = self.emitter.label();
                let step = self.emitter.label();
                let end = self.emitter.label();
                self.emitter.mark(top);
                self.emitter.ldloc(index_local);
                self.emitter.ldloc(array_local);
                self.emitter.op(OpCode::Ldlen);
                self.emitter.op(OpCode::Conv_I4);
                self.emitter.op(OpCode::Clt);
                self.emitter.branch(OpCode::Brfalse, end);
                self.emitter.ldloc(array_local);
                self.emitter.ldloc(index_local);
                let opcode = array_element_opcode(ty, false)
                    .ok_or_else(|| self.error(format!("unsupported array element {ty:?}")))?;
                self.emitter.op(opcode);
                self.emitter.stloc(element_local);
                self.controls.push(ControlTarget {
                    end,
                    next: Some(step),
                    protected_depth: self.protected_depth,
                    finally_depth: self.finally_depth,
                });
                self.scoped_block(body)?;
                self.controls.pop();
                self.emitter.mark(step);
                self.emitter.ldloc(index_local);
                self.emitter.ldc_i4(1);
                self.emitter.op(OpCode::Add);
                self.emitter.stloc(index_local);
                self.emitter.branch(OpCode::Br, top);
                self.emitter.mark(end);
                match previous {
                    Some(entry) => {
                        self.locals.insert(name.clone(), entry);
                    }
                    None => {
                        self.locals.remove(name);
                    }
                }
                Ok(())
            }
            Stmt::AssignMemberOp {
                target,
                name,
                op,
                value,
            } => {
                let opcode = match op.as_str() {
                    "+=" => OpCode::Add,
                    "-=" => OpCode::Sub,
                    "*=" => OpCode::Mul,
                    "/=" => OpCode::Div,
                    "%=" => OpCode::Rem,
                    "&=" => OpCode::And,
                    "|=" => OpCode::Or,
                    "^=" => OpCode::Xor,
                    other => return Err(self.error(format!("unsupported operator '{other}'"))),
                };
                if let Some((ty, field)) = self.static_field(target, name, true)? {
                    let StaticFieldValue::Location(token) = field else {
                        return Err(self.error("constant fields cannot be assigned"));
                    };
                    self.validate_compound(&ty, op)?;
                    self.emitter.emit(Instruction::new(
                        OpCode::Ldsfld,
                        Operand::Token(token.clone()),
                    ));
                    let rhs = self.expr(value)?;
                    self.convert_compound(&rhs, &ty, value, op)
                        .map_err(|message| self.error(message))?;
                    self.emitter.op(opcode);
                    self.finish_compound(&ty, op);
                    self.emitter
                        .emit(Instruction::new(OpCode::Stsfld, Operand::Token(token)));
                    return Ok(());
                }
                let target_type = self.expr(target)?;
                let Ty::Class(type_name) = target_type else {
                    return Err(self.error("compound assignment requires a class instance"));
                };
                if let Some(field) = self.field_symbol(&type_name, name) {
                    self.require_field_access(&field.entry, true, Some(&type_name))?;
                    if field.is_static {
                        return Err(self.error(format!("'{type_name}.{name}' is static")));
                    }
                    self.require_field_writable(
                        &field.entry,
                        true,
                        matches!(target, Expr::Name(receiver) if receiver == "this"),
                    )?;
                    self.validate_compound(&field.ty, op)?;
                    self.emitter.dup();
                    self.emitter.emit(Instruction::new(
                        OpCode::Ldfld,
                        Operand::Token(MemberToken::Field(field.entry.clone())),
                    ));
                    let rhs = self.expr(value)?;
                    self.convert_compound(&rhs, &field.ty, value, op)
                        .map_err(|message| self.error(message))?;
                    self.emitter.op(opcode);
                    self.finish_compound(&field.ty, op);
                    self.emitter.emit(Instruction::new(
                        OpCode::Stfld,
                        Operand::Token(MemberToken::Field(field.entry)),
                    ));
                    return Ok(());
                }
                let property = self
                    .symbols
                    .get(&type_name)
                    .and_then(|symbol| symbol.properties.get(name))
                    .cloned();
                let Some(property) = property else {
                    return Err(self.error(format!("unknown member '{type_name}.{name}'")));
                };
                self.validate_compound(&property.ty, op)?;
                let (Some(getter), Some(setter)) =
                    (property.getter.clone(), property.setter.clone())
                else {
                    return Err(self.error(format!("property '{name}' needs a getter and setter")));
                };
                self.require_method_access(&getter, true, Some(&type_name))?;
                self.require_method_access(&setter, true, Some(&type_name))?;
                self.emitter.dup();
                self.emitter.callvirt(getter);
                let rhs = self.expr(value)?;
                self.convert_compound(&rhs, &property.ty, value, op)
                    .map_err(|message| self.error(message))?;
                self.emitter.op(opcode);
                self.finish_compound(&property.ty, op);
                self.emitter.callvirt(setter);
                Ok(())
            }
            Stmt::AssignMember {
                target,
                name,
                value,
            } => {
                if let Some((ty, field)) = self.static_field(target, name, true)? {
                    let StaticFieldValue::Location(token) = field else {
                        return Err(self.error("constant fields cannot be assigned"));
                    };
                    let actual = self.expr(value)?;
                    self.convert_expression(
                        &actual,
                        &ty,
                        value,
                        &format!("assignment to '{name}'"),
                    )
                    .map_err(|message| self.error(message))?;
                    self.emitter
                        .emit(Instruction::new(OpCode::Stsfld, Operand::Token(token)));
                    return Ok(());
                }
                if let Some(type_name) = self.struct_local_address(target) {
                    let short = type_name.rsplit('.').next().unwrap_or(&type_name);
                    let field = self.field_symbol(short, name).ok_or_else(|| {
                        self.error(format!("unknown member '{type_name}.{name}'"))
                    })?;
                    self.require_field_access(&field.entry, true, Some(short))?;
                    let actual = self.expr(value)?;
                    self.convert_expression(
                        &actual,
                        &field.ty,
                        value,
                        &format!("assignment to '{name}'"),
                    )
                    .map_err(|message| self.error(message))?;
                    self.emitter.emit(Instruction::new(
                        OpCode::Stfld,
                        Operand::Token(MemberToken::Field(field.entry)),
                    ));
                    return Ok(());
                }
                let target_type = self.expr(target)?;
                let Ty::Class(type_name) = target_type else {
                    return Err(self.error(format!("type {target_type:?} has no fields")));
                };
                if let Some(field) = self.field_symbol(&type_name, name) {
                    self.require_field_access(&field.entry, true, Some(&type_name))?;
                    if field.is_static {
                        return Err(self.error(format!("'{type_name}.{name}' is static")));
                    }
                    self.require_field_writable(
                        &field.entry,
                        true,
                        matches!(target, Expr::Name(receiver) if receiver == "this"),
                    )?;
                    let actual = self.expr(value)?;
                    self.convert_expression(
                        &actual,
                        &field.ty,
                        value,
                        &format!("assignment to '{name}'"),
                    )
                    .map_err(|message| self.error(message))?;
                    self.emitter.emit(Instruction::new(
                        OpCode::Stfld,
                        Operand::Token(MemberToken::Field(field.entry)),
                    ));
                    return Ok(());
                }
                let setter = self
                    .symbols
                    .get(&type_name)
                    .and_then(|symbol| symbol.properties.get(name))
                    .map(|property| (property.ty.clone(), property.setter.clone()));
                let Some((property_ty, setter)) = setter else {
                    let owner = self
                        .external_base_of(&type_name)
                        .unwrap_or_else(|| type_name.clone());
                    if let Some((index, entry)) =
                        self.externals
                            .method_in_chain(&owner, &format!("set_{name}"), 1)
                    {
                        let (flags, signature) = {
                            let method = entry.get();
                            (method.flags, method.signature.clone())
                        };
                        if self
                            .require_method_access(&entry, false, Some(&type_name))
                            .is_err()
                            || flags.contains(MethodFlags::Static)
                            || !flags.contains(MethodFlags::SpecialName)
                        {
                            return Err(self.error(format!(
                                "property '{type_name}.{name}' has no accessible instance setter"
                            )));
                        }
                        let expected = external_ty(&signature.params[0], self.enums, false);
                        let actual = self.expr(value)?;
                        self.convert_assignable(
                            &actual,
                            &expected,
                            &format!("assignment to '{name}'"),
                        )
                        .map_err(|message| self.error(message))?;
                        let token = self
                            .module
                            .import_foreign_method(&entry, &self.externals.modules[index])
                            .map_err(|error| self.error(error.to_string()))?;
                        self.emitter.callvirt(token);
                        return Ok(());
                    }
                    return Err(self.error(format!("unknown member '{type_name}.{name}'")));
                };
                let Some(setter) = setter else {
                    return Err(self.error(format!("property '{type_name}.{name}' has no setter")));
                };
                self.require_method_access(&setter, true, Some(&type_name))?;
                let actual = self.expr(value)?;
                self.convert_expression(
                    &actual,
                    &property_ty,
                    value,
                    &format!("assignment to '{name}'"),
                )
                .map_err(|message| self.error(message))?;
                self.emitter.callvirt(setter);
                Ok(())
            }
            Stmt::Expression(expression) => {
                let ty = self.expr(expression)?;
                if ty != Ty::Void {
                    self.emitter.pop();
                }
                Ok(())
            }
            Stmt::Return(value) => {
                if self.finally_depth > 0 {
                    return Err(self.error("cannot return from a finally block"));
                }
                match value {
                    Some(expression) => {
                        let actual = self.expr(expression)?;
                        let return_ty = self.return_ty.clone();
                        self.convert_expression(&actual, &return_ty, expression, "return")
                            .map_err(|message| self.error(message))?;
                    }
                    None => {
                        if self.return_ty != Ty::Void {
                            return Err(self.error("a value must be returned"));
                        }
                    }
                }
                if self.protected_depth > 0 {
                    let (target, local) = match self.return_target {
                        Some(target) => target,
                        None => {
                            let target = self.emitter.label();
                            let local = if self.return_ty == Ty::Void {
                                None
                            } else {
                                let signature = resolve_type(
                                    &self.return_ty,
                                    self.types,
                                    self.externals,
                                    &mut *self.module,
                                )
                                .map_err(|message| self.error(message))?;
                                Some(self.emitter.declare_local(signature))
                            };
                            self.return_target = Some((target, local));
                            (target, local)
                        }
                    };
                    if let Some(local) = local {
                        self.emitter.stloc(local);
                    }
                    self.emitter.branch(OpCode::Leave, target);
                } else {
                    self.emitter.ret();
                }
                Ok(())
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let actual = self.expr(condition)?;
                self.convert_assignable(&actual, &Ty::Bool, "if condition")
                    .map_err(|message| self.error(message))?;
                let alternative = self.emitter.label();
                let end = self.emitter.label();
                self.emitter.branch(OpCode::Brfalse, alternative);
                self.scoped_block(then_branch)?;
                self.emitter.branch(OpCode::Br, end);
                self.emitter.mark(alternative);
                if let Some(branch) = else_branch {
                    self.scoped_block(branch)?;
                }
                self.emitter.mark(end);
                Ok(())
            }
            Stmt::While { condition, body } => {
                let top = self.emitter.label();
                let end = self.emitter.label();
                self.emitter.mark(top);
                let actual = self.expr(condition)?;
                self.convert_assignable(&actual, &Ty::Bool, "while condition")
                    .map_err(|message| self.error(message))?;
                self.emitter.branch(OpCode::Brfalse, end);
                self.controls.push(ControlTarget {
                    end,
                    next: Some(top),
                    protected_depth: self.protected_depth,
                    finally_depth: self.finally_depth,
                });
                self.scoped_block(body)?;
                self.controls.pop();
                self.emitter.branch(OpCode::Br, top);
                self.emitter.mark(end);
                Ok(())
            }
            Stmt::DoWhile { body, condition } => {
                let top = self.emitter.label();
                let test = self.emitter.label();
                let end = self.emitter.label();
                self.emitter.mark(top);
                self.controls.push(ControlTarget {
                    end,
                    next: Some(test),
                    protected_depth: self.protected_depth,
                    finally_depth: self.finally_depth,
                });
                self.scoped_block(body)?;
                self.controls.pop();
                self.emitter.mark(test);
                let actual = self.expr(condition)?;
                self.convert_assignable(&actual, &Ty::Bool, "do-while condition")
                    .map_err(|message| self.error(message))?;
                self.emitter.branch(OpCode::Brtrue, top);
                self.emitter.mark(end);
                Ok(())
            }
            Stmt::For {
                init,
                condition,
                step,
                body,
            } => {
                let previous_local = init.as_deref().and_then(|statement| match statement {
                    Stmt::Local { name, .. } => {
                        Some((name.clone(), self.locals.get(name).cloned()))
                    }
                    _ => None,
                });
                if let Some(init) = init {
                    self.statement(init)?;
                }
                let top = self.emitter.label();
                let step_label = self.emitter.label();
                let end = self.emitter.label();
                self.emitter.mark(top);
                if let Some(condition) = condition {
                    let actual = self.expr(condition)?;
                    self.convert_assignable(&actual, &Ty::Bool, "for condition")
                        .map_err(|message| self.error(message))?;
                    self.emitter.branch(OpCode::Brfalse, end);
                }
                self.controls.push(ControlTarget {
                    end,
                    next: Some(step_label),
                    protected_depth: self.protected_depth,
                    finally_depth: self.finally_depth,
                });
                self.scoped_block(body)?;
                self.controls.pop();
                self.emitter.mark(step_label);
                if let Some(step) = step {
                    let ty = self.expr(step)?;
                    if ty != Ty::Void {
                        self.emitter.pop();
                    }
                }
                self.emitter.branch(OpCode::Br, top);
                self.emitter.mark(end);
                if let Some((name, previous)) = previous_local {
                    if let Some(previous) = previous {
                        self.locals.insert(name, previous);
                    } else {
                        self.locals.remove(&name);
                    }
                }
                Ok(())
            }
        }
    }

    fn assign(&mut self, name: &str, value: &Expr) -> Result<(), CompileError> {
        if let Some((index, ty)) = self.locals.get(name).cloned() {
            let actual = self.expr(value)?;
            self.convert_expression(&actual, &ty, value, &format!("assignment to '{name}'"))
                .map_err(|message| self.error(message))?;
            self.emitter.stloc(index);
            return Ok(());
        }
        if let Some(position) = self
            .params
            .iter()
            .position(|(parameter, _)| parameter == name)
        {
            let ty = self.params[position].1.clone();
            let actual = self.expr(value)?;
            self.convert_expression(&actual, &ty, value, &format!("assignment to '{name}'"))
                .map_err(|message| self.error(message))?;
            self.emitter.starg(self.argument_index(position));
            return Ok(());
        }
        let Some(field) = self.field_symbol(&self.current.clone(), name) else {
            self.assign_property(name, value, false)?;
            return Ok(());
        };
        self.require_field_writable(&field.entry, true, true)?;
        if field.is_static {
            let actual = self.expr(value)?;
            self.convert_expression(
                &actual,
                &field.ty,
                value,
                &format!("assignment to '{name}'"),
            )
            .map_err(|message| self.error(message))?;
            self.emitter.emit(Instruction::new(
                OpCode::Stsfld,
                Operand::Token(MemberToken::Field(field.entry)),
            ));
            return Ok(());
        }
        if self.is_static {
            return Err(self.error(format!(
                "instance field '{name}' cannot be used from a static method"
            )));
        }
        self.emitter.ldarg(0);
        let actual = self.expr(value)?;
        self.convert_expression(
            &actual,
            &field.ty,
            value,
            &format!("assignment to '{name}'"),
        )
        .map_err(|message| self.error(message))?;
        self.emitter.emit(Instruction::new(
            OpCode::Stfld,
            Operand::Token(MemberToken::Field(field.entry)),
        ));
        Ok(())
    }

    fn assign_expression(&mut self, name: &str, value: &Expr) -> Result<Ty, CompileError> {
        let ty = self.assign_value(name, value)?;
        if ty == Ty::Void {
            return Err(self.error("cannot assign a void expression"));
        }
        Ok(ty)
    }

    fn assign_value(&mut self, name: &str, value: &Expr) -> Result<Ty, CompileError> {
        if let Some((index, ty)) = self.locals.get(name).cloned() {
            let actual = self.expr(value)?;
            self.convert_expression(&actual, &ty, value, &format!("assignment to '{name}'"))
                .map_err(|message| self.error(message))?;
            self.emitter.dup();
            self.emitter.stloc(index);
            return Ok(ty);
        }
        if let Some(position) = self
            .params
            .iter()
            .position(|(parameter, _)| parameter == name)
        {
            let ty = self.params[position].1.clone();
            let actual = self.expr(value)?;
            self.convert_expression(&actual, &ty, value, &format!("assignment to '{name}'"))
                .map_err(|message| self.error(message))?;
            self.emitter.dup();
            self.emitter.starg(self.argument_index(position));
            return Ok(ty);
        }
        let Some(field) = self.field_symbol(&self.current.clone(), name) else {
            return self.assign_property(name, value, true);
        };
        self.require_field_writable(&field.entry, true, true)?;
        if field.is_static {
            let actual = self.expr(value)?;
            self.convert_expression(
                &actual,
                &field.ty,
                value,
                &format!("assignment to '{name}'"),
            )
            .map_err(|message| self.error(message))?;
            self.emitter.dup();
            self.emitter.emit(Instruction::new(
                OpCode::Stsfld,
                Operand::Token(MemberToken::Field(field.entry)),
            ));
            return Ok(field.ty);
        }
        if self.is_static {
            return Err(self.error(format!(
                "instance field '{name}' cannot be used from a static method"
            )));
        }
        self.emitter.ldarg(0);
        let actual = self.expr(value)?;
        self.convert_expression(
            &actual,
            &field.ty,
            value,
            &format!("assignment to '{name}'"),
        )
        .map_err(|message| self.error(message))?;
        let signature = resolve_type(&field.ty, self.types, self.externals, &mut *self.module)
            .map_err(|message| self.error(message))?;
        let temporary = self.emitter.declare_local(signature);
        self.emitter.dup();
        self.emitter.stloc(temporary);
        self.emitter.emit(Instruction::new(
            OpCode::Stfld,
            Operand::Token(MemberToken::Field(field.entry)),
        ));
        self.emitter.ldloc(temporary);
        Ok(field.ty)
    }

    fn assign_property(
        &mut self,
        name: &str,
        value: &Expr,
        leave_value: bool,
    ) -> Result<Ty, CompileError> {
        let property = self
            .symbols
            .get(&self.current)
            .and_then(|symbol| symbol.properties.get(name))
            .cloned()
            .ok_or_else(|| self.error(format!("undefined identifier '{name}'")))?;
        let setter = property
            .setter
            .ok_or_else(|| self.error(format!("property '{name}' has no setter")))?;
        if !property.is_static {
            if self.is_static {
                return Err(self.error(format!(
                    "instance property '{name}' cannot be used from a static method"
                )));
            }
            self.emitter.ldarg(0);
        }
        let actual = self.expr(value)?;
        self.convert_expression(
            &actual,
            &property.ty,
            value,
            &format!("assignment to '{name}'"),
        )
        .map_err(|message| self.error(message))?;
        let temporary = if leave_value {
            let signature =
                resolve_type(&property.ty, self.types, self.externals, &mut *self.module)
                    .map_err(|message| self.error(message))?;
            let temporary = self.emitter.declare_local(signature);
            self.emitter.dup().stloc(temporary);
            Some(temporary)
        } else {
            None
        };
        if property.is_static {
            self.emitter.call(setter);
        } else {
            self.emitter.callvirt(setter);
        }
        if let Some(temporary) = temporary {
            self.emitter.ldloc(temporary);
        }
        Ok(property.ty)
    }

    fn load(&mut self, name: &str) -> Result<Ty, CompileError> {
        if self.initializing_struct.as_deref() == Some(name) {
            return Err(self.error(format!(
                "struct local '{name}' is used in its own initializer"
            )));
        }
        if name == "this" {
            if self.is_static {
                return Err(self.error("'this' cannot be used from a static method"));
            }
            self.emitter.ldarg(0);
            return Ok(Ty::Class(self.current.clone()));
        }
        if name == "base" {
            if self.is_static {
                return Err(self.error("'base' cannot be used from a static method"));
            }
            let base = self
                .base
                .clone()
                .unwrap_or_else(|| "System.Object".to_string());
            self.emitter.ldarg(0);
            return Ok(Ty::Class(base));
        }
        if let Some((index, ty)) = self.locals.get(name).cloned() {
            self.emitter.ldloc(index);
            return Ok(ty);
        }
        if let Some(position) = self
            .params
            .iter()
            .position(|(parameter, _)| parameter == name)
        {
            let ty = self.params[position].1.clone();
            self.emitter.ldarg(self.argument_index(position));
            return Ok(ty);
        }
        let Some(field) = self.field_symbol(&self.current.clone(), name) else {
            let current = self.current.clone();
            let property = self
                .symbols
                .get(&current)
                .and_then(|symbol| symbol.properties.get(name))
                .cloned();
            let Some(property) = property else {
                return Err(self.error(format!("undefined identifier '{name}'")));
            };
            let Some(getter) = property.getter else {
                return Err(self.error(format!("property '{name}' has no getter")));
            };
            if property.is_static {
                self.emitter.call(getter);
            } else {
                if self.is_static {
                    return Err(self.error(format!(
                        "instance property '{name}' cannot be used from a static method"
                    )));
                }
                self.emitter.ldarg(0);
                self.emitter.callvirt(getter);
            }
            return Ok(property.ty);
        };
        if field.entry.get().flags.contains(FieldFlags::Literal) {
            let constant = field
                .entry
                .get()
                .constant
                .clone()
                .ok_or_else(|| self.error("constant field has no metadata value"))?;
            self.emit_literal(&field.ty, &constant)?;
            return Ok(field.ty);
        }
        if field.is_static {
            self.emitter.emit(Instruction::new(
                OpCode::Ldsfld,
                Operand::Token(MemberToken::Field(field.entry)),
            ));
            return Ok(field.ty);
        }
        if self.is_static {
            return Err(self.error(format!(
                "instance field '{name}' cannot be used from a static method"
            )));
        }
        self.emitter.ldarg(0);
        self.emitter.emit(Instruction::new(
            OpCode::Ldfld,
            Operand::Token(MemberToken::Field(field.entry)),
        ));
        Ok(field.ty)
    }

    fn struct_local_address(&mut self, target: &Expr) -> Option<String> {
        let Expr::Name(name) = target else {
            return None;
        };
        if let Some((index, Ty::Struct(type_name))) = self.locals.get(name).cloned() {
            self.emitter.ldloca(index);
            return Some(type_name);
        }
        let position = self
            .params
            .iter()
            .position(|(parameter, ty)| parameter == name && matches!(ty, Ty::Struct(_)))?;
        let Ty::Struct(type_name) = self.params[position].1.clone() else {
            unreachable!()
        };
        self.emitter.emit(Instruction::new(
            OpCode::Ldarga,
            Operand::Arg(self.argument_index(position)),
        ));
        Some(type_name)
    }

    fn default_struct(&mut self, ty: &Ty) -> Result<(), CompileError> {
        let signature = resolve_type(ty, self.types, self.externals, &mut *self.module)
            .map_err(|message| self.error(message))?;
        let TypeSig::ValueType(TypeDefOrRef::TypeDef(entry)) = &signature else {
            return Err(self.error("struct must be a local value type"));
        };
        let local = self.emitter.declare_local(signature.clone());
        self.emitter.ldloca(local);
        self.emitter.emit(Instruction::new(
            OpCode::Initobj,
            Operand::Token(MemberToken::TypeDef(entry.clone())),
        ));
        self.emitter.ldloc(local);
        Ok(())
    }

    fn call_base_constructor(&mut self, args: Option<&[Expr]>) -> Result<(), CompileError> {
        let base_name = self
            .base
            .clone()
            .unwrap_or_else(|| "System.Object".to_string());
        let short = base_name
            .rsplit('.')
            .next()
            .unwrap_or(&base_name)
            .to_string();
        let arity = args.map(<[Expr]>::len).unwrap_or(0);
        self.emitter.ldarg(0);
        if let Some(symbol) = self.symbols.get(&short) {
            let Some(ctor) = symbol
                .ctors
                .iter()
                .find(|ctor| ctor.params.len() == arity)
                .cloned()
            else {
                return Err(self.error(format!(
                    "base '{base_name}' has no constructor with {arity} argument(s)"
                )));
            };
            self.require_method_access(&ctor.entry, true, None)?;
            if let Some(args) = args {
                for (index, argument) in args.iter().enumerate() {
                    let actual = self.expr(argument)?;
                    self.convert_expression(
                        &actual,
                        &ctor.params[index],
                        argument,
                        "base argument",
                    )
                    .map_err(|message| self.error(message))?;
                }
            }
            self.emitter.call(ctor.entry);
            return Ok(());
        }
        if base_name == "System.Object" {
            let ctor = self
                .module
                .import_corlib_method(
                    "System.Object",
                    ".ctor",
                    MethodSig::new_instance(TypeSig::VOID, vec![]),
                )
                .map_err(|error| self.error(error.to_string()))?;
            self.emitter.call(ctor);
            return Ok(());
        }
        if let Some(name) = core_exception_name(&base_name)
            && self.externals.find(&base_name).is_none()
        {
            let actual = args.unwrap_or(&[]);
            let params = match actual.len() {
                0 => Vec::new(),
                1 => vec![Ty::String],
                _ => {
                    return Err(self.error(format!(
                        "unsupported exception constructor '{name}' with {} arguments",
                        actual.len()
                    )));
                }
            };
            self.push_arguments(actual, &params, "base argument")?;
            let signature = MethodSig::new_instance(
                TypeSig::VOID,
                params.iter().map(|_| TypeSig::STRING).collect(),
            );
            let ctor = self
                .module
                .import_corlib_method(name, ".ctor", signature)
                .map_err(|error| self.error(error.to_string()))?;
            self.emitter.call(ctor);
            return Ok(());
        }
        if let Some((index, entry)) = self.externals.method(&base_name, ".ctor", arity) {
            self.require_method_access(&entry, false, None)?;
            let signature = {
                let guard = entry.get();
                guard.signature.clone()
            };
            let params: Vec<Ty> = signature
                .params
                .iter()
                .map(|sig| external_ty(sig, self.enums, false))
                .collect();
            if let Some(args) = args {
                for (index, argument) in args.iter().enumerate() {
                    let actual = self.expr(argument)?;
                    self.convert_expression(&actual, &params[index], argument, "base argument")
                        .map_err(|message| self.error(message))?;
                }
            }
            let token = {
                let module = &mut *self.module;
                let foreign = &self.externals.modules[index];
                module
                    .import_foreign_method(&entry, foreign)
                    .map_err(|error| self.error(error.to_string()))?
            };
            self.emitter.call(token);
            return Ok(());
        }
        Err(self.error(format!(
            "base type '{base_name}' has no constructor with {arity} argument(s)"
        )))
    }

    fn store_top(&mut self, name: &str) -> Result<(), CompileError> {
        if let Some((index, _)) = self.locals.get(name).cloned() {
            self.emitter.stloc(index);
            return Ok(());
        }
        if let Some(position) = self
            .params
            .iter()
            .position(|(parameter, _)| parameter == name)
        {
            self.emitter.starg(self.argument_index(position));
            return Ok(());
        }
        let Some(field) = self.field_symbol(&self.current.clone(), name) else {
            let property = self
                .symbols
                .get(&self.current)
                .and_then(|symbol| symbol.properties.get(name))
                .cloned()
                .ok_or_else(|| self.error(format!("undefined identifier '{name}'")))?;
            let setter = property
                .setter
                .ok_or_else(|| self.error(format!("property '{name}' has no setter")))?;
            if property.is_static {
                self.emitter.call(setter);
            } else {
                if self.is_static {
                    return Err(self.error(format!(
                        "instance property '{name}' cannot be used from a static method"
                    )));
                }
                let signature =
                    resolve_type(&property.ty, self.types, self.externals, &mut *self.module)
                        .map_err(|message| self.error(message))?;
                let temporary = self.emitter.declare_local(signature);
                self.emitter
                    .stloc(temporary)
                    .ldarg(0)
                    .ldloc(temporary)
                    .callvirt(setter);
            }
            return Ok(());
        };
        self.require_field_writable(&field.entry, true, true)?;
        if !field.is_static {
            if self.is_static {
                return Err(self.error(format!(
                    "instance field '{name}' cannot be used from a static method"
                )));
            }
            let signature = resolve_type(&field.ty, self.types, self.externals, &mut *self.module)
                .map_err(|message| self.error(message))?;
            let temporary = self.emitter.declare_local(signature);
            self.emitter.stloc(temporary).ldarg(0).ldloc(temporary);
            self.emitter.emit(Instruction::new(
                OpCode::Stfld,
                Operand::Token(MemberToken::Field(field.entry)),
            ));
            return Ok(());
        }
        self.emitter.emit(Instruction::new(
            OpCode::Stsfld,
            Operand::Token(MemberToken::Field(field.entry)),
        ));
        Ok(())
    }
}

impl<'a> FnEmitter<'a> {
    fn increment_top(&mut self, ty: &Ty, delta: i32) -> Result<(), CompileError> {
        match ty {
            Ty::Int => {
                self.emitter.ldc_i4(delta.abs());
            }
            Ty::Enum(_, base) if base.width() == 8 => {
                self.emitter.emit(Instruction::new(
                    OpCode::Ldc_I8,
                    Operand::Int64(i64::from(delta.abs())),
                ));
            }
            Ty::Enum(_, _) => {
                self.emitter.ldc_i4(delta.abs());
            }
            _ => {
                return Err(
                    self.error(format!("increment requires an integer or enum, got {ty:?}"))
                );
            }
        }
        self.emitter
            .op(if delta > 0 { OpCode::Add } else { OpCode::Sub });
        if let Ty::Enum(_, base) = ty
            && base.width() < 4
        {
            self.emitter.op(base.conversion());
        }
        Ok(())
    }

    fn increment_member(
        &mut self,
        target: &Expr,
        name: &str,
        delta: i32,
        prefix: bool,
    ) -> Result<Ty, CompileError> {
        if let Some((ty, field)) = self.static_field(target, name, true)? {
            let StaticFieldValue::Location(token) = field else {
                return Err(self.error("constant fields cannot be assigned"));
            };
            self.emitter.emit(Instruction::new(
                OpCode::Ldsfld,
                Operand::Token(token.clone()),
            ));
            if !prefix {
                self.emitter.dup();
            }
            self.increment_top(&ty, delta)?;
            if prefix {
                self.emitter.dup();
            }
            self.emitter
                .emit(Instruction::new(OpCode::Stsfld, Operand::Token(token)));
            return Ok(ty);
        }
        if let Some(type_name) = Self::type_path(target)
            && self.types.contains_key(&type_name)
            && !self.locals.contains_key(&type_name)
            && !self
                .params
                .iter()
                .any(|(parameter, _)| parameter == &type_name)
            && self.field_symbol(&self.current, &type_name).is_none()
            && self
                .symbols
                .get(&self.current)
                .and_then(|symbol| symbol.properties.get(&type_name))
                .is_none()
        {
            let (ty, member) = if let Some(field) = self.field_symbol(&type_name, name) {
                self.require_field_access(&field.entry, true, None)?;
                if !field.is_static {
                    return Err(self.error(format!("'{type_name}.{name}' is not static")));
                }
                self.require_field_writable(&field.entry, true, true)?;
                (field.ty, IncrementMemberTarget::Field(field.entry))
            } else {
                let property = self
                    .symbols
                    .get(&type_name)
                    .and_then(|symbol| symbol.properties.get(name))
                    .cloned()
                    .ok_or_else(|| self.error(format!("unknown member '{type_name}.{name}'")))?;
                if !property.is_static {
                    return Err(self.error(format!("'{type_name}.{name}' is not static")));
                }
                let (Some(getter), Some(setter)) = (property.getter, property.setter) else {
                    return Err(self.error(format!(
                        "property '{type_name}.{name}' requires both accessors"
                    )));
                };
                self.require_method_access(&getter, true, None)?;
                self.require_method_access(&setter, true, None)?;
                (
                    property.ty,
                    IncrementMemberTarget::Property { getter, setter },
                )
            };
            match &member {
                IncrementMemberTarget::Field(field) => {
                    self.emitter.emit(Instruction::new(
                        OpCode::Ldsfld,
                        Operand::Token(MemberToken::Field(field.clone())),
                    ));
                }
                IncrementMemberTarget::Property { getter, .. } => {
                    self.emitter.call(getter.clone());
                }
            }
            if !prefix {
                self.emitter.dup();
            }
            self.increment_top(&ty, delta)?;
            if prefix {
                self.emitter.dup();
            }
            match member {
                IncrementMemberTarget::Field(field) => {
                    self.emitter.emit(Instruction::new(
                        OpCode::Stsfld,
                        Operand::Token(MemberToken::Field(field)),
                    ));
                }
                IncrementMemberTarget::Property { setter, .. } => {
                    self.emitter.call(setter);
                }
            }
            return Ok(ty);
        }
        let receiver_type = self.expr(target)?;
        let Ty::Class(type_name) = &receiver_type else {
            return Err(self.error(format!("type {receiver_type:?} has no assignable members")));
        };
        let (ty, member) = if let Some(field) = self.field_symbol(type_name, name) {
            self.require_field_access(&field.entry, true, Some(type_name))?;
            if field.is_static {
                return Err(self.error(format!("'{type_name}.{name}' is static")));
            }
            self.require_field_writable(
                &field.entry,
                true,
                matches!(target, Expr::Name(receiver) if receiver == "this"),
            )?;
            (field.ty, IncrementMemberTarget::Field(field.entry))
        } else {
            let property = self
                .symbols
                .get(type_name)
                .and_then(|symbol| symbol.properties.get(name))
                .cloned()
                .ok_or_else(|| self.error(format!("unknown member '{type_name}.{name}'")))?;
            if property.is_static {
                return Err(self.error(format!("'{type_name}.{name}' is static")));
            }
            let (Some(getter), Some(setter)) = (property.getter, property.setter) else {
                return Err(self.error(format!(
                    "property '{type_name}.{name}' requires both accessors"
                )));
            };
            self.require_method_access(&getter, true, Some(type_name))?;
            self.require_method_access(&setter, true, Some(type_name))?;
            (
                property.ty,
                IncrementMemberTarget::Property { getter, setter },
            )
        };
        let receiver_signature = resolve_type(
            &receiver_type,
            self.types,
            self.externals,
            &mut *self.module,
        )
        .map_err(|message| self.error(message))?;
        let receiver = self.emitter.declare_local(receiver_signature);
        self.emitter.stloc(receiver).ldloc(receiver);
        match &member {
            IncrementMemberTarget::Field(field) => {
                self.emitter.emit(Instruction::new(
                    OpCode::Ldfld,
                    Operand::Token(MemberToken::Field(field.clone())),
                ));
            }
            IncrementMemberTarget::Property { getter, .. } => {
                self.emitter.callvirt(getter.clone());
            }
        }
        if !prefix {
            self.emitter.dup();
        }
        self.increment_top(&ty, delta)?;
        if prefix {
            self.emitter.dup();
        }
        let value_signature = resolve_type(&ty, self.types, self.externals, &mut *self.module)
            .map_err(|message| self.error(message))?;
        let value = self.emitter.declare_local(value_signature);
        self.emitter.stloc(value).ldloc(receiver).ldloc(value);
        match member {
            IncrementMemberTarget::Field(field) => {
                self.emitter.emit(Instruction::new(
                    OpCode::Stfld,
                    Operand::Token(MemberToken::Field(field)),
                ));
            }
            IncrementMemberTarget::Property { setter, .. } => {
                self.emitter.callvirt(setter);
            }
        }
        Ok(ty)
    }

    fn increment_index(
        &mut self,
        target: &Expr,
        index: &Expr,
        delta: i32,
        prefix: bool,
    ) -> Result<Ty, CompileError> {
        let array_type = self.expr(target)?;
        let Ty::Array(element) = &array_type else {
            return Err(self.error("increment indexing requires an array"));
        };
        let array_signature =
            resolve_type(&array_type, self.types, self.externals, &mut *self.module)
                .map_err(|message| self.error(message))?;
        let array = self.emitter.declare_local(array_signature);
        self.emitter.stloc(array);
        let index_type = self.expr(index)?;
        Self::require(&index_type, &Ty::Int, "array index")
            .map_err(|message| self.error(message))?;
        let position = self.emitter.declare_local(TypeSig::INT32);
        self.emitter.stloc(position).ldloc(array).ldloc(position);
        let load = array_element_opcode(element, false)
            .ok_or_else(|| self.error(format!("unsupported array element {element:?}")))?;
        self.emitter.op(load);
        if !prefix {
            self.emitter.dup();
        }
        self.increment_top(element, delta)?;
        if prefix {
            self.emitter.dup();
        }
        let signature = resolve_type(element, self.types, self.externals, &mut *self.module)
            .map_err(|message| self.error(message))?;
        let value = self.emitter.declare_local(signature);
        self.emitter
            .stloc(value)
            .ldloc(array)
            .ldloc(position)
            .ldloc(value);
        let store = array_element_opcode(element, true)
            .ok_or_else(|| self.error(format!("unsupported array element {element:?}")))?;
        self.emitter.op(store);
        Ok((**element).clone())
    }

    fn expr(&mut self, expression: &Expr) -> Result<Ty, CompileError> {
        if matches!(expression, Expr::Negate(_) | Expr::Binary(_, _, _)) {
            self.evaluate_int_constant(expression)?;
        }
        match expression {
            Expr::Int(value, _) => {
                self.emitter.ldc_i4(*value);
                Ok(Ty::Int)
            }
            Expr::Null => {
                self.emitter.ldnull();
                Ok(Ty::Null)
            }
            Expr::Float(value) => {
                self.emitter
                    .emit(Instruction::new(OpCode::Ldc_R4, Operand::Float32(*value)));
                Ok(Ty::Float)
            }
            Expr::Bool(value) => {
                self.emitter.ldc_i4(i32::from(*value));
                Ok(Ty::Bool)
            }
            Expr::Str(value) => {
                self.emitter.ldstr(value.clone());
                Ok(Ty::String)
            }
            Expr::Default(ty) => {
                match ty {
                    Ty::Int | Ty::Bool => {
                        self.emitter.ldc_i4(0);
                    }
                    Ty::Float => {
                        self.emitter
                            .emit(Instruction::new(OpCode::Ldc_R4, Operand::Float32(0.0)));
                    }
                    Ty::Object | Ty::String => {
                        self.emitter.ldnull();
                    }
                    Ty::Enum(_, base) => {
                        resolve_type(ty, self.types, self.externals, &mut *self.module)
                            .map_err(|message| self.error(message))?;
                        if base.width() == 8 {
                            self.emitter
                                .emit(Instruction::new(OpCode::Ldc_I8, Operand::Int64(0)));
                        } else {
                            self.emitter.ldc_i4(0);
                        }
                    }
                    Ty::Struct(_) => self.default_struct(ty)?,
                    _ => return Err(self.error("default(T) is not supported for this type")),
                }
                Ok(ty.clone())
            }
            Expr::Name(name) => self.load(name),
            Expr::Negate(inner) => {
                let ty = self.expr(inner)?;
                if !matches!(ty, Ty::Int | Ty::Float) {
                    return Err(
                        self.error(format!("unary '-' requires a numeric operand, got {ty:?}"))
                    );
                }
                self.emitter.op(OpCode::Neg);
                Ok(ty)
            }
            Expr::Complement(inner) => {
                let ty = self.expr(inner)?;
                match &ty {
                    Ty::Int if Self::has_non_int_literal(inner) => {
                        return Err(self.error(
                            "bitwise complement of unsigned or long literals is not supported",
                        ));
                    }
                    Ty::Int => {
                        self.emitter.op(OpCode::Not);
                    }
                    Ty::Enum(_, base) => {
                        self.emitter.op(OpCode::Not);
                        if base.width() < 4 {
                            self.emitter.op(base.conversion());
                        }
                    }
                    _ => {
                        return Err(self
                            .error(format!("unary '~' requires an integer or enum, got {ty:?}")));
                    }
                }
                Ok(ty)
            }
            Expr::Conditional {
                condition,
                when_true,
                when_false,
            } => {
                let condition_ty = self.expr(condition)?;
                self.convert_assignable(&condition_ty, &Ty::Bool, "?: condition")
                    .map_err(|message| self.error(message))?;
                let otherwise = self.emitter.label();
                let end = self.emitter.label();
                self.emitter.branch(OpCode::Brfalse, otherwise);
                let result = self.expr(when_true)?;
                self.emitter.branch(OpCode::Br, end);
                self.emitter.mark(otherwise);
                let alternative = self.expr(when_false)?;
                self.convert_assignable(&alternative, &result, "?: branches")
                    .map_err(|message| self.error(message))?;
                self.emitter.mark(end);
                Ok(result)
            }
            Expr::Cast(target, inner) => {
                let enum_constant = self.enum_constant_value(inner);
                let source = self.expr(inner)?;
                match (&source, target) {
                    _ if &source == target => {}
                    (Ty::Enum(_, _), Ty::Int) => {
                        if enum_constant.is_some_and(|value| {
                            value < i32::MIN as i128 || value > i32::MAX as i128
                        }) {
                            return Err(
                                self.error("constant enum conversion exceeds the target type")
                            );
                        }
                        self.emitter.op(OpCode::Conv_I4);
                    }
                    (Ty::Enum(_, _), Ty::Object) => {
                        let token = self
                            .enum_type_token(&source)
                            .map_err(|message| self.error(message))?;
                        self.emitter
                            .emit(Instruction::new(OpCode::Box, Operand::Token(token)));
                    }
                    (Ty::Object, Ty::Enum(_, _)) => {
                        let token = self
                            .enum_type_token(target)
                            .map_err(|message| self.error(message))?;
                        self.emitter
                            .emit(Instruction::new(OpCode::Unbox_Any, Operand::Token(token)));
                    }
                    (Ty::Enum(_, _), Ty::Class(_)) if self.enum_boxing_cost(target).is_some() => {
                        let token = self
                            .enum_type_token(&source)
                            .map_err(|message| self.error(message))?;
                        self.emitter
                            .emit(Instruction::new(OpCode::Box, Operand::Token(token)));
                    }
                    (Ty::Class(_), Ty::Enum(_, _)) if self.enum_boxing_cost(&source).is_some() => {
                        let token = self
                            .enum_type_token(target)
                            .map_err(|message| self.error(message))?;
                        self.emitter
                            .emit(Instruction::new(OpCode::Unbox_Any, Operand::Token(token)));
                    }
                    (Ty::Int, Ty::Enum(_, base)) => {
                        let constant = match inner.as_ref() {
                            Expr::Int(value, _) => Some(i128::from(*value)),
                            Expr::Negate(value) => match value.as_ref() {
                                Expr::Int(value, _) => Some(-i128::from(*value)),
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(value) = constant {
                            let (min, max) = base.bounds();
                            if value < min || value > max {
                                return Err(self.error(
                                    "constant enum conversion exceeds the underlying type",
                                ));
                            }
                        }
                        self.emitter.op(base.conversion());
                    }
                    (Ty::Enum(_, source_base), Ty::Enum(_, target_base)) => {
                        let (min, max) = target_base.bounds();
                        if enum_constant.is_some_and(|value| value < min || value > max) {
                            return Err(
                                self.error("constant enum conversion exceeds the target type")
                            );
                        }
                        let conversion = if *source_base == EnumBase::U4 && target_base.width() == 8
                        {
                            OpCode::Conv_U8
                        } else {
                            target_base.conversion()
                        };
                        self.emitter.op(conversion);
                    }
                    (Ty::Int, Ty::Float) => {
                        self.emitter.op(OpCode::Conv_R4);
                    }
                    (Ty::Float, Ty::Int) => {
                        self.emitter.op(OpCode::Conv_I4);
                    }
                    (Ty::Class(_) | Ty::Object | Ty::String | Ty::Array(_), Ty::Class(_)) => {
                        let signature =
                            resolve_type(target, self.types, self.externals, &mut *self.module)
                                .map_err(|message| self.error(message))?;
                        let token = match signature {
                            TypeSig::Class(TypeDefOrRef::TypeRef(entry)) => {
                                MemberToken::TypeRef(entry)
                            }
                            TypeSig::Class(TypeDefOrRef::TypeDef(entry)) => {
                                MemberToken::TypeDef(entry)
                            }
                            TypeSig::Class(TypeDefOrRef::TypeSpec(entry)) => {
                                MemberToken::TypeSpec(entry)
                            }
                            _ => return Err(self.error("cast target is not a reference type")),
                        };
                        self.emitter
                            .emit(Instruction::new(OpCode::Castclass, Operand::Token(token)));
                    }
                    (Ty::Class(_), Ty::Int) => {
                        self.emitter.op(OpCode::Unbox_Any);
                        self.emitter.emit(Instruction::new(
                            OpCode::Unbox_Any,
                            Operand::Token(MemberToken::TypeRef(
                                match self
                                    .module
                                    .corlib_type("System.Int32")
                                    .map_err(|error| self.error(error.to_string()))?
                                {
                                    TypeDefOrRef::TypeRef(entry) => entry,
                                    _ => {
                                        return Err(
                                            self.error("System.Int32 is not a type reference")
                                        );
                                    }
                                },
                            )),
                        ));
                    }
                    _ => {
                        return Err(self.error(format!("cannot cast {source:?} to {target:?} yet")));
                    }
                }
                Ok(target.clone())
            }
            Expr::As(inner, target) => {
                let _source = self.expr(inner)?;
                match target {
                    Ty::Class(_) => {
                        let signature =
                            resolve_type(target, self.types, self.externals, &mut *self.module)
                                .map_err(|message| self.error(message))?;
                        let token = match signature {
                            TypeSig::Class(TypeDefOrRef::TypeRef(entry)) => {
                                MemberToken::TypeRef(entry)
                            }
                            TypeSig::Class(TypeDefOrRef::TypeDef(entry)) => {
                                MemberToken::TypeDef(entry)
                            }
                            TypeSig::Class(TypeDefOrRef::TypeSpec(entry)) => {
                                MemberToken::TypeSpec(entry)
                            }
                            _ => return Err(self.error("'as' requires a reference type")),
                        };
                        self.emitter
                            .emit(Instruction::new(OpCode::Isinst, Operand::Token(token)));
                        Ok(target.clone())
                    }
                    _ => Err(self.error(format!("'as {target:?}' is not supported"))),
                }
            }
            Expr::Coalesce(left, right) => {
                let ty = self.expr(left)?;
                if !is_reference(&ty) && ty != Ty::Object {
                    return Err(
                        self.error("left operand of '??' must be a supported reference type")
                    );
                }
                let end = self.emitter.label();
                self.emitter.dup();
                self.emitter.branch(OpCode::Brtrue, end);
                self.emitter.pop();
                let right_ty = self.expr(right)?;
                self.convert_assignable(&right_ty, &ty, "??")
                    .map_err(|message| self.error(message))?;
                self.emitter.mark(end);
                Ok(ty)
            }
            Expr::Not(inner) => {
                let ty = self.expr(inner)?;
                self.convert_assignable(&ty, &Ty::Bool, "unary '!'")
                    .map_err(|message| self.error(message))?;
                self.emitter.ldc_i4(0);
                self.emitter.op(OpCode::Ceq);
                Ok(Ty::Bool)
            }
            Expr::Binary(op, left, right) => self.binary(*op, left, right),
            Expr::Assign { name, value } => self.assign_expression(name, value),
            Expr::Increment {
                name,
                delta,
                prefix,
            } => {
                let ty = self.load(name)?;
                if !*prefix {
                    self.emitter.dup();
                }
                self.increment_top(&ty, *delta)?;
                if *prefix {
                    self.emitter.dup();
                }
                self.store_top(name)?;
                Ok(ty)
            }
            Expr::IncrementMember {
                target,
                name,
                delta,
                prefix,
            } => self.increment_member(target, name, *delta, *prefix),
            Expr::IncrementIndex {
                target,
                index,
                delta,
                prefix,
            } => self.increment_index(target, index, *delta, *prefix),
            Expr::New { ty, args, fields } => match ty {
                Ty::Struct(_) => {
                    if !args.is_empty() || !fields.is_empty() {
                        return Err(self
                            .error("struct constructors and initializers are not supported yet"));
                    }
                    self.default_struct(ty)?;
                    Ok(ty.clone())
                }
                Ty::Class(type_name) => {
                    if !self.types.contains_key(type_name)
                        && self.externals.find(type_name).is_none()
                        && let Some(name) = core_exception_name(type_name)
                    {
                        if !fields.is_empty() {
                            return Err(
                                self.error("exception object initializers are not supported yet")
                            );
                        }
                        let params =
                            match args.len() {
                                0 => Vec::new(),
                                1 => vec![Ty::String],
                                _ => return Err(self.error(format!(
                                    "unsupported exception constructor '{name}' with {} arguments",
                                    args.len()
                                ))),
                            };
                        self.push_arguments(args, &params, "argument")?;
                        let signature = MethodSig::new_instance(
                            TypeSig::VOID,
                            params.iter().map(|_| TypeSig::STRING).collect(),
                        );
                        let constructor = self
                            .module
                            .import_corlib_method(name, ".ctor", signature)
                            .map_err(|error| self.error(error.to_string()))?;
                        self.emitter.newobj(constructor);
                        return Ok(Ty::Class(type_name.clone()));
                    }
                    let Some(constructor) = self.constructor(type_name, args.len()) else {
                        return Err(self.error(format!(
                            "no constructor for '{type_name}' taking {} argument(s)",
                            args.len()
                        )));
                    };
                    self.require_method_access(&constructor.entry, true, Some(type_name))?;
                    self.push_arguments(args, &constructor.params, "argument")?;
                    self.emitter.newobj(constructor.entry);
                    self.apply_initializer(type_name, fields)?;
                    Ok(Ty::Class(type_name.clone()))
                }
                Ty::Generic(type_name, type_args) => {
                    let (token, params, _) =
                        self.generic_member(type_name, type_args, ".ctor", args.len())?;
                    self.push_arguments(args, &params, "argument")?;
                    self.emitter.newobj(token);
                    if !fields.is_empty() {
                        return Err(self
                            .error("object initializers on generic types are not supported yet"));
                    }
                    Ok(ty.clone())
                }
                other => Err(self.error(format!("'new {other:?}' is not supported"))),
            },
            Expr::Member { target, name } => self.member_read(target, name),
            Expr::Call {
                receiver,
                name,
                generic_args,
                args,
            } => self.call(receiver.as_deref(), name, generic_args, args),
            Expr::NewArray { element, size } => {
                let size_type = self.expr(size)?;
                Self::require(&size_type, &Ty::Int, "array size")
                    .map_err(|message| self.error(message))?;
                let token = self.element_token(element)?;
                self.emitter
                    .emit(Instruction::new(OpCode::Newarr, Operand::Token(token)));
                Ok(Ty::Array(Box::new(element.clone())))
            }
            Expr::Index { target, index } => {
                let target_type = self.expr(target)?;
                let Ty::Array(element) = target_type else {
                    return Err(self.error("indexing requires an array"));
                };
                let index_type = self.expr(index)?;
                Self::require(&index_type, &Ty::Int, "array index")
                    .map_err(|message| self.error(message))?;
                let opcode = array_element_opcode(&element, false)
                    .ok_or_else(|| self.error(format!("unsupported array element {element:?}")))?;
                self.emitter.op(opcode);
                Ok(*element)
            }
        }
    }

    fn element_token(&mut self, element: &Ty) -> Result<MemberToken, CompileError> {
        match element {
            Ty::Object => match self
                .module
                .corlib_type("System.Object")
                .map_err(|error| self.error(error.to_string()))?
            {
                TypeDefOrRef::TypeRef(entry) => Ok(MemberToken::TypeRef(entry)),
                _ => Err(self.error("System.Object is not a type reference")),
            },
            Ty::Class(type_name) => {
                let Some(entry) = self.types.get(type_name) else {
                    return Err(self.error(format!("unknown type '{type_name}'")));
                };
                Ok(MemberToken::TypeDef(entry.clone()))
            }
            Ty::Enum(_, _) => self
                .enum_type_token(element)
                .map_err(|message| self.error(message)),
            Ty::Int => match self
                .module
                .corlib_type("System.Int32")
                .map_err(|error| self.error(error.to_string()))?
            {
                TypeDefOrRef::TypeRef(entry) => Ok(MemberToken::TypeRef(entry)),
                _ => Err(self.error("System.Int32 is not a type reference")),
            },
            Ty::Bool => match self
                .module
                .corlib_type("System.Boolean")
                .map_err(|error| self.error(error.to_string()))?
            {
                TypeDefOrRef::TypeRef(entry) => Ok(MemberToken::TypeRef(entry)),
                _ => Err(self.error("System.Boolean is not a type reference")),
            },
            Ty::Float => match self
                .module
                .corlib_type("System.Single")
                .map_err(|error| self.error(error.to_string()))?
            {
                TypeDefOrRef::TypeRef(entry) => Ok(MemberToken::TypeRef(entry)),
                _ => Err(self.error("System.Single is not a type reference")),
            },
            Ty::String => match self
                .module
                .corlib_type("System.String")
                .map_err(|error| self.error(error.to_string()))?
            {
                TypeDefOrRef::TypeRef(entry) => Ok(MemberToken::TypeRef(entry)),
                _ => Err(self.error("System.String is not a type reference")),
            },
            other => Err(self.error(format!("arrays of {other:?} are not supported"))),
        }
    }

    fn enum_constant_value(&self, expression: &Expr) -> Option<i128> {
        self.enum_constant(expression).map(|(_, value)| value)
    }

    fn enum_constant(&self, expression: &Expr) -> Option<(&EnumInfo, i128)> {
        match expression {
            Expr::Default(Ty::Enum(name, _)) => Some((self.enums.get(name)?.as_ref(), 0)),
            Expr::Member { target, name } => {
                let type_name = Self::type_path(target)?;
                let root = type_name.split('.').next()?;
                if self.locals.contains_key(root)
                    || self.params.iter().any(|(name, _)| name == root)
                {
                    return None;
                }
                let info = find_enum(self.enums, &type_name, &self.namespace, self.enum_imports)?;
                Some((info, *info.values.get(name)?))
            }
            Expr::Cast(Ty::Enum(name, base), inner) => {
                let info = self.enums.get(name)?.as_ref();
                let value = match inner.as_ref() {
                    Expr::Int(value, _) => i128::from(*value),
                    other => self
                        .enum_constant(other)
                        .map(|(_, value)| value)
                        .or_else(|| Self::constant_int(other).map(i128::from))?,
                };
                let (min, max) = base.bounds();
                (min..=max).contains(&value).then_some((info, value))
            }
            Expr::Complement(inner) => {
                let (info, value) = self.enum_constant(inner)?;
                Some((info, info.base.decode_bits(!(value as u64))))
            }
            Expr::Binary(op @ (BinaryOp::Add | BinaryOp::Subtract), left, right) => {
                let (info, enum_value, number, add) =
                    if let Some((info, value)) = self.enum_constant(left) {
                        if !Self::enum_integer_allowed(right, info.base) {
                            return None;
                        }
                        (
                            info,
                            value,
                            Self::constant_int(right)?,
                            matches!(op, BinaryOp::Add),
                        )
                    } else if matches!(op, BinaryOp::Add) {
                        let (info, value) = self.enum_constant(right)?;
                        if !Self::enum_integer_allowed(left, info.base) {
                            return None;
                        }
                        (info, value, Self::constant_int(left)?, true)
                    } else {
                        return None;
                    };
                let result = if add {
                    enum_value + i128::from(number)
                } else {
                    enum_value - i128::from(number)
                };
                let (min, max) = info.base.bounds();
                (min..=max).contains(&result).then_some((info, result))
            }
            Expr::Binary(
                op @ (BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor),
                left,
                right,
            ) => {
                let left_constant = self.enum_constant(left);
                let right_constant = self.enum_constant(right);
                let (info, left_value, right_value) = match (left_constant, right_constant) {
                    (Some((info, left)), Some((other, right)))
                        if info.full_name() == other.full_name() =>
                    {
                        (info, left, right)
                    }
                    (Some((info, left)), None) if Self::is_enum_zero(right) => (info, left, 0),
                    (None, Some((info, right))) if Self::is_enum_zero(left) => (info, 0, right),
                    _ => return None,
                };
                let value = match op {
                    BinaryOp::BitAnd => (left_value as u64) & (right_value as u64),
                    BinaryOp::BitOr => (left_value as u64) | (right_value as u64),
                    BinaryOp::BitXor => (left_value as u64) ^ (right_value as u64),
                    _ => unreachable!(),
                };
                Some((info, info.base.decode_bits(value)))
            }
            _ => None,
        }
    }

    fn enum_switch_constant(&self, expression: &Expr, expected: &Ty) -> Option<i128> {
        let Ty::Enum(expected_name, base) = expected else {
            return None;
        };
        if let Some((info, value)) = self.enum_constant(expression) {
            return (info.full_name() == *expected_name).then_some(value);
        }
        if let Expr::Cast(target, inner) = expression
            && target == expected
        {
            let value = match inner.as_ref() {
                Expr::Int(value, _) => i128::from(*value),
                other => self
                    .enum_constant_value(other)
                    .or_else(|| Self::constant_int(other).map(i128::from))?,
            };
            let (min, max) = base.bounds();
            return (min..=max).contains(&value).then_some(value);
        }
        Self::is_enum_zero(expression).then_some(0)
    }

    fn member_read(&mut self, target: &Expr, name: &str) -> Result<Ty, CompileError> {
        if let Some(type_name) = Self::type_path(target)
            && let Some(info) =
                find_enum(self.enums, &type_name, &self.namespace, self.enum_imports)
            && !self
                .locals
                .contains_key(type_name.split('.').next().unwrap_or(&type_name))
            && !self.params.iter().any(|(parameter, _)| {
                parameter == type_name.split('.').next().unwrap_or(&type_name)
            })
        {
            let Some(value) = info.values.get(name) else {
                return Err(self.error(format!("enum '{type_name}' has no member '{name}'")));
            };
            if info.base.width() == 8 {
                self.emitter.emit(Instruction::new(
                    OpCode::Ldc_I8,
                    Operand::Int64(*value as u64 as i64),
                ));
            } else {
                self.emitter.ldc_i4(*value as i32);
            }
            return Ok(Ty::Enum(info.full_name(), info.base));
        }
        if let Expr::Name(builtin) = target
            && !self.types.contains_key(builtin)
        {
            let is_value = self.locals.contains_key(builtin)
                || self
                    .params
                    .iter()
                    .any(|(parameter, _)| parameter == builtin);
            if !is_value {
                match (builtin.as_str(), name) {
                    ("string", "Empty") => {
                        self.emitter.ldstr("");
                        return Ok(Ty::String);
                    }
                    ("int", "MaxValue") => {
                        self.emitter.ldc_i4(i32::MAX);
                        return Ok(Ty::Int);
                    }
                    ("int", "MinValue") => {
                        self.emitter.ldc_i4(i32::MIN);
                        return Ok(Ty::Int);
                    }
                    _ => {}
                }
            }
        }
        if let Some((ty, field)) = self.static_field(target, name, false)? {
            match field {
                StaticFieldValue::Location(token) => {
                    self.emitter
                        .emit(Instruction::new(OpCode::Ldsfld, Operand::Token(token)));
                }
                StaticFieldValue::Literal(constant) => self.emit_literal(&ty, &constant)?,
            }
            return Ok(ty);
        }
        // `Type.StaticProperty` does not evaluate the type name as a value.
        if let Expr::Name(type_name) = target
            && self.types.contains_key(type_name)
            && !self.shadows_type_root(type_name)
        {
            if let Some(property) = self
                .symbols
                .get(type_name)
                .and_then(|symbol| symbol.properties.get(name))
                .filter(|property| property.is_static)
            {
                let Some(getter) = property.getter.clone() else {
                    return Err(self.error(format!("property '{type_name}.{name}' has no getter")));
                };
                self.require_method_access(&getter, true, None)?;
                let ty = property.ty.clone();
                self.emitter.call(getter);
                return Ok(ty);
            }
            return Err(self.error(format!("unknown static member '{type_name}.{name}'")));
        }
        let target_type = self.expr(target)?;
        if let Ty::Struct(type_name) = &target_type {
            let short = type_name.rsplit('.').next().unwrap_or(type_name);
            let field = self
                .field_symbol(short, name)
                .ok_or_else(|| self.error(format!("unknown member '{type_name}.{name}'")))?;
            self.require_field_access(&field.entry, true, Some(short))?;
            let signature =
                resolve_type(&target_type, self.types, self.externals, &mut *self.module)
                    .map_err(|message| self.error(message))?;
            let temporary = self.emitter.declare_local(signature);
            self.emitter.stloc(temporary).ldloca(temporary);
            self.emitter.emit(Instruction::new(
                OpCode::Ldfld,
                Operand::Token(MemberToken::Field(field.entry)),
            ));
            return Ok(field.ty);
        }
        if let Ty::Array(_) = target_type {
            if name == "Length" {
                self.emitter.op(OpCode::Ldlen);
                self.emitter.op(OpCode::Conv_I4);
                return Ok(Ty::Int);
            }
            return Err(self.error(format!("arrays have no member '{name}'")));
        }
        if let Ty::Generic(type_name, type_args) = &target_type {
            let (token, params, ret) =
                self.generic_member(type_name, type_args, &format!("get_{name}"), 0)?;
            debug_assert!(params.is_empty());
            self.emitter.callvirt(token);
            return Ok(ret);
        }
        let Ty::Class(type_name) = target_type else {
            return Err(self.error(format!("type {target_type:?} has no members")));
        };
        if let Some(field) = self.field_symbol(&type_name, name) {
            self.require_field_access(&field.entry, true, Some(&type_name))?;
            if field.is_static {
                return Err(self.error(format!("'{type_name}.{name}' is static")));
            }
            self.emitter.emit(Instruction::new(
                OpCode::Ldfld,
                Operand::Token(MemberToken::Field(field.entry)),
            ));
            return Ok(field.ty);
        }
        // Inherited external members: fields first, then property getters.
        let external_owner = self.external_base_of(&type_name);
        let external_field = external_owner
            .as_deref()
            .and_then(|base| self.externals.field_in_chain(base, name));
        if let Some((index, entry)) = external_field {
            self.require_field_access(&entry, false, Some(&type_name))?;
            if entry.get().flags.contains(FieldFlags::Static) {
                return Err(self.error(format!("'{type_name}.{name}' is static")));
            }
            let signature = {
                let guard = entry.get();
                guard.signature.clone()
            };
            let token = {
                let module = &mut *self.module;
                let foreign = &self.externals.modules[index];
                module
                    .import_foreign_field(&entry, foreign)
                    .map_err(|error| self.error(error.to_string()))?
            };
            self.emitter.emit(Instruction::new(
                OpCode::Ldfld,
                Operand::Token(token.clone()),
            ));
            return Ok(external_ty(&signature.ty, self.enums, false));
        }
        let property = self
            .symbols
            .get(&type_name)
            .and_then(|symbol| symbol.properties.get(name))
            .map(|property| (property.ty.clone(), property.getter.clone()));
        let Some((ty, getter)) = property else {
            let getter_name = format!("get_{name}");
            let getter_owner = self
                .external_base_of(&type_name)
                .unwrap_or_else(|| type_name.clone());
            if let Some((index, entry)) =
                self.externals
                    .method_in_chain(&getter_owner, &getter_name, 0)
            {
                self.require_method_access(&entry, false, Some(&type_name))?;
                let flags = entry.get().flags;
                if flags.contains(MethodFlags::Static) || !flags.contains(MethodFlags::SpecialName)
                {
                    return Err(self.error(format!(
                        "property '{type_name}.{name}' has no accessible instance getter"
                    )));
                }
                let signature = {
                    let guard = entry.get();
                    guard.signature.clone()
                };
                let token = {
                    let module = &mut *self.module;
                    let foreign = &self.externals.modules[index];
                    module
                        .import_foreign_method(&entry, foreign)
                        .map_err(|error| self.error(error.to_string()))?
                };
                self.emitter.callvirt(token);
                return Ok(external_ty(&signature.ret, self.enums, false));
            }
            return Err(self.error(format!("unknown member '{type_name}.{name}'")));
        };
        let Some(getter) = getter else {
            return Err(self.error(format!("property '{type_name}.{name}' has no getter")));
        };
        self.require_method_access(&getter, true, Some(&type_name))?;
        self.emitter.callvirt(getter);
        Ok(ty)
    }

    /// Applies `new T(...) { Name = value }` assignments, keeping the object on
    /// the stack as the expression result.
    fn apply_initializer(
        &mut self,
        type_name: &str,
        fields: &[(String, Expr)],
    ) -> Result<(), CompileError> {
        for (name, value) in fields {
            if let Some(field) = self.field_symbol(type_name, name) {
                self.require_field_access(&field.entry, true, Some(type_name))?;
                if field.is_static {
                    return Err(self.error(format!("'{name}' is static")));
                }
                self.require_field_writable(&field.entry, true, false)?;
                self.emitter.dup();
                let actual = self.expr(value)?;
                self.convert_expression(&actual, &field.ty, value, "initializer")
                    .map_err(|message| self.error(message))?;
                self.emitter.emit(Instruction::new(
                    OpCode::Stfld,
                    Operand::Token(MemberToken::Field(field.entry)),
                ));
                continue;
            }
            let property = self
                .symbols
                .get(type_name)
                .and_then(|symbol| symbol.properties.get(name))
                .cloned();
            let Some(property) = property else {
                return Err(self.error(format!("unknown member '{type_name}.{name}'")));
            };
            let Some(setter) = property.setter else {
                return Err(self.error(format!("property '{name}' has no setter")));
            };
            self.require_method_access(&setter, true, Some(type_name))?;
            self.emitter.dup();
            let actual = self.expr(value)?;
            self.convert_expression(&actual, &property.ty, value, "initializer")
                .map_err(|message| self.error(message))?;
            self.emitter.callvirt(setter);
        }
        Ok(())
    }

    fn push_arguments(
        &mut self,
        args: &[Expr],
        params: &[Ty],
        what: &str,
    ) -> Result<(), CompileError> {
        for (index, argument) in args.iter().enumerate() {
            let actual = self.expr(argument)?;
            self.convert_expression(&actual, &params[index], argument, what)
                .map_err(|message| self.error(message))?;
        }
        Ok(())
    }

    fn binary(&mut self, op: BinaryOp, left: &Expr, right: &Expr) -> Result<Ty, CompileError> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let end = self.emitter.label();
            let done = self.emitter.label();
            let ty = self.expr(left)?;
            self.convert_assignable(&ty, &Ty::Bool, "logical operand")
                .map_err(|message| self.error(message))?;
            let short_circuit = if matches!(op, BinaryOp::And) {
                OpCode::Brfalse
            } else {
                OpCode::Brtrue
            };
            self.emitter.branch(short_circuit, end);
            let ty = self.expr(right)?;
            self.convert_assignable(&ty, &Ty::Bool, "logical operand")
                .map_err(|message| self.error(message))?;
            self.emitter.branch(short_circuit, end);
            self.emitter
                .ldc_i4(if matches!(op, BinaryOp::And) { 1 } else { 0 });
            self.emitter.branch(OpCode::Br, done);
            self.emitter.mark(end);
            self.emitter
                .ldc_i4(if matches!(op, BinaryOp::And) { 0 } else { 1 });
            self.emitter.mark(done);
            return Ok(Ty::Bool);
        }

        let mut left_ty = self.expr(left)?;
        let mut right_ty = self.expr(right)?;
        if matches!(op, BinaryOp::Add | BinaryOp::Subtract)
            && (matches!(left_ty, Ty::Enum(_, _)) || matches!(right_ty, Ty::Enum(_, _)))
        {
            if self.has_untracked_enum_constant(left)? || self.has_untracked_enum_constant(right)? {
                return Err(self.error("arithmetic with enum constant fields is not supported yet"));
            }
            let number = if left_ty == Ty::Int {
                Some(left)
            } else if right_ty == Ty::Int {
                Some(right)
            } else {
                None
            };
            if let Some(number) = number
                && self.evaluate_int_constant(number)?.is_some()
                && Self::constant_int(number).is_none()
            {
                return Err(
                    self.error("enum arithmetic with integer constant fields is not supported yet")
                );
            }
        }
        if matches!(op, BinaryOp::Add | BinaryOp::Subtract) {
            match (&left_ty, &right_ty) {
                (Ty::Enum(_, base), Ty::Int) => {
                    if Self::enum_integer_allowed(right, *base) {
                        if let (Some((_, enum_value)), Some(number)) =
                            (self.enum_constant(left), Self::constant_int(right))
                        {
                            let result = if matches!(op, BinaryOp::Add) {
                                enum_value + i128::from(number)
                            } else {
                                enum_value - i128::from(number)
                            };
                            let (min, max) = base.bounds();
                            if !(min..=max).contains(&result) {
                                return Err(self.error(
                                    "constant enum arithmetic exceeds the underlying type",
                                ));
                            }
                        }
                        self.convert_enum_integer(*base);
                        self.emitter.op(if matches!(op, BinaryOp::Add) {
                            OpCode::Add
                        } else {
                            OpCode::Sub
                        });
                        self.finish_enum_arithmetic(*base);
                        return Ok(left_ty);
                    }
                    if matches!(op, BinaryOp::Subtract)
                        && *base == EnumBase::I4
                        && Self::is_enum_zero(right)
                    {
                        self.emitter.op(OpCode::Sub);
                        return Ok(Ty::Int);
                    }
                    return Err(
                        self.error("integer operand cannot convert to the enum underlying type")
                    );
                }
                (Ty::Int, Ty::Enum(_, base)) => {
                    if matches!(op, BinaryOp::Add) && Self::enum_integer_allowed(left, *base) {
                        if let (Some(number), Some((_, enum_value))) =
                            (Self::constant_int(left), self.enum_constant(right))
                        {
                            let result = i128::from(number) + enum_value;
                            let (min, max) = base.bounds();
                            if !(min..=max).contains(&result) {
                                return Err(self.error(
                                    "constant enum arithmetic exceeds the underlying type",
                                ));
                            }
                        }
                        let signature =
                            resolve_type(&right_ty, self.types, self.externals, &mut *self.module)
                                .map_err(|message| self.error(message))?;
                        let temporary = self.emitter.declare_local(signature);
                        self.emitter.stloc(temporary);
                        self.convert_enum_integer(*base);
                        self.emitter.ldloc(temporary).op(OpCode::Add);
                        self.finish_enum_arithmetic(*base);
                        return Ok(right_ty);
                    }
                    if matches!(op, BinaryOp::Subtract)
                        && *base == EnumBase::I4
                        && Self::is_enum_zero(left)
                    {
                        if let Some((_, value)) = self.enum_constant(right)
                            && !(i32::MIN as i128..=i32::MAX as i128).contains(&-value)
                        {
                            return Err(
                                self.error("constant enum difference exceeds the underlying type")
                            );
                        }
                        self.emitter.op(OpCode::Sub);
                        return Ok(Ty::Int);
                    }
                    return Err(self.error("integer operand cannot be used with this enum"));
                }
                (Ty::Enum(left_name, base), Ty::Enum(right_name, _))
                    if matches!(op, BinaryOp::Subtract)
                        && left_name == right_name
                        && *base == EnumBase::I4 =>
                {
                    if let (Some((_, left)), Some((_, right))) =
                        (self.enum_constant(left), self.enum_constant(right))
                        && !(i32::MIN as i128..=i32::MAX as i128).contains(&(left - right))
                    {
                        return Err(
                            self.error("constant enum difference exceeds the underlying type")
                        );
                    }
                    self.emitter.op(OpCode::Sub);
                    return Ok(Ty::Int);
                }
                _ => {}
            }
        }
        if matches!(
            op,
            BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessOrEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterOrEqual
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
        ) {
            if matches!(left_ty, Ty::Enum(_, _)) && right_ty == Ty::Int && Self::is_enum_zero(right)
            {
                self.convert_expression(&right_ty, &left_ty, right, "enum comparison")
                    .map_err(|message| self.error(message))?;
                right_ty = left_ty.clone();
            } else if matches!(right_ty, Ty::Enum(_, _))
                && left_ty == Ty::Int
                && Self::is_enum_zero(left)
            {
                let signature =
                    resolve_type(&right_ty, self.types, self.externals, &mut *self.module)
                        .map_err(|message| self.error(message))?;
                let temporary = self.emitter.declare_local(signature);
                self.emitter.stloc(temporary);
                self.convert_expression(&left_ty, &right_ty, left, "enum comparison")
                    .map_err(|message| self.error(message))?;
                self.emitter.ldloc(temporary);
                left_ty = right_ty.clone();
            }
        }
        if let (Ty::Enum(left_name, base), Ty::Enum(right_name, _)) = (&left_ty, &right_ty)
            && left_name == right_name
        {
            if matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor) {
                self.emitter.op(match op {
                    BinaryOp::BitAnd => OpCode::And,
                    BinaryOp::BitOr => OpCode::Or,
                    BinaryOp::BitXor => OpCode::Xor,
                    _ => unreachable!(),
                });
                return Ok(left_ty);
            }
            let unsigned = base.is_unsigned();
            match op {
                BinaryOp::Equal => {
                    self.emitter.op(OpCode::Ceq);
                }
                BinaryOp::NotEqual => {
                    self.emitter.op(OpCode::Ceq).ldc_i4(0).op(OpCode::Ceq);
                }
                BinaryOp::Less => {
                    self.emitter.op(if unsigned {
                        OpCode::Clt_Un
                    } else {
                        OpCode::Clt
                    });
                }
                BinaryOp::Greater => {
                    self.emitter.op(if unsigned {
                        OpCode::Cgt_Un
                    } else {
                        OpCode::Cgt
                    });
                }
                BinaryOp::LessOrEqual => {
                    self.emitter
                        .op(if unsigned {
                            OpCode::Cgt_Un
                        } else {
                            OpCode::Cgt
                        })
                        .ldc_i4(0)
                        .op(OpCode::Ceq);
                }
                BinaryOp::GreaterOrEqual => {
                    self.emitter
                        .op(if unsigned {
                            OpCode::Clt_Un
                        } else {
                            OpCode::Clt
                        })
                        .ldc_i4(0)
                        .op(OpCode::Ceq);
                }
                _ => return Err(self.error("this enum operator is not supported yet")),
            }
            return Ok(Ty::Bool);
        }
        if matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor)
            && left_ty == right_ty
            && matches!(left_ty, Ty::Int | Ty::Bool)
        {
            if left_ty == Ty::Int
                && (Self::has_non_int_literal(left) || Self::has_non_int_literal(right))
            {
                return Err(
                    self.error("bitwise operations on unsigned or long literals are not supported")
                );
            }
            self.emitter.op(match op {
                BinaryOp::BitAnd => OpCode::And,
                BinaryOp::BitOr => OpCode::Or,
                BinaryOp::BitXor => OpCode::Xor,
                _ => unreachable!(),
            });
            return Ok(left_ty);
        }
        if matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor) {
            return Err(self.error(format!(
                "bitwise operands are incompatible: {left_ty:?} and {right_ty:?}"
            )));
        }
        if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual)
            && left_ty == Ty::String
            && right_ty == Ty::String
        {
            let method = if matches!(op, BinaryOp::Equal) {
                "op_Equality"
            } else {
                "op_Inequality"
            };
            let entry = self
                .module
                .import_corlib_method(
                    "System.String",
                    method,
                    MethodSig::new_static(TypeSig::BOOL, vec![TypeSig::STRING, TypeSig::STRING]),
                )
                .map_err(|error| self.error(error.to_string()))?;
            self.emitter.call(entry);
            return Ok(Ty::Bool);
        }
        if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual)
            && (left_ty == Ty::Null || right_ty == Ty::Null)
        {
            let other = if left_ty == Ty::Null {
                &right_ty
            } else {
                &left_ty
            };
            if *other != Ty::Null && *other != Ty::Object && !is_reference(other) {
                return Err(self.error("null cannot be compared with a non-nullable value"));
            }
            // Reference comparison against null: `node == null`.
            self.emitter.op(OpCode::Ceq);
            if matches!(op, BinaryOp::NotEqual) {
                self.emitter.ldc_i4(0);
                self.emitter.op(OpCode::Ceq);
            }
            return Ok(Ty::Bool);
        }
        if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual)
            && left_ty == Ty::Bool
            && right_ty == Ty::Bool
        {
            self.emitter.op(OpCode::Ceq);
            if matches!(op, BinaryOp::NotEqual) {
                self.emitter.ldc_i4(0);
                self.emitter.op(OpCode::Ceq);
            }
            return Ok(Ty::Bool);
        }
        if matches!(op, BinaryOp::Add) && left_ty == Ty::String && right_ty == Ty::String {
            let entry = self
                .module
                .import_corlib_method(
                    "System.String",
                    "Concat",
                    MethodSig::new_static(TypeSig::STRING, vec![TypeSig::STRING, TypeSig::STRING]),
                )
                .map_err(|error| self.error(error.to_string()))?;
            self.emitter.call(entry);
            return Ok(Ty::String);
        }
        match (&left_ty, &right_ty) {
            (Ty::Float, Ty::Float) | (Ty::Int, Ty::Int) => {}
            (Ty::Float, Ty::Int) => {
                self.emitter.op(OpCode::Conv_R4);
            }
            (Ty::Int, Ty::Float) => {
                let temporary = self
                    .emitter
                    .declare_local(TypeSig::Primitive(PrimitiveType::R4));
                self.emitter
                    .stloc(temporary)
                    .op(OpCode::Conv_R4)
                    .ldloc(temporary);
            }
            _ => return Err(self.error(format!(
                "operands are {left_ty:?} and {right_ty:?}; non-numeric arithmetic is not supported"
            ))),
        }
        let floating = left_ty == Ty::Float || right_ty == Ty::Float;
        match op {
            BinaryOp::Add => {
                self.emitter.op(OpCode::Add);
            }
            BinaryOp::Subtract => {
                self.emitter.op(OpCode::Sub);
            }
            BinaryOp::Multiply => {
                self.emitter.op(OpCode::Mul);
            }
            BinaryOp::Divide => {
                self.emitter.op(OpCode::Div);
            }
            BinaryOp::Remainder => {
                self.emitter.op(OpCode::Rem);
            }
            BinaryOp::Equal => {
                self.emitter.op(OpCode::Ceq);
            }
            BinaryOp::NotEqual => {
                self.emitter.op(OpCode::Ceq);
                self.emitter.ldc_i4(0);
                self.emitter.op(OpCode::Ceq);
            }
            BinaryOp::Less => {
                self.emitter.op(OpCode::Clt);
            }
            BinaryOp::Greater => {
                self.emitter.op(OpCode::Cgt);
            }
            BinaryOp::LessOrEqual => {
                self.emitter.op(if floating {
                    OpCode::Cgt_Un
                } else {
                    OpCode::Cgt
                });
                self.emitter.ldc_i4(0);
                self.emitter.op(OpCode::Ceq);
            }
            BinaryOp::GreaterOrEqual => {
                self.emitter.op(if floating {
                    OpCode::Clt_Un
                } else {
                    OpCode::Clt
                });
                self.emitter.ldc_i4(0);
                self.emitter.op(OpCode::Ceq);
            }
            BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::And
            | BinaryOp::Or => unreachable!(),
        }
        if matches!(
            op,
            BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::Greater
                | BinaryOp::LessOrEqual
                | BinaryOp::GreaterOrEqual
        ) {
            return Ok(Ty::Bool);
        }
        Ok(if floating { Ty::Float } else { Ty::Int })
    }

    /// Reconstructs `A.B.C` from a member chain so qualified type names resolve.
    fn type_path(receiver: &Expr) -> Option<String> {
        match receiver {
            Expr::Name(name) => Some(name.clone()),
            Expr::Member { target, name } => {
                Self::type_path(target).map(|prefix| format!("{prefix}.{name}"))
            }
            _ => None,
        }
    }

    fn call(
        &mut self,
        receiver: Option<&Expr>,
        name: &str,
        generic_args: &[Ty],
        args: &[Expr],
    ) -> Result<Ty, CompileError> {
        if let Some(receiver) = receiver {
            if let Expr::Name(receiver_name) = receiver
                && receiver_name == "base"
            {
                let Some(base) = self.base.clone() else {
                    return Err(self.error("'base' is used in a type without a base class"));
                };
                let short = base.rsplit('.').next().unwrap_or(&base).to_string();
                if let Some(method) = self.inherited_method(&short, name, args.len()) {
                    self.require_method_access(&method.entry, true, None)?;
                    self.emitter.ldarg(0);
                    self.push_arguments(args, &method.params, "argument")?;
                    // `base.M()` binds the base implementation, so this is a
                    // non-virtual call.
                    self.emitter.call(method.entry);
                    return Ok(method.ret);
                }
                if let Some((index, entry)) =
                    self.externals.method_in_chain(&base, name, args.len())
                {
                    self.require_method_access(&entry, false, None)?;
                    if entry.get().flags.contains(MethodFlags::Static) {
                        return Err(self.error(format!("'{base}.{name}' is static")));
                    }
                    let signature = {
                        let guard = entry.get();
                        guard.signature.clone()
                    };
                    let params: Vec<Ty> = signature
                        .params
                        .iter()
                        .map(|sig| external_ty(sig, self.enums, false))
                        .collect();
                    self.emitter.ldarg(0);
                    self.push_arguments(args, &params, "argument")?;
                    let token = {
                        let module = &mut *self.module;
                        let foreign = &self.externals.modules[index];
                        module
                            .import_foreign_method(&entry, foreign)
                            .map_err(|error| self.error(error.to_string()))?
                    };
                    self.emitter.call(token);
                    return Ok(external_ty(&signature.ret, self.enums, false));
                }
                return Err(self.error(format!(
                    "base type '{base}' has no method '{name}' with {} parameter(s)",
                    args.len()
                )));
            }
            if let Expr::Name(receiver_name) = receiver {
                let is_value = receiver_name == "this"
                    || self.locals.contains_key(receiver_name)
                    || self
                        .params
                        .iter()
                        .any(|(parameter, _)| parameter == receiver_name)
                    || {
                        let current = self.current.clone();
                        self.field_symbol(&current, receiver_name).is_some()
                    };
                if !is_value {
                    if !self.types.contains_key(receiver_name) {
                        if let Some(result) = self.bcl_call(receiver_name, name, args)? {
                            return Ok(result);
                        }
                        if self.externals.find(receiver_name).is_none() {
                            return Err(self.error(format!("unknown type '{receiver_name}'")));
                        }
                    }
                    return self.static_call(receiver_name, name, generic_args, args);
                }
            }
            // A dotted chain may be a qualified type (`System.Text.Regex`).
            if let Some(path) = Self::type_path(receiver) {
                let short = path.rsplit('.').next().unwrap_or(&path).to_string();
                let is_value = self.locals.contains_key(&short)
                    || self.params.iter().any(|(parameter, _)| *parameter == short)
                    || {
                        let current = self.current.clone();
                        self.field_symbol(&current, &short).is_some()
                    };
                if !is_value
                    && (self.types.contains_key(&short) || self.externals.find(&path).is_some())
                {
                    if let Some(result) = self.bcl_call(&path, name, args)? {
                        return Ok(result);
                    }
                    return self.static_call(&short, name, generic_args, args);
                }
            }
            let receiver_type = self.expr(receiver)?;
            if matches!(receiver_type, Ty::Enum(_, _)) {
                return self.enum_call(&receiver_type, name, generic_args, args);
            }
            if matches!(
                receiver_type,
                Ty::Int | Ty::Float | Ty::Bool | Ty::String | Ty::Object
            ) {
                return self.primitive_call(&receiver_type, name, generic_args, args);
            }
            if let Ty::Generic(type_name, type_args) = &receiver_type {
                let (token, params, ret) =
                    self.generic_member(type_name, type_args, name, args.len())?;
                self.push_arguments(args, &params, "argument")?;
                self.emitter.callvirt(token);
                return Ok(ret);
            }
            let Ty::Class(type_name) = receiver_type else {
                return Err(self.error(format!("type {receiver_type:?} has no methods")));
            };
            if let Some(method) = self.inherited_method(&type_name, name, args.len()) {
                self.require_method_access(&method.entry, true, Some(&type_name))?;
                self.push_arguments(args, &method.params, "argument")?;
                self.emitter.callvirt(method.entry);
                return Ok(method.ret);
            }
            if let Some((index, entry)) = self.externals.method(&type_name, name, args.len()) {
                self.require_method_access(&entry, false, Some(&type_name))?;
                if entry.get().flags.contains(MethodFlags::Static) {
                    return Err(self.error(format!("'{type_name}.{name}' is static")));
                }
                let signature = {
                    let guard = entry.get();
                    guard.signature.clone()
                };
                if !generic_args.is_empty() {
                    return self.generic_call(index, &entry, &signature, generic_args, args);
                }
                let params: Vec<Ty> = signature
                    .params
                    .iter()
                    .map(|sig| external_ty(sig, self.enums, false))
                    .collect();
                self.push_arguments(args, &params, "argument")?;
                let token = {
                    let module = &mut *self.module;
                    let foreign = &self.externals.modules[index];
                    module
                        .import_foreign_method(&entry, foreign)
                        .map_err(|error| self.error(error.to_string()))?
                };
                self.emitter.callvirt(token);
                return Ok(external_ty(&signature.ret, self.enums, false));
            }
            return Err(self.error(format!(
                "no instance method '{name}' with {} parameter(s) on '{type_name}'",
                args.len()
            )));
        }
        if !self.is_static {
            let current = self.current.clone();
            if let Some(method) = self.method_symbol(&current, name, args.len(), false) {
                self.emitter.ldarg(0);
                self.push_arguments(args, &method.params, "argument")?;
                self.emitter.callvirt(method.entry);
                return Ok(method.ret);
            }
        }
        let current = self.current.clone();
        self.static_call(&current, name, generic_args, args)
    }

    /// The receiver is already on the evaluation stack and is evaluated once.
    /// Value types are stored in a temporary and passed by address, avoiding boxing.
    fn enum_call(
        &mut self,
        receiver: &Ty,
        name: &str,
        generic_args: &[Ty],
        args: &[Expr],
    ) -> Result<Ty, CompileError> {
        if !generic_args.is_empty() {
            return Err(self.error(format!("unsupported enum method '{name}'")));
        }
        if name == "HasFlag" && args.len() == 1 {
            return self.enum_has_flag(receiver, &args[0]);
        }
        if matches!(name, "Equals" | "CompareTo") && args.len() == 1 {
            let token = self
                .enum_type_token(receiver)
                .map_err(|message| self.error(message))?;
            self.emitter
                .emit(Instruction::new(OpCode::Box, Operand::Token(token)));
            let argument = self.expr(&args[0])?;
            self.convert_assignable(&argument, &Ty::Object, "argument")
                .map_err(|message| self.error(message))?;
            let (ty, signature) = if name == "Equals" {
                (Ty::Bool, TypeSig::BOOL)
            } else {
                (Ty::Int, TypeSig::INT32)
            };
            let method = self.import_bcl_method(
                "System.Runtime",
                "System.Enum",
                name,
                MethodSig::new_instance(signature, vec![TypeSig::OBJECT]),
            )?;
            self.emitter.callvirt(method);
            return Ok(ty);
        }
        if name == "ToString" && args.len() == 1 {
            let token = self
                .enum_type_token(receiver)
                .map_err(|message| self.error(message))?;
            self.emitter
                .emit(Instruction::new(OpCode::Box, Operand::Token(token)));
            let format = self.expr(&args[0])?;
            if format == Ty::Null {
                return Err(self.error("ToString(null) is ambiguous between format and provider"));
            }
            self.convert_assignable(&format, &Ty::String, "argument")
                .map_err(|message| self.error(message))?;
            let method = self.import_bcl_method(
                "System.Runtime",
                "System.Enum",
                name,
                MethodSig::new_instance(TypeSig::STRING, vec![TypeSig::STRING]),
            )?;
            self.emitter.callvirt(method);
            return Ok(Ty::String);
        }
        if !args.is_empty() {
            return Err(self.error(format!("unsupported enum method '{name}'")));
        }
        let (ty, signature) = match name {
            "ToString" => (Ty::String, TypeSig::STRING),
            "GetHashCode" => (Ty::Int, TypeSig::INT32),
            _ => return Err(self.error(format!("unsupported enum method '{name}'"))),
        };
        let local = resolve_type(receiver, self.types, self.externals, &mut *self.module)
            .map_err(|message| self.error(message))?;
        let temporary = self.emitter.declare_local(local);
        self.emitter.stloc(temporary).ldloca(temporary);
        let enum_token = self
            .enum_type_token(receiver)
            .map_err(|message| self.error(message))?;
        self.emitter.emit(Instruction::new(
            OpCode::Constrained,
            Operand::Token(enum_token),
        ));
        let method = self.import_bcl_method(
            "System.Runtime",
            "System.Object",
            name,
            MethodSig::new_instance(signature, vec![]),
        )?;
        self.emitter.callvirt(method);
        Ok(ty)
    }

    fn enum_has_flag(&mut self, receiver: &Ty, argument: &Expr) -> Result<Ty, CompileError> {
        let argument_type = self.expr(argument)?;
        if argument_type == *receiver {
            let signature = resolve_type(receiver, self.types, self.externals, &mut *self.module)
                .map_err(|message| self.error(message))?;
            let temporary = self.emitter.declare_local(signature);
            self.emitter
                .dup()
                .stloc(temporary)
                .op(OpCode::And)
                .ldloc(temporary)
                .op(OpCode::Ceq);
            return Ok(Ty::Bool);
        }
        let enum_reference = self
            .module
            .corlib_type("System.Enum")
            .map_err(|error| self.error(error.to_string()))?;
        let argument_signature = match &argument_type {
            Ty::Enum(_, _) => resolve_type(
                &argument_type,
                self.types,
                self.externals,
                &mut *self.module,
            )
            .map_err(|message| self.error(message))?,
            Ty::Null => TypeSig::Class(enum_reference.clone()),
            Ty::Class(_) if self.enum_boxing_cost(&argument_type) == Some(1) => resolve_type(
                &argument_type,
                self.types,
                self.externals,
                &mut *self.module,
            )
            .map_err(|message| self.error(message))?,
            _ => return Err(self.error("HasFlag requires an enum argument")),
        };
        let temporary = self.emitter.declare_local(argument_signature);
        self.emitter.stloc(temporary);
        let receiver_token = self
            .enum_type_token(receiver)
            .map_err(|message| self.error(message))?;
        self.emitter.emit(Instruction::new(
            OpCode::Box,
            Operand::Token(receiver_token),
        ));
        self.emitter.ldloc(temporary);
        if matches!(argument_type, Ty::Enum(_, _)) {
            let argument_token = self
                .enum_type_token(&argument_type)
                .map_err(|message| self.error(message))?;
            self.emitter.emit(Instruction::new(
                OpCode::Box,
                Operand::Token(argument_token),
            ));
        }
        let method = self.import_bcl_method(
            "System.Runtime",
            "System.Enum",
            "HasFlag",
            MethodSig::new_instance(TypeSig::BOOL, vec![TypeSig::Class(enum_reference)]),
        )?;
        self.emitter.callvirt(method);
        Ok(Ty::Bool)
    }

    fn primitive_call(
        &mut self,
        receiver: &Ty,
        name: &str,
        generic_args: &[Ty],
        args: &[Expr],
    ) -> Result<Ty, CompileError> {
        if !generic_args.is_empty() {
            return Err(self.error("primitive methods do not accept type arguments"));
        }
        let (type_name, local_type) = match receiver {
            Ty::Int => ("System.Int32", Some(TypeSig::INT32)),
            Ty::Float => ("System.Single", Some(TypeSig::Primitive(PrimitiveType::R4))),
            Ty::Bool => ("System.Boolean", Some(TypeSig::BOOL)),
            Ty::String => ("System.String", None),
            Ty::Object => ("System.Object", None),
            _ => return Err(self.error("unsupported primitive receiver")),
        };
        let (ret, signature_ret, params) = match (name, args.len(), receiver) {
            ("ToString", 0, _) => (Ty::String, TypeSig::STRING, vec![]),
            ("ToString", 1, Ty::Int | Ty::Float) => {
                (Ty::String, TypeSig::STRING, vec![TypeSig::STRING])
            }
            ("GetHashCode", 0, _) => (Ty::Int, TypeSig::INT32, vec![]),
            _ => return Err(self.error(format!("unsupported method '{name}' for {receiver:?}"))),
        };
        if let Some(local_type) = local_type {
            let temporary = self.emitter.declare_local(local_type);
            self.emitter.stloc(temporary).ldloca(temporary);
        }
        if args.len() == 1 {
            let actual = self.expr(&args[0])?;
            if actual == Ty::Null {
                return Err(self.error("ToString(null) is ambiguous between format and provider"));
            }
            self.convert_assignable(&actual, &Ty::String, "argument")
                .map_err(|message| self.error(message))?;
        }
        let token = self.import_bcl_method(
            "System.Runtime",
            type_name,
            name,
            MethodSig::new_instance(signature_ret, params),
        )?;
        if matches!(receiver, Ty::String | Ty::Object) {
            self.emitter.callvirt(token);
        } else {
            self.emitter.call(token);
        }
        Ok(ret)
    }

    fn static_call(
        &mut self,
        type_name: &str,
        name: &str,
        generic_args: &[Ty],
        args: &[Expr],
    ) -> Result<Ty, CompileError> {
        let short = type_name
            .rsplit('.')
            .next()
            .unwrap_or(type_name)
            .to_string();
        if let Some(method) = self.method_symbol(&short, name, args.len(), true) {
            self.require_method_access(&method.entry, true, None)?;
            self.push_arguments(args, &method.params, "argument")?;
            self.emitter.call(method.entry);
            return Ok(method.ret);
        }
        if let Some((index, entry)) = self.externals.method(type_name, name, args.len()) {
            self.require_method_access(&entry, false, None)?;
            if !entry.get().flags.contains(MethodFlags::Static) {
                return Err(self.error(format!("'{type_name}.{name}' is an instance method")));
            }
            let signature = {
                let guard = entry.get();
                guard.signature.clone()
            };
            if !generic_args.is_empty() {
                return self.generic_call(index, &entry, &signature, generic_args, args);
            }
            let params: Vec<Ty> = signature
                .params
                .iter()
                .map(|sig| external_ty(sig, self.enums, false))
                .collect();
            self.push_arguments(args, &params, "argument")?;
            let token = {
                let module = &mut *self.module;
                let foreign = &self.externals.modules[index];
                module
                    .import_foreign_method(&entry, foreign)
                    .map_err(|error| self.error(error.to_string()))?
            };
            self.emitter.call(token);
            return Ok(external_ty(&signature.ret, self.enums, false));
        }
        Err(self.error(format!(
            "no static method '{name}' with {} parameter(s) on '{type_name}'",
            args.len()
        )))
    }

    /// Resolves a member of a generic *type instance*: the MemberRef is parented
    /// at a TypeSpec so the runtime binds the type arguments.
    fn generic_member(
        &mut self,
        type_name: &str,
        type_args: &[Ty],
        member: &str,
        arity: usize,
    ) -> Result<(MemberToken, Vec<Ty>, Ty), CompileError> {
        let Some((index, entry)) = self
            .externals
            .find_generic(type_name, type_args.len())
            .map(|(index, entry)| (*index, entry.clone()))
        else {
            return Err(self.error(format!("unknown generic type (member) '{type_name}'")));
        };
        let full_name = entry.get().full_name();
        let Some((_, method_entry)) = self.externals.method(&full_name, member, arity) else {
            return Err(self.error(format!(
                "no member '{member}' with {arity} parameter(s) on '{full_name}'"
            )));
        };
        self.require_method_access(&method_entry, false, Some(type_name))?;
        let signature = {
            let guard = method_entry.get();
            guard.signature.clone()
        };
        let mut instantiation = Vec::with_capacity(type_args.len());
        for argument in type_args {
            instantiation.push(
                resolve_type(argument, self.types, self.externals, &mut *self.module)
                    .map_err(|message| self.error(message))?,
            );
        }
        let open = {
            let module = &mut *self.module;
            let foreign = &self.externals.modules[index];
            module
                .import_foreign_type(&TypeDefOrRef::TypeDef(entry.clone()), foreign)
                .map_err(|error| self.error(error.to_string()))?
        };
        let spec = Entry::new(TypeSpec {
            signature: TypeSig::GenericInst {
                generic: Box::new(TypeSig::Class(open)),
                args: instantiation.clone(),
            },
            custom_attributes: Vec::new(),
        });
        self.module.type_specs.push(spec.clone());
        let imported = {
            let module = &mut *self.module;
            let foreign = &self.externals.modules[index];
            module
                .import_foreign_method_sig(&signature, foreign)
                .map_err(|error| self.error(error.to_string()))?
        };
        let token = {
            let module = &mut *self.module;
            module
                .import_method_on(&TypeDefOrRef::TypeSpec(spec), member, imported.clone())
                .map_err(|error| self.error(error.to_string()))?
        };
        let params: Vec<Ty> = imported
            .params
            .iter()
            .map(|param| substitute(param, &instantiation, self.enums))
            .collect();
        let ret = substitute(&imported.ret, &instantiation, self.enums);
        Ok((token, params, ret))
    }

    /// Emits `call` on a MethodSpec so a generic method can be instantiated.
    fn generic_call(
        &mut self,
        index: usize,
        entry: &Entry<Method>,
        signature: &MethodSig,
        generic_args: &[Ty],
        args: &[Expr],
    ) -> Result<Ty, CompileError> {
        if signature.generic_param_count as usize != generic_args.len() {
            return Err(self.error(format!(
                "method expects {} generic argument(s), found {}",
                signature.generic_param_count,
                generic_args.len()
            )));
        }
        let instantiation = generic_args
            .iter()
            .map(|ty| resolve_type(ty, self.types, self.externals, &mut *self.module))
            .collect::<Result<Vec<_>, String>>()
            .map_err(|message| self.error(message))?;
        let token = {
            let module = &mut *self.module;
            let foreign = &self.externals.modules[index];
            module
                .import_foreign_method(entry, foreign)
                .map_err(|error| self.error(error.to_string()))?
        };
        let method = match token {
            MemberToken::Method(method) => MethodDefOrRef::Method(method),
            MemberToken::MemberRef(reference) => MethodDefOrRef::MemberRef(reference),
            _ => return Err(self.error("generic call target is not a method")),
        };
        let spec = Entry::new(MethodSpec {
            method,
            instantiation: instantiation.clone(),
            custom_attributes: Vec::new(),
        });
        self.module.method_specs.push(spec.clone());
        let params: Vec<Ty> = signature
            .params
            .iter()
            .map(|param| substitute(param, &instantiation, self.enums))
            .collect();
        self.push_arguments(args, &params, "argument")?;
        self.emitter.call(MemberToken::MethodSpec(spec));
        Ok(substitute(&signature.ret, &instantiation, self.enums))
    }

    fn bcl_call(
        &mut self,
        receiver: &str,
        name: &str,
        args: &[Expr],
    ) -> Result<Option<Ty>, CompileError> {
        let trimmed = receiver.trim_start_matches("System.");
        let qualified = trimmed.rsplit('.').next().unwrap_or(trimmed);
        let result = match (qualified, name) {
            ("Console", "WriteLine") => {
                let ty = self.single_argument(args, name)?;
                let parameter = match ty {
                    Ty::Int => TypeSig::INT32,
                    Ty::Object => TypeSig::OBJECT,
                    Ty::Float => TypeSig::Primitive(PrimitiveType::R4),
                    Ty::Bool => TypeSig::BOOL,
                    Ty::String => TypeSig::STRING,
                    _ => return Err(self.error("Console.WriteLine has no supported overload")),
                };
                let entry = self.import_bcl_method(
                    "System.Console",
                    "System.Console",
                    "WriteLine",
                    MethodSig::new_static(TypeSig::VOID, vec![parameter]),
                )?;
                self.emitter.call(entry);
                Ty::Void
            }
            ("Console", "Write") => {
                let ty = self.single_argument(args, name)?;
                let parameter = match ty {
                    Ty::Int => TypeSig::INT32,
                    Ty::Object => TypeSig::OBJECT,
                    Ty::Float => TypeSig::Primitive(PrimitiveType::R4),
                    Ty::String => TypeSig::STRING,
                    _ => return Err(self.error("Console.Write has no supported overload")),
                };
                let entry = self.import_bcl_method(
                    "System.Console",
                    "System.Console",
                    "Write",
                    MethodSig::new_static(TypeSig::VOID, vec![parameter]),
                )?;
                self.emitter.call(entry);
                Ty::Void
            }
            ("Math", "Max") | ("Math", "Min") => {
                self.expect_arity(args, 2, name)?;
                self.push_arguments(args, &[Ty::Int, Ty::Int], name)?;
                let entry = self.import_bcl_method(
                    "System.Runtime",
                    "System.Math",
                    name,
                    MethodSig::new_static(TypeSig::INT32, vec![TypeSig::INT32, TypeSig::INT32]),
                )?;
                self.emitter.call(entry);
                Ty::Int
            }
            ("Math", "Abs") => {
                self.expect_arity(args, 1, name)?;
                self.push_arguments(args, &[Ty::Int], name)?;
                let entry = self.import_bcl_method(
                    "System.Runtime",
                    "System.Math",
                    name,
                    MethodSig::new_static(TypeSig::INT32, vec![TypeSig::INT32]),
                )?;
                self.emitter.call(entry);
                Ty::Int
            }
            ("Convert", "ToString") => {
                self.expect_arity(args, 1, name)?;
                self.push_arguments(args, &[Ty::Int], name)?;
                let entry = self.import_bcl_method(
                    "System.Runtime",
                    "System.Convert",
                    "ToString",
                    MethodSig::new_static(TypeSig::STRING, vec![TypeSig::INT32]),
                )?;
                self.emitter.call(entry);
                Ty::String
            }
            _ => return Ok(None),
        };
        Ok(Some(result))
    }

    fn import_bcl_method(
        &mut self,
        assembly: &str,
        type_name: &str,
        method: &str,
        signature: MethodSig,
    ) -> Result<MemberToken, CompileError> {
        if assembly.eq_ignore_ascii_case("System.Console") {
            static TOKEN: [u8; 8] = [0xb0, 0x3f, 0x5f, 0x7f, 0x11, 0xd5, 0x0a, 0x3a];
            self.module.add_assembly_ref(
                "System.Console",
                Version {
                    major: 10,
                    minor: 0,
                    build: 0,
                    revision: 0,
                },
                &TOKEN,
            );
            let ty = self
                .module
                .import_type_from("System.Console", type_name)
                .map_err(|error| self.error(error.to_string()))?;
            return self
                .module
                .import_method_on(&ty, method, signature)
                .map_err(|error| self.error(error.to_string()));
        }
        self.module
            .import_corlib_method(type_name, method, signature)
            .map_err(|error| self.error(error.to_string()))
    }

    fn single_argument(&mut self, args: &[Expr], name: &str) -> Result<Ty, CompileError> {
        self.expect_arity(args, 1, name)?;
        self.expr(&args[0])
    }

    fn expect_arity(&self, args: &[Expr], expected: usize, name: &str) -> Result<(), CompileError> {
        if args.len() == expected {
            Ok(())
        } else {
            Err(self.error(format!(
                "'{name}' expects {expected} argument(s), found {}",
                args.len()
            )))
        }
    }

    fn field_store(&mut self, field: &FieldDecl, value: &Expr) -> Result<(), CompileError> {
        let current = self.current.clone();
        let Some(symbol) = self.field_symbol(&current, &field.name) else {
            return Err(self.error(format!("unknown field '{}'", field.name)));
        };
        if symbol.is_static {
            let actual = self.expr(value)?;
            self.convert_expression(&actual, &symbol.ty, value, "field initializer")
                .map_err(|message| self.error(message))?;
            self.emitter.emit(Instruction::new(
                OpCode::Stsfld,
                Operand::Token(MemberToken::Field(symbol.entry)),
            ));
        } else {
            self.emitter.ldarg(0);
            let actual = self.expr(value)?;
            self.convert_expression(&actual, &symbol.ty, value, "field initializer")
                .map_err(|message| self.error(message))?;
            self.emitter.emit(Instruction::new(
                OpCode::Stfld,
                Operand::Token(MemberToken::Field(symbol.entry)),
            ));
        }
        Ok(())
    }
}
