//! Keyword-argument parsers for the param-like attributes (param / query /
//! returned), header, and success / error. Split out of lib.rs to keep every
//! file under 500 lines; parsers are pure data extraction, no expansion logic.

use syn::parse::{Parse, ParseStream};
use syn::{braced, bracketed, Ident, LitStr, Token};

/// 通用 key=value 循环：每轮解析 `key = "value"` 交给 `apply`，支持尾逗号。
/// success/error/header 等纯键值参数共用。
fn kv_loop(input: ParseStream, mut apply: impl FnMut(Ident, LitStr) -> syn::Result<()>) -> syn::Result<()> {
    while !input.is_empty() {
        let key: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let value: LitStr = input.parse()?;
        apply(key, value)?;
        if input.is_empty() {
            break;
        }
        input.parse::<Token![,]>()?;
    }
    Ok(())
}

/// Parsed `code` / `example` keyword arguments of success / error. Keeps the
/// raw literals so validation errors point at the exact token, not the whole
/// attribute.
pub struct SuccessArgs {
    pub code: Option<LitStr>,
    pub example: Option<LitStr>,
}

impl Parse for SuccessArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut out = SuccessArgs { code: None, example: None };
        kv_loop(input, |key, value| {
            match key.to_string().as_str() {
                "code" => out.code = Some(value),
                "example" => out.example = Some(value),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown argument `{other}`; expected code or example"),
                    ));
                }
            }
            Ok(())
        })?;
        Ok(out)
    }
}

/// Parsed first literal + arbitrary `key = "value"` options of `protocol`.
/// The kind is validated against the canonical list by the caller; options are
/// kept as written so each protocol family can read its own keys without the
/// macro knowing them all.
pub struct ProtocolArgs {
    pub kind: LitStr,
    pub opts: Vec<(Ident, LitStr)>,
}

impl Parse for ProtocolArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kind: LitStr = input.parse()?;
        let mut opts = Vec::new();
        if !input.is_empty() {
            input.parse::<Token![,]>()?;
        }
        kv_loop(input, |key, value| {
            opts.push((key, value));
            Ok(())
        })?;
        Ok(ProtocolArgs { kind, opts })
    }
}

/// 每协议别名注解的参数：可选的第一个位置参数作为 target，后续任意 key = "value"。
pub struct ProtocolOptsArgs {
    pub opts: Vec<(Ident, LitStr)>,
}

impl Parse for ProtocolOptsArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut opts = Vec::new();
        if input.peek(LitStr) {
            let lit: LitStr = input.parse()?;
            opts.push((Ident::new("target", lit.span()), lit));
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        kv_loop(input, |key, value| {
            opts.push((key, value));
            Ok(())
        })?;
        Ok(ProtocolOptsArgs { opts })
    }
}

/// Parsed keyword arguments of `event` (direction / name / ty / desc / mock).
pub struct EventArgs {
    pub direction: Option<LitStr>,
    pub name: Option<LitStr>,
    pub ty: Option<LitStr>,
    pub desc: Option<LitStr>,
    pub mock: Option<LitStr>,
}

impl Parse for EventArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut out = EventArgs { direction: None, name: None, ty: None, desc: None, mock: None };
        kv_loop(input, |key, value| {
            match key.to_string().as_str() {
                "direction" => out.direction = Some(value),
                "name" => out.name = Some(value),
                "ty" | "type" => out.ty = Some(value),
                "desc" => out.desc = Some(value),
                "mock" => out.mock = Some(value),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown argument `{other}`; expected direction, name, ty, desc or mock"
                        ),
                    ));
                }
            }
            Ok(())
        })?;
        Ok(out)
    }
}

/// Parsed `name` / `desc` keyword arguments of header. Dedicated parser so
/// unknown keys (ty / required / mock / children) fail loudly instead of
/// being silently dropped. Literals are kept raw so errors can point at the
/// exact token rather than the whole attribute.
pub struct HeaderArgs {
    pub name: Option<LitStr>,
    pub desc: Option<LitStr>,
}

impl Parse for HeaderArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut out = HeaderArgs { name: None, desc: None };
        kv_loop(input, |key, value| {
            match key.to_string().as_str() {
                "name" => out.name = Some(value),
                "desc" => out.desc = Some(value),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown argument `{other}`; expected name or desc"),
                    ));
                }
            }
            Ok(())
        })?;
        Ok(out)
    }
}

/// Parsed keyword arguments of param / query / returned. Literals are kept raw
/// so errors point at the exact token rather than the whole attribute.
pub struct ParamArgs {
    pub name: Option<LitStr>,
    pub ty: Option<LitStr>,
    pub required: bool,
    pub default: Option<LitStr>,
    pub desc: Option<LitStr>,
    pub mock: Option<LitStr>,
    /// 协议字段号（protobuf tag / Thrift field id / ISO 8583 域号）。
    pub field: Option<LitStr>,
    pub children: Vec<ParamArgs>,
}

impl Parse for ParamArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut out = ParamArgs {
            name: None,
            ty: None,
            required: false,
            default: None,
            desc: None,
            mock: None,
            field: None,
            children: Vec::new(),
        };
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            if key == "required" {
                out.required = true;
            } else if key == "children" {
                input.parse::<Token![=]>()?;
                let content;
                bracketed!(content in input);
                while !content.is_empty() {
                    let inner;
                    braced!(inner in content);
                    out.children.push(inner.parse::<ParamArgs>()?);
                    if content.is_empty() {
                        break;
                    }
                    content.parse::<Token![,]>()?;
                }
            } else {
                input.parse::<Token![=]>()?;
                let value: LitStr = input.parse()?;
                match key.to_string().as_str() {
                    "name" => out.name = Some(value),
                    "ty" | "type" => out.ty = Some(value),
                    "default" => out.default = Some(value),
                    "desc" => out.desc = Some(value),
                    "mock" => out.mock = Some(value),
                    "field" => out.field = Some(value),
                    other => {
                        return Err(syn::Error::new(
                            key.span(),
                            format!(
                                "unknown argument `{}`; expected name, ty, required, default, desc, mock, field or children",
                                other
                            ),
                        ));
                    }
                }
            }
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(out)
    }
}
