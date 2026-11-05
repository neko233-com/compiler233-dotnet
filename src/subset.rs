//! Native, deliberately restricted C# parser. Unsupported syntax is an error.
//! The token stream retains all non-trivia token ranges; nodes form a postorder
//! arena with child indices. The original source supplies token and trivia text.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Class,
    Static,
    Int,
    Return,
    Identifier,
    Number,
    LBrace,
    RBrace,
    LParen,
    RParen,
    Comma,
    Semicolon,
    Plus,
    Minus,
    Star,
    Slash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    CompilationUnit,
    Class,
    Method,
    Parameter,
    Return,
    Binary,
    Identifier,
    Number,
    Parenthesized,
}

#[derive(Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub first_token: usize,
    pub end_token: usize,
    pub first_child: usize,
    pub child_count: usize,
}

#[derive(Debug)]
pub struct SyntaxTree {
    pub tokens: Vec<Token>,
    pub nodes: Vec<Node>,
    pub children: Vec<usize>,
    pub root: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub message: &'static str,
}

fn reserved(s: &str) -> bool {
    matches!(
        s,
        "__arglist"
            | "__makeref"
            | "__reftype"
            | "__refvalue"
            | "abstract"
            | "as"
            | "base"
            | "bool"
            | "break"
            | "byte"
            | "case"
            | "catch"
            | "char"
            | "checked"
            | "class"
            | "const"
            | "continue"
            | "decimal"
            | "default"
            | "delegate"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "event"
            | "explicit"
            | "extern"
            | "false"
            | "finally"
            | "fixed"
            | "float"
            | "for"
            | "foreach"
            | "goto"
            | "if"
            | "implicit"
            | "in"
            | "int"
            | "interface"
            | "internal"
            | "is"
            | "lock"
            | "long"
            | "namespace"
            | "new"
            | "null"
            | "object"
            | "operator"
            | "out"
            | "override"
            | "params"
            | "private"
            | "protected"
            | "public"
            | "readonly"
            | "ref"
            | "return"
            | "sbyte"
            | "sealed"
            | "short"
            | "sizeof"
            | "stackalloc"
            | "static"
            | "string"
            | "struct"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "uint"
            | "ulong"
            | "unchecked"
            | "unsafe"
            | "ushort"
            | "using"
            | "virtual"
            | "void"
            | "volatile"
            | "while"
    )
}

fn lex(source: &str) -> Result<Vec<Token>, Error> {
    if let Some(offset) = source.bytes().position(|b| !b.is_ascii()) {
        return Err(Error {
            offset,
            message: "non-ASCII source is outside the supported subset",
        });
    }
    let bytes = source.as_bytes();
    let mut tokens = Vec::with_capacity(bytes.len() / 4);
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let kind = match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                if i + 1 >= bytes.len() {
                    return Err(Error {
                        offset: start,
                        message: "unterminated comment",
                    });
                }
                i += 2;
                continue;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                match &source[start..i] {
                    "class" => TokenKind::Class,
                    "static" => TokenKind::Static,
                    "int" => TokenKind::Int,
                    "return" => TokenKind::Return,
                    s if reserved(s) => {
                        return Err(Error {
                            offset: start,
                            message: "unsupported keyword",
                        });
                    }
                    _ => TokenKind::Identifier,
                }
            }
            b'0'..=b'9' => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if source[start..i].parse::<u64>().is_err() {
                    return Err(Error {
                        offset: start,
                        message: "integer literal exceeds u64",
                    });
                }
                TokenKind::Number
            }
            b'{' => {
                i += 1;
                TokenKind::LBrace
            }
            b'}' => {
                i += 1;
                TokenKind::RBrace
            }
            b'(' => {
                i += 1;
                TokenKind::LParen
            }
            b')' => {
                i += 1;
                TokenKind::RParen
            }
            b',' => {
                i += 1;
                TokenKind::Comma
            }
            b';' => {
                i += 1;
                TokenKind::Semicolon
            }
            b'+' => {
                i += 1;
                TokenKind::Plus
            }
            b'-' => {
                i += 1;
                TokenKind::Minus
            }
            b'*' => {
                i += 1;
                TokenKind::Star
            }
            b'/' => {
                i += 1;
                TokenKind::Slash
            }
            _ => {
                return Err(Error {
                    offset: i,
                    message: "unsupported character or syntax",
                });
            }
        };
        tokens.push(Token {
            kind,
            start,
            end: i,
        });
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    nodes: Vec<Node>,
    children: Vec<usize>,
    pos: usize,
    source_len: usize,
}
impl Parser {
    fn error(&self) -> Error {
        Error {
            offset: self
                .tokens
                .get(self.pos)
                .map_or(self.source_len, |t| t.start),
            message: "unexpected token or unsupported syntax",
        }
    }
    fn at(&self, kind: TokenKind) -> bool {
        self.tokens.get(self.pos).is_some_and(|t| t.kind == kind)
    }
    fn take(&mut self, kind: TokenKind) -> Result<(), Error> {
        if !self.at(kind) {
            return Err(self.error());
        }
        self.pos += 1;
        Ok(())
    }
    fn node(&mut self, kind: NodeKind, start: usize, children: &[usize]) -> usize {
        let first_child = self.children.len();
        self.children.extend_from_slice(children);
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            first_token: start,
            end_token: self.pos,
            first_child,
            child_count: children.len(),
        });
        id
    }
    fn expression(&mut self, precedence: u8, depth: usize) -> Result<usize, Error> {
        if depth > 256 {
            return Err(self.error());
        }
        let start = self.pos;
        let mut left = if self.at(TokenKind::Identifier) {
            self.pos += 1;
            self.node(NodeKind::Identifier, start, &[])
        } else if self.at(TokenKind::Number) {
            self.pos += 1;
            self.node(NodeKind::Number, start, &[])
        } else if self.at(TokenKind::LParen) {
            self.pos += 1;
            let inner = self.expression(0, depth + 1)?;
            self.take(TokenKind::RParen)?;
            self.node(NodeKind::Parenthesized, start, &[inner])
        } else {
            return Err(self.error());
        };
        loop {
            let op_precedence = match self.tokens.get(self.pos).map(|t| t.kind) {
                Some(TokenKind::Plus | TokenKind::Minus) => 1,
                Some(TokenKind::Star | TokenKind::Slash) => 2,
                _ => break,
            };
            if op_precedence < precedence {
                break;
            }
            self.pos += 1;
            let right = self.expression(op_precedence + 1, depth + 1)?;
            left = self.node(NodeKind::Binary, start, &[left, right]);
        }
        Ok(left)
    }
    fn method(&mut self) -> Result<usize, Error> {
        let start = self.pos;
        self.take(TokenKind::Static)?;
        self.take(TokenKind::Int)?;
        self.take(TokenKind::Identifier)?;
        self.take(TokenKind::LParen)?;
        let mut children = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                let param = self.pos;
                self.take(TokenKind::Int)?;
                self.take(TokenKind::Identifier)?;
                children.push(self.node(NodeKind::Parameter, param, &[]));
                if !self.at(TokenKind::Comma) {
                    break;
                }
                self.pos += 1;
            }
        }
        self.take(TokenKind::RParen)?;
        self.take(TokenKind::LBrace)?;
        let ret = self.pos;
        self.take(TokenKind::Return)?;
        let expr = self.expression(0, 0)?;
        self.take(TokenKind::Semicolon)?;
        children.push(self.node(NodeKind::Return, ret, &[expr]));
        self.take(TokenKind::RBrace)?;
        Ok(self.node(NodeKind::Method, start, &children))
    }
    fn class(&mut self) -> Result<usize, Error> {
        let start = self.pos;
        self.take(TokenKind::Class)?;
        self.take(TokenKind::Identifier)?;
        self.take(TokenKind::LBrace)?;
        let mut methods = Vec::new();
        while !self.at(TokenKind::RBrace) {
            if self.pos == self.tokens.len() {
                return Err(self.error());
            }
            methods.push(self.method()?);
        }
        self.pos += 1;
        Ok(self.node(NodeKind::Class, start, &methods))
    }
}

pub fn parse(source: &str) -> Result<SyntaxTree, Error> {
    let tokens = lex(source)?;
    let capacity = tokens.len();
    let mut parser = Parser {
        tokens,
        nodes: Vec::with_capacity(capacity),
        children: Vec::with_capacity(capacity),
        pos: 0,
        source_len: source.len(),
    };
    let mut classes = Vec::new();
    while parser.pos < parser.tokens.len() {
        classes.push(parser.class()?);
    }
    let root = parser.node(NodeKind::CompilationUnit, 0, &classes);
    Ok(SyntaxTree {
        tokens: parser.tokens,
        nodes: parser.nodes,
        children: parser.children,
        root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_structure_and_precedence() {
        let tree =
            parse("class C { static int F(int a, int b) { return a + b * (2 - 1); } }").unwrap();
        assert_eq!(tree.nodes[tree.root].kind, NodeKind::CompilationUnit);
        assert_eq!(
            tree.nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Binary)
                .count(),
            3
        );
        assert!(tree.nodes.iter().any(|n| n.kind == NodeKind::Parenthesized));
        assert_eq!(tree.nodes[tree.root].end_token, tree.tokens.len());
    }
    #[test]
    fn rejects_invalid_and_unsupported() {
        for source in [
            "class C { static int F() { return 1 + ; } }",
            "class C { static int F(int return) { return 1; } }",
            "public class C {}",
            "class C {/*",
            "class C { static int F() { return @x; } }",
            "class __arglist {}",
            "class C { static int F() { return 18446744073709551616; } }",
            "// comment\u{2028}class C {",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
    }
    #[test]
    fn bounds_nesting() {
        let source = format!(
            "class C {{ static int F() {{ return {}1{}; }} }}",
            "(".repeat(300),
            ")".repeat(300)
        );
        assert!(parse(&source).is_err());
    }
}
