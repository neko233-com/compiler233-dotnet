//! Native C# syntax front end.
//!
//! The lexer handles trivia, comments and preprocessor directives (including
//! conditional compilation); the parser validates the declaration, statement
//! and expression grammar. Both fail with an offset instead of guessing, so a
//! caller never mistakes a partially understood file for a correct one.
pub mod lexer;
pub mod parser;

use std::fmt;

pub use lexer::{Token, TokenKind, lex, lex_with_defines};
pub use parser::{CompilationUnit, parse, parse_with_defines};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "byte {}: {}", self.offset, self.message)
    }
}

impl SyntaxError {
    pub fn line_column(&self, source: &str) -> (usize, usize) {
        let end = self.offset.min(source.len());
        let prefix = &source[..end];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = match prefix.rfind('\n') {
            Some(index) => prefix.len() - index,
            None => prefix.len() + 1,
        };
        (line, column)
    }
}
