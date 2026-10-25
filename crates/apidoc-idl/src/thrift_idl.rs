//! Hand-written `.thrift` IDL parser → apidoc annotation code.
//!
//! Supports `namespace` / `struct` / `union` / `exception` / `enum` / `service` /
//! `typedef` / `const` and the usual type grammar (`list<T>`, `set<T>`, `map<K,V>`).
//! `include` is never expanded (nothing is read from disk or the network); it is
//! reported as a comment. File size and nesting depth are bounded.

use crate::ensure_input_size;
use std::collections::HashMap;

const TARGET: &str = "tcp://127.0.0.1:9090";

pub fn generate(source: &str) -> Result<String, String> {
    ensure_input_size(source)?;
    let ast = parse(source)?;
    Ok(render(&ast))
}

// ---------- AST ----------

#[derive(Default)]
struct Ast {
    typedefs: HashMap<String, Ty>,
    structs: HashMap<String, Vec<Field>>,
    enums: HashMap<String, Vec<String>>,
    services: Vec<Service>,
    includes: Vec<String>,
}

#[derive(Clone, Debug)]
enum Ty {
    Void,
    List(Box<Ty>),
    Set(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    Named(String),
}

#[derive(Clone, Debug)]
struct Field {
    name: String,
    id: i32,
    ty: Ty,
    optional: bool,
}

#[derive(Clone, Debug)]
struct Method {
    name: String,
    return_ty: Ty,
    params: Vec<Field>,
}

#[derive(Clone, Debug)]
struct Service {
    name: String,
    methods: Vec<Method>,
}

mod emit;
mod parser;

use emit::render;
use parser::{parse_enum, parse_service, parse_struct, skip_until_semicolon, tokenize, Parser, Tok};

fn parse(source: &str) -> Result<Ast, String> {
    let tokens = tokenize(source)?;
    let mut p = Parser::new(tokens);
    let mut ast = Ast::default();
    while let Some(token) = p.peek().cloned() {
        let Tok::Ident(keyword) = token.kind else {
            return Err(format!("line {}: unexpected token", token.line));
        };
        match keyword.as_str() {
            "namespace" => {
                p.next();
                p.next().ok_or_else(|| p.err("expected a namespace scope"))?;
                p.next().ok_or_else(|| p.err("expected a namespace name"))?;
            }
            "include" | "cpp_include" => {
                p.next();
                ast.includes.push(p.take_str()?);
                p.eat_separator();
            }
            "typedef" => {
                p.next();
                let ty = p.parse_type(0)?;
                let name = p.take_ident()?;
                p.skip_annotations()?;
                p.eat_separator();
                ast.typedefs.insert(name, ty);
            }
            "const" => {
                p.next();
                skip_until_semicolon(&mut p)?;
            }
            "enum" => {
                p.next();
                parse_enum(&mut p, &mut ast)?;
            }
            "struct" | "union" | "exception" => {
                p.next();
                parse_struct(&mut p, &mut ast)?;
            }
            "service" => {
                p.next();
                parse_service(&mut p, &mut ast)?;
            }
            other => return Err(format!("line {}: unexpected keyword `{other}`", token.line)),
        }
    }
    Ok(ast)
}

#[cfg(test)]
mod tests {
    use super::emit::resolve_ty;
    use super::*;

    #[test]
    fn tokenizer_reports_line_numbers() {
        let err = tokenize("struct X {\n  @bad\n}").unwrap_err();
        assert!(err.contains("line 2"), "{err}");
    }

    #[test]
    fn typedef_cycles_error_instead_of_looping() {
        let a = Ty::Named("A".into());
        let mut defs = HashMap::new();
        defs.insert("A".to_string(), Ty::Named("B".into()));
        defs.insert("B".to_string(), Ty::Named("A".into()));
        assert!(resolve_ty(&a, &defs, 0).is_err());
    }
}
