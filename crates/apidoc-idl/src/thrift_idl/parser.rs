//! `.thrift` tokenizer + recursive-descent parser (split from thrift_idl.rs).

use super::{Ast, Field, Method, Service, Ty};
use crate::MAX_DEPTH;

// ---------- Tokenizer ----------

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Tok {
    Ident(String),
    Num(String),
    Str(String),
    Sym(char),
}

#[derive(Clone, Debug)]
pub(super) struct Token {
    pub(super) kind: Tok,
    pub(super) line: usize,
}

pub(super) fn tokenize(source: &str) -> Result<Vec<Token>, String> {
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

pub(super) struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub(super) fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    pub(super) fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    pub(super) fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn line(&self) -> usize {
        self.peek().or_else(|| self.tokens.last()).map_or(1, |t| t.line)
    }

    pub(super) fn err(&self, message: impl Into<String>) -> String {
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

    pub(super) fn take_ident(&mut self) -> Result<String, String> {
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

    pub(super) fn take_str(&mut self) -> Result<String, String> {
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

    pub(super) fn eat_separator(&mut self) {
        if self.peek_sym(',') || self.peek_sym(';') {
            self.pos += 1;
        }
    }

    /// Skip `( ... )` annotations after a definition/field.
    pub(super) fn skip_annotations(&mut self) -> Result<(), String> {
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

    pub(super) fn parse_type(&mut self, depth: usize) -> Result<Ty, String> {
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

pub(super) fn skip_until_semicolon(p: &mut Parser) -> Result<(), String> {
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

pub(super) fn parse_enum(p: &mut Parser, ast: &mut Ast) -> Result<(), String> {
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

pub(super) fn parse_struct(p: &mut Parser, ast: &mut Ast) -> Result<(), String> {
    let name = p.take_ident()?;
    p.skip_annotations()?;
    p.expect_sym('{')?;
    let mut auto = -1i32;
    let fields = p.parse_fields('}', &mut auto)?;
    ast.structs.insert(name, fields);
    Ok(())
}

pub(super) fn parse_service(p: &mut Parser, ast: &mut Ast) -> Result<(), String> {
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

