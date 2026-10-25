//! Hand-written `.thrift` IDL parser → apidoc annotation code.
//!
//! Supports `namespace` / `struct` / `union` / `exception` / `enum` / `service` /
//! `typedef` / `const` and the usual type grammar (`list<T>`, `set<T>`, `map<K,V>`).
//! `include` is never expanded (nothing is read from disk or the network); it is
//! reported as a comment. File size and nesting depth are bounded.

use crate::{ensure_input_size, push_fn, render_param, FnNames, Param, MAX_DEPTH};
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

// ---------- Tokenizer ----------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Num(String),
    Str(String),
    Sym(char),
}

#[derive(Clone, Debug)]
struct Token {
    kind: Tok,
    line: usize,
}

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut line = 1;
    let mut out = Vec::new();
    while i < bytes.len() {
        let byte = bytes[i];
        match byte {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                loop {
                    if i + 1 >= bytes.len() {
                        return Err(format!("line {line}: unterminated block comment"));
                    }
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            b'"' | b'\'' => {
                let quote = byte;
                let start_line = line;
                i += 1;
                let mut value = String::new();
                loop {
                    let Some(&b) = bytes.get(i) else {
                        return Err(format!("line {start_line}: unterminated string"));
                    };
                    if b == quote {
                        i += 1;
                        break;
                    }
                    if b == b'\n' {
                        line += 1;
                    }
                    if b == b'\\' {
                        i += 1;
                        let Some(&esc) = bytes.get(i) else {
                            return Err(format!("line {start_line}: unterminated string escape"));
                        };
                        value.push(match esc {
                            b'n' => '\n',
                            b'r' => '\r',
                            b't' => '\t',
                            other => other as char,
                        });
                    } else {
                        value.push(b as char);
                    }
                    i += 1;
                }
                out.push(Token { kind: Tok::Str(value), line: start_line });
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                if byte == b'-' {
                    i += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                // Float literal in a const default: consume it as one token.
                if i < bytes.len()
                    && bytes[i] == b'.'
                    && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
                {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                out.push(Token { kind: Tok::Num(source[start..i].to_string()), line });
            }
            _ if byte.is_ascii_alphabetic() || byte == b'_' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    i += 1;
                }
                out.push(Token { kind: Tok::Ident(source[start..i].to_string()), line });
            }
            _ => {
                let c = byte as char;
                if "{}()<>,;:=[]*".contains(c) {
                    out.push(Token { kind: Tok::Sym(c), line });
                    i += 1;
                } else {
                    return Err(format!("line {line}: unexpected character `{c}`"));
                }
            }
        }
    }
    Ok(out)
}

// ---------- Parser ----------

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn line(&self) -> usize {
        self.peek().or_else(|| self.tokens.last()).map_or(1, |t| t.line)
    }

    fn err(&self, message: impl Into<String>) -> String {
        format!("line {}: {}", self.line(), message.into())
    }

    fn peek_ident(&self, name: &str) -> bool {
        matches!(self.peek(), Some(Token { kind: Tok::Ident(id), .. }) if id == name)
    }

    fn eat_ident(&mut self, name: &str) -> bool {
        if self.peek_ident(name) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn take_ident(&mut self) -> Result<String, String> {
        match self.next() {
            Some(Token { kind: Tok::Ident(id), .. }) => Ok(id),
            Some(_) => Err(self.err("expected an identifier")),
            None => Err(self.err("unexpected end of file")),
        }
    }

    fn peek_sym(&self, c: char) -> bool {
        matches!(self.peek(), Some(Token { kind: Tok::Sym(s), .. }) if *s == c)
    }

    fn eat_sym(&mut self, c: char) -> bool {
        if self.peek_sym(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_sym(&mut self, c: char) -> Result<(), String> {
        if self.eat_sym(c) {
            Ok(())
        } else {
            Err(self.err(format!("expected `{c}`")))
        }
    }

    fn take_str(&mut self) -> Result<String, String> {
        match self.next() {
            Some(Token { kind: Tok::Str(s), .. }) => Ok(s),
            Some(_) => Err(self.err("expected a string literal")),
            None => Err(self.err("unexpected end of file")),
        }
    }

    fn take_num(&mut self) -> Result<i64, String> {
        match self.next() {
            Some(Token { kind: Tok::Num(n), .. }) => {
                n.parse::<i64>().map_err(|_| self.err(format!("invalid integer `{n}`")))
            }
            Some(_) => Err(self.err("expected an integer")),
            None => Err(self.err("unexpected end of file")),
        }
    }

    fn eat_separator(&mut self) {
        if self.peek_sym(',') || self.peek_sym(';') {
            self.pos += 1;
        }
    }

    /// Skip `( ... )` annotations after a definition/field.
    fn skip_annotations(&mut self) -> Result<(), String> {
        if !self.peek_sym('(') {
            return Ok(());
        }
        self.skip_balanced('(', ')')
    }

    fn skip_balanced(&mut self, open: char, close: char) -> Result<(), String> {
        self.expect_sym(open)?;
        let mut depth = 1usize;
        while let Some(token) = self.next() {
            if token.kind == Tok::Sym(open) {
                depth += 1;
            } else if token.kind == Tok::Sym(close) {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
        }
        Err(self.err(format!("unterminated `{open}`")))
    }

    /// Consume a default value up to the field separator (or the closing brace
    /// of the enclosing struct). Nested `[] {} ()` are balanced.
    fn skip_default(&mut self) -> Result<(), String> {
        let mut depth = 0usize;
        loop {
            let Some(token) = self.peek() else {
                return Err(self.err("unexpected end of file in default value"));
            };
            match token.kind {
                Tok::Sym('{') | Tok::Sym('[') | Tok::Sym('(') => {
                    depth += 1;
                    self.pos += 1;
                }
                Tok::Sym('}') | Tok::Sym(']') | Tok::Sym(')') => {
                    if depth == 0 {
                        return Ok(());
                    }
                    depth -= 1;
                    self.pos += 1;
                }
                Tok::Sym(',') | Tok::Sym(';') if depth == 0 => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => self.pos += 1,
            }
        }
    }

    fn parse_type(&mut self, depth: usize) -> Result<Ty, String> {
        if depth > MAX_DEPTH {
            return Err(self.err("type nesting exceeds 64 levels"));
        }
        let name = self.take_ident()?;
        match name.as_str() {
            "list" | "set" => {
                self.expect_sym('<')?;
                let inner = self.parse_type(depth + 1)?;
                self.expect_sym('>')?;
                Ok(if name == "list" {
                    Ty::List(Box::new(inner))
                } else {
                    Ty::Set(Box::new(inner))
                })
            }
            "map" => {
                self.expect_sym('<')?;
                let key = self.parse_type(depth + 1)?;
                self.expect_sym(',')?;
                let value = self.parse_type(depth + 1)?;
                self.expect_sym('>')?;
                Ok(Ty::Map(Box::new(key), Box::new(value)))
            }
            "void" => Ok(Ty::Void),
            other => Ok(Ty::Named(other.to_string())),
        }
    }

    fn parse_field(&mut self, next_auto: &mut i32) -> Result<Field, String> {
        let explicit_id = if matches!(self.peek(), Some(Token { kind: Tok::Num(_), .. })) {
            let id = self.take_num()?;
            let id = i32::try_from(id).map_err(|_| self.err("field id out of range"))?;
            self.expect_sym(':')?;
            Some(id)
        } else {
            None
        };
        let mut optional = false;
        if !self.eat_ident("required") && self.eat_ident("optional") {
            optional = true;
        }
        let ty = self.parse_type(0)?;
        let name = self.take_ident()?;
        if self.eat_sym('=') {
            self.skip_default()?;
        }
        self.skip_annotations()?;
        let id = match explicit_id {
            Some(id) => id,
            None => {
                // Thrift auto-assigns ids from -1 downwards when omitted.
                let id = *next_auto;
                *next_auto -= 1;
                id
            }
        };
        Ok(Field { name, id, ty, optional })
    }

    fn parse_fields(&mut self, close: char, auto: &mut i32) -> Result<Vec<Field>, String> {
        let mut fields = Vec::new();
        loop {
            if self.eat_sym(close) {
                return Ok(fields);
            }
            if self.peek().is_none() {
                return Err(self.err(format!("unexpected end of file, expected `{close}`")));
            }
            fields.push(self.parse_field(auto)?);
            self.eat_separator();
        }
    }
}

fn parse(source: &str) -> Result<Ast, String> {
    let tokens = tokenize(source)?;
    let mut p = Parser { tokens, pos: 0 };
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

fn skip_until_semicolon(p: &mut Parser) -> Result<(), String> {
    let mut depth = 0usize;
    while let Some(token) = p.next() {
        match token.kind {
            Tok::Sym('{') | Tok::Sym('[') | Tok::Sym('(') => depth += 1,
            Tok::Sym('}') | Tok::Sym(']') | Tok::Sym(')') => depth = depth.saturating_sub(1),
            Tok::Sym(';') if depth == 0 => return Ok(()),
            _ => {}
        }
    }
    Err(p.err("unexpected end of file in const declaration"))
}

fn parse_enum(p: &mut Parser, ast: &mut Ast) -> Result<(), String> {
    let name = p.take_ident()?;
    p.skip_annotations()?;
    p.expect_sym('{')?;
    let mut values = Vec::new();
    loop {
        if p.eat_sym('}') {
            break;
        }
        values.push(p.take_ident()?);
        if p.eat_sym('=') {
            p.take_num()?;
        }
        p.skip_annotations()?;
        p.eat_separator();
    }
    ast.enums.insert(name, values);
    Ok(())
}

fn parse_struct(p: &mut Parser, ast: &mut Ast) -> Result<(), String> {
    let name = p.take_ident()?;
    p.skip_annotations()?;
    p.expect_sym('{')?;
    let mut auto = -1i32;
    let fields = p.parse_fields('}', &mut auto)?;
    ast.structs.insert(name, fields);
    Ok(())
}

fn parse_service(p: &mut Parser, ast: &mut Ast) -> Result<(), String> {
    let name = p.take_ident()?;
    if p.eat_ident("extends") {
        p.take_ident()?;
    }
    p.skip_annotations()?;
    p.expect_sym('{')?;
    let mut methods = Vec::new();
    loop {
        if p.eat_sym('}') {
            break;
        }
        if p.peek().is_none() {
            return Err(p.err("unexpected end of file in service"));
        }
        methods.push(parse_method(p)?);
        p.eat_separator();
    }
    ast.services.push(Service { name, methods });
    Ok(())
}

fn parse_method(p: &mut Parser) -> Result<Method, String> {
    p.eat_ident("oneway");
    let return_ty = p.parse_type(0)?;
    let name = p.take_ident()?;
    p.expect_sym('(')?;
    let mut auto = -1i32;
    let params = p.parse_fields(')', &mut auto)?;
    if p.eat_ident("throws") {
        p.expect_sym('(')?;
        let mut throws_auto = -1i32;
        p.parse_fields(')', &mut throws_auto)?;
    }
    p.skip_annotations()?;
    Ok(Method { name, return_ty, params })
}

// ---------- Generation ----------

fn render(ast: &Ast) -> String {
    let mut out = String::from("// Generated by apidoc-idl. Do not edit.\n\n");
    for include in &ast.includes {
        out.push_str(&format!(
            "// include \"{include}\" not expanded (imports are not supported)\n"
        ));
    }
    if !ast.includes.is_empty() {
        out.push('\n');
    }
    let mut names = FnNames::new();
    for service in &ast.services {
        for method in &service.methods {
            let fn_name = names.alloc(&method.name);
            let mut params = Vec::new();
            let mut comments = Vec::new();
            for field in &method.params {
                let (param, mut field_comments) = field_param(field, ast, 0, &mut Vec::new());
                params.push(param);
                comments.append(&mut field_comments);
            }
            let (returned, out_comments) = return_params(method, ast);
            comments.extend(out_comments);

            out.push_str("#[allow(dead_code)]\n");
            out.push_str(&format!("#[apidoc::title(\"{}\")]\n", method.name));
            out.push_str(&format!(
                "#[apidoc::protocol(\"thrift\", target = \"{TARGET}\", service = \"{}\", method = \"{}\")]\n",
                service.name, method.name
            ));
            for p in &params {
                out.push_str(&render_param("param", p, true));
                out.push('\n');
            }
            for p in &returned {
                out.push_str(&render_param("returned", p, false));
                out.push('\n');
            }
            push_fn(&mut out, &fn_name, &comments);
        }
    }
    out
}

fn field_param(
    field: &Field,
    ast: &Ast,
    depth: usize,
    stack: &mut Vec<String>,
) -> (Param, Vec<String>) {
    let resolved = match resolve_ty(&field.ty, &ast.typedefs, 0) {
        Ok(ty) => ty,
        Err(e) => {
            return (
                Param {
                    name: field.name.clone(),
                    ty: "object".to_string(),
                    field: Some(field.id.to_string()),
                    required: !field.optional,
                    children: Vec::new(),
                },
                vec![format!("TODO: {e}")],
            )
        }
    };
    let (ty, children, comments) = map_ty(&resolved, ast, depth, stack);
    (
        Param {
            name: field.name.clone(),
            ty,
            field: Some(field.id.to_string()),
            required: !field.optional,
            children,
        },
        comments,
    )
}

fn return_params(method: &Method, ast: &Ast) -> (Vec<Param>, Vec<String>) {
    let resolved = match resolve_ty(&method.return_ty, &ast.typedefs, 0) {
        Ok(ty) => ty,
        Err(e) => return (Vec::new(), vec![format!("TODO: {e}")]),
    };
    if matches!(resolved, Ty::Void) {
        return (Vec::new(), Vec::new());
    }
    let (ty, children, comments) = map_ty(&resolved, ast, 0, &mut Vec::new());
    // Struct return → document its fields directly (same shape as proto messages).
    if ty == "object" && !children.is_empty() {
        (children, comments)
    } else {
        (
            vec![Param { name: "return".to_string(), ty, field: None, required: false, children }],
            comments,
        )
    }
}

fn resolve_ty(ty: &Ty, typedefs: &HashMap<String, Ty>, depth: usize) -> Result<Ty, String> {
    if depth > MAX_DEPTH {
        return Err("typedef nesting exceeds 64 levels".to_string());
    }
    match ty {
        Ty::Named(name) => match typedefs.get(name) {
            Some(inner) => resolve_ty(inner, typedefs, depth + 1),
            None => Ok(ty.clone()),
        },
        Ty::List(inner) => Ok(Ty::List(Box::new(resolve_ty(inner, typedefs, depth + 1)?))),
        Ty::Set(inner) => Ok(Ty::Set(Box::new(resolve_ty(inner, typedefs, depth + 1)?))),
        Ty::Map(key, value) => Ok(Ty::Map(
            Box::new(resolve_ty(key, typedefs, depth + 1)?),
            Box::new(resolve_ty(value, typedefs, depth + 1)?),
        )),
        Ty::Void => Ok(Ty::Void),
    }
}

fn map_ty(
    ty: &Ty,
    ast: &Ast,
    depth: usize,
    stack: &mut Vec<String>,
) -> (String, Vec<Param>, Vec<String>) {
    match ty {
        Ty::Void => ("void".to_string(), Vec::new(), Vec::new()),
        Ty::List(inner) | Ty::Set(inner) => {
            let (_, children, comments) = map_ty(inner, ast, depth, stack);
            ("array".to_string(), children, comments)
        }
        // Thrift maps have no field model in apidoc; documented as an object.
        Ty::Map(_, _) => ("object".to_string(), Vec::new(), Vec::new()),
        Ty::Named(name) => {
            if let Some(base) = base_type(name) {
                return (base.to_string(), Vec::new(), Vec::new());
            }
            if ast.enums.contains_key(name) {
                return ("enum".to_string(), Vec::new(), Vec::new());
            }
            let Some(fields) = ast.structs.get(name) else {
                return (
                    "object".to_string(),
                    Vec::new(),
                    vec![format!("TODO: unresolved type `{name}` (include not expanded)")],
                );
            };
            if depth >= MAX_DEPTH {
                return (
                    "object".to_string(),
                    Vec::new(),
                    vec!["TODO: type nesting depth limit reached".to_string()],
                );
            }
            if stack.iter().any(|s| s == name) {
                return (
                    "object".to_string(),
                    Vec::new(),
                    vec![format!("TODO: recursive type `{name}` not expanded")],
                );
            }
            stack.push(name.clone());
            let (children, comments) = struct_params(fields, ast, depth + 1, stack);
            stack.pop();
            ("object".to_string(), children, comments)
        }
    }
}

fn struct_params(
    fields: &[Field],
    ast: &Ast,
    depth: usize,
    stack: &mut Vec<String>,
) -> (Vec<Param>, Vec<String>) {
    let mut params = Vec::new();
    let mut comments = Vec::new();
    for field in fields {
        let (param, mut field_comments) = field_param(field, ast, depth, stack);
        params.push(param);
        comments.append(&mut field_comments);
    }
    (params, comments)
}

fn base_type(name: &str) -> Option<&'static str> {
    match name {
        "i8" | "i16" | "i32" | "i64" | "byte" => Some("int"),
        "double" | "float" => Some("float"),
        "bool" => Some("bool"),
        "string" => Some("string"),
        "binary" => Some("bytes"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
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
