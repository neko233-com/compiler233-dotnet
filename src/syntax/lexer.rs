//! C# lexer: trivia, comments, literals and preprocessor directives.
use super::SyntaxError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Identifier,
    Keyword,
    Number,
    String,
    Char,
    Punctuation,
    EndOfFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

const KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

/// Punctuation recognized by the lexer, longest first. `>>` is kept as one
/// token; the parser splits it when closing nested type arguments.
const PUNCTUATION: &[&str] = &[
    "<<=", ">>=", "??=", "...", "..", "==", "!=", "<=", ">=", "&&", "||", "++", "--", "+=", "-=",
    "*=", "/=", "%=", "&=", "|=", "^=", "=>", "->", "::", "??", "?.", "<<", "{", "}", "[", "]",
    "(", ")", ";", ":", ",", ".", "?", "+", "-", "*", "/", "%", "&", "|", "^", "!", "~", "=", "<",
    ">", "@",
];

pub fn lex(source: &str) -> Result<Vec<Token>, SyntaxError> {
    lex_with_defines(source, &[])
}

pub fn lex_with_defines(source: &str, defines: &[String]) -> Result<Vec<Token>, SyntaxError> {
    Lexer::with_defines(source, defines).run()
}

struct Conditional {
    parent_active: bool,
    active: bool,
    taken: bool,
    seen_else: bool,
}

struct Lexer<'a> {
    source: &'a str,
    position: usize,
    tokens: Vec<Token>,
    defines: Vec<String>,
    conditionals: Vec<Conditional>,
}

impl<'a> Lexer<'a> {
    fn with_defines(source: &'a str, defines: &[String]) -> Self {
        Lexer {
            source,
            position: 0,
            tokens: Vec::new(),
            defines: defines.to_vec(),
            conditionals: Vec::new(),
        }
    }

    fn run(mut self) -> Result<Vec<Token>, SyntaxError> {
        loop {
            self.skip_trivia()?;
            if self.position >= self.source.len() {
                if !self.conditionals.is_empty() {
                    return Err(self.error(self.position, "unterminated #if directive"));
                }
                let end = self.source.len();
                self.tokens.push(Token {
                    kind: TokenKind::EndOfFile,
                    start: end,
                    end,
                });
                return Ok(self.tokens);
            }
            let start = self.position;
            let character = self.peek().expect("checked above");
            let kind = if is_identifier_start(character)
                || (character == '@' && self.peek_at(1).is_some_and(is_identifier_start))
            {
                self.lex_identifier()?
            } else if character.is_ascii_digit()
                || (character == '.' && self.peek_at(1).is_some_and(|next| next.is_ascii_digit()))
            {
                self.lex_number()?
            } else if character == '"' {
                self.lex_string(false, false)?
            } else if character == '@' && self.peek_at(1) == Some('"') {
                self.lex_string(true, false)?
            } else if character == '$'
                && (self.peek_at(1) == Some('"')
                    || (self.peek_at(1) == Some('@') && self.peek_at(2) == Some('"')))
            {
                self.lex_string(false, true)?
            } else if character == '\'' {
                self.lex_char()?
            } else {
                self.lex_punctuation()?
            };
            self.tokens.push(Token {
                kind,
                start,
                end: self.position,
            });
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.position..].chars().next()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.source[self.position..].chars().nth(offset)
    }

    fn advance(&mut self) {
        if let Some(character) = self.peek() {
            self.position += character.len_utf8();
        }
    }

    fn error(&self, offset: usize, message: impl Into<String>) -> SyntaxError {
        SyntaxError {
            offset,
            message: message.into(),
        }
    }

    fn active(&self) -> bool {
        self.conditionals.last().is_none_or(|frame| frame.active)
    }

    fn at_line_start(&self) -> bool {
        self.source[..self.position]
            .chars()
            .rev()
            .take_while(|character| *character != '\n')
            .all(|character| character.is_whitespace() || character == '\u{feff}')
    }

    fn skip_trivia(&mut self) -> Result<(), SyntaxError> {
        loop {
            if !self.active() {
                self.skip_inactive()?;
                if self.position >= self.source.len() {
                    return Ok(());
                }
                continue;
            }
            let Some(character) = self.peek() else {
                return Ok(());
            };
            if character.is_whitespace() || character == '\u{feff}' {
                self.advance();
                continue;
            }
            if character == '/' && self.peek_at(1) == Some('/') {
                while let Some(current) = self.peek() {
                    if current == '\n' {
                        break;
                    }
                    self.advance();
                }
                continue;
            }
            if character == '/' && self.peek_at(1) == Some('*') {
                let start = self.position;
                self.advance();
                self.advance();
                loop {
                    match self.peek() {
                        None => return Err(self.error(start, "unterminated block comment")),
                        Some('*') if self.peek_at(1) == Some('/') => {
                            self.advance();
                            self.advance();
                            break;
                        }
                        _ => self.advance(),
                    }
                }
                continue;
            }
            if character == '#' && self.at_line_start() {
                self.process_directive()?;
                continue;
            }
            return Ok(());
        }
    }

    /// Skips the body of an inactive `#if` region while still tracking nested
    /// directives, so `#endif` and `#elif` are matched correctly.
    fn skip_inactive(&mut self) -> Result<(), SyntaxError> {
        while self.position < self.source.len() {
            if self.peek() == Some('#') && self.at_line_start() {
                self.process_directive()?;
                if self.active() {
                    return Ok(());
                }
                continue;
            }
            self.advance();
        }
        Ok(())
    }

    fn process_directive(&mut self) -> Result<(), SyntaxError> {
        let source = self.source;
        let start = self.position;
        let end = source[start..]
            .find('\n')
            .map_or(source.len(), |offset| start + offset);
        let line = &source[start..end];
        self.position = end;
        let mut words = line[1..].split_whitespace();
        let name = words.next().unwrap_or_default();
        let rest: Vec<&str> = words.collect();
        match name {
            "if" => {
                let parent_active = self.active();
                let condition = self.evaluate_condition(&rest)?;
                let active = parent_active && condition;
                self.conditionals.push(Conditional {
                    parent_active,
                    active,
                    taken: active,
                    seen_else: false,
                });
            }
            "elif" => {
                let Some((parent_active, taken, seen_else)) = self
                    .conditionals
                    .last()
                    .map(|frame| (frame.parent_active, frame.taken, frame.seen_else))
                else {
                    return Err(self.error(start, "#elif without a matching #if"));
                };
                if seen_else {
                    return Err(self.error(start, "#elif after #else"));
                }
                let condition = self.evaluate_condition_with_parent(&rest, parent_active, taken)?;
                if let Some(frame) = self.conditionals.last_mut() {
                    frame.active = parent_active && !taken && condition;
                    frame.taken = frame.taken || frame.active;
                }
            }
            "else" => {
                let Some(seen_else) = self.conditionals.last().map(|frame| frame.seen_else) else {
                    return Err(self.error(start, "#else without a matching #if"));
                };
                if seen_else {
                    return Err(self.error(start, "duplicate #else"));
                }
                if let Some(frame) = self.conditionals.last_mut() {
                    frame.seen_else = true;
                    frame.active = frame.parent_active && !frame.taken;
                    frame.taken = true;
                }
            }
            "endif" => {
                if self.conditionals.pop().is_none() {
                    return Err(self.error(start, "#endif without a matching #if"));
                }
            }
            "define" => {
                if self.active()
                    && let Some(symbol) = rest.first()
                {
                    self.defines.push((*symbol).to_string());
                }
            }
            "undef" => {
                if self.active()
                    && let Some(symbol) = rest.first()
                {
                    self.defines.retain(|existing| existing != symbol);
                }
            }
            "error" if self.active() => {
                return Err(self.error(start, format!("#error {}", rest.join(" "))));
            }
            _ => {}
        }
        Ok(())
    }

    fn evaluate_condition(&self, words: &[&str]) -> Result<bool, SyntaxError> {
        self.evaluate_condition_with_parent(words, true, false)
    }

    fn evaluate_condition_with_parent(
        &self,
        words: &[&str],
        parent_active: bool,
        taken: bool,
    ) -> Result<bool, SyntaxError> {
        if !parent_active || taken {
            return Ok(false);
        }
        let joined = words.join(" ");
        let tokens = split_condition(&joined);
        if tokens.is_empty() {
            return Ok(false);
        }
        let mut position = 0;
        let value = evaluate_or(&tokens, &mut position, &self.defines);
        Ok(value)
    }

    fn lex_identifier(&mut self) -> Result<TokenKind, SyntaxError> {
        let start = self.position;
        let verbatim = self.peek() == Some('@');
        if verbatim {
            self.advance();
        }
        let body_start = self.position;
        while let Some(character) = self.peek() {
            if is_identifier_part(character) {
                self.advance();
            } else {
                break;
            }
        }
        if self.position == body_start {
            return Err(self.error(start, "expected an identifier after '@'"));
        }
        if verbatim {
            return Ok(TokenKind::Identifier);
        }
        let text = &self.source[start..self.position];
        if KEYWORDS.contains(&text) {
            Ok(TokenKind::Keyword)
        } else {
            Ok(TokenKind::Identifier)
        }
    }

    fn lex_number(&mut self) -> Result<TokenKind, SyntaxError> {
        let start = self.position;
        if self.peek() == Some('.') {
            // Leading-dot real literal such as `.5f`.
            self.advance();
            self.consume_digits(|character| character.is_ascii_digit());
            if matches!(self.peek(), Some('e') | Some('E')) {
                self.advance();
                if matches!(self.peek(), Some('-') | Some('+')) {
                    self.advance();
                }
                self.consume_digits(|character| character.is_ascii_digit());
            }
            if matches!(
                self.peek(),
                Some('f') | Some('F') | Some('d') | Some('D') | Some('m') | Some('M')
            ) {
                self.advance();
            }
            return Ok(TokenKind::Number);
        }
        if self.peek() == Some('0') {
            match self.peek_at(1) {
                Some('x') | Some('X') => {
                    self.advance();
                    self.advance();
                    self.consume_digits(|character| character.is_ascii_hexdigit());
                    self.consume_integer_suffix();
                    return Ok(TokenKind::Number);
                }
                Some('b') | Some('B') => {
                    self.advance();
                    self.advance();
                    self.consume_digits(|character| matches!(character, '0' | '1'));
                    self.consume_integer_suffix();
                    return Ok(TokenKind::Number);
                }
                _ => {}
            }
        }
        self.consume_digits(|character| character.is_ascii_digit());
        let mut is_real = false;
        if self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            is_real = true;
            self.advance();
            self.consume_digits(|character| character.is_ascii_digit());
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            let sign = matches!(self.peek_at(1), Some('-') | Some('+'));
            let digit_offset = if sign { 2 } else { 1 };
            if self
                .peek_at(digit_offset)
                .is_some_and(|c| c.is_ascii_digit())
            {
                is_real = true;
                self.advance();
                if sign {
                    self.advance();
                }
                self.consume_digits(|character| character.is_ascii_digit());
            }
        }
        let suffix = self.peek().unwrap_or(' ');
        if is_real {
            if matches!(suffix, 'f' | 'F' | 'd' | 'D' | 'm' | 'M') {
                self.advance();
            }
        } else {
            match suffix {
                'u' | 'U' => {
                    self.advance();
                    if matches!(self.peek(), Some('l') | Some('L')) {
                        self.advance();
                    }
                }
                'l' | 'L' => {
                    self.advance();
                    if matches!(self.peek(), Some('u') | Some('U')) {
                        self.advance();
                    }
                }
                'f' | 'F' | 'd' | 'D' | 'm' | 'M' => {
                    self.advance();
                }
                _ => {}
            }
        }
        if self.position == start {
            return Err(self.error(start, "invalid numeric literal"));
        }
        Ok(TokenKind::Number)
    }

    fn consume_integer_suffix(&mut self) {
        match self.peek() {
            Some('u') | Some('U') => {
                self.advance();
                if matches!(self.peek(), Some('l') | Some('L')) {
                    self.advance();
                }
            }
            Some('l') | Some('L') => {
                self.advance();
                if matches!(self.peek(), Some('u') | Some('U')) {
                    self.advance();
                }
            }
            _ => {}
        }
    }

    fn consume_digits(&mut self, predicate: impl Fn(char) -> bool) {
        while let Some(character) = self.peek() {
            if predicate(character) || character == '_' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn lex_string(&mut self, verbatim: bool, interpolated: bool) -> Result<TokenKind, SyntaxError> {
        let start = self.position;
        let mut verbatim = verbatim;
        if interpolated {
            self.advance();
        }
        if self.peek() == Some('@') {
            // `$@"..."` is verbatim as well as interpolated.
            self.advance();
            verbatim = true;
        }
        if self.peek() != Some('"') {
            return Err(self.error(start, "expected a string literal"));
        }
        self.advance();
        loop {
            let Some(character) = self.peek() else {
                return Err(self.error(start, "unterminated string literal"));
            };
            match character {
                '"' => {
                    self.advance();
                    if !verbatim {
                        return Ok(TokenKind::String);
                    }
                    if self.peek() == Some('"') {
                        self.advance();
                        continue;
                    }
                    return Ok(TokenKind::String);
                }
                '\\' if !verbatim => {
                    self.advance();
                    if self.peek().is_none() {
                        return Err(self.error(start, "unterminated string literal"));
                    }
                    self.advance();
                }
                '\n' if !verbatim => {
                    return Err(self.error(start, "newline in string literal"));
                }
                '{' if interpolated => {
                    self.advance();
                    if self.peek() == Some('{') {
                        self.advance();
                    } else {
                        self.skip_interpolation_hole(start)?;
                    }
                }
                '}' if interpolated && self.peek_at(1) == Some('}') => {
                    self.advance();
                    self.advance();
                }
                _ => self.advance(),
            }
        }
    }

    fn skip_interpolation_hole(&mut self, start: usize) -> Result<(), SyntaxError> {
        let mut depth = 1usize;
        loop {
            let Some(character) = self.peek() else {
                return Err(self.error(start, "unterminated interpolation hole"));
            };
            match character {
                '"' => {
                    self.skip_nested_string(start)?;
                }
                '\'' => {
                    self.advance();
                    while let Some(current) = self.peek() {
                        if current == '\\' {
                            self.advance();
                            self.advance();
                            continue;
                        }
                        self.advance();
                        if current == '\'' {
                            break;
                        }
                    }
                }
                '{' => {
                    depth += 1;
                    self.advance();
                }
                '}' => {
                    depth -= 1;
                    self.advance();
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => self.advance(),
            }
        }
    }

    fn skip_nested_string(&mut self, start: usize) -> Result<(), SyntaxError> {
        self.advance();
        loop {
            let Some(character) = self.peek() else {
                return Err(self.error(start, "unterminated string inside interpolation"));
            };
            self.advance();
            match character {
                '\\' => self.advance(),
                '"' => return Ok(()),
                _ => {}
            }
        }
    }

    fn lex_char(&mut self) -> Result<TokenKind, SyntaxError> {
        let start = self.position;
        self.advance();
        loop {
            let Some(character) = self.peek() else {
                return Err(self.error(start, "unterminated character literal"));
            };
            if character == '\n' {
                return Err(self.error(start, "newline in character literal"));
            }
            self.advance();
            if character == '\\' {
                if self.peek().is_none() {
                    return Err(self.error(start, "unterminated character literal"));
                }
                self.advance();
                continue;
            }
            if character == '\'' {
                return Ok(TokenKind::Char);
            }
        }
    }

    fn lex_punctuation(&mut self) -> Result<TokenKind, SyntaxError> {
        let start = self.position;
        for candidate in PUNCTUATION {
            if self.source[start..].starts_with(candidate) {
                for _ in 0..candidate.chars().count() {
                    self.advance();
                }
                return Ok(TokenKind::Punctuation);
            }
        }
        Err(self.error(
            start,
            format!("unexpected character '{}'", self.peek().unwrap_or('?')),
        ))
    }
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_part(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn split_condition(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character.is_whitespace() {
            continue;
        }
        if is_identifier_start(character) {
            let mut identifier = String::from(character);
            while let Some(next) = characters.peek() {
                if is_identifier_part(*next) {
                    identifier.push(*next);
                    characters.next();
                } else {
                    break;
                }
            }
            tokens.push(identifier);
            continue;
        }
        let pair = if character == '&' && characters.peek() == Some(&'&') {
            characters.next();
            "&&".to_string()
        } else if character == '|' && characters.peek() == Some(&'|') {
            characters.next();
            "||".to_string()
        } else if (character == '=' || character == '!') && characters.peek() == Some(&'=') {
            characters.next();
            format!("{character}=")
        } else {
            character.to_string()
        };
        tokens.push(pair);
    }
    tokens
}

fn evaluate_or(tokens: &[String], position: &mut usize, defines: &[String]) -> bool {
    let mut value = evaluate_and(tokens, position, defines);
    while tokens.get(*position).map(String::as_str) == Some("||") {
        *position += 1;
        let right = evaluate_and(tokens, position, defines);
        value = value || right;
    }
    value
}

fn evaluate_and(tokens: &[String], position: &mut usize, defines: &[String]) -> bool {
    let mut value = evaluate_equality(tokens, position, defines);
    while tokens.get(*position).map(String::as_str) == Some("&&") {
        *position += 1;
        let right = evaluate_equality(tokens, position, defines);
        value = value && right;
    }
    value
}

fn evaluate_equality(tokens: &[String], position: &mut usize, defines: &[String]) -> bool {
    let mut value = evaluate_unary(tokens, position, defines);
    loop {
        match tokens.get(*position).map(String::as_str) {
            Some("==") => {
                *position += 1;
                value = value == evaluate_unary(tokens, position, defines);
            }
            Some("!=") => {
                *position += 1;
                value = value != evaluate_unary(tokens, position, defines);
            }
            _ => return value,
        }
    }
}

fn evaluate_unary(tokens: &[String], position: &mut usize, defines: &[String]) -> bool {
    match tokens.get(*position).map(String::as_str) {
        Some("!") => {
            *position += 1;
            !evaluate_unary(tokens, position, defines)
        }
        Some("(") => {
            *position += 1;
            let value = evaluate_or(tokens, position, defines);
            if tokens.get(*position).map(String::as_str) == Some(")") {
                *position += 1;
            }
            value
        }
        Some("true") => {
            *position += 1;
            true
        }
        Some("false") => {
            *position += 1;
            false
        }
        Some(symbol) => {
            *position += 1;
            defines.iter().any(|define| define == symbol)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        lex(source)
            .expect("valid source")
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn classifies_identifiers_and_keywords() {
        let tokens = lex("class Foo { int bar; }").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Keyword);
        assert_eq!(tokens[1].kind, TokenKind::Identifier);
        assert_eq!(tokens[2].kind, TokenKind::Punctuation);
        assert_eq!(tokens[3].kind, TokenKind::Keyword);
        assert_eq!(tokens.last().unwrap().kind, TokenKind::EndOfFile);
    }

    #[test]
    fn verbatim_identifier_is_not_a_keyword() {
        let tokens = lex("@class").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Identifier);
        assert_eq!(tokens[0].start, 0);
        assert_eq!(tokens[0].end, 6);
    }

    #[test]
    fn lexes_numbers_with_suffixes() {
        assert_eq!(
            kinds("0x1F 0b1010 1_000 1.5f 2e10 3UL"),
            vec![
                TokenKind::Number,
                TokenKind::Number,
                TokenKind::Number,
                TokenKind::Number,
                TokenKind::Number,
                TokenKind::Number,
                TokenKind::EndOfFile
            ]
        );
    }

    #[test]
    fn member_access_after_number_is_not_a_decimal_point() {
        let tokens = lex("1.ToString()").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Number);
        assert_eq!(&tokens[1].kind, &TokenKind::Punctuation);
    }

    #[test]
    fn lexes_strings_including_verbatim_and_interpolated() {
        assert_eq!(
            kinds(r#""a\"b" @"c""d" $"x{y}z" $@"{p}""#),
            vec![
                TokenKind::String,
                TokenKind::String,
                TokenKind::String,
                TokenKind::String,
                TokenKind::EndOfFile
            ]
        );
    }

    #[test]
    fn reports_unterminated_string() {
        let error = lex("\"abc").unwrap_err();
        assert!(error.message.contains("unterminated string"));
    }

    #[test]
    fn skips_comments() {
        assert_eq!(
            kinds("// a\n/* b */ class C"),
            vec![
                TokenKind::Keyword,
                TokenKind::Identifier,
                TokenKind::EndOfFile
            ]
        );
    }

    #[test]
    fn respects_conditional_compilation() {
        let source = "#if UNITY_EDITOR\nclass A {}\n#else\nclass B {}\n#endif\n";
        let tokens = lex(source).unwrap();
        let names: Vec<&str> = tokens
            .iter()
            .filter(|token| token.kind == TokenKind::Identifier)
            .map(|token| &source[token.start..token.end])
            .collect();
        assert_eq!(names, vec!["B"]);
    }

    #[test]
    fn define_changes_conditional_compilation() {
        let source = "#define UNITY_EDITOR\n#if UNITY_EDITOR\nclass A {}\n#endif\n";
        let tokens = lex(source).unwrap();
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == TokenKind::Identifier)
        );
    }

    #[test]
    fn supports_negated_and_nested_conditions() {
        let source =
            "#if !DEBUG\n#if DEBUG\nclass Wrong {}\n#else\nclass Right {}\n#endif\n#endif\n";
        let tokens = lex(source).unwrap();
        let name = tokens
            .iter()
            .find(|token| token.kind == TokenKind::Identifier)
            .map(|token| &source[token.start..token.end]);
        assert_eq!(name, Some("Right"));
    }

    #[test]
    fn reports_unterminated_conditional() {
        let error = lex("#if DEBUG\nclass A {}\n").unwrap_err();
        assert!(error.message.contains("unterminated #if"));
    }

    #[test]
    fn lexes_shift_as_two_adjacent_tokens() {
        // `>>` stays two tokens so nested type arguments close unambiguously;
        // the parser recombines adjacent tokens into a shift.
        let tokens = lex("a >> b").unwrap();
        assert_eq!(&"a >> b"[tokens[1].start..tokens[1].end], ">");
        assert_eq!(&"a >> b"[tokens[2].start..tokens[2].end], ">");
        assert_eq!(tokens[1].end, tokens[2].start);
    }
}
