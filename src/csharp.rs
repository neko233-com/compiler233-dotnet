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
        entries::{Entry, Field, Method, Param, TypeDef},
        il::{EmitLabel, Emitter, Instruction, MemberToken, MethodBody, OpCode, Operand},
        md::streams::tables_stream::{FieldFlags, MethodFlags, TypeAttributes},
        module::{ModuleKind, TargetRuntime},
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
    Named(String),
}

impl Ty {
    fn signature(&self) -> TypeSig {
        match self {
            Ty::Void => TypeSig::VOID,
            Ty::Int => TypeSig::INT32,
            Ty::Bool => TypeSig::BOOL,
            Ty::String => TypeSig::STRING,
            Ty::Named(_) => TypeSig::OBJECT,
        }
    }
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
}

#[derive(Debug)]
struct FieldDecl {
    name: String,
    ty: Ty,
    is_static: bool,
    init: Option<Expr>,
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
        receiver: Option<String>,
        name: String,
        args: Vec<Expr>,
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

pub fn compile(sources: &[(String, String)], assembly_name: &str) -> Result<Vec<u8>, CompileError> {
    let mut program = Program { types: Vec::new() };
    for (file, text) in sources {
        let parsed = parse_file(file, text)?;
        program.types.extend(parsed);
    }
    emit(&program, assembly_name, sources)
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
            other => Ty::Named(other.to_string()),
        };
        // Generic/array suffixes are accepted for signature compatibility but
        // the bounded core only supports the scalar types above.
        if self.check("<") {
            self.skip_balanced("<", ">")?;
            ty = Ty::Named(name.clone());
        }
        while self.eat("[") {
            self.expect("]")?;
            ty = Ty::Named(format!("{name}[]"));
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
            "throw" | "try" | "foreach" | "switch" | "do" | "case" | "default" | "goto"
            | "lock" | "using" | "yield" | "checked" | "unchecked" | "fixed" | "unsafe" => {
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
            if self.check("(") {
                let args = self.parse_arguments()?;
                expression = match expression {
                    Expr::Name(name) => Expr::Call {
                        receiver: None,
                        name,
                        args,
                    },
                    Expr::Call {
                        receiver: None,
                        name,
                        args: existing,
                    } => Expr::Call {
                        receiver: None,
                        name: format!("{name}?"),
                        args: existing,
                    },
                    Expr::Call {
                        receiver: Some(_), ..
                    } => return Err(self.error("nested call targets are not supported")),
                    _ => return Err(self.error("expected a callable name before '('")),
                };
                continue;
            }
            if self.check(".") {
                let Expr::Name(receiver) = expression else {
                    return Err(self.error("member access requires a simple receiver"));
                };
                self.bump();
                let name = self.identifier()?;
                if self.check("(") {
                    let args = self.parse_arguments()?;
                    expression = Expr::Call {
                        receiver: Some(receiver),
                        name,
                        args,
                    };
                } else {
                    expression = Expr::Name(format!("{receiver}.{name}"));
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

struct FieldSymbol {
    ty: Ty,
    is_static: bool,
    entry: Entry<Field>,
}

struct MethodSymbol {
    name: String,
    is_static: bool,
    ret: Ty,
    params: Vec<Ty>,
    entry: Entry<Method>,
}

#[derive(Default)]
struct TypeSymbol {
    fields: HashMap<String, FieldSymbol>,
    methods: Vec<MethodSymbol>,
    ctor: Option<Entry<Method>>,
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

fn signature_of(ty: &Ty) -> TypeSig {
    ty.signature()
}

fn emit(
    program: &Program,
    assembly_name: &str,
    sources: &[(String, String)],
) -> Result<Vec<u8>, CompileError> {
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

    let mut symbols: HashMap<String, TypeSymbol> = HashMap::new();
    let mut type_entries: HashMap<String, Entry<TypeDef>> = HashMap::new();
    let mut jobs: Vec<(usize, Entry<Method>, Job)> = Vec::new();

    for (index, decl) in program.types.iter().enumerate() {
        if symbols.contains_key(&decl.name) {
            return Err(type_error(
                decl,
                sources,
                format!("duplicate type '{}'", decl.name),
            ));
        }
        let ty = module.add_type(TypeDef::new(
            TypeAttributes::Public | TypeAttributes::BeforeFieldInit,
            &decl.namespace,
            &decl.name,
            Some(object.clone()),
        ));
        type_entries.insert(decl.name.clone(), ty.clone());
        let mut symbol = TypeSymbol::default();

        for field in &decl.fields {
            if matches!(field.ty, Ty::Named(_)) {
                return Err(type_error(
                    decl,
                    sources,
                    format!("field '{}' has an unsupported type", field.name),
                ));
            }
            let flags = if field.is_static {
                FieldFlags::Public | FieldFlags::Static
            } else {
                FieldFlags::Public
            };
            let entry = TypeDef::add_field(
                &ty,
                Field::new(
                    flags,
                    &field.name,
                    FieldSig {
                        ty: signature_of(&field.ty),
                    },
                ),
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

        let mut has_ctor = false;
        for (method_index, method) in decl.methods.iter().enumerate() {
            if matches!(method.ret, Ty::Named(_))
                || method
                    .params
                    .iter()
                    .any(|(ty, _)| matches!(ty, Ty::Named(_)))
            {
                return Err(type_error(
                    decl,
                    sources,
                    format!(
                        "method '{}' uses a type the bounded core does not support",
                        method.name
                    ),
                ));
            }
            let params: Vec<TypeSig> = method
                .params
                .iter()
                .map(|(ty, _)| signature_of(ty))
                .collect();
            let (flags, name, sig) = if method.is_ctor {
                has_ctor = true;
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
                    MethodSig::new_static(signature_of(&method.ret), params),
                )
            } else {
                (
                    MethodFlags::Public | MethodFlags::HideBySig,
                    method.name.clone(),
                    MethodSig::new_instance(signature_of(&method.ret), params),
                )
            };
            let entry = TypeDef::add_method(
                &ty,
                Method::new(flags, name.clone(), sig).with_body(Emitter::new().finish()),
            );
            let mut param_entries = Vec::new();
            for (parameter_index, (_, parameter_name)) in method.params.iter().enumerate() {
                param_entries.push(Entry::new(Param::new(
                    parameter_index as u16 + 1,
                    parameter_name.clone(),
                )));
            }
            entry.get_mut().params = param_entries;
            entry.edit_body(|body| *body = Emitter::new().finish());
            if method.is_ctor {
                symbol.ctor = Some(entry.clone());
            } else {
                symbol.methods.push(MethodSymbol {
                    name: name.clone(),
                    is_static: method.is_static,
                    ret: method.ret.clone(),
                    params: method.params.iter().map(|(ty, _)| ty.clone()).collect(),
                    entry: entry.clone(),
                });
            }
            jobs.push((index, entry, Job::Declared(method_index)));
        }

        if !has_ctor {
            let flags = MethodFlags::Public
                | MethodFlags::HideBySig
                | MethodFlags::SpecialName
                | MethodFlags::RTSpecialName;
            let sig = MethodSig::new_instance(TypeSig::VOID, vec![]);
            let entry = TypeDef::add_method(
                &ty,
                Method::new(flags, ".ctor", sig).with_body(Emitter::new().finish()),
            );
            entry.edit_body(|body| *body = Emitter::new().finish());
            symbol.ctor = Some(entry.clone());
            jobs.push((index, entry, Job::DefaultCtor));
        }

        symbols.insert(decl.name.clone(), symbol);
    }

    let has_static_initializer = program.types.iter().any(|decl| {
        decl.fields
            .iter()
            .any(|field| field.is_static && field.init.is_some())
    });
    if has_static_initializer {
        for (index, decl) in program.types.iter().enumerate() {
            if !decl
                .fields
                .iter()
                .any(|field| field.is_static && field.init.is_some())
            {
                continue;
            }
            let Some(ty) = type_entries.get(&decl.name).cloned() else {
                continue;
            };
            {
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
                entry.edit_body(|body| *body = Emitter::new().finish());
                jobs.push((index, entry, Job::StaticCtor));
            }
        }
    }

    for (index, entry, job) in jobs {
        let decl = &program.types[index];
        let body = emit_body(&mut module, &symbols, decl, &job, sources)?;
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
}

fn emit_body(
    module: &mut Module,
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
        symbols,
        current: decl.name.clone(),
        is_static: true,
        return_ty: Ty::Void,
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
            match &method.body {
                Some(body) => context.block(body)?,
                None => {
                    return Err(context.error(format!(
                        "method '{}' has no body; extern methods are not supported",
                        method.name
                    )));
                }
            }
            if method.is_ctor || method.ret == Ty::Void {
                context.emitter.ret();
            } else {
                let ends_with_return = method
                    .body
                    .as_ref()
                    .is_some_and(|block| matches!(block.statements.last(), Some(Stmt::Return(_))));
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
    symbols: &'a HashMap<String, TypeSymbol>,
    current: String,
    is_static: bool,
    return_ty: Ty,
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
        let Some(field) = self.field_symbol(name) else {
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

    fn field_symbol(&self, name: &str) -> Option<FieldSymbol> {
        self.symbols
            .get(&self.current)
            .and_then(|symbol| symbol.fields.get(name))
            .map(|field| FieldSymbol {
                ty: field.ty.clone(),
                is_static: field.is_static,
                entry: field.entry.clone(),
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
                if matches!(ty, Ty::Named(_)) {
                    return Err(self.error(format!("local '{name}' has an unsupported type")));
                }
                let index = self.emitter.declare_local(signature_of(ty));
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
                let end = self.emitter.label();
                self.emitter.mark(top);
                if let Some(condition) = condition {
                    let actual = self.expr(condition)?;
                    Self::require(&actual, &Ty::Bool, "for condition")
                        .map_err(|message| self.error(message))?;
                    self.emitter.branch(OpCode::Brfalse, end);
                }
                let step_label = self.emitter.label();
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
        let Some(field) = self.field_symbol(name) else {
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

    fn load(&mut self, name: &str) -> Result<Ty, CompileError> {
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
        let Some(field) = self.field_symbol(name) else {
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
            Expr::Call {
                receiver,
                name,
                args,
            } => self.call(receiver.as_deref(), name, args),
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
        }
    }

    fn binary(&mut self, op: BinaryOp, left: &Expr, right: &Expr) -> Result<Ty, CompileError> {
        match op {
            BinaryOp::And | BinaryOp::Or => {
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
            _ => {}
        }

        let left_ty = self.expr(left)?;
        let right_ty = self.expr(right)?;
        if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual)
            && left_ty == Ty::String
            && right_ty == Ty::String
        {
            let method = self.import_bcl_method(
                "System.Runtime",
                "System.String",
                if matches!(op, BinaryOp::Equal) {
                    "op_Equality"
                } else {
                    "op_Inequality"
                },
                MethodSig::new_static(TypeSig::BOOL, vec![TypeSig::STRING, TypeSig::STRING]),
            )?;
            self.emitter.call(method);
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
            let concat = self
                .module
                .import_corlib_method(
                    "System.String",
                    "Concat",
                    MethodSig::new_static(TypeSig::STRING, vec![TypeSig::STRING, TypeSig::STRING]),
                )
                .map_err(|error| self.error(error.to_string()))?;
            self.emitter.call(concat);
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
        receiver: Option<&str>,
        name: &str,
        args: &[Expr],
    ) -> Result<Ty, CompileError> {
        if let Some(receiver) = receiver {
            let qualified = receiver.trim_start_matches("System.");
            match (qualified, name) {
                ("Console", "WriteLine") => {
                    let ty = self.single_argument(args, name)?;
                    let parameter = match ty {
                        Ty::Int => TypeSig::INT32,
                        Ty::Bool => TypeSig::BOOL,
                        Ty::String => TypeSig::STRING,
                        _ => return Err(self.error("Console.WriteLine has no supported overload")),
                    };
                    let method = self.import_bcl_method(
                        "System.Console",
                        "System.Console",
                        "WriteLine",
                        MethodSig::new_static(TypeSig::VOID, vec![parameter]),
                    )?;
                    self.emitter.call(method);
                    return Ok(Ty::Void);
                }
                ("Console", "Write") => {
                    let ty = self.single_argument(args, name)?;
                    let parameter = match ty {
                        Ty::String => TypeSig::STRING,
                        Ty::Int => TypeSig::INT32,
                        _ => return Err(self.error("Console.Write has no supported overload")),
                    };
                    let method = self.import_bcl_method(
                        "System.Console",
                        "System.Console",
                        "Write",
                        MethodSig::new_static(TypeSig::VOID, vec![parameter]),
                    )?;
                    self.emitter.call(method);
                    return Ok(Ty::Void);
                }
                ("Math", "Max") | ("Math", "Min") => {
                    self.expect_arity(args, 2, name)?;
                    let ty = self.call_arguments(args)?;
                    Self::require(&ty, &Ty::Int, name).map_err(|message| self.error(message))?;
                    let method = self
                        .module
                        .import_corlib_method(
                            "System.Math",
                            name,
                            MethodSig::new_static(
                                TypeSig::INT32,
                                vec![TypeSig::INT32, TypeSig::INT32],
                            ),
                        )
                        .map_err(|error| self.error(error.to_string()))?;
                    self.emitter.call(method);
                    return Ok(Ty::Int);
                }
                ("Math", "Abs") => {
                    let ty = self.single_argument(args, name)?;
                    Self::require(&ty, &Ty::Int, name).map_err(|message| self.error(message))?;
                    let method = self
                        .module
                        .import_corlib_method(
                            "System.Math",
                            "Abs",
                            MethodSig::new_static(TypeSig::INT32, vec![TypeSig::INT32]),
                        )
                        .map_err(|error| self.error(error.to_string()))?;
                    self.emitter.call(method);
                    return Ok(Ty::Int);
                }
                ("Convert", "ToString") => {
                    let ty = self.single_argument(args, name)?;
                    Self::require(&ty, &Ty::Int, name).map_err(|message| self.error(message))?;
                    let method = self
                        .module
                        .import_corlib_method(
                            "System.Convert",
                            "ToString",
                            MethodSig::new_static(TypeSig::STRING, vec![TypeSig::INT32]),
                        )
                        .map_err(|error| self.error(error.to_string()))?;
                    self.emitter.call(method);
                    return Ok(Ty::String);
                }
                _ => {}
            }
            return self.call_declared(Some(receiver), name, args);
        }
        self.call_declared(None, name, args)
    }

    fn call_declared(
        &mut self,
        receiver: Option<&str>,
        name: &str,
        args: &[Expr],
    ) -> Result<Ty, CompileError> {
        let owner = receiver.unwrap_or(&self.current);
        let Some(symbol) = self.symbols.get(owner) else {
            return Err(self.error(format!("unknown type '{owner}'")));
        };
        let Some(method) = symbol
            .methods
            .iter()
            .find(|method| method.name == name && method.params.len() == args.len())
            .map(|method| MethodSymbol {
                name: method.name.clone(),
                is_static: method.is_static,
                ret: method.ret.clone(),
                params: method.params.clone(),
                entry: method.entry.clone(),
            })
        else {
            return Err(self.error(format!(
                "no method '{name}' with {} parameter(s) on '{owner}'",
                args.len()
            )));
        };
        if !method.is_static {
            if self.is_static || receiver.is_some() {
                return Err(self.error(format!("instance method '{name}' requires an instance")));
            }
            self.emitter.ldarg(0);
        }
        for (index, argument) in args.iter().enumerate() {
            let actual = self.expr(argument)?;
            Self::require(&actual, &method.params[index], "argument")
                .map_err(|message| self.error(message))?;
        }
        if method.is_static {
            self.emitter.call(method.entry);
        } else {
            self.emitter.callvirt(method.entry);
        }
        Ok(method.ret)
    }

    /// Imports a method from a framework assembly, adding the assembly
    /// reference on demand. `System.Console` lives outside corelib.
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

    fn call_arguments(&mut self, args: &[Expr]) -> Result<Ty, CompileError> {
        let mut last = Ty::Void;
        for argument in args {
            last = self.expr(argument)?;
        }
        Ok(last)
    }

    fn field_store(&mut self, field: &FieldDecl, value: &Expr) -> Result<(), CompileError> {
        let Some(symbol) = self.field_symbol(&field.name) else {
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

impl<'a> FnEmitter<'a> {
    /// Assignment used as a value: the value stays on the stack for the caller.
    fn assign_expression(&mut self, name: &str, value: &Expr) -> Result<Ty, CompileError> {
        if let Some((index, ty)) = self.locals.get(name).cloned() {
            let actual = self.expr(value)?;
            Self::require(&actual, &ty, &format!("assignment to '{name}'"))
                .map_err(|message| self.error(message))?;
            if ty == Ty::Void {
                return Err(self.error("cannot assign a void expression"));
            }
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
        let Some(field) = self.field_symbol(name) else {
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
}
