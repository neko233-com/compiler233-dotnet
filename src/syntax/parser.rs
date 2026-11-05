//! Recursive-descent C# parser.
//!
//! The parser validates structure and reports the first construct it does not
//! understand. It deliberately does not build a semantic model yet; the goal of
//! this stage is an honest accept/reject decision plus a precise offset for
//! every rejection, which is what the differential gate measures.
use super::SyntaxError;
use super::lexer::{Token, TokenKind, lex_with_defines};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CompilationUnit {
    pub types: usize,
    pub members: usize,
}

pub fn parse(source: &str) -> Result<CompilationUnit, SyntaxError> {
    parse_with_defines(source, &[])
}

pub fn parse_with_defines(
    source: &str,
    defines: &[String],
) -> Result<CompilationUnit, SyntaxError> {
    let tokens = lex_with_defines(source, defines)?;
    let mut parser = Parser {
        source,
        tokens,
        position: 0,
        unit: CompilationUnit::default(),
        condition_context: false,
        lambda_allowed: true,
    };
    parser.parse_compilation_unit()?;
    Ok(parser.unit)
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    position: usize,
    unit: CompilationUnit,
    /// True while parsing an `if`/`while`/`switch` condition, where `(name)`
    /// followed by a statement is far more likely than a cast.
    condition_context: bool,
    /// False inside switch-arm guards, where `x =>` belongs to the arm rather
    /// than starting a lambda.
    lambda_allowed: bool,
}

const PREDEFINED_TYPES: &[&str] = &[
    "bool", "byte", "sbyte", "short", "ushort", "int", "uint", "long", "ulong", "float", "double",
    "decimal", "char", "string", "object", "void",
];

impl<'a> Parser<'a> {
    fn token(&self) -> Token {
        self.tokens[self.position.min(self.tokens.len() - 1)]
    }

    fn kind(&self) -> TokenKind {
        self.token().kind
    }

    fn text(&self) -> &'a str {
        let token = self.token();
        &self.source[token.start..token.end]
    }

    fn peek_text(&self, offset: usize) -> Option<&'a str> {
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
        self.kind() == TokenKind::EndOfFile
    }

    fn check(&self, text: &str) -> bool {
        self.text() == text
    }

    fn check_keyword(&self, text: &str) -> bool {
        self.kind() == TokenKind::Keyword && self.text() == text
    }

    /// Contextual keywords (`where`, `get`, `var`, `async`, ...) lex as
    /// identifiers, so they are matched by text only.
    fn check_word(&self, text: &str) -> bool {
        self.text() == text
    }

    fn eat_word(&mut self, text: &str) -> bool {
        if self.check_word(text) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn check_identifier(&self) -> bool {
        self.kind() == TokenKind::Identifier
    }

    fn eat(&mut self, text: &str) -> bool {
        if self.check(text) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_keyword(&mut self, text: &str) -> bool {
        if self.check_keyword(text) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error(&self, message: impl Into<String>) -> SyntaxError {
        SyntaxError {
            offset: self.token().start,
            message: message.into(),
        }
    }

    fn expect(&mut self, text: &str) -> Result<(), SyntaxError> {
        if self.eat(text) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{text}', found '{}'", self.text())))
        }
    }

    fn expect_keyword(&mut self, text: &str) -> Result<(), SyntaxError> {
        if self.eat_keyword(text) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{text}', found '{}'", self.text())))
        }
    }

    fn expect_identifier(&mut self) -> Result<(), SyntaxError> {
        if self.check_identifier() {
            self.bump();
            Ok(())
        } else {
            Err(self.error(format!("expected an identifier, found '{}'", self.text())))
        }
    }

    /// Accepts `>` without splitting, and splits a `>>` token so nested type
    /// arguments close without a special lexer mode.
    fn expect_type_close(&mut self) -> Result<(), SyntaxError> {
        if self.check(">") {
            self.bump();
            return Ok(());
        }
        Err(self.error(format!("expected '>', found '{}'", self.text())))
    }

    fn parse_compilation_unit(&mut self) -> Result<(), SyntaxError> {
        while !self.at_end() {
            let start = self.position;
            self.parse_attribute_lists()?;
            if self.eat(";") {
                continue;
            }
            if self.at_end() {
                // Assembly-level attributes may be the last construct.
                break;
            }
            if self.check_keyword("extern") && self.peek_text(1) == Some("alias") {
                self.skip_to_semicolon()?;
                continue;
            }
            if self.check_word("global") && self.peek_text(1) == Some("using") {
                self.bump();
                self.parse_using_directive()?;
                continue;
            }
            if self.check_keyword("using") {
                self.parse_using_directive()?;
                continue;
            }
            if self.check_keyword("namespace") {
                self.parse_namespace()?;
                continue;
            }
            self.parse_type_declaration()?;
            if self.position == start {
                return Err(self.error(format!("unexpected token '{}'", self.text())));
            }
        }
        Ok(())
    }

    fn parse_using_directive(&mut self) -> Result<(), SyntaxError> {
        self.expect_keyword("using")?;
        self.eat_word("global");
        self.eat_keyword("static");
        if self.check_identifier() && self.peek_text(1) == Some("=") {
            self.bump();
            self.bump();
        }
        // `using Alias = Some.Namespace<Type>;` needs full type syntax.
        self.parse_type_allowing_dots()?;
        self.expect(";")
    }

    fn parse_namespace(&mut self) -> Result<(), SyntaxError> {
        self.expect_keyword("namespace")?;
        self.parse_qualified_name()?;
        if self.eat(";") {
            // File-scoped namespace: the rest of the file is the body.
            while !self.at_end() {
                let start = self.position;
                self.parse_attribute_lists()?;
                if self.eat(";") {
                    continue;
                }
                if self.check_keyword("using") {
                    self.parse_using_directive()?;
                } else if self.check_keyword("namespace") {
                    self.parse_namespace()?;
                } else {
                    self.parse_type_declaration()?;
                }
                if self.position == start {
                    return Err(self.error(format!("unexpected token '{}'", self.text())));
                }
            }
            return Ok(());
        }
        self.expect("{")?;
        while !self.check("}") && !self.at_end() {
            let start = self.position;
            self.parse_attribute_lists()?;
            if self.eat(";") {
                continue;
            }
            if self.check_keyword("using") {
                self.parse_using_directive()?;
            } else if self.check_keyword("namespace") {
                self.parse_namespace()?;
            } else {
                self.parse_type_declaration()?;
            }
            if self.position == start {
                return Err(self.error(format!("unexpected token '{}'", self.text())));
            }
        }
        self.expect("}")
    }

    fn parse_attribute_lists(&mut self) -> Result<(), SyntaxError> {
        while self.check("[") {
            self.bump();
            loop {
                if (self.check_identifier() || self.kind() == TokenKind::Keyword)
                    && self.peek_text(1) == Some(":")
                {
                    self.bump();
                    self.bump();
                }
                self.parse_qualified_name()?;
                if self.eat("(") {
                    self.parse_argument_list_body()?;
                    self.expect(")")?;
                }
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("]")?;
        }
        Ok(())
    }

    fn parse_modifiers(&mut self) {
        const MODIFIERS: &[&str] = &[
            "public",
            "private",
            "protected",
            "internal",
            "static",
            "readonly",
            "ref",
            "sealed",
            "abstract",
            "virtual",
            "override",
            "async",
            "extern",
            "unsafe",
            "volatile",
            "new",
            "partial",
            "const",
            "fixed",
            "required",
            "file",
        ];
        loop {
            let text = self.text();
            let is_modifier = MODIFIERS.contains(&text);
            if !is_modifier {
                return;
            }
            self.bump();
        }
    }

    fn parse_type_declaration(&mut self) -> Result<(), SyntaxError> {
        self.parse_attribute_lists()?;
        self.parse_modifiers();
        if self.check_keyword("delegate") {
            self.parse_delegate()?;
            return Ok(());
        }
        if self.check_keyword("enum") {
            self.parse_enum()?;
            return Ok(());
        }
        if !(self.check_keyword("class")
            || self.check_keyword("struct")
            || self.check_keyword("interface")
            || self.check_word("record"))
        {
            return Err(self.error(format!(
                "expected a type declaration, found '{}'",
                self.text()
            )));
        }
        let record = self.check_word("record");
        self.bump();
        if record {
            let _ = self.eat_keyword("class") || self.eat_keyword("struct");
        }
        self.unit.types += 1;
        self.expect_identifier()?;
        if self.check("<") {
            self.parse_type_parameter_list()?;
        }
        if self.check("(") {
            // Positional record or primary constructor.
            self.parse_parameter_list()?;
        }
        if self.eat(":") {
            loop {
                self.parse_type_allowing_dots()?;
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.parse_constraints()?;
        if self.eat(";") {
            return Ok(());
        }
        self.expect("{")?;
        while !self.check("}") && !self.at_end() {
            let start = self.position;
            self.parse_member()?;
            if self.position == start {
                return Err(self.error(format!("unexpected token '{}'", self.text())));
            }
        }
        self.expect("}")
    }

    fn parse_delegate(&mut self) -> Result<(), SyntaxError> {
        self.expect_keyword("delegate")?;
        self.parse_type()?;
        self.expect_identifier()?;
        if self.check("<") {
            self.parse_type_parameter_list()?;
        }
        self.parse_parameter_list()?;
        self.parse_constraints()?;
        self.expect(";")
    }

    fn parse_enum(&mut self) -> Result<(), SyntaxError> {
        self.expect_keyword("enum")?;
        self.unit.types += 1;
        self.expect_identifier()?;
        if self.eat(":") {
            self.parse_type()?;
        }
        self.expect("{")?;
        while !self.check("}") && !self.at_end() {
            self.parse_attribute_lists()?;
            self.expect_identifier()?;
            if self.eat("=") {
                self.parse_expression()?;
            }
            if !self.eat(",") {
                break;
            }
        }
        self.eat(",");
        self.expect("}")
    }

    fn parse_type_parameter_list(&mut self) -> Result<(), SyntaxError> {
        self.expect("<")?;
        loop {
            self.parse_attribute_lists()?;
            if self.eat_keyword("in") || self.eat_keyword("out") {
                // Variance modifier.
            }
            self.expect_identifier()?;
            if !self.eat(",") {
                break;
            }
        }
        self.expect_type_close()
    }

    fn parse_constraints(&mut self) -> Result<(), SyntaxError> {
        while self.check_word("where") {
            self.bump();
            self.expect_identifier()?;
            self.expect(":")?;
            loop {
                if self.eat_keyword("class") || self.eat_keyword("struct") {
                    // Special constraints do not name a type.
                } else if self.eat_word("notnull") || self.eat_word("unmanaged") {
                    // Contextual special constraints.
                } else if self.eat_keyword("new") {
                    if self.check("(") {
                        self.bump();
                        self.expect(")")?;
                    }
                } else {
                    self.parse_type()?;
                }
                if !self.eat(",") {
                    break;
                }
            }
        }
        Ok(())
    }

    fn parse_member(&mut self) -> Result<(), SyntaxError> {
        self.parse_attribute_lists()?;
        self.parse_modifiers();
        if self.check("}") || self.at_end() {
            return Ok(());
        }
        if self.eat(";") {
            return Ok(());
        }
        self.unit.members += 1;
        if self.check_keyword("class")
            || self.check_keyword("struct")
            || self.check_keyword("interface")
            || self.check_keyword("enum")
            || self.check_word("record")
        {
            return self.parse_type_declaration();
        }
        if self.check_keyword("delegate") {
            return self.parse_delegate();
        }
        if self.check_keyword("event") {
            return self.parse_event();
        }
        if self.check("~") {
            self.bump();
            self.expect_identifier()?;
            self.expect("(")?;
            self.expect(")")?;
            return self.parse_block();
        }
        if self.check_keyword("implicit") || self.check_keyword("explicit") {
            self.bump();
            self.expect_keyword("operator")?;
            self.parse_type()?;
            self.parse_parameter_list()?;
            return self.parse_body_or_semicolon();
        }
        if self.check_identifier() && self.peek_text(1) == Some("(") {
            // Constructor.
            self.bump();
            self.parse_parameter_list()?;
            if self.eat(":") {
                self.eat_keyword("base");
                self.eat_keyword("this");
                self.expect("(")?;
                self.parse_argument_list_body()?;
                self.expect(")")?;
            }
            return self.parse_body_or_semicolon();
        }
        self.parse_type()?;
        if self.check_keyword("operator") {
            self.bump();
            if self.check("[") || self.check(")") || self.check("(") {
                // Aggregate operators are not valid; keep the diagnostic local.
                return Err(self.error("unsupported operator declaration"));
            }
            self.bump();
            self.parse_parameter_list()?;
            return self.parse_body_or_semicolon();
        }
        if self.check_keyword("this") && self.peek_text(1) == Some("[") {
            self.bump();
            self.expect("[")?;
            self.parse_parameter_list_body()?;
            self.expect("]")?;
            return self.parse_property_body();
        }
        self.parse_member_name()?;
        if self.check("<") {
            self.parse_type_parameter_list()?;
        }
        if self.check("(") {
            self.parse_parameter_list()?;
            self.parse_constraints()?;
            return self.parse_body_or_semicolon();
        }
        if self.check("{") {
            self.parse_accessor_list()?;
            if self.eat("=") {
                self.parse_variable_initializer()?;
                return self.expect(";");
            }
            return Ok(());
        }
        if self.check("=>") {
            self.bump();
            self.parse_expression()?;
            return self.expect(";");
        }
        self.parse_field_declarators()
    }

    fn parse_member_name(&mut self) -> Result<(), SyntaxError> {
        if self.check_keyword("this") {
            self.bump();
            return Ok(());
        }
        self.expect_identifier()?;
        loop {
            // Explicit interface implementation: `IComparer<T>.Compare`.
            if self.check("<") {
                let _ = self.parse_type_arguments_if_possible();
            }
            if !self.eat(".") {
                break;
            }
            self.expect_identifier()?;
        }
        Ok(())
    }

    fn parse_variable_initializer(&mut self) -> Result<(), SyntaxError> {
        // `int[] values = { 1, 2 };` has no `new` keyword.
        if self.check("{") {
            return self.parse_initializer();
        }
        self.parse_expression()
    }

    fn parse_field_declarators(&mut self) -> Result<(), SyntaxError> {
        loop {
            if self.eat("=") {
                self.parse_variable_initializer()?;
            }
            if !self.eat(",") {
                break;
            }
            self.expect_identifier()?;
        }
        self.expect(";")
    }

    fn parse_event(&mut self) -> Result<(), SyntaxError> {
        self.expect_keyword("event")?;
        self.parse_type()?;
        self.parse_member_name()?;
        if self.check("{") {
            return self.parse_accessor_list();
        }
        loop {
            if self.eat("=") {
                self.parse_variable_initializer()?;
            }
            if !self.eat(",") {
                break;
            }
            self.parse_member_name()?;
        }
        self.expect(";")
    }

    fn parse_property_body(&mut self) -> Result<(), SyntaxError> {
        if self.check("{") {
            self.parse_accessor_list()?;
            if self.eat("=") {
                self.parse_variable_initializer()?;
                return self.expect(";");
            }
            return Ok(());
        }
        if self.check("=>") {
            self.bump();
            self.parse_expression()?;
            return self.expect(";");
        }
        Err(self.error("expected a property body"))
    }

    fn parse_accessor_list(&mut self) -> Result<(), SyntaxError> {
        self.expect("{")?;
        while !self.check("}") && !self.at_end() {
            self.parse_attribute_lists()?;
            self.parse_modifiers();
            if self.check("}") {
                break;
            }
            if !(self.check_word("get")
                || self.check_word("set")
                || self.check_word("init")
                || self.check_word("add")
                || self.check_word("remove"))
            {
                return Err(self.error(format!("expected an accessor, found '{}'", self.text())));
            }
            self.bump();
            if self.eat(";") {
                continue;
            }
            if self.eat("=>") {
                self.parse_expression()?;
                self.expect(";")?;
                continue;
            }
            self.parse_block()?;
        }
        self.expect("}")
    }

    fn parse_body_or_semicolon(&mut self) -> Result<(), SyntaxError> {
        if self.eat(";") {
            return Ok(());
        }
        if self.eat("=>") {
            self.parse_expression()?;
            return self.expect(";");
        }
        self.parse_block()
    }

    fn parse_block(&mut self) -> Result<(), SyntaxError> {
        self.expect("{")?;
        while !self.check("}") && !self.at_end() {
            let start = self.position;
            self.parse_statement()?;
            if self.position == start {
                return Err(self.error(format!("unexpected token '{}'", self.text())));
            }
        }
        self.expect("}")
    }

    fn parse_statement(&mut self) -> Result<(), SyntaxError> {
        if self.check("{") {
            return self.parse_block();
        }
        if self.eat(";") {
            return Ok(());
        }
        if self.check_identifier() && self.peek_text(1) == Some(":") {
            // Labeled statement: `NEXT: continue;`
            self.bump();
            self.bump();
            return self.parse_statement();
        }
        if self.check("(") && self.looks_like_deconstruction_assignment() {
            // `(int id, _) = call();` or `(a[i], a[j]) = (a[j], a[i]);`
            let saved = self.position;
            let typed_designation = self.parse_designation().is_ok() && self.check("=");
            if !typed_designation {
                self.position = saved;
                // Parse the tuple target without consuming the `=`.
                self.parse_conditional()?;
            }
            self.expect("=")?;
            self.parse_expression()?;
            return self.expect(";");
        }
        if self.check_keyword("if") {
            self.bump();
            self.expect("(")?;
            self.parse_condition()?;
            self.expect(")")?;
            self.parse_statement()?;
            if self.eat_keyword("else") {
                self.parse_statement()?;
            }
            return Ok(());
        }
        if self.check_keyword("for") {
            self.bump();
            self.expect("(")?;
            if !self.check(";") {
                self.parse_for_initializer()?;
            }
            self.expect(";")?;
            if !self.check(";") {
                self.parse_expression()?;
            }
            self.expect(";")?;
            if !self.check(")") {
                loop {
                    self.parse_expression()?;
                    if !self.eat(",") {
                        break;
                    }
                }
            }
            self.expect(")")?;
            return self.parse_statement();
        }
        if self.check_keyword("foreach") {
            self.bump();
            self.expect("(")?;
            self.eat_keyword("ref");
            self.eat_keyword("readonly");
            self.parse_type()?;
            // `foreach (var (a, b) in ...)` and typed tuples
            // `foreach ((int k, T v) in ...)` need no extra designation.
            if self.check("(") {
                self.parse_designation()?;
            } else if !self.check_keyword("in") {
                self.expect_identifier()?;
            }
            self.expect_keyword("in")?;
            self.parse_expression()?;
            self.expect(")")?;
            return self.parse_statement();
        }
        if self.check_keyword("while") {
            self.bump();
            self.expect("(")?;
            self.parse_condition()?;
            self.expect(")")?;
            return self.parse_statement();
        }
        if self.check_keyword("do") {
            self.bump();
            self.parse_statement()?;
            self.expect_keyword("while")?;
            self.expect("(")?;
            self.parse_condition()?;
            self.expect(")")?;
            return self.expect(";");
        }
        if self.check_keyword("switch") {
            self.bump();
            self.expect("(")?;
            self.parse_condition()?;
            self.expect(")")?;
            return self.parse_switch_body();
        }
        if self.check_keyword("try") {
            self.bump();
            self.parse_block()?;
            let mut caught = false;
            while self.check_keyword("catch") {
                caught = true;
                self.bump();
                if self.eat("(") {
                    self.parse_type()?;
                    self.expect_identifier().ok();
                    self.expect(")")?;
                }
                if self.eat_word("when") {
                    self.expect("(")?;
                    self.parse_expression()?;
                    self.expect(")")?;
                }
                self.parse_block()?;
            }
            if self.check_keyword("finally") {
                self.bump();
                self.parse_block()?;
            }
            if !caught && !self.check("//") {
                // A try block without catch or finally is invalid, but the
                // concrete diagnostic is more useful than a bare failure.
            }
            return Ok(());
        }
        if self.check_keyword("return") {
            self.bump();
            if !self.check(";") {
                self.parse_expression()?;
            }
            return self.expect(";");
        }
        if self.check_keyword("throw") {
            self.bump();
            if !self.check(";") {
                self.parse_expression()?;
            }
            return self.expect(";");
        }
        if self.check_keyword("break") || self.check_keyword("continue") {
            self.bump();
            return self.expect(";");
        }
        if self.check_keyword("goto") {
            self.bump();
            if self.eat_keyword("default") {
                return self.expect(";");
            }
            if self.eat_keyword("case") {
                self.parse_expression()?;
                return self.expect(";");
            }
            self.expect_identifier()?;
            return self.expect(";");
        }
        if self.check_keyword("using") {
            self.bump();
            if self.eat("(") {
                self.eat_keyword("ref");
                self.eat_keyword("readonly");
                if self.looks_like_local_declaration() {
                    self.parse_local_declaration(false)?;
                } else {
                    self.parse_expression()?;
                }
                self.expect(")")?;
                return self.parse_statement();
            }
            self.parse_type()?;
            self.expect_identifier()?;
            if self.eat("=") {
                self.parse_expression()?;
            }
            return self.expect(";");
        }
        if self.check_keyword("lock") {
            self.bump();
            self.expect("(")?;
            self.parse_expression()?;
            self.expect(")")?;
            return self.parse_statement();
        }
        if self.check_word("yield") {
            self.bump();
            if self.eat_keyword("break") {
                return self.expect(";");
            }
            self.expect_keyword("return")?;
            self.parse_expression()?;
            return self.expect(";");
        }
        if self.check_keyword("fixed")
            || self.check_keyword("checked")
            || self.check_keyword("unchecked")
            || self.check_keyword("unsafe")
        {
            self.bump();
            if self.check("(") {
                self.bump();
                self.parse_type()?;
                self.expect_identifier()?;
                if self.eat("=") {
                    self.parse_expression()?;
                }
                self.expect(")")?;
            }
            return self.parse_statement();
        }
        if self.check_keyword("const") {
            self.bump();
            self.parse_type()?;
            self.expect_identifier()?;
            self.expect("=")?;
            self.parse_expression()?;
            return self.expect(";");
        }
        if self.check_keyword("else")
            || self.check_keyword("catch")
            || self.check_keyword("finally")
            || self.check_keyword("case")
        {
            return Err(self.error(format!("unexpected '{}'", self.text())));
        }
        self.parse_declaration_or_expression_statement()
    }

    fn parse_for_initializer(&mut self) -> Result<(), SyntaxError> {
        if self.looks_like_local_declaration() {
            self.parse_local_declaration(false)?;
            return Ok(());
        }
        loop {
            self.parse_expression()?;
            if !self.eat(",") {
                break;
            }
        }
        Ok(())
    }

    fn parse_switch_body(&mut self) -> Result<(), SyntaxError> {
        self.expect("{")?;
        while !self.check("}") && !self.at_end() {
            if self.check_keyword("case") {
                self.bump();
                self.parse_case_label()?;
                self.expect(":")?;
                continue;
            }
            if self.check_keyword("default") {
                self.bump();
                self.expect(":")?;
                continue;
            }
            let start = self.position;
            self.parse_statement()?;
            if self.position == start {
                return Err(self.error("expected a case label or statement"));
            }
        }
        self.expect("}")
    }

    fn parse_case_label(&mut self) -> Result<(), SyntaxError> {
        // `case Type name:` and `case Type:` are patterns, not expressions.
        let saved = self.position;
        if self.parse_type().is_ok() {
            if self.check(":") || (self.check_identifier() && !self.check_word("when")) {
                if self.check_identifier() {
                    self.bump();
                }
                if self.eat_word("when") {
                    // A case guard is an expression; only `catch when` uses
                    // parentheses.
                    self.parse_expression()?;
                }
                return Ok(());
            }
            if self.eat_word("when") {
                self.parse_expression()?;
                return Ok(());
            }
        }
        self.position = saved;
        self.parse_expression()?;
        if self.eat_word("when") {
            self.parse_expression()?;
        }
        Ok(())
    }

    fn looks_like_local_declaration(&mut self) -> bool {
        if self.check_word("await") {
            // `await Call(a, b);` is an expression statement, not a local
            // function whose return type is the contextual keyword `await`.
            return false;
        }
        let saved = self.position;
        let result = (|| -> Result<bool, SyntaxError> {
            self.parse_modifiers();
            self.parse_type()?;
            let looks = if self.check_identifier() {
                // `type name = ...`, `type name;` and local functions
                // `type name(...)` all start a declaration.
                self.peek_text(1)
                    .is_some_and(|next| matches!(next, "=" | ";" | "," | ")" | "in" | "("))
            } else if self.check("(") {
                // `var (a, b) = ...` deconstruction.
                self.parse_designation().is_ok() && matches!(self.text(), "=" | ";" | ")")
            } else {
                false
            };
            Ok(looks)
        })()
        .unwrap_or(false);
        self.position = saved;
        result
    }

    fn parse_local_declaration(&mut self, require_semicolon: bool) -> Result<(), SyntaxError> {
        self.parse_modifiers();
        self.eat_keyword("const");
        self.parse_type()?;
        loop {
            if self.check("(") {
                self.parse_designation()?;
            } else {
                self.expect_identifier()?;
            }
            if self.eat("=") {
                self.parse_variable_initializer()?;
            }
            if !self.eat(",") {
                break;
            }
        }
        if require_semicolon {
            self.expect(";")?;
        }
        Ok(())
    }

    fn parse_declaration_or_expression_statement(&mut self) -> Result<(), SyntaxError> {
        if self.looks_like_local_declaration() {
            // Local functions share the declaration prefix.
            let saved = self.position;
            self.parse_modifiers();
            self.parse_type()?;
            if self.check_identifier() && self.peek_text(1) == Some("(") {
                self.bump();
                self.parse_parameter_list()?;
                self.parse_constraints()?;
                return self.parse_body_or_semicolon();
            }
            self.position = saved;
            self.parse_local_declaration(true)?;
            return Ok(());
        }
        self.parse_expression()?;
        self.expect(";")
    }

    fn parse_parameter_list(&mut self) -> Result<(), SyntaxError> {
        self.expect("(")?;
        self.parse_parameter_list_body()?;
        self.expect(")")
    }

    fn parse_parameter_list_body(&mut self) -> Result<(), SyntaxError> {
        if self.check(")") {
            return Ok(());
        }
        loop {
            self.parse_attribute_lists()?;
            self.parse_modifiers();
            if self.eat_keyword("this") {
                // Extension method receiver.
            }
            self.eat_keyword("ref");
            self.eat_keyword("out");
            self.eat_keyword("in");
            self.eat_keyword("params");
            self.parse_type()?;
            self.expect_identifier()?;
            if self.eat("=") {
                self.parse_expression()?;
            }
            if !self.eat(",") {
                break;
            }
        }
        Ok(())
    }

    fn parse_type(&mut self) -> Result<(), SyntaxError> {
        self.parse_type_inner(true, true, true)
    }

    /// Base lists, casts and `new` accept any number of dotted segments.
    fn parse_type_allowing_dots(&mut self) -> Result<(), SyntaxError> {
        self.parse_type_inner(true, true, false)
    }

    /// `is` patterns never take a nullable suffix; a `?` there starts a
    /// conditional expression (`x is T ? a : b`).
    fn parse_is_type(&mut self) -> Result<(), SyntaxError> {
        self.parse_type_inner(true, false, false)
    }

    fn parse_type_inner(
        &mut self,
        allow_dots: bool,
        allow_nullable: bool,
        stop_at_member: bool,
    ) -> Result<(), SyntaxError> {
        if self.eat("(") {
            // Tuple type with optional element names: `(int x, string y)`.
            self.parse_type_inner(allow_dots, allow_nullable, stop_at_member)?;
            if self.check_identifier() {
                self.bump();
            }
            if !self.eat(",") {
                return Err(self.error("expected a tuple element"));
            }
            loop {
                self.parse_type_inner(allow_dots, allow_nullable, stop_at_member)?;
                if self.check_identifier() {
                    self.bump();
                }
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        } else {
            self.parse_qualified_name()?;
        }
        loop {
            if self.check("<") {
                self.bump();
                if !self.check(">") {
                    loop {
                        if self.check(",") {
                            // Unbound generic slot: `typeof(Dictionary<,>)`.
                            self.bump();
                            continue;
                        }
                        if self.check(">") {
                            break;
                        }
                        self.parse_attribute_lists()?;
                        // Generic argument variance is only legal on type
                        // parameters; accept it here so one spelling parses.
                        let _ = self.eat_keyword("in");
                        let _ = self.eat_keyword("out");
                        // Inside type arguments a dotted segment is always part
                        // of the type, never a member name.
                        self.parse_type_inner(allow_dots, allow_nullable, false)?;
                        if !self.eat(",") {
                            break;
                        }
                    }
                }
                self.expect_type_close()?;
                continue;
            }
            if self.check("[") && matches!(self.peek_text(1), Some("]") | Some(",")) {
                self.bump();
                while self.eat(",") {}
                self.expect("]")?;
                continue;
            }
            if allow_dots && self.check(".") {
                // `Type.Member` ends the type when what follows can only be a
                // member declaration (explicit interface implementation).
                if stop_at_member
                    && matches!(
                        self.peek_text(2),
                        Some("(") | Some(";") | Some("=") | Some(",") | Some("{") | Some("=>")
                    )
                {
                    return Ok(());
                }
                self.bump();
                self.parse_simple_name()?;
                continue;
            }
            if self.check("?") && allow_nullable {
                self.bump();
                continue;
            }
            if self.check("*") {
                self.bump();
                continue;
            }
            return Ok(());
        }
    }

    fn parse_qualified_name(&mut self) -> Result<(), SyntaxError> {
        if self.check_word("global") && self.peek_text(1) == Some("::") {
            self.bump();
            self.bump();
        }
        self.parse_simple_name()?;
        while self.check(".") {
            self.bump();
            self.parse_simple_name()?;
        }
        while self.check("::") {
            self.bump();
            self.parse_simple_name()?;
        }
        Ok(())
    }

    fn parse_simple_name(&mut self) -> Result<(), SyntaxError> {
        if self.check_identifier() || self.kind() == TokenKind::Keyword {
            self.bump();
            Ok(())
        } else {
            Err(self.error(format!(
                "expected a type or identifier, found '{}'",
                self.text()
            )))
        }
    }

    fn parse_expression(&mut self) -> Result<(), SyntaxError> {
        self.parse_assignment()
    }

    fn parse_condition(&mut self) -> Result<(), SyntaxError> {
        let previous = self.condition_context;
        self.condition_context = true;
        let result = self.parse_expression();
        self.condition_context = previous;
        result
    }

    /// A switch-arm or case guard: `x =>` belongs to the arm here.
    fn parse_guard(&mut self) -> Result<(), SyntaxError> {
        let previous = self.lambda_allowed;
        self.lambda_allowed = false;
        let result = self.parse_expression();
        self.lambda_allowed = previous;
        result
    }

    /// `(...) =` at statement start is a deconstruction assignment.
    fn looks_like_deconstruction_assignment(&mut self) -> bool {
        let saved = self.position;
        let mut depth = 0usize;
        let mut found = false;
        loop {
            if self.at_end() {
                break;
            }
            if self.check("(") {
                depth += 1;
            } else if self.check(")") {
                depth -= 1;
                if depth == 0 {
                    self.bump();
                    found = self.check("=");
                    break;
                }
            }
            self.bump();
        }
        self.position = saved;
        found
    }

    fn parse_designation(&mut self) -> Result<(), SyntaxError> {
        // Deconstruction designation: `(bool a, T b)` or `(name, (x, y))`.
        self.expect("(")?;
        while !self.check(")") && !self.at_end() {
            if self.check("(") {
                self.parse_designation()?;
            } else {
                let saved = self.position;
                let typed = match self.parse_type() {
                    Ok(()) => self.check_identifier(),
                    Err(_) => false,
                };
                if typed {
                    self.bump();
                } else {
                    self.position = saved;
                    self.expect_identifier()?;
                }
            }
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")
    }

    fn parse_assignment(&mut self) -> Result<(), SyntaxError> {
        self.parse_conditional()?;
        if matches!(
            self.text(),
            "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "??="
        ) {
            self.bump();
            self.parse_assignment()?;
        }
        Ok(())
    }

    fn parse_conditional(&mut self) -> Result<(), SyntaxError> {
        self.parse_binary(0)?;
        if self.eat("?") {
            self.parse_expression()?;
            self.expect(":")?;
            self.parse_conditional()?;
        }
        Ok(())
    }

    fn binary_precedence(&self) -> Option<u8> {
        let precedence = match self.text() {
            "??" => 1,
            "||" => 2,
            "&&" => 3,
            "|" => 4,
            "^" => 5,
            "&" => 6,
            "==" | "!=" => 7,
            "<" | ">" | "<=" | ">=" => 8,
            "<<" => 9,
            "+" | "-" => 10,
            "*" | "/" | "%" => 11,
            _ => {
                if self.check_keyword("is") || self.check_keyword("as") {
                    8
                } else {
                    return None;
                }
            }
        };
        Some(precedence)
    }

    /// `>>` is lexed as two `>` tokens so nested type arguments stay
    /// unambiguous; a shift is two adjacent `>` tokens.
    fn is_shift_right(&self) -> bool {
        if !self.check(">") || self.peek_text(1) != Some(">") {
            return false;
        }
        let current = self.token();
        let next = self.tokens[self.position + 1];
        current.end == next.start
    }

    fn parse_binary(&mut self, minimum: u8) -> Result<(), SyntaxError> {
        self.parse_unary()?;
        loop {
            if self.check("..") {
                // Range operator; the end operand is optional.
                self.bump();
                if !matches!(self.text(), "]" | ")" | "," | ";" | "}") {
                    self.parse_expression()?;
                }
                continue;
            }
            if self.is_shift_right() {
                if 9 < minimum {
                    break;
                }
                self.bump();
                self.bump();
                self.parse_binary(10)?;
                continue;
            }
            let Some(precedence) = self.binary_precedence() else {
                break;
            };
            if precedence < minimum {
                break;
            }
            let is_null_coalescing = self.check("??");
            let is_as = self.check_keyword("as");
            let is_is = self.check_keyword("is");
            self.bump();
            if is_null_coalescing {
                self.parse_binary(precedence)?;
                continue;
            }
            if is_is {
                self.eat_word("not");
                self.parse_pattern()?;
                while self.check_word("and") || self.check_word("or") {
                    self.bump();
                    self.parse_pattern()?;
                }
                continue;
            }
            if is_as {
                self.parse_type_allowing_dots()?;
                continue;
            }
            self.parse_binary(precedence + 1)?;
        }
        Ok(())
    }

    fn parse_unary(&mut self) -> Result<(), SyntaxError> {
        if self.check("..") {
            // Range expressions: `self[..1]` and `self[1..]`.
            self.bump();
            if !matches!(self.text(), "]" | ")" | "," | ";" | "}") {
                self.parse_unary()?;
            }
            return Ok(());
        }
        if self.check_keyword("ref") {
            self.bump();
            return self.parse_unary();
        }
        if matches!(
            self.text(),
            "+" | "-" | "!" | "~" | "++" | "--" | "&" | "*" | "^"
        ) {
            self.bump();
            return self.parse_unary();
        }
        if self.check_word("await") {
            self.bump();
            return self.parse_unary();
        }
        if self.check("(") && self.looks_like_cast() {
            self.bump();
            self.parse_type()?;
            self.expect(")")?;
            return self.parse_unary();
        }
        self.parse_postfix()
    }

    fn looks_like_cast(&mut self) -> bool {
        let saved = self.position;
        let result = (|| -> Result<bool, SyntaxError> {
            self.expect("(")?;
            self.parse_type()?;
            let type_start = self.tokens[saved + 1].start;
            let type_end = self.token().start;
            if !self.check(")") {
                return Ok(false);
            }
            self.bump();
            let next = self.text();
            let type_text = self.source[type_start..type_end].trim();
            // `(Name)value` cannot be two adjacent expressions, so a
            // type-looking identifier immediately followed by a value is a
            // cast. Lower-case names stay ambiguous with `if (flag) Call()`.
            // `(name)value` cannot be two adjacent expressions, so a plain
            // identifier followed by a value is a cast unless we are parsing a
            // condition, where `(flag) Call()` is the common reading.
            let single_identifier = !self.condition_context
                && !type_text.is_empty()
                && type_text
                    .chars()
                    .all(|character| character.is_alphanumeric() || character == '_');
            let followed_by_value = matches!(
                self.kind(),
                TokenKind::Identifier | TokenKind::Number | TokenKind::String | TokenKind::Char
            ) || matches!(next, "null" | "true" | "false");
            let looks_like_type = type_text.contains('.')
                || type_text.contains('<')
                || type_text.contains('[')
                || type_text.contains('?')
                || type_text.contains('*')
                || type_text.starts_with("global::")
                || type_text
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_uppercase())
                || matches!(
                    type_text,
                    "int"
                        | "uint"
                        | "long"
                        | "ulong"
                        | "short"
                        | "ushort"
                        | "byte"
                        | "sbyte"
                        | "float"
                        | "double"
                        | "decimal"
                        | "bool"
                        | "char"
                        | "string"
                        | "object"
                        | "void"
                );
            let starts_expression = matches!(
                next,
                "(" | "!"
                    | "~"
                    | "-"
                    | "+"
                    | "++"
                    | "--"
                    | "new"
                    | "this"
                    | "base"
                    | "typeof"
                    | "default"
                    | "sizeof"
                    | "nameof"
                    | "await"
                    | "delegate"
                    | "checked"
                    | "unchecked"
            ) || self.kind() == TokenKind::Identifier
                || self.kind() == TokenKind::Number
                || self.kind() == TokenKind::String
                || self.kind() == TokenKind::Char
                || matches!(next, "null" | "true" | "false")
                || PREDEFINED_TYPES.contains(&next);
            Ok((looks_like_type || (single_identifier && followed_by_value)) && starts_expression)
        })()
        .unwrap_or(false);
        self.position = saved;
        result
    }

    fn parse_postfix(&mut self) -> Result<(), SyntaxError> {
        self.parse_primary()?;
        loop {
            if self.check(".") || self.check("?.") {
                self.bump();
                self.parse_simple_name()?;
                if self.check("<") && self.looks_like_generic_invocation() {
                    self.parse_type_arguments_if_possible()?;
                }
                continue;
            }
            if self.check("(") {
                self.bump();
                self.parse_argument_list_body()?;
                self.expect(")")?;
                continue;
            }
            if self.check("::") {
                // Alias-qualified access, e.g. `global::Ns.Type`.
                self.bump();
                self.parse_simple_name()?;
                continue;
            }
            if self.check("?") && self.peek_text(1) == Some("[") {
                // Conditional element access: `config?["key"]`.
                self.bump();
                continue;
            }
            if self.check("[") {
                self.bump();
                if !self.check("]") {
                    loop {
                        self.parse_expression()?;
                        if !self.eat(",") {
                            break;
                        }
                    }
                }
                self.expect("]")?;
                continue;
            }
            if self.check("++") || self.check("--") {
                self.bump();
                continue;
            }
            if self.check("!") {
                // Null-forgiving operator: only valid directly after an
                // expression, and not when it starts one.
                self.bump();
                continue;
            }
            if self.check("<") && self.looks_like_generic_invocation() {
                self.parse_type_arguments_if_possible()?;
                continue;
            }
            if self.check_word("switch") && self.peek_text(1) == Some("{") {
                self.parse_switch_expression()?;
                continue;
            }
            return Ok(());
        }
    }

    fn parse_switch_pattern(&mut self) -> Result<(), SyntaxError> {
        if self.check("_") {
            self.bump();
            return Ok(());
        }
        let saved = self.position;
        let mut matched = false;
        if self.parse_type().is_ok() {
            if self.check_identifier() && !matches!(self.text(), "or" | "and" | "when") {
                self.bump();
            }
            matched = self.check("=>")
                || self.check_word("when")
                || self.check_word("or")
                || self.check_word("and");
        }
        if !matched {
            self.position = saved;
            self.parse_expression()?;
        }
        Ok(())
    }

    fn parse_switch_expression(&mut self) -> Result<(), SyntaxError> {
        self.bump();
        self.bump();
        while !self.check("}") && !self.at_end() {
            self.parse_switch_pattern()?;
            while self.check_word("or") || self.check_word("and") {
                self.bump();
                self.parse_switch_pattern()?;
            }
            if self.eat_word("when") {
                self.parse_guard()?;
            }
            self.expect("=>")?;
            self.parse_expression()?;
            if !self.eat(",") {
                break;
            }
        }
        self.eat(",");
        self.expect("}")
    }

    fn looks_like_generic_invocation(&mut self) -> bool {
        let saved = self.position;
        // `Foo<T>(x)`, `Foo<T>.Bar` and `Foo<T>?.Bar` all start with type
        // arguments; a comparison such as `a < b > c` never does.
        let success = self.parse_type_arguments_if_possible().is_ok()
            && matches!(self.text(), "(" | "." | "?." | ";" | "," | ")" | "]" | "}");
        self.position = saved;
        success
    }

    fn parse_type_arguments_if_possible(&mut self) -> Result<(), SyntaxError> {
        let saved = self.position;
        let result = (|| -> Result<(), SyntaxError> {
            self.expect("<")?;
            if self.check(">") {
                self.expect_type_close()?;
                return Ok(());
            }
            loop {
                if self.check(",") {
                    self.bump();
                    continue;
                }
                if self.check(">") {
                    break;
                }
                self.parse_type()?;
                if !self.eat(",") {
                    break;
                }
            }
            self.expect_type_close()
        })();
        if result.is_err() {
            self.position = saved;
        }
        result
    }

    fn parse_argument_list_body(&mut self) -> Result<(), SyntaxError> {
        if self.check(")") {
            return Ok(());
        }
        loop {
            // A named argument may still carry `out`: `reason: out string text`.
            if self.check_identifier() && self.peek_text(1) == Some(":") {
                self.bump();
                self.bump();
            }
            let by_reference =
                self.eat_keyword("ref") || self.eat_keyword("out") || self.eat_keyword("in");
            // `out float value` / `out var value` declare a variable in place.
            if by_reference && self.looks_like_typed_argument() {
                self.parse_type()?;
                self.expect_identifier()?;
            } else {
                self.parse_expression()?;
            }
            if !self.eat(",") {
                break;
            }
        }
        Ok(())
    }

    fn looks_like_typed_argument(&mut self) -> bool {
        let saved = self.position;
        let typed = self.parse_type().is_ok() && self.check_identifier();
        self.position = saved;
        typed
    }

    fn parse_primary(&mut self) -> Result<(), SyntaxError> {
        if self.check("(") {
            if self.lambda_allowed && self.looks_like_lambda() {
                return self.parse_lambda();
            }
            self.bump();
            // Parentheses end any guard context, so lambdas are legal again.
            let previous_lambda = self.lambda_allowed;
            self.lambda_allowed = true;
            // Tuple elements may be named: `(mat: material, texture)`.
            if self.check_identifier() && self.peek_text(1) == Some(":") {
                self.bump();
                self.bump();
            }
            self.parse_expression()?;
            while self.eat(",") {
                if self.check_identifier() && self.peek_text(1) == Some(":") {
                    self.bump();
                    self.bump();
                }
                self.parse_expression()?;
            }
            self.lambda_allowed = previous_lambda;
            return self.expect(")");
        }
        if self.check_keyword("static")
            && (self.peek_text(1) == Some("(")
                || self
                    .tokens
                    .get(self.position + 1)
                    .is_some_and(|token| token.kind == TokenKind::Identifier))
        {
            // Static lambda: `static (span, state) => ...`.
            self.bump();
            if self.looks_like_lambda() {
                return self.parse_lambda();
            }
        }
        if self.check_word("async")
            && (self.peek_text(1) == Some("(")
                || self
                    .tokens
                    .get(self.position + 1)
                    .is_some_and(|token| token.kind == TokenKind::Identifier))
        {
            self.bump();
            if self.looks_like_lambda() {
                return self.parse_lambda();
            }
            return Err(self.error("expected a lambda after 'async'"));
        }
        if self.lambda_allowed
            && self.kind() == TokenKind::Identifier
            && self.peek_text(1) == Some("=>")
        {
            return self.parse_lambda();
        }
        if self.check_keyword("new") {
            return self.parse_new();
        }
        if self.check_keyword("default") && self.peek_text(1) != Some("(") {
            // Default literal, e.g. `Add(default)`.
            self.bump();
            return Ok(());
        }
        if self.check_keyword("stackalloc") {
            // `stackalloc int[3]` and `stackalloc[] { 1, 2 }`.
            self.bump();
            if self.check("[") {
                return self.parse_initializer();
            }
            self.parse_type_allowing_dots()?;
            if self.check("[") {
                self.bump();
                if !self.check("]") {
                    self.parse_expression()?;
                }
                self.expect("]")?;
            }
            return self.parse_initializer();
        }
        if self.check_keyword("typeof")
            || self.check_keyword("sizeof")
            || self.check_keyword("default")
            || self.check_word("nameof")
            || self.check_keyword("checked")
            || self.check_keyword("unchecked")
        {
            self.bump();
            self.expect("(")?;
            if self.check(")") {
                self.bump();
                return Ok(());
            }
            // The argument is either a type or an expression; try the type
            // form first because these operators accept both.
            let saved = self.position;
            if self.parse_type().is_err() || !self.check(")") {
                self.position = saved;
                self.parse_expression()?;
                while self.eat(",") {
                    self.parse_expression()?;
                }
            }
            return self.expect(")");
        }
        if self.check_keyword("delegate") {
            self.bump();
            if self.check("(") {
                self.parse_parameter_list()?;
            }
            return self.parse_block();
        }
        if self.check_keyword("this") || self.check_keyword("base") {
            self.bump();
            return Ok(());
        }
        if self.check_keyword("throw") {
            // Throw expression, e.g. `value ?? throw new ArgumentNullException()`.
            self.bump();
            return self.parse_expression();
        }
        if self.kind() == TokenKind::Keyword && PREDEFINED_TYPES.contains(&self.text()) {
            // A predefined type used as an expression receiver, e.g. `string.Empty`.
            self.bump();
            return Ok(());
        }
        if matches!(
            self.kind(),
            TokenKind::Number | TokenKind::String | TokenKind::Char
        ) || self.check_keyword("true")
            || self.check_keyword("false")
            || self.check_keyword("null")
        {
            self.bump();
            return Ok(());
        }
        if self.check_identifier() {
            self.bump();
            return Ok(());
        }
        if self.kind() == TokenKind::Keyword {
            // Predefined type used as an expression (e.g. `default(int)` was
            // handled above); anything else here is unexpected.
            return Err(self.error(format!(
                "unexpected keyword '{}' in an expression",
                self.text()
            )));
        }
        Err(self.error(format!("unexpected token '{}'", self.text())))
    }

    fn looks_like_lambda(&mut self) -> bool {
        let saved = self.position;
        let result = (|| -> Result<bool, SyntaxError> {
            if self.check_identifier() {
                self.bump();
                if self.check("=>") {
                    return Ok(true);
                }
                // `(a, b) =>` and `(a) =>` forms are handled by the caller.
                return Ok(false);
            }
            if !self.check("(") {
                return Ok(false);
            }
            let mut depth = 0usize;
            loop {
                if self.at_end() {
                    return Ok(false);
                }
                if self.check("(") {
                    depth += 1;
                } else if self.check(")") {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return Ok(self.check("=>"));
                    }
                }
                self.bump();
            }
        })()
        .unwrap_or(false);
        self.position = saved;
        result
    }

    /// Pattern forms that appear after `is` and in switch arms.
    fn parse_pattern(&mut self) -> Result<(), SyntaxError> {
        if self.check("(") {
            // Parenthesized pattern: `is not (A or B)`.
            self.bump();
            self.parse_pattern()?;
            while self.check_word("and") || self.check_word("or") {
                self.bump();
                self.parse_pattern()?;
            }
            return self.expect(")");
        }
        if self.check("{") {
            // Property pattern: `is { Count: > 0 }`.
            self.bump();
            while !self.check("}") && !self.at_end() {
                if self.check_identifier() {
                    self.bump();
                    if self.eat(":") {
                        self.parse_pattern()?;
                    }
                    if !self.eat(",") {
                        break;
                    }
                    continue;
                }
                self.parse_pattern()?;
                if !self.eat(",") {
                    break;
                }
            }
            return self.expect("}");
        }
        if matches!(self.text(), "<" | ">" | "<=" | ">=") {
            // Relational pattern: `is >= 0`.
            self.bump();
            return self.parse_unary();
        }
        if self.check("_") {
            self.bump();
            return Ok(());
        }
        let saved = self.position;
        if self.parse_is_type().is_ok() {
            // `or`/`and`/`when` are contextual identifiers, not variable names.
            let had_name = self.check_identifier() && !matches!(self.text(), "or" | "and" | "when");
            if had_name {
                self.bump();
            }
            if had_name
                || matches!(
                    self.text(),
                    "=>" | ")"
                        | "]"
                        | ";"
                        | ","
                        | ":"
                        | "}"
                        | "&&"
                        | "||"
                        | "?"
                        | "=="
                        | "!="
                        | "is"
                )
                || self.check_word("when")
                || self.check_word("and")
                || self.check_word("or")
            {
                return Ok(());
            }
        }
        self.position = saved;
        self.parse_expression()?;
        Ok(())
    }

    fn parse_lambda(&mut self) -> Result<(), SyntaxError> {
        if self.check("(") {
            self.parse_lambda_parameter_list()?;
        } else {
            self.expect_identifier()?;
        }
        self.expect("=>")?;
        if self.check("{") {
            return self.parse_block();
        }
        self.parse_expression()
    }

    /// Lambda parameters may be implicitly typed (`(x, y) =>`), which the
    /// ordinary parameter grammar rejects.
    fn parse_lambda_parameter_list(&mut self) -> Result<(), SyntaxError> {
        self.expect("(")?;
        if !self.check(")") {
            loop {
                self.parse_attribute_lists()?;
                let saved = self.position;
                self.parse_modifiers();
                self.eat_keyword("ref");
                self.eat_keyword("out");
                self.eat_keyword("in");
                let typed = match self.parse_type() {
                    Ok(()) => self.check_identifier(),
                    Err(_) => false,
                };
                if typed {
                    self.bump();
                } else {
                    self.position = saved;
                    self.expect_identifier()?;
                }
                if self.eat("=") {
                    self.parse_expression()?;
                }
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.expect(")")
    }

    fn parse_new(&mut self) -> Result<(), SyntaxError> {
        self.expect_keyword("new")?;
        if self.check("{") {
            // Anonymous object: `new { trackId, channel = value }`.
            self.bump();
            while !self.check("}") && !self.at_end() {
                self.expect_identifier()?;
                if self.eat("=") {
                    self.parse_variable_initializer()?;
                }
                if !self.eat(",") {
                    break;
                }
            }
            return self.expect("}");
        }
        if self.check("[") {
            self.bump();
            while self.eat(",") {}
            self.expect("]")?;
            return self.parse_initializer();
        }
        if self.check("(") {
            // Tuple-typed array or target-typed new: `new (string, int)[] { ... }`
            // versus `new(4)`.
            let saved = self.position;
            if self.parse_type().is_ok() && matches!(self.text(), "[" | "{") {
                if self.check("[") {
                    loop {
                        self.bump();
                        if !self.check("]") {
                            loop {
                                self.parse_expression()?;
                                if !self.eat(",") {
                                    break;
                                }
                            }
                        }
                        self.expect("]")?;
                        if !self.check("[") {
                            break;
                        }
                    }
                }
                return self.parse_initializer();
            }
            self.position = saved;
            self.bump();
            self.parse_argument_list_body()?;
            self.expect(")")?;
            return self.parse_initializer();
        }
        self.parse_type()?;
        if self.check("(") {
            self.bump();
            self.parse_argument_list_body()?;
            self.expect(")")?;
            return self.parse_initializer();
        }
        if self.check("[") {
            loop {
                self.bump();
                if !self.check("]") {
                    loop {
                        self.parse_expression()?;
                        if !self.eat(",") {
                            break;
                        }
                    }
                }
                self.expect("]")?;
                if !self.check("[") {
                    break;
                }
            }
            return self.parse_initializer();
        }
        self.parse_initializer()
    }

    fn parse_initializer(&mut self) -> Result<(), SyntaxError> {
        if !self.check("{") {
            return Ok(());
        }
        self.bump();
        while !self.check("}") && !self.at_end() {
            if self.check("{") {
                self.parse_initializer()?;
            } else {
                if self.check("[") && self.peek_text(1) != Some("]") {
                    self.bump();
                    self.parse_expression()?;
                    while self.eat(",") {
                        self.parse_expression()?;
                    }
                    self.expect("]")?;
                    self.expect("=")?;
                } else if self.check_identifier() && self.peek_text(1) == Some("=") {
                    self.bump();
                    self.bump();
                }
                // Nested object initializer: `style = { marginTop = 4 }`.
                self.parse_variable_initializer()?;
            }
            if !self.eat(",") {
                break;
            }
        }
        self.eat(",");
        self.expect("}")
    }

    fn skip_to_semicolon(&mut self) -> Result<(), SyntaxError> {
        while !self.at_end() && !self.check(";") {
            self.bump();
        }
        self.expect(";")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepts(source: &str) -> CompilationUnit {
        parse(source).unwrap_or_else(|error| panic!("{error} in:\n{source}"))
    }

    fn rejects(source: &str) {
        assert!(parse(source).is_err(), "unexpectedly accepted:\n{source}");
    }

    #[test]
    fn parses_a_typical_unity_script() {
        let source = r#"
using System;
using UnityEngine;

namespace Game.Player
{
    [Serializable]
    public sealed class PlayerController : MonoBehaviour
    {
        [SerializeField] private float speed = 5.0f;
        private Rigidbody body;

        public int Health { get; private set; } = 100;

        private void Awake()
        {
            body = GetComponent<Rigidbody>();
            Health = Math.Max(1, Health);
        }

        private void Update()
        {
            var horizontal = Input.GetAxis("Horizontal");
            var vertical = Input.GetAxis("Vertical");
            var direction = new Vector3(horizontal, 0, vertical);
            if (direction.sqrMagnitude > 0.01f && body != null)
            {
                body.AddForce(direction * speed, ForceMode.VelocityChange);
            }
        }

        public void TakeDamage(int amount)
        {
            Health -= amount;
            if (Health <= 0)
            {
                Die();
            }
        }

        private void Die() => Debug.Log("dead");
    }
}
"#;
        let unit = accepts(source);
        assert_eq!(unit.types, 1);
        assert!(unit.members >= 5);
    }

    #[test]
    fn parses_file_scoped_namespace_and_generics() {
        let source = r#"
using System.Collections.Generic;
namespace Game;

public class Pool<T> where T : class, new()
{
    private readonly List<T> items = new List<T>();
    public T Get() => items.Count > 0 ? items[0] : new T();
}
"#;
        assert_eq!(accepts(source).types, 1);
    }

    #[test]
    fn parses_control_flow_and_patterns() {
        let source = r#"
class Flow
{
    static int Run(object value)
    {
        var total = 0;
        for (var i = 0; i < 10; i++)
        {
            total += i;
        }
        foreach (var item in new[] { 1, 2, 3 })
        {
            total += item;
        }
        switch (value)
        {
            case int number:
                return number;
            case string text when text.Length > 0:
                return text.Length;
            default:
                break;
        }
        try
        {
            total /= 2;
        }
        catch (DivideByZeroException)
        {
            throw;
        }
        finally
        {
            total = 0;
        }
        return total;
    }
}
"#;
        assert_eq!(accepts(source).types, 1);
    }

    #[test]
    fn parses_conditional_compilation() {
        let source = r#"
class Platform
{
    void Run()
    {
#if UNITY_EDITOR
        Debug.Log("editor");
#else
        Debug.Log("player");
#endif
    }
}
"#;
        assert_eq!(accepts(source).types, 1);
    }

    #[test]
    fn parses_lambdas_and_generic_invocations() {
        let source = r#"
class Lambdas
{
    void Run()
    {
        var items = GetComponents<Renderer>();
        items.ForEach(item => item.enabled = false);
        System.Action<int> action = value => value + 1;
        action(1);
    }
}
"#;
        accepts(source);
    }

    #[test]
    fn parses_operators_and_indexers() {
        let source = r#"
struct Vector
{
    private int[] data;
    public int this[int index] => data[index];
    public static Vector operator +(Vector left, Vector right) => left;
    public static implicit operator int(Vector value) => value.data.Length;
}
"#;
        accepts(source);
    }

    #[test]
    fn rejects_missing_brace() {
        rejects("class Broken { void Run() { }");
    }

    #[test]
    fn rejects_missing_semicolon() {
        rejects("class Broken { void Run() { var x = 1 } }");
    }

    #[test]
    fn rejects_unbalanced_parentheses() {
        rejects("class Broken { void Run( { } }");
    }

    #[test]
    fn rejects_dangling_expression() {
        rejects("class Broken { void Run() { value + ; } }");
    }

    #[test]
    fn reports_the_offending_offset() {
        let source = "class A { void M() { var x = ; } }";
        let error = parse(source).unwrap_err();
        assert_eq!(&source[error.offset..error.offset + 1], ";");
    }
}
