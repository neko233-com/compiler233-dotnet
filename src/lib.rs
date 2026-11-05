//! Experimental C# syntax frontends and limited .NET DLL compilation.
#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
compile_error!("macOS is supported only on ARM64 (aarch64-apple-darwin)");
use tree_sitter::{Node, Parser, Tree};

pub mod compile;
pub mod subset;

pub fn parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .expect("compatible C# grammar");
    parser
}

pub fn parse(parser: &mut Parser, source: &str) -> Tree {
    parser
        .parse(source, None)
        .expect("no cancellation or timeout configured")
}

#[derive(Debug, PartialEq, Eq)]
pub struct SyntaxIssue {
    pub line: usize,
    pub column: usize,
    pub kind: String,
}

pub fn syntax_issues(tree: &Tree) -> Vec<SyntaxIssue> {
    fn visit(node: Node<'_>, issues: &mut Vec<SyntaxIssue>) {
        if node.is_error() || node.is_missing() {
            let position = node.start_position();
            issues.push(SyntaxIssue {
                line: position.row + 1,
                column: position.column + 1,
                kind: if node.is_missing() {
                    format!("missing {}", node.kind())
                } else {
                    "syntax error".into()
                },
            });
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.has_error() || child.is_missing() || child.is_error() {
                visit(child, issues);
            }
        }
    }
    let mut issues = Vec::new();
    visit(tree.root_node(), &mut issues);
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_valid_csharp() {
        let tree = parse(
            &mut parser(),
            "namespace Demo; class Example { static int Add(int a, int b) => a + b; }",
        );
        assert!(!tree.root_node().has_error());
        assert!(syntax_issues(&tree).is_empty());
    }
    #[test]
    fn reports_invalid_csharp() {
        let tree = parse(&mut parser(), "class Example { void M( { }");
        assert!(tree.root_node().has_error());
        assert!(!syntax_issues(&tree).is_empty());
    }
    #[test]
    fn location_is_one_based_and_byte_based() {
        let tree = parse(&mut parser(), "// 中文\nclass A { void M( { }");
        assert!(
            syntax_issues(&tree)
                .iter()
                .all(|d| d.line >= 2 && d.column >= 1)
        );
    }
}
