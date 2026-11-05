//! A real C# to CIL pipeline for a deliberately bounded language core.
//!
//! Unlike [`crate::syntax`], which only validates syntax, this module builds an
//! abstract syntax tree, resolves names against a two-pass symbol table and
//! emits executable IL. Anything outside the supported core is rejected with a
//! located diagnostic instead of being approximated.
use std::collections::HashMap;

use hao::{
    Module,
    dotnet::entries::Version,
    dotnet::{
        entries::{Entry, Field, Method, Param, TypeDef, TypeDefOrRef},
        il::{EmitLabel, Emitter, Instruction, MemberToken, MethodBody, OpCode, Operand},
        md::streams::tables_stream::{FieldFlags, MethodFlags, TypeAttributes},
        module::{ModuleKind, TargetRuntime},
        signature::PrimitiveType,
        signature::{FieldSig, MethodSig, TypeSig},
    },
};

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
    Int,
    Bool,
    String,
    Class(String),
    Array(Box<Ty>),
    Named(String),
}

#[derive(Debug)]
struct Program {
    types: Vec<TypeDecl>,
}

#[derive(Debug)]
struct TypeDecl {
    file: String,
    offset: usize,
    namespace: String,
    name: String,
    fields: Vec<FieldDecl>,
    methods: Vec<MethodDecl>,
    properties: Vec<PropertyDecl>,
}

#[derive(Debug)]
struct FieldDecl {
    name: String,
    ty: Ty,
    is_static: bool,
    init: Option<Expr>,
}

#[derive(Debug)]
struct PropertyDecl {
    name: String,
    ty: Ty,
    is_static: bool,
    has_getter: bool,
    has_setter: bool,
    getter: Option<Block>,
    setter: Option<Block>,
}

#[derive(Debug)]
struct MethodDecl {
    name: String,
    is_static: bool,
    ret: Ty,
    params: Vec<(Ty, String)>,
    body: Option<Block>,
    is_ctor: bool,
}

#[derive(Debug)]
struct Block {
    statements: Vec<Stmt>,
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
    If {
        condition: Expr,
        then_branch: Block,
        else_branch: Option<Block>,
    },
    While {
        condition: Expr,
        body: Block,
    },
    For {
        init: Option<Box<Stmt>>,
        condition: Option<Expr>,
        step: Option<Expr>,
        body: Block,
    },
    Empty,
    Block(Block),
    Break,
    Continue,
    AssignMember {
        target: Expr,
        name: String,
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
    And,
    Or,
}

#[derive(Debug)]
enum Expr {
    Int(i32),
    Bool(bool),
    Str(String),
    Name(String),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    Negate(Box<Expr>),
    Not(Box<Expr>),
    Call {
        receiver: Option<Box<Expr>>,
        name: String,
        args: Vec<Expr>,
    },
    Member {
        target: Box<Expr>,
        name: String,
    },
    New {
        type_name: String,
        args: Vec<Expr>,
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
}

pub fn compile(
    sources: &[(String, String)],
    assembly_name: &str,
    references: &[String],
) -> Result<Vec<u8>, CompileError> {
    let mut program = Program { types: Vec::new() };
    for (file, text) in sources {
        let parsed = parse_file(file, text)?;
        program.types.extend(parsed);
    }
    emit(&program, assembly_name, sources, references)
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
}

impl Externals {
    fn load(paths: &[String], sources: &[(String, String)]) -> Result<Self, CompileError> {
        let mut modules = Vec::new();
        let mut types = HashMap::new();
        let mut by_short_name: HashMap<String, Vec<String>> = HashMap::new();
        let mut methods: HashMap<String, Vec<(usize, Entry<Method>)>> = HashMap::new();
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
        })
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
}

// ---------------------------------------------------------------- parsing

struct Parser<'a> {
    file: &'a str,
    source: &'a str,
    tokens: Vec<syntax::Token>,
    position: usize,
}

fn parse_file(file: &str, source: &str) -> Result<Vec<TypeDecl>, CompileError> {
    let tokens = syntax::lex(source).map_err(|error: SyntaxError| {
        CompileError::new(file, source, error.offset, error.message)
    })?;
    let mut parser = Parser {
        file,
        source,
        tokens,
        position: 0,
    };
    parser.parse_unit()
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
        while !self.at_end() {
            self.skip_attribute_lists()?;
            if self.eat(";") {
                continue;
            }
            let text = self.text().to_string();
            match text.as_str() {
                "using" => {
                    while !self.at_end() && !self.check(";") {
                        self.bump();
                    }
                    self.expect(";")?;
                }
                "namespace" => {
                    self.bump();
                    namespace = self.parse_qualified_name()?;
                    if self.eat(";") {
                        continue;
                    }
                    self.expect("{")?;
                }
                "public" | "internal" | "private" | "protected" | "static" | "sealed"
                | "abstract" | "partial" | "unsafe" | "readonly" => {
                    self.bump();
                }
                "class" | "struct" | "interface" | "record" => {
                    if self.check("interface") {
                        return Err(self.error("interfaces are not supported yet"));
                    }
                    types.push(self.parse_type(&namespace)?);
                    if self.check("}") && !namespace.is_empty() {
                        self.bump();
                        namespace.clear();
                    }
                }
                "}" => {
                    if namespace.is_empty() {
                        return Err(self.error("unexpected '}'"));
                    }
                    self.bump();
                    namespace.clear();
                }
                _ => return Err(self.error(format!("unexpected top-level token '{text}'"))),
            }
        }
        Ok(types)
    }

    fn parse_new_type(&mut self) -> Result<Ty, CompileError> {
        let name = self.parse_qualified_name()?;
        Ok(match name.as_str() {
            "int" => Ty::Int,
            "bool" => Ty::Bool,
            "string" | "String" => Ty::String,
            other => Ty::Class(other.to_string()),
        })
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

    fn parse_type(&mut self, namespace: &str) -> Result<TypeDecl, CompileError> {
        let class_offset = self.token().start;
        self.bump(); // class / struct / record
        if self.check("record") {
            self.bump();
        }
        let name = self.identifier()?;
        if self.eat(":") {
            // Base list: accept but ignore; Object is the implicit base.
            loop {
                self.parse_qualified_name()?;
                if self.check("<") {
                    self.skip_balanced("<", ">")?;
                }
                if !self.eat(",") {
                    break;
                }
            }
        }
        let mut decl = TypeDecl {
            file: self.file.to_string(),
            offset: class_offset,
            namespace: namespace.to_string(),
            name,
            fields: Vec::new(),
            methods: Vec::new(),
            properties: Vec::new(),
        };
        self.expect("{")?;
        while !self.check("}") && !self.at_end() {
            self.parse_member(&mut decl)?;
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
        let mut is_public = false;
        loop {
            match self.text() {
                "public" => {
                    is_public = true;
                    self.bump();
                }
                "private" | "internal" | "protected" => {
                    self.bump();
                }
                "static" => {
                    is_static = true;
                    self.bump();
                }
                "readonly" | "const" | "sealed" | "override" | "virtual" | "unsafe" | "extern"
                | "async" | "partial" => {
                    self.bump();
                }
                _ => break,
            }
        }
        let _ = is_public;
        if self.check("}") || self.at_end() {
            return Ok(());
        }

        // Constructor: `Name(...)`.
        if self.token().kind == syntax::TokenKind::Identifier && self.peek(1) == Some("(") {
            let name = self.identifier()?;
            let params = self.parse_parameter_list()?;
            let body = self.parse_block()?;
            decl.methods.push(MethodDecl {
                name,
                is_static: false,
                ret: Ty::Void,
                params,
                body: Some(body),
                is_ctor: true,
            });
            return Ok(());
        }

        let ty = self.parse_type_reference()?;
        let name = self.identifier()?;
        if self.check("{") {
            // Property with accessors: `int Value { get; set; }`.
            self.bump();
            let mut getter = None;
            let mut setter = None;
            let mut has_getter = false;
            let mut has_setter = false;
            while !self.check("}") && !self.at_end() {
                self.skip_attribute_lists()?;
                while matches!(
                    self.text(),
                    "public" | "private" | "internal" | "protected" | "static" | "readonly"
                ) {
                    self.bump();
                }
                let accessor = self.identifier_like()?;
                let body = if self.eat(";") {
                    None
                } else {
                    Some(self.parse_block()?)
                };
                match accessor.as_str() {
                    "get" => {
                        has_getter = true;
                        getter = body;
                    }
                    "set" => {
                        has_setter = true;
                        setter = body;
                    }
                    other => {
                        return Err(self.error(format!("unsupported accessor '{other}'")));
                    }
                }
            }
            self.expect("}")?;
            decl.properties.push(PropertyDecl {
                name,
                ty,
                is_static,
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
            } else {
                self.expect(";")?;
                None
            };
            decl.methods.push(MethodDecl {
                name,
                is_static,
                ret: ty,
                params,
                body,
                is_ctor: false,
            });
            return Ok(());
        }

        // Field declarators, with optional initializers.
        let mut current_name = name;
        loop {
            let init = if self.eat("=") {
                Some(self.parse_expression()?)
            } else {
                None
            };
            decl.fields.push(FieldDecl {
                name: current_name.clone(),
                ty: ty.clone(),
                is_static,
                init,
            });
            if !self.eat(",") {
                break;
            }
            current_name = self.identifier()?;
        }
        self.expect(";")
    }

    fn parse_type_reference(&mut self) -> Result<Ty, CompileError> {
        let name = self.parse_qualified_name()?;
        let mut ty = match name.as_str() {
            "void" => Ty::Void,
            "int" => Ty::Int,
            "bool" => Ty::Bool,
            "string" | "String" => Ty::String,
            other => Ty::Class(other.to_string()),
        };
        // Generic/array suffixes are accepted for signature compatibility but
        // the bounded core only supports the scalar types above.
        if self.check("<") {
            self.skip_balanced("<", ">")?;
            ty = Ty::Named(format!("{name}[]"));
        }
        while self.eat("[") {
            self.expect("]")?;
            ty = Ty::Array(Box::new(ty.clone()));
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
            "throw" | "try" | "switch" | "do" | "case" | "default" | "goto" | "lock" | "using"
            | "yield" | "checked" | "unchecked" | "fixed" | "unsafe" => {
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
        // Assignment: `name = expr;`
        if self.token().kind == syntax::TokenKind::Identifier && self.peek(1) == Some("=") {
            let name = self.identifier()?;
            self.expect("=")?;
            let value = self.parse_expression()?;
            self.expect(";")?;
            return Ok(Stmt::Assign { name, value });
        }
        let expression = self.parse_expression()?;
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
        let mut left = self.parse_equality()?;
        while self.check("&&") {
            self.bump();
            let right = self.parse_equality()?;
            left = Expr::Binary(BinaryOp::And, Box::new(left), Box::new(right));
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
        if (self.check("++") || self.check("--"))
            && self
                .tokens
                .get(self.position + 1)
                .is_some_and(|token| token.kind == syntax::TokenKind::Identifier)
        {
            let delta = if self.check("++") { 1 } else { -1 };
            self.bump();
            let name = self.identifier()?;
            return Ok(Expr::Increment {
                name,
                delta,
                prefix: true,
            });
        }
        match self.text() {
            "-" => {
                self.bump();
                Ok(Expr::Negate(Box::new(self.parse_unary()?)))
            }
            "!" => {
                self.bump();
                Ok(Expr::Not(Box::new(self.parse_unary()?)))
            }
            "(" => {
                self.bump();
                let inner = self.parse_expression()?;
                self.expect(")")?;
                Ok(inner)
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, CompileError> {
        let mut expression = self.parse_primary()?;
        if let Expr::Name(name) = &expression
            && (self.check("++") || self.check("--"))
        {
            let delta = if self.check("++") { 1 } else { -1 };
            let name = name.clone();
            self.bump();
            return Ok(Expr::Increment {
                name,
                delta,
                prefix: false,
            });
        }
        loop {
            if self.check(".") {
                self.bump();
                let name = self.identifier()?;
                if self.check("(") {
                    let args = self.parse_arguments()?;
                    expression = Expr::Call {
                        receiver: Some(Box::new(expression)),
                        name,
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
            if self.check("(") {
                let args = self.parse_arguments()?;
                match expression {
                    Expr::Name(name) => {
                        expression = Expr::Call {
                            receiver: None,
                            name,
                            args,
                        };
                    }
                    _ => return Err(self.error("only a named method can be called")),
                }
                continue;
            }
            return Ok(expression);
        }
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

    fn parse_primary(&mut self) -> Result<Expr, CompileError> {
        if self.check("new") {
            self.bump();
            let element = self.parse_new_type()?;
            if self.eat("[") {
                let size = self.parse_expression()?;
                self.expect("]")?;
                return Ok(Expr::NewArray {
                    element,
                    size: Box::new(size),
                });
            }
            let Ty::Class(type_name) = element else {
                return Err(self.error("only classes can be constructed with 'new Type(...)'"));
            };
            let args = self.parse_arguments()?;
            return Ok(Expr::New { type_name, args });
        }
        if self.check("this") {
            self.bump();
            return Ok(Expr::Name("this".to_string()));
        }
        let token = self.token();
        match token.kind {
            syntax::TokenKind::Number => {
                let text = self.text().to_string();
                self.bump();
                let cleaned = text.replace('_', "");
                let value = cleaned
                    .trim_end_matches(['u', 'U', 'l', 'L'])
                    .parse::<i32>()
                    .map_err(|_| {
                        self.error(format!("integer literal '{text}' is not supported"))
                    })?;
                Ok(Expr::Int(value))
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
                "true" => {
                    self.bump();
                    Ok(Expr::Bool(true))
                }
                "false" => {
                    self.bump();
                    Ok(Expr::Bool(false))
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
    let body = raw
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| CompileError {
            file: String::new(),
            line: 1,
            column: 1,
            message: "unsupported string literal".into(),
            syntax: true,
        })?;
    let mut value = String::with_capacity(body.len());
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            value.push(character);
            continue;
        }
        let Some(escaped) = characters.next() else {
            return Err(CompileError {
                file: String::new(),
                line: 1,
                column: 1,
                message: "unterminated escape sequence".into(),
                syntax: true,
            });
        };
        value.push(match escaped {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            '0' => '\0',
            other => other,
        });
    }
    Ok(value)
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

struct PropertySymbol {
    ty: Ty,
    is_static: bool,
    getter: Option<Entry<Method>>,
    setter: Option<Entry<Method>>,
}

#[derive(Default)]
struct TypeSymbol {
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

fn is_reference(ty: &Ty) -> bool {
    matches!(ty, Ty::String | Ty::Class(_) | Ty::Array(_))
}

fn external_ty(sig: &TypeSig) -> Ty {
    match sig {
        TypeSig::Primitive(PrimitiveType::I4) => Ty::Int,
        TypeSig::Primitive(PrimitiveType::Boolean) => Ty::Bool,
        TypeSig::Primitive(PrimitiveType::String) => Ty::String,
        TypeSig::Primitive(PrimitiveType::Void) => Ty::Void,
        TypeSig::Class(TypeDefOrRef::TypeDef(entry)) => Ty::Class(entry.get().full_name()),
        TypeSig::Class(TypeDefOrRef::TypeRef(entry)) => Ty::Class(entry.get().full_name()),
        TypeSig::SzArray(inner) => Ty::Array(Box::new(external_ty(inner))),
        other => Ty::Named(format!("{other:?}")),
    }
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
        Ty::Bool => TypeSig::BOOL,
        Ty::String => TypeSig::STRING,
        Ty::Class(name) => match types.get(name) {
            Some(entry) => TypeSig::Class(TypeDefOrRef::TypeDef(entry.clone())),
            None => match externals.find(name) {
                Some((index, entry)) => {
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
        Ty::Named(name) => return Err(format!("type '{name}' is not supported yet")),
    })
}

fn emit(
    program: &Program,
    assembly_name: &str,
    sources: &[(String, String)],
    references: &[String],
) -> Result<Vec<u8>, CompileError> {
    let externals = Externals::load(references, sources)?;
    if program.types.is_empty() {
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
        .map_err(|error| type_error(&program.types[0], sources, error.to_string()))?;

    // Pass 0: declare every type so fields and signatures can reference types
    // that are declared later in the compilation.
    let mut types: HashMap<String, Entry<TypeDef>> = HashMap::new();
    for decl in &program.types {
        if types.contains_key(&decl.name) {
            return Err(type_error(
                decl,
                sources,
                format!("duplicate type '{}'", decl.name),
            ));
        }
        let entry = module.add_type(TypeDef::new(
            TypeAttributes::Public | TypeAttributes::BeforeFieldInit,
            &decl.namespace,
            &decl.name,
            Some(object.clone()),
        ));
        types.insert(decl.name.clone(), entry);
    }

    // Pass 1: fields, method shells and constructors.
    let mut symbols: HashMap<String, TypeSymbol> = HashMap::new();
    let mut jobs: Vec<(usize, Entry<Method>, Job)> = Vec::new();
    for (index, decl) in program.types.iter().enumerate() {
        let ty = types[&decl.name].clone();
        let mut symbol = TypeSymbol::default();

        for field in &decl.fields {
            let signature = resolve_type(&field.ty, &types, &externals, &mut module)
                .map_err(|message| type_error(decl, sources, message))?;
            let flags = if field.is_static {
                FieldFlags::Public | FieldFlags::Static
            } else {
                FieldFlags::Public
            };
            let entry = TypeDef::add_field(
                &ty,
                Field::new(flags, &field.name, FieldSig { ty: signature }),
            );
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
                .map(|(ty, _)| resolve_type(ty, &types, &externals, &mut module))
                .collect::<Result<Vec<_>, String>>()
                .map_err(|message| type_error(decl, sources, message))?;
            let return_signature = resolve_type(&method.ret, &types, &externals, &mut module)
                .map_err(|message| type_error(decl, sources, message))?;
            let (flags, name, signature) = if method.is_ctor {
                (
                    MethodFlags::Public
                        | MethodFlags::HideBySig
                        | MethodFlags::SpecialName
                        | MethodFlags::RTSpecialName,
                    ".ctor".to_string(),
                    MethodSig::new_instance(TypeSig::VOID, params),
                )
            } else if method.is_static {
                (
                    MethodFlags::Public | MethodFlags::Static | MethodFlags::HideBySig,
                    method.name.clone(),
                    MethodSig::new_static(return_signature, params),
                )
            } else {
                (
                    MethodFlags::Public | MethodFlags::HideBySig,
                    method.name.clone(),
                    MethodSig::new_instance(return_signature, params),
                )
            };
            let entry = TypeDef::add_method(
                &ty,
                Method::new(flags, name.clone(), signature).with_body(Emitter::new().finish()),
            );
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
            jobs.push((index, entry, Job::Declared(method_index)));
        }

        for (property_index, property) in decl.properties.iter().enumerate() {
            let signature = resolve_type(&property.ty, &types, &externals, &mut module)
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
            let extra = if property.is_static {
                MethodFlags::Public
                    | MethodFlags::Static
                    | MethodFlags::HideBySig
                    | MethodFlags::SpecialName
            } else {
                MethodFlags::Public | MethodFlags::HideBySig | MethodFlags::SpecialName
            };
            if property.has_getter {
                let signature = if property.is_static {
                    MethodSig::new_static(signature.clone(), vec![])
                } else {
                    MethodSig::new_instance(signature.clone(), vec![])
                };
                let entry = TypeDef::add_method(
                    &ty,
                    Method::new(extra, format!("get_{}", property.name), signature)
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
                    Method::new(extra, format!("set_{}", property.name), signature)
                        .with_body(Emitter::new().finish()),
                );
                entry.get_mut().params = vec![Entry::new(Param::new(1, "value"))];
                property_symbol.setter = Some(entry.clone());
                jobs.push((index, entry, Job::Setter(property_index)));
            }
            symbol
                .properties
                .insert(property.name.clone(), property_symbol);
        }

        if symbol.ctors.is_empty() {
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

    // Static constructors exist only when a static field has an initializer.
    for (index, decl) in program.types.iter().enumerate() {
        if !decl
            .fields
            .iter()
            .any(|field| field.is_static && field.init.is_some())
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
            &externals,
            &types,
            &symbols,
            &program.types[index],
            &job,
            sources,
        )?;
        entry.edit_body(|slot| *slot = body);
    }

    module.write().map_err(|error| {
        type_error(
            &program.types[0],
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

fn emit_body(
    module: &mut Module,
    externals: &Externals,
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
        is_static: true,
        return_ty: Ty::Void,
        externals,
        locals: HashMap::new(),
        params: Vec::new(),
        loops: Vec::new(),
        emitter: Emitter::new(),
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
                return Err(context.error(format!(
                    "method '{}' has no body; extern methods are not supported",
                    method.name
                )));
            };
            context.block(body)?;
            if method.is_ctor || method.ret == Ty::Void {
                context.emitter.ret();
            } else {
                let ends_with_return = body
                    .statements
                    .last()
                    .is_some_and(|statement| matches!(statement, Stmt::Return(_)));
                if !ends_with_return {
                    return Err(context.error(format!(
                        "method '{}' must end with a return statement",
                        method.name
                    )));
                }
            }
        }
        Job::DefaultCtor => {
            context.is_static = false;
            let object_ctor = context
                .module
                .import_corlib_method(
                    "System.Object",
                    ".ctor",
                    MethodSig::new_instance(TypeSig::VOID, vec![]),
                )
                .map_err(|error| context.error(error.to_string()))?;
            context.emitter.ldarg(0).call(object_ctor);
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
                    context.block(body)?;
                    let ends = body
                        .statements
                        .last()
                        .is_some_and(|statement| matches!(statement, Stmt::Return(_)));
                    if !ends {
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
            for field in &decl.fields {
                if !field.is_static {
                    continue;
                }
                if let Some(initializer) = &field.init {
                    context.field_store(field, initializer)?;
                }
            }
            context.emitter.ret();
        }
    }
    Ok(context.emitter.finish())
}

struct FnEmitter<'a> {
    module: &'a mut Module,
    types: &'a HashMap<String, Entry<TypeDef>>,
    symbols: &'a HashMap<String, TypeSymbol>,
    current: String,
    is_static: bool,
    return_ty: Ty,
    externals: &'a Externals,
    locals: HashMap<String, (u16, Ty)>,
    params: Vec<(String, Ty)>,
    loops: Vec<(EmitLabel, EmitLabel)>,
    emitter: Emitter,
    file: String,
    source: String,
    offset: usize,
}

impl<'a> FnEmitter<'a> {
    fn error(&self, message: impl Into<String>) -> CompileError {
        CompileError::new(&self.file, &self.source, self.offset, message)
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

    fn constructor(&self, type_name: &str, arity: usize) -> Option<MethodSymbol> {
        self.symbols.get(type_name).and_then(|symbol| {
            symbol
                .ctors
                .iter()
                .find(|ctor| ctor.params.len() == arity)
                .cloned()
        })
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

    fn statement(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        match statement {
            Stmt::Block(block) => self.block(block),
            Stmt::Empty => Ok(()),
            Stmt::Break => {
                let Some((end, _)) = self.loops.last().copied() else {
                    return Err(self.error("'break' outside of a loop"));
                };
                self.emitter.branch(OpCode::Br, end);
                Ok(())
            }
            Stmt::Continue => {
                let Some((_, next)) = self.loops.last().copied() else {
                    return Err(self.error("'continue' outside of a loop"));
                };
                self.emitter.branch(OpCode::Br, next);
                Ok(())
            }
            Stmt::Local { ty, name, init } => {
                let signature = resolve_type(ty, self.types, self.externals, &mut *self.module)
                    .map_err(|message| self.error(message))?;
                let index = self.emitter.declare_local(signature);
                self.locals.insert(name.clone(), (index, ty.clone()));
                if let Some(value) = init {
                    let actual = self.expr(value)?;
                    Self::require(&actual, ty, &format!("initializer of '{name}'"))
                        .map_err(|message| self.error(message))?;
                    self.emitter.stloc(index);
                }
                Ok(())
            }
            Stmt::Assign { name, value } => self.assign(name, value),
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
                Self::require(&value_type, &element, "array element")
                    .map_err(|message| self.error(message))?;
                self.emitter.op(if is_reference(&element) {
                    OpCode::Stelem_Ref
                } else {
                    OpCode::Stelem_I4
                });
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
                self.emitter.op(if is_reference(ty) {
                    OpCode::Ldelem_Ref
                } else {
                    OpCode::Ldelem_I4
                });
                self.emitter.stloc(element_local);
                self.loops.push((end, step));
                self.block(body)?;
                self.loops.pop();
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
            Stmt::AssignMember {
                target,
                name,
                value,
            } => {
                let target_type = self.expr(target)?;
                let Ty::Class(type_name) = target_type else {
                    return Err(self.error(format!("type {target_type:?} has no fields")));
                };
                if let Some(field) = self.field_symbol(&type_name, name) {
                    if field.is_static {
                        return Err(self.error(format!("'{type_name}.{name}' is static")));
                    }
                    let actual = self.expr(value)?;
                    Self::require(&actual, &field.ty, &format!("assignment to '{name}'"))
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
                    return Err(self.error(format!("unknown member '{type_name}.{name}'")));
                };
                let Some(setter) = setter else {
                    return Err(self.error(format!("property '{type_name}.{name}' has no setter")));
                };
                let actual = self.expr(value)?;
                Self::require(&actual, &property_ty, &format!("assignment to '{name}'"))
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
                match value {
                    Some(expression) => {
                        let actual = self.expr(expression)?;
                        Self::require(&actual, &self.return_ty, "return")
                            .map_err(|message| self.error(message))?;
                    }
                    None => {
                        if self.return_ty != Ty::Void {
                            return Err(self.error("a value must be returned"));
                        }
                    }
                }
                self.emitter.ret();
                Ok(())
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let actual = self.expr(condition)?;
                Self::require(&actual, &Ty::Bool, "if condition")
                    .map_err(|message| self.error(message))?;
                let alternative = self.emitter.label();
                let end = self.emitter.label();
                self.emitter.branch(OpCode::Brfalse, alternative);
                self.block(then_branch)?;
                self.emitter.branch(OpCode::Br, end);
                self.emitter.mark(alternative);
                if let Some(branch) = else_branch {
                    self.block(branch)?;
                }
                self.emitter.mark(end);
                Ok(())
            }
            Stmt::While { condition, body } => {
                let top = self.emitter.label();
                let end = self.emitter.label();
                self.emitter.mark(top);
                let actual = self.expr(condition)?;
                Self::require(&actual, &Ty::Bool, "while condition")
                    .map_err(|message| self.error(message))?;
                self.emitter.branch(OpCode::Brfalse, end);
                self.loops.push((end, top));
                self.block(body)?;
                self.loops.pop();
                self.emitter.branch(OpCode::Br, top);
                self.emitter.mark(end);
                Ok(())
            }
            Stmt::For {
                init,
                condition,
                step,
                body,
            } => {
                if let Some(init) = init {
                    self.statement(init)?;
                }
                let top = self.emitter.label();
                let step_label = self.emitter.label();
                let end = self.emitter.label();
                self.emitter.mark(top);
                if let Some(condition) = condition {
                    let actual = self.expr(condition)?;
                    Self::require(&actual, &Ty::Bool, "for condition")
                        .map_err(|message| self.error(message))?;
                    self.emitter.branch(OpCode::Brfalse, end);
                }
                self.loops.push((end, step_label));
                self.block(body)?;
                self.loops.pop();
                self.emitter.mark(step_label);
                if let Some(step) = step {
                    let ty = self.expr(step)?;
                    if ty != Ty::Void {
                        self.emitter.pop();
                    }
                }
                self.emitter.branch(OpCode::Br, top);
                self.emitter.mark(end);
                Ok(())
            }
        }
    }

    fn assign(&mut self, name: &str, value: &Expr) -> Result<(), CompileError> {
        if let Some((index, ty)) = self.locals.get(name).cloned() {
            let actual = self.expr(value)?;
            Self::require(&actual, &ty, &format!("assignment to '{name}'"))
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
            Self::require(&actual, &ty, &format!("assignment to '{name}'"))
                .map_err(|message| self.error(message))?;
            self.emitter.starg(self.argument_index(position));
            return Ok(());
        }
        let Some(field) = self.field_symbol(&self.current.clone(), name) else {
            return Err(self.error(format!("undefined identifier '{name}'")));
        };
        if field.is_static {
            let actual = self.expr(value)?;
            Self::require(&actual, &field.ty, &format!("assignment to '{name}'"))
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
        Self::require(&actual, &field.ty, &format!("assignment to '{name}'"))
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
            Self::require(&actual, &ty, &format!("assignment to '{name}'"))
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
            Self::require(&actual, &ty, &format!("assignment to '{name}'"))
                .map_err(|message| self.error(message))?;
            self.emitter.dup();
            self.emitter.starg(self.argument_index(position));
            return Ok(ty);
        }
        let Some(field) = self.field_symbol(&self.current.clone(), name) else {
            return Err(self.error(format!("undefined identifier '{name}'")));
        };
        if field.is_static {
            let actual = self.expr(value)?;
            Self::require(&actual, &field.ty, &format!("assignment to '{name}'"))
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
        Self::require(&actual, &field.ty, &format!("assignment to '{name}'"))
            .map_err(|message| self.error(message))?;
        self.emitter.dup();
        self.emitter.emit(Instruction::new(
            OpCode::Stfld,
            Operand::Token(MemberToken::Field(field.entry)),
        ));
        Ok(field.ty)
    }

    fn load(&mut self, name: &str) -> Result<Ty, CompileError> {
        if name == "this" {
            if self.is_static {
                return Err(self.error("'this' cannot be used from a static method"));
            }
            self.emitter.ldarg(0);
            return Ok(Ty::Class(self.current.clone()));
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
            return Err(self.error(format!("undefined identifier '{name}'")));
        };
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
            return Err(self.error(format!("undefined identifier '{name}'")));
        };
        if !field.is_static {
            return Err(self.error(format!(
                "incrementing instance field '{name}' is not supported yet"
            )));
        }
        self.emitter.emit(Instruction::new(
            OpCode::Stsfld,
            Operand::Token(MemberToken::Field(field.entry)),
        ));
        Ok(())
    }
}

impl<'a> FnEmitter<'a> {
    fn expr(&mut self, expression: &Expr) -> Result<Ty, CompileError> {
        match expression {
            Expr::Int(value) => {
                self.emitter.ldc_i4(*value);
                Ok(Ty::Int)
            }
            Expr::Bool(value) => {
                self.emitter.ldc_i4(i32::from(*value));
                Ok(Ty::Bool)
            }
            Expr::Str(value) => {
                self.emitter.ldstr(value.clone());
                Ok(Ty::String)
            }
            Expr::Name(name) => self.load(name),
            Expr::Negate(inner) => {
                let ty = self.expr(inner)?;
                Self::require(&ty, &Ty::Int, "unary '-'").map_err(|message| self.error(message))?;
                self.emitter.op(OpCode::Neg);
                Ok(Ty::Int)
            }
            Expr::Not(inner) => {
                let ty = self.expr(inner)?;
                Self::require(&ty, &Ty::Bool, "unary '!'")
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
                Self::require(&ty, &Ty::Int, "increment").map_err(|message| self.error(message))?;
                if *prefix {
                    self.emitter.ldc_i4(delta.abs());
                    self.emitter
                        .op(if *delta > 0 { OpCode::Add } else { OpCode::Sub });
                    self.emitter.dup();
                    self.store_top(name)?;
                } else {
                    self.emitter.dup();
                    self.emitter.ldc_i4(delta.abs());
                    self.emitter
                        .op(if *delta > 0 { OpCode::Add } else { OpCode::Sub });
                    self.store_top(name)?;
                }
                Ok(Ty::Int)
            }
            Expr::New { type_name, args } => {
                let Some(constructor) = self.constructor(type_name, args.len()) else {
                    return Err(self.error(format!(
                        "no constructor for '{type_name}' taking {} argument(s)",
                        args.len()
                    )));
                };
                self.push_arguments(args, &constructor.params, "argument")?;
                self.emitter.newobj(constructor.entry);
                Ok(Ty::Class(type_name.clone()))
            }
            Expr::Member { target, name } => self.member_read(target, name),
            Expr::Call {
                receiver,
                name,
                args,
            } => self.call(receiver.as_deref(), name, args),
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
                self.emitter.op(if is_reference(&element) {
                    OpCode::Ldelem_Ref
                } else {
                    OpCode::Ldelem_I4
                });
                Ok(*element)
            }
        }
    }

    fn element_token(&mut self, element: &Ty) -> Result<MemberToken, CompileError> {
        match element {
            Ty::Class(type_name) => {
                let Some(entry) = self.types.get(type_name) else {
                    return Err(self.error(format!("unknown type '{type_name}'")));
                };
                Ok(MemberToken::TypeDef(entry.clone()))
            }
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

    fn member_read(&mut self, target: &Expr, name: &str) -> Result<Ty, CompileError> {
        // `Type.StaticProperty` does not evaluate the type name as a value.
        if let Expr::Name(type_name) = target
            && self.types.contains_key(type_name)
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
                let ty = property.ty.clone();
                self.emitter.call(getter);
                return Ok(ty);
            }
            return Err(self.error(format!("unknown static member '{type_name}.{name}'")));
        }
        let target_type = self.expr(target)?;
        if let Ty::Array(_) = target_type {
            if name == "Length" {
                self.emitter.op(OpCode::Ldlen);
                self.emitter.op(OpCode::Conv_I4);
                return Ok(Ty::Int);
            }
            return Err(self.error(format!("arrays have no member '{name}'")));
        }
        let Ty::Class(type_name) = target_type else {
            return Err(self.error(format!("type {target_type:?} has no members")));
        };
        if let Some(field) = self.field_symbol(&type_name, name) {
            if field.is_static {
                return Err(self.error(format!("'{type_name}.{name}' is static")));
            }
            self.emitter.emit(Instruction::new(
                OpCode::Ldfld,
                Operand::Token(MemberToken::Field(field.entry)),
            ));
            return Ok(field.ty);
        }
        let property = self
            .symbols
            .get(&type_name)
            .and_then(|symbol| symbol.properties.get(name))
            .map(|property| (property.ty.clone(), property.getter.clone()));
        let Some((ty, getter)) = property else {
            return Err(self.error(format!("unknown member '{type_name}.{name}'")));
        };
        let Some(getter) = getter else {
            return Err(self.error(format!("property '{type_name}.{name}' has no getter")));
        };
        self.emitter.callvirt(getter);
        Ok(ty)
    }

    fn push_arguments(
        &mut self,
        args: &[Expr],
        params: &[Ty],
        what: &str,
    ) -> Result<(), CompileError> {
        for (index, argument) in args.iter().enumerate() {
            let actual = self.expr(argument)?;
            Self::require(&actual, &params[index], what).map_err(|message| self.error(message))?;
        }
        Ok(())
    }

    fn binary(&mut self, op: BinaryOp, left: &Expr, right: &Expr) -> Result<Ty, CompileError> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let end = self.emitter.label();
            let done = self.emitter.label();
            let ty = self.expr(left)?;
            Self::require(&ty, &Ty::Bool, "logical operand")
                .map_err(|message| self.error(message))?;
            let short_circuit = if matches!(op, BinaryOp::And) {
                OpCode::Brfalse
            } else {
                OpCode::Brtrue
            };
            self.emitter.branch(short_circuit, end);
            let ty = self.expr(right)?;
            Self::require(&ty, &Ty::Bool, "logical operand")
                .map_err(|message| self.error(message))?;
            self.emitter.branch(short_circuit, end);
            self.emitter.ldc_i4(1);
            self.emitter.branch(OpCode::Br, done);
            self.emitter.mark(end);
            self.emitter
                .ldc_i4(if matches!(op, BinaryOp::And) { 0 } else { 1 });
            self.emitter.mark(done);
            return Ok(Ty::Bool);
        }

        let left_ty = self.expr(left)?;
        let right_ty = self.expr(right)?;
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
        Self::require(&left_ty, &Ty::Int, "left operand").map_err(|message| self.error(message))?;
        Self::require(&right_ty, &Ty::Int, "right operand")
            .map_err(|message| self.error(message))?;
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
                self.emitter.op(OpCode::Cgt);
                self.emitter.ldc_i4(0);
                self.emitter.op(OpCode::Ceq);
            }
            BinaryOp::GreaterOrEqual => {
                self.emitter.op(OpCode::Clt);
                self.emitter.ldc_i4(0);
                self.emitter.op(OpCode::Ceq);
            }
            BinaryOp::And | BinaryOp::Or => unreachable!(),
        }
        Ok(
            if matches!(
                op,
                BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::Less
                    | BinaryOp::Greater
                    | BinaryOp::LessOrEqual
                    | BinaryOp::GreaterOrEqual
            ) {
                Ty::Bool
            } else {
                Ty::Int
            },
        )
    }

    fn call(
        &mut self,
        receiver: Option<&Expr>,
        name: &str,
        args: &[Expr],
    ) -> Result<Ty, CompileError> {
        if let Some(receiver) = receiver {
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
                    return self.static_call(receiver_name, name, args);
                }
            }
            let receiver_type = self.expr(receiver)?;
            let Ty::Class(type_name) = receiver_type else {
                return Err(self.error(format!("type {receiver_type:?} has no methods")));
            };
            if let Some(method) = self.method_symbol(&type_name, name, args.len(), false) {
                self.push_arguments(args, &method.params, "argument")?;
                self.emitter.callvirt(method.entry);
                return Ok(method.ret);
            }
            if let Some((index, entry)) = self.externals.method(&type_name, name, args.len()) {
                let signature = {
                    let guard = entry.get();
                    guard.signature.clone()
                };
                let params: Vec<Ty> = signature.params.iter().map(external_ty).collect();
                self.push_arguments(args, &params, "argument")?;
                let token = {
                    let module = &mut *self.module;
                    let foreign = &self.externals.modules[index];
                    module
                        .import_foreign_method(&entry, foreign)
                        .map_err(|error| self.error(error.to_string()))?
                };
                self.emitter.callvirt(token);
                return Ok(external_ty(&signature.ret));
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
        self.static_call(&current, name, args)
    }

    fn static_call(
        &mut self,
        type_name: &str,
        name: &str,
        args: &[Expr],
    ) -> Result<Ty, CompileError> {
        if let Some(method) = self.method_symbol(type_name, name, args.len(), true) {
            self.push_arguments(args, &method.params, "argument")?;
            self.emitter.call(method.entry);
            return Ok(method.ret);
        }
        if let Some((index, entry)) = self.externals.method(type_name, name, args.len()) {
            let signature = {
                let guard = entry.get();
                guard.signature.clone()
            };
            let params: Vec<Ty> = signature.params.iter().map(external_ty).collect();
            self.push_arguments(args, &params, "argument")?;
            let token = {
                let module = &mut *self.module;
                let foreign = &self.externals.modules[index];
                module
                    .import_foreign_method(&entry, foreign)
                    .map_err(|error| self.error(error.to_string()))?
            };
            self.emitter.call(token);
            return Ok(external_ty(&signature.ret));
        }
        Err(self.error(format!(
            "no static method '{name}' with {} parameter(s) on '{type_name}'",
            args.len()
        )))
    }

    fn bcl_call(
        &mut self,
        receiver: &str,
        name: &str,
        args: &[Expr],
    ) -> Result<Option<Ty>, CompileError> {
        let qualified = receiver.trim_start_matches("System.");
        let result = match (qualified, name) {
            ("Console", "WriteLine") => {
                let ty = self.single_argument(args, name)?;
                let parameter = match ty {
                    Ty::Int => TypeSig::INT32,
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
            Self::require(&actual, &symbol.ty, "field initializer")
                .map_err(|message| self.error(message))?;
            self.emitter.emit(Instruction::new(
                OpCode::Stsfld,
                Operand::Token(MemberToken::Field(symbol.entry)),
            ));
        } else {
            self.emitter.ldarg(0);
            let actual = self.expr(value)?;
            Self::require(&actual, &symbol.ty, "field initializer")
                .map_err(|message| self.error(message))?;
            self.emitter.emit(Instruction::new(
                OpCode::Stfld,
                Operand::Token(MemberToken::Field(symbol.entry)),
            ));
        }
        Ok(())
    }
}
