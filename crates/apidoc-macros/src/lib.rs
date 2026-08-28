//! Attribute macros for apidoc: title / desc / method / url / param / query / returned.
//!
//! Each macro keeps the annotated function unchanged and emits a statically
//! registered `DocFragmentEntry` on the distributed slice `apidoc::DOC_FRAGMENTS`,
//! so documentation fragments are collected at zero runtime cost.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use std::sync::atomic::{AtomicU32, Ordering};
use syn::parse::{Parse, ParseStream};
use syn::{braced, bracketed, parse_macro_input, Ident, Item, ItemFn, LitStr, Token};

const HTTP_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

// Monotonic sequence number per annotation, assigned in expansion order (which
// follows source order in practice). Restores declaration order at collect
// time, because linkme's linker-section iteration order is not source order.
static SEQ: AtomicU32 = AtomicU32::new(0);

#[proc_macro_attribute]
pub fn title(args: TokenStream, item: TokenStream) -> TokenStream {
    simple_fragment("title", args, item)
}

#[proc_macro_attribute]
pub fn desc(args: TokenStream, item: TokenStream) -> TokenStream {
    simple_fragment("desc", args, item)
}

#[proc_macro_attribute]
pub fn method(args: TokenStream, item: TokenStream) -> TokenStream {
    simple_fragment("method", args, item)
}

#[proc_macro_attribute]
pub fn url(args: TokenStream, item: TokenStream) -> TokenStream {
    simple_fragment("url", args, item)
}

#[proc_macro_attribute]
pub fn param(args: TokenStream, item: TokenStream) -> TokenStream {
    param_fragment("param", args, item)
}

#[proc_macro_attribute]
pub fn query(args: TokenStream, item: TokenStream) -> TokenStream {
    param_fragment("query", args, item)
}

#[proc_macro_attribute]
pub fn returned(args: TokenStream, item: TokenStream) -> TokenStream {
    param_fragment("returned", args, item)
}

/// title / desc / method / url: a single string literal plus validation.
fn simple_fragment(kind: &str, args: TokenStream, item: TokenStream) -> TokenStream {
    let lit = parse_macro_input!(args as LitStr);
    let value = lit.value();
    let err = match kind {
        "url" if !value.starts_with('/') => {
            Some(syn::Error::new(lit.span(), "apidoc::url must start with '/'"))
        }
        "method" if !HTTP_METHODS.contains(&value.as_str()) => Some(syn::Error::new(
            lit.span(),
            format!(
                "apidoc::method must be one of {:?}, got `{}`",
                HTTP_METHODS, value
            ),
        )),
        _ => None,
    };
    if let Some(err) = err {
        return err.to_compile_error().into();
    }
    let item_fn = match parse_item_fn(kind, item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    let variant = variant_ident(kind);
    let frag = quote! { apidoc::DocFragment::#variant(#lit) };
    emit(kind, item_fn, frag)
}

/// param / query / returned: keyword-style arguments, e.g.
/// `#[apidoc::param(name = "id", ty = "int", required, desc = "ID", mock = "1")]`
/// with nested `children = [{ name = "x", ... }, ...]`.
fn param_fragment(kind: &str, args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as ParamArgs);
    let item_fn = match parse_item_fn(kind, item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    if args.name.as_deref().is_none_or(str::is_empty) {
        return syn::Error::new(
            Span::call_site(),
            format!("apidoc::{kind} requires a non-empty `name`"),
        )
        .to_compile_error()
        .into();
    }
    let variant = variant_ident(kind);
    let doc_param = doc_param_expr(&args);
    let frag = quote! { apidoc::DocFragment::#variant(#doc_param) };
    emit(kind, item_fn, frag)
}

/// DocFragment enum variant name for a macro kind: "title" -> Title.
fn variant_ident(kind: &str) -> Ident {
    let mut chars = kind.chars();
    let first = chars.next().unwrap().to_ascii_uppercase();
    Ident::new(&format!("{first}{}", chars.as_str()), Span::call_site())
}

fn parse_item_fn(kind: &str, item: TokenStream) -> syn::Result<ItemFn> {
    let item = syn::parse::<Item>(item)?;
    match item {
        Item::Fn(f) => Ok(f),
        other => Err(syn::Error::new_spanned(
            other,
            format!("apidoc::{kind} can only be applied to a function"),
        )),
    }
}

fn doc_param_expr(args: &ParamArgs) -> proc_macro2::TokenStream {
    let name = args.name.as_deref().unwrap_or("");
    let ty = args.ty.as_deref().unwrap_or("string");
    let required = args.required;
    let default = opt_lit(&args.default);
    let desc = opt_lit(&args.desc);
    let mock = opt_lit(&args.mock);
    let children = args.children.iter().map(doc_param_expr);
    quote! {
        apidoc::DocParam {
            name: #name,
            ty: #ty,
            required: #required,
            default: #default,
            desc: #desc,
            mock: #mock,
            children: &[#(#children),*],
        }
    }
}

fn opt_lit(value: &Option<String>) -> proc_macro2::TokenStream {
    match value {
        Some(s) => quote! { Some(#s) },
        None => quote! { None },
    }
}

/// Emits the original function plus its fragment registration.
fn emit(kind: &str, item_fn: ItemFn, frag: proc_macro2::TokenStream) -> TokenStream {
    let fn_ident = item_fn.sig.ident.clone();
    let kind_upper = kind.to_uppercase();
    // seq makes the static name unique even for repeated same-name params on
    // one function, which would otherwise collide into a misleading E0428.
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let static_ident = format_ident!("__APIDOC_{kind_upper}_{fn_ident}_{seq}");
    quote! {
        #item_fn
        #[apidoc::distributed_slice(apidoc::DOC_FRAGMENTS)]
        static #static_ident: apidoc::DocFragmentEntry = apidoc::DocFragmentEntry {
            id: concat!(module_path!(), "::", stringify!(#fn_ident)),
            seq: #seq,
            frag: #frag,
        };
    }
    .into()
}

/// Parsed keyword arguments of param / query / returned.
struct ParamArgs {
    name: Option<String>,
    ty: Option<String>,
    required: bool,
    default: Option<String>,
    desc: Option<String>,
    mock: Option<String>,
    children: Vec<ParamArgs>,
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
                    "name" => out.name = Some(value.value()),
                    "ty" | "type" => out.ty = Some(value.value()),
                    "default" => out.default = Some(value.value()),
                    "desc" => out.desc = Some(value.value()),
                    "mock" => out.mock = Some(value.value()),
                    other => {
                        return Err(syn::Error::new(
                            key.span(),
                            format!(
                                "unknown argument `{}`; expected name, ty, required, default, desc, mock or children",
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
