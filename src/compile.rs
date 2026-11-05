//! A deliberately limited C# to .NET DLL compiler.
//! Unsupported constructs fail closed; this is not yet a Roslyn replacement.
use std::collections::{HashMap, HashSet};

use hao::{
    Module,
    dotnet::{
        entries::{Entry, Method, Param, TypeDef},
        il::{Emitter, OpCode},
        md::streams::tables_stream::{MethodFlags, TypeAttributes},
        module::{ModuleKind, TargetRuntime},
        signature::{MethodSig, TypeSig},
    },
};

use crate::subset::{self, NodeKind, SyntaxTree, TokenKind};

fn children(tree: &SyntaxTree, id: usize) -> &[usize] {
    let node = &tree.nodes[id];
    &tree.children[node.first_child..node.first_child + node.child_count]
}

fn text<'a>(source: &'a str, tree: &SyntaxTree, index: usize) -> &'a str {
    let token = tree.tokens[index];
    &source[token.start..token.end]
}

fn compile_expr(
    source: &str,
    tree: &SyntaxTree,
    root: usize,
    parameters: &HashMap<&str, u16>,
    emitter: &mut Emitter,
) -> Result<(), String> {
    let mut stack = vec![(root, false)];
    let mut values: Vec<Option<i32>> = Vec::new();
    while let Some((id, visited)) = stack.pop() {
        let node = &tree.nodes[id];
        match node.kind {
            NodeKind::Number => {
                let token = text(source, tree, node.first_token);
                let n = token.parse::<i32>().map_err(|_| {
                    format!("integer literal {token} is outside the supported int range")
                })?;
                emitter.ldc_i4(n);
                values.push(Some(n));
            }
            NodeKind::Identifier => {
                let name = text(source, tree, node.first_token);
                let index = parameters
                    .get(name)
                    .ok_or_else(|| format!("undefined identifier '{name}'"))?;
                emitter.ldarg(*index);
                values.push(None);
            }
            NodeKind::Parenthesized => {
                stack.push((children(tree, id)[0], false));
            }
            NodeKind::Binary if !visited => {
                let c = children(tree, id);
                stack.push((id, true));
                stack.push((c[1], false));
                stack.push((c[0], false));
            }
            NodeKind::Binary => {
                let right = values.pop().expect("valid syntax tree");
                let left = values.pop().expect("valid syntax tree");
                let op = tree.tokens[tree.nodes[children(tree, id)[0]].end_token].kind;
                let opcode = match op {
                    TokenKind::Plus => OpCode::Add,
                    TokenKind::Minus => OpCode::Sub,
                    TokenKind::Star => OpCode::Mul,
                    TokenKind::Slash => OpCode::Div,
                    _ => return Err("internal: invalid binary operator".into()),
                };
                let constant = match (left, right) {
                    (Some(a), Some(b)) => {
                        let v = match op {
                            TokenKind::Plus => a.checked_add(b),
                            TokenKind::Minus => a.checked_sub(b),
                            TokenKind::Star => a.checked_mul(b),
                            TokenKind::Slash => a.checked_div(b),
                            _ => unreachable!(),
                        };
                        Some(v.ok_or_else(|| {
                            "constant expression overflows or divides by zero".to_string()
                        })?)
                    }
                    _ => None,
                };
                emitter.op(opcode);
                values.push(constant);
            }
            _ => return Err("internal: invalid expression node".into()),
        }
    }
    Ok(())
}

/// Compile the supported subset into a .NET 10 DLL. No file is written on failure.
pub fn compile(source: &str, assembly_name: &str) -> Result<Vec<u8>, String> {
    if assembly_name.is_empty()
        || !assembly_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
    {
        return Err(
            "assembly name must contain only ASCII letters, digits, '.', '-' or '_'".into(),
        );
    }
    let tree = subset::parse(source).map_err(|e| format!("byte {}: {}", e.offset, e.message))?;
    let mut module = Module::new(assembly_name, ModuleKind::Dll, TargetRuntime::NetCore(10));
    let object = module
        .corlib_type("System.Object")
        .map_err(|e| e.to_string())?;
    let object_ctor = module
        .import_corlib_method(
            "System.Object",
            ".ctor",
            MethodSig::new_instance(TypeSig::VOID, vec![]),
        )
        .map_err(|e| e.to_string())?;
    let mut class_names = HashSet::new();
    for &class_id in children(&tree, tree.root) {
        let class = &tree.nodes[class_id];
        let class_name = text(source, &tree, class.first_token + 1);
        if !class_names.insert(class_name) {
            return Err(format!("duplicate class '{class_name}'"));
        }
        let ty = module.add_type(TypeDef::new(
            TypeAttributes::NotPublic | TypeAttributes::BeforeFieldInit,
            "",
            class_name,
            Some(object.clone()),
        ));
        let mut ctor = Emitter::new();
        ctor.ldarg(0).call(&object_ctor).ret();
        let constructor = Method::new(
            MethodFlags::Public
                | MethodFlags::HideBySig
                | MethodFlags::SpecialName
                | MethodFlags::RTSpecialName,
            ".ctor",
            MethodSig::new_instance(TypeSig::VOID, vec![]),
        )
        .with_body(ctor.finish());
        TypeDef::add_method(&ty, constructor);
        let mut signatures = HashSet::new();
        for &method_id in children(&tree, class_id) {
            let method_node = &tree.nodes[method_id];
            let name = text(source, &tree, method_node.first_token + 2);
            if name == class_name {
                return Err(format!(
                    "member '{name}' cannot have the same name as its enclosing type"
                ));
            }
            let parts = children(&tree, method_id);
            let param_nodes = &parts[..parts.len() - 1];
            if param_nodes.len() > u16::MAX as usize {
                return Err("too many parameters".into());
            }
            if !signatures.insert((name, param_nodes.len())) {
                return Err(format!(
                    "duplicate method '{name}' with {} parameters",
                    param_nodes.len()
                ));
            }
            let mut parameters = HashMap::new();
            let mut params = Vec::new();
            for (index, &param_id) in param_nodes.iter().enumerate() {
                let param_name = text(source, &tree, tree.nodes[param_id].first_token + 1);
                if parameters.contains_key(param_name) {
                    return Err(format!("duplicate parameter '{param_name}'"));
                }
                parameters.insert(param_name, index as u16);
                params.push(Entry::new(Param::new(index as u16 + 1, param_name)));
            }
            let ret = parts[parts.len() - 1];
            let mut emitter = Emitter::new();
            compile_expr(
                source,
                &tree,
                children(&tree, ret)[0],
                &parameters,
                &mut emitter,
            )?;
            emitter.ret();
            let mut method = Method::new(
                MethodFlags::Private | MethodFlags::Static | MethodFlags::HideBySig,
                name,
                MethodSig::new_static(TypeSig::INT32, vec![TypeSig::INT32; param_nodes.len()]),
            )
            .with_body(emitter.finish());
            method.params = params;
            TypeDef::add_method(&ty, method);
        }
    }
    module
        .write()
        .map_err(|e| format!("assembly emission failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::compile;
    #[test]
    fn rejects_semantic_errors() {
        for source in [
            "class C { static int F(int a, int a) { return a; } }",
            "class C { static int F() { return a; } }",
            "class C {} class C {}",
            "class C { static int F() { return 2147483647 + 1; } }",
            "class C { static int F() { return 1 / 0; } }",
            "class C { static int F() { return 1; } static int F() { return 2; } }",
        ] {
            assert!(compile(source, "Test").is_err(), "{source}");
        }
    }
    #[test]
    fn emits_pe_header() {
        let bytes = compile("class C { static int F(int a) { return a + 1; } }", "Test").unwrap();
        assert_eq!(&bytes[..2], b"MZ");
    }
}
