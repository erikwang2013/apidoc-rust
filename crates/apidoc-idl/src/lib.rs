//! Generate apidoc annotation code from `.proto` / `.thrift` IDL.
//!
//! Both generators are pure functions over source text: imports are never
//! fetched from disk or the network (only the built-in `google/protobuf/*`
//! well-known types are available for proto), input is capped at 1 MiB, and
//! recursive type nesting is capped at [`MAX_DEPTH`].

pub mod proto;
pub mod thrift_idl;

/// Hard cap for a single IDL input (1 MiB).
pub const MAX_INPUT: usize = 1024 * 1024;

/// Maximum nesting depth for messages/types/typedefs.
pub(crate) const MAX_DEPTH: usize = 64;

pub fn ensure_input_size(source: &str) -> Result<(), String> {
    if source.len() > MAX_INPUT {
        return Err(format!(
            "input is {} bytes, the limit is {} bytes (1 MiB)",
            source.len(),
            MAX_INPUT
        ));
    }
    Ok(())
}

/// One generated `param` / `returned` field.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Param {
    pub name: String,
    pub ty: String,
    pub field: Option<String>,
    pub required: bool,
    pub children: Vec<Param>,
}

/// `#[apidoc::{kind}(...)]`. `include_required` is false for `returned`
/// (requiredness is meaningless there; proto snapshot in the plan omits it).
pub(crate) fn render_param(kind: &str, p: &Param, include_required: bool) -> String {
    format!("#[apidoc::{kind}({})]", param_args(p, include_required))
}

/// 生成 Rust 字符串字面量内容；IDL 是不可信输入，必须转义。
fn rust_str(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// 生成合法的 Rust 函数名；非标识符字符折叠为 `_`。
fn rust_ident(value: &str) -> String {
    let mut out: String = value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if out.is_empty() {
        out.push('_');
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn param_args(p: &Param, include_required: bool) -> String {
    let mut args = format!(
        "name = \"{}\", ty = \"{}\"",
        rust_str(&p.name),
        rust_str(&p.ty)
    );
    if let Some(field) = &p.field {
        args.push_str(&format!(", field = \"{}\"", rust_str(field)));
    }
    if include_required && p.required {
        args.push_str(", required");
    }
    if !p.children.is_empty() {
        let kids: Vec<String> = p
            .children
            .iter()
            .map(|c| format!("{{ {} }}", param_args(c, include_required)))
            .collect();
        args.push_str(&format!(", children = [{}]", kids.join(", ")));
    }
    args
}

/// Emit the generated function, with any TODO comments in its body.
pub(crate) fn push_fn(out: &mut String, name: &str, comments: &[String]) {
    let name = rust_ident(name);
    if comments.is_empty() {
        out.push_str(&format!("fn {name}() {{}}\n\n"));
        return;
    }
    out.push_str(&format!("fn {name}() {{\n"));
    for comment in comments {
        out.push_str(&format!("    // {comment}\n"));
    }
    out.push_str("}\n\n");
}

/// Unique function-name allocator: `SayHello` → `say_hello`, duplicates get `_2`.
pub(crate) struct FnNames {
    used: std::collections::HashSet<String>,
}

impl FnNames {
    pub(crate) fn new() -> Self {
        FnNames { used: std::collections::HashSet::new() }
    }

    pub(crate) fn alloc(&mut self, method: &str) -> String {
        let base = snake_case(method);
        let mut name = base.clone();
        let mut n = 2;
        while !self.used.insert(name.clone()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        name
    }
}

fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    let out = if out.is_empty() { "generated".to_string() } else { out.to_string() };
    if out.as_bytes()[0].is_ascii_digit() {
        return format!("_{out}");
    }
    if is_keyword(&out) {
        format!("{out}_")
    } else {
        out
    }
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "try"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_handles_camel_pascal_and_keywords() {
        assert_eq!(snake_case("SayHello"), "say_hello");
        assert_eq!(snake_case("SayHelloStream"), "say_hello_stream");
        assert_eq!(snake_case("type"), "type_");
        assert_eq!(snake_case("2fast"), "_2fast");
    }

    #[test]
    fn nested_children_render_inline() {
        let p = Param {
            name: "user".into(),
            ty: "object".into(),
            field: Some("1".into()),
            required: true,
            children: vec![Param {
                name: "id".into(),
                ty: "int".into(),
                field: Some("1".into()),
                required: true,
                children: vec![],
            }],
        };
        assert_eq!(
            render_param("param", &p, true),
            r#"#[apidoc::param(name = "user", ty = "object", field = "1", required, children = [{ name = "id", ty = "int", field = "1", required }])]"#
        );
    }
}
