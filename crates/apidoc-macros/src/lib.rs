//! Attribute macros for apidoc: title / desc / method / url / param / query /
//! returned (M1), tag / group / author / header / route_param /
//! response_status / success / error / not_debug / md / sort / ref (M3),
//! app (M6b), and table (v2).
//!
//! Each macro keeps the annotated function unchanged and emits a statically
//! registered `DocFragmentEntry` on the distributed slice `apidoc::DOC_FRAGMENTS`,
//! so documentation fragments are collected at zero runtime cost.

mod args;
mod protocols;

use args::{
    DefinitionArgs, EventArgs, HeaderArgs, ParamArgs, ProtocolArgs, ProtocolOptsArgs, SuccessArgs,
};
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote, quote_spanned};
use std::sync::atomic::{AtomicU32, Ordering};
use syn::parse::ParseStream;
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Ident, Item, ItemFn, LitBool, LitInt, LitStr, Token};

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

// —— M3: 13 new annotations ——

#[proc_macro_attribute]
pub fn tag(args: TokenStream, item: TokenStream) -> TokenStream {
    litstr_fragment("tag", args, item)
}

#[proc_macro_attribute]
pub fn group(args: TokenStream, item: TokenStream) -> TokenStream {
    simple_fragment("group", args, item)
}

#[proc_macro_attribute]
pub fn author(args: TokenStream, item: TokenStream) -> TokenStream {
    simple_fragment("author", args, item)
}

#[proc_macro_attribute]
pub fn header(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as HeaderArgs);
    let item_fn = match parse_item_fn("header", item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    if args.name.as_ref().is_none_or(|l| l.value().is_empty()) {
        return syn::Error::new(
            name_span(args.name.as_ref()),
            "apidoc::header requires a non-empty `name`",
        )
        .to_compile_error()
        .into();
    }
    let name = args.name.as_ref().unwrap();
    let desc = opt_lit(&args.desc);
    let frag = quote! { apidoc::DocFragment::Header(apidoc::DocHeader { name: #name, desc: #desc }) };
    emit_many("header", item_fn, vec![frag])
}

#[proc_macro_attribute]
pub fn route_param(args: TokenStream, item: TokenStream) -> TokenStream {
    param_fragment("route_param", args, item)
}

#[proc_macro_attribute]
pub fn response_status(args: TokenStream, item: TokenStream) -> TokenStream {
    litstr_fragment("response_status", args, item)
}

#[proc_macro_attribute]
pub fn success(args: TokenStream, item: TokenStream) -> TokenStream {
    example_fragment("success", args, item)
}

#[proc_macro_attribute]
pub fn error(args: TokenStream, item: TokenStream) -> TokenStream {
    example_fragment("error", args, item)
}

#[proc_macro_attribute]
pub fn not_debug(args: TokenStream, item: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(
            Span::call_site(),
            "apidoc::not_debug takes no arguments",
        )
        .to_compile_error()
        .into();
    }
    let item_fn = match parse_item_fn("not_debug", item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    emit_many("not_debug", item_fn, vec![quote! { apidoc::DocFragment::NotDebug }])
}

#[proc_macro_attribute]
pub fn md(args: TokenStream, item: TokenStream) -> TokenStream {
    simple_fragment("md", args, item)
}

#[proc_macro_attribute]
pub fn sort(args: TokenStream, item: TokenStream) -> TokenStream {
    // Integer literal, optionally negative: #[apidoc::sort(-1)].
    let parser = |input: ParseStream| -> syn::Result<i32> {
        let neg = input.peek(Token![-]);
        if neg {
            input.parse::<Token![-]>()?;
        }
        let lit: LitInt = input.parse()?;
        let n = lit.base10_parse::<i32>()?;
        Ok(if neg { -n } else { n })
    };
    let n = match syn::parse::Parser::parse2(parser, args.into()) {
        Ok(n) => n,
        Err(e) => return e.to_compile_error().into(),
    };
    let item_fn = match parse_item_fn("sort", item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    let frag = quote! { apidoc::DocFragment::Sort(#n) };
    emit_many("sort", item_fn, vec![frag])
}

#[proc_macro_attribute]
pub fn r#ref(args: TokenStream, item: TokenStream) -> TokenStream {
    simple_fragment("ref", args, item)
}

/// M6b: 将接口挂到指定应用/版本 key 下（key 须在 ApidocConfig.apps 中配置）。
#[proc_macro_attribute]
pub fn app(args: TokenStream, item: TokenStream) -> TokenStream {
    simple_fragment("app", args, item)
}

/// v2: 引用 ApidocConfig.tables 中的数据表 key，字段在 collect_doc 时平铺并入
/// returned（key 未配置只警告不报错，与 ref 同策略）。
#[proc_macro_attribute]
pub fn table(args: TokenStream, item: TokenStream) -> TokenStream {
    simple_fragment("table", args, item)
}

/// P0: 声明接口协议，例如
/// `#[apidoc::protocol("grpc", target = "http://127.0.0.1:50051", service = "x.Y", method = "Z")]`。
/// kind 在编译期归一（REST→http、ws→websocket…），未知 kind 直接编译报错。
#[proc_macro_attribute]
pub fn protocol(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as ProtocolArgs);
    let item_fn = match parse_item_fn("protocol", item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    let Some(kind) = protocols::normalize(&args.kind.value()) else {
        return syn::Error::new(
            args.kind.span(),
            format!(
                "unknown apidoc::protocol kind `{}`; expected one of {:?}",
                args.kind.value(),
                protocols::CANONICAL
            ),
        )
        .to_compile_error()
        .into();
    };
    let kind_lit = LitStr::new(kind, args.kind.span());
    let opts = args.opts.iter().map(|(key, value)| {
        let key_lit = LitStr::new(&key.to_string(), key.span());
        quote! { (#key_lit, #value) }
    });
    let frag = quote! {
        apidoc::DocFragment::Protocol(apidoc::DocProtocol {
            kind: #kind_lit,
            opts: &[#(#opts),*],
        })
    };
    emit_many("protocol", item_fn, vec![frag])
}

/// 每协议别名的共享实现：`#[apidoc::websocket(...)]` 展开为
/// `DocFragment::Protocol { kind: "websocket", opts }`。
fn protocol_alias_impl(
    kind: &'static str,
    macro_name: &str,
    args: TokenStream,
    item: TokenStream,
) -> TokenStream {
    let args = parse_macro_input!(args as ProtocolOptsArgs);
    let item_fn = match parse_item_fn(macro_name, item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    let kind_lit = LitStr::new(kind, Span::call_site());
    let opts = args.opts.iter().map(|(key, value)| {
        let key_lit = LitStr::new(&key.to_string(), key.span());
        quote! { (#key_lit, #value) }
    });
    let frag = quote! {
        apidoc::DocFragment::Protocol(apidoc::DocProtocol {
            kind: #kind_lit,
            opts: &[#(#opts),*],
        })
    };
    emit_many(&format!("protocol_{macro_name}"), item_fn, vec![frag])
}

macro_rules! protocol_aliases {
    ($(($fn_name:ident, $kind:literal)),* $(,)?) => {
        $(
            #[proc_macro_attribute]
            pub fn $fn_name(args: TokenStream, item: TokenStream) -> TokenStream {
                protocol_alias_impl($kind, stringify!($fn_name), args, item)
            }
        )*
    };
}

// 45 个规范协议的独立注解别名；底层与 protocol("kind", ...) 完全等价。
// 三个带连字符的 kind 用下划线作为宏名（Rust 标识符不能含 `-`）。
protocol_aliases!(
    (http, "http"),
    (http2, "http2"),
    (http3, "http3"),
    (soap, "soap"),
    (graphql, "graphql"),
    (websocket, "websocket"),
    (sse, "sse"),
    (socketio, "socketio"),
    (webrtc, "webrtc"),
    (grpc, "grpc"),
    (dubbo, "dubbo"),
    (thrift, "thrift"),
    (jsonrpc, "jsonrpc"),
    (xmlrpc, "xmlrpc"),
    (hessian, "hessian"),
    (amqp, "amqp"),
    (mqtt, "mqtt"),
    (stomp, "stomp"),
    (kafka, "kafka"),
    (webhook, "webhook"),
    (tcp, "tcp"),
    (udp, "udp"),
    (coap, "coap"),
    (lwm2m, "lwm2m"),
    (xmpp, "xmpp"),
    (sip, "sip"),
    (rtsp, "rtsp"),
    (rtmp, "rtmp"),
    (hls, "hls"),
    (mcp, "mcp"),
    (a2a, "a2a"),
    (iso8583, "iso8583"),
    (odata, "odata"),
    (raw, "raw"),
    (unix, "unix"),
    (http_binary, "http-binary"),
    (ws_binary, "ws-binary"),
    (stomp_binary, "stomp-binary"),
    (opcua, "opcua"),
    (modbus, "modbus"),
    (dnp3, "dnp3"),
    (iec104, "iec104"),
    (bacnet, "bacnet"),
    (rtp, "rtp"),
    (rtcp, "rtcp"),
);

/// P0: 声明长连接/流式协议的一条消息，例如
/// `#[apidoc::event(direction = "in", name = "message", ty = "json", desc = "收到消息")]`。
/// 同一函数可重复使用；direction 只接受 in / out。
#[proc_macro_attribute]
pub fn event(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as EventArgs);
    let item_fn = match parse_item_fn("event", item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    let Some(direction) = args.direction.as_ref() else {
        return syn::Error::new(Span::call_site(), "apidoc::event requires `direction`")
            .to_compile_error()
            .into();
    };
    let dir = direction.value().trim().to_ascii_lowercase();
    if dir != "in" && dir != "out" {
        return syn::Error::new(
            direction.span(),
            format!("apidoc::event direction must be `in` or `out`, got `{}`", direction.value()),
        )
        .to_compile_error()
        .into();
    }
    let dir_lit = LitStr::new(&dir, direction.span());
    let Some(name) = args.name.as_ref() else {
        return syn::Error::new(Span::call_site(), "apidoc::event requires a non-empty `name`")
            .to_compile_error()
            .into();
    };
    if name.value().trim().is_empty() {
        return syn::Error::new(name.span(), "apidoc::event requires a non-empty `name`")
            .to_compile_error()
            .into();
    }
    let ty = args.ty.as_ref().cloned().unwrap_or_else(|| LitStr::new("json", name.span()));
    let desc = opt_lit(&args.desc);
    let mock = opt_lit(&args.mock);
    let frag = quote! {
        apidoc::DocFragment::Event(apidoc::DocEvent {
            direction: #dir_lit,
            name: #name,
            ty: #ty,
            desc: #desc,
            mock: #mock,
        })
    };
    emit_many("event", item_fn, vec![frag])
}

/// UIC-1 Phase 4: 为端点挂一个外部协议定义文件，例如
/// `#[apidoc::definition(file = "api/openapi.yaml", operation = "getUser")]`。
/// 路径相对声明方 crate 的 `CARGO_MANIFEST_DIR`；文件在详情请求时解析。
#[proc_macro_attribute]
pub fn definition(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as DefinitionArgs);
    let Some(file) = args.file.as_ref() else {
        return syn::Error::new(Span::call_site(), "apidoc::definition requires `file`")
            .to_compile_error()
            .into();
    };
    if file.value().trim().is_empty() {
        return syn::Error::new(
            file.span(),
            "apidoc::definition requires a non-empty `file`",
        )
        .to_compile_error()
        .into();
    }
    let item_fn = match parse_item_fn("definition", item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    let empty = || LitStr::new("", Span::call_site());
    let format = args.format.unwrap_or_else(empty);
    let operation = args.operation.unwrap_or_else(empty);
    let message = args.message.unwrap_or_else(empty);
    let direction = args.direction.unwrap_or_else(empty);
    let sample = args.sample.unwrap_or_else(empty);
    let sample_file = args.sample_file.unwrap_or_else(empty);
    let sample_encoding = args.sample_encoding.unwrap_or_else(empty);
    let strict = args.strict.as_ref().map_or(true, LitBool::value);
    let frag = quote! {
        apidoc::DocFragment::Definition(apidoc::DocDefinition {
            file: #file,
            root: env!("CARGO_MANIFEST_DIR"),
            format: #format,
            operation: #operation,
            message: #message,
            direction: #direction,
            sample: #sample,
            sample_file: #sample_file,
            sample_encoding: #sample_encoding,
            strict: #strict,
        })
    };
    emit_many("definition", item_fn, vec![frag])
}

/// title / desc / method / url / group / author / md / ref / app / table:
/// a single string literal plus validation.
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
        "group" | "author" | "ref" | "app" | "table" if value.trim().is_empty() => Some(syn::Error::new(
            lit.span(),
            format!("apidoc::{kind} must not be empty"),
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
    emit_many(kind, item_fn, vec![frag])
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
    if args.name.as_ref().is_none_or(|l| l.value().is_empty()) {
        return syn::Error::new(
            name_span(args.name.as_ref()),
            format!("apidoc::{kind} requires a non-empty `name`"),
        )
        .to_compile_error()
        .into();
    }
    let variant = variant_ident(kind);
    let doc_param = doc_param_expr(&args);
    let frag = quote! { apidoc::DocFragment::#variant(#doc_param) };
    emit_many(kind, item_fn, vec![frag])
}

/// DocFragment enum variant name for a macro kind: "title" -> Title,
/// "response_status" -> ResponseStatus, "not_debug" -> NotDebug.
fn variant_ident(kind: &str) -> Ident {
    let upper = match kind {
        "response_status" => "ResponseStatus".to_string(),
        "route_param" => "RouteParam".to_string(),
        "not_debug" => "NotDebug".to_string(),
        _ => {
            let mut chars = kind.chars();
            let first = chars.next().unwrap().to_ascii_uppercase();
            format!("{first}{}", chars.as_str())
        }
    };
    Ident::new(&upper, Span::call_site())
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
    // 子参数允许省略 name（顶层在 param_fragment 里已校验非空）
    let name = args.name.as_ref().map(|l| l.value()).unwrap_or_default();
    let ty = args.ty.as_ref().map_or_else(|| "string".to_string(), |l| l.value());
    let required = args.required;
    let default = opt_lit(&args.default);
    let desc = opt_lit(&args.desc);
    let mock = opt_lit(&args.mock);
    let field = opt_lit(&args.field);
    let children = args.children.iter().map(doc_param_expr);
    quote! {
        apidoc::DocParam {
            name: #name,
            ty: #ty,
            required: #required,
            default: #default,
            desc: #desc,
            mock: #mock,
            field: #field,
            children: &[#(#children),*],
        }
    }
}

fn opt_lit(value: &Option<LitStr>) -> proc_macro2::TokenStream {
    match value {
        Some(lit) => quote! { Some(#lit) },
        None => quote! { None },
    }
}

/// 出错位置：给了字面量就指向它（精确到 token），没给才落回整个属性。
fn name_span(lit: Option<&LitStr>) -> Span {
    lit.map_or_else(Span::call_site, LitStr::span)
}

/// Emits the original function plus one static fragment registration per
/// fragment. Variadic annotations (tag / response_status) expand to several
/// statics from a single attribute.
fn emit_many(kind: &str, item_fn: ItemFn, frags: Vec<proc_macro2::TokenStream>) -> TokenStream {
    let fn_ident = item_fn.sig.ident.clone();
    // id 以「注解项所在行」收尾：同一函数的每个注解拿到同一个 id（行号相同），
    // 同模块内重名的两个函数（impl 方法、嵌套 fn）拿到不同 id。只用
    // module_path!()::fn_name 时二者会静默合并成一个端点。
    // line!() 的 span 钉在函数名上，才会报「项的所在行」而不是各注解自己所在的行。
    let line_tokens = quote_spanned! {fn_ident.span()=> line!() };
    let kind_upper = kind.to_uppercase();
    let mut out = quote! { #item_fn };
    for frag in frags {
        // seq makes the static name unique even for repeated same-name params
        // on one function, which would otherwise collide into a misleading
        // E0428; it also restores declaration order at collect time.
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let static_ident = format_ident!("__APIDOC_{kind_upper}_{fn_ident}_{seq}");
        out.extend(quote! {
            #[apidoc::distributed_slice(apidoc::DOC_FRAGMENTS)]
            static #static_ident: apidoc::DocFragmentEntry = apidoc::DocFragmentEntry {
                id: concat!(module_path!(), "::", stringify!(#fn_ident), "#", #line_tokens),
                seq: #seq,
                frag: #frag,
            };
        });
    }
    out.into()
}

/// tag / response_status: one or more string literals, each validated and
/// expanded into its own fragment registration.
fn litstr_fragment(kind: &str, args: TokenStream, item: TokenStream) -> TokenStream {
    let lits = match litstr_list(args) {
        Ok(l) => l,
        Err(e) => return e.to_compile_error().into(),
    };
    let item_fn = match parse_item_fn(kind, item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    for lit in &lits {
        if let Some(err) = validate_lit(kind, lit) {
            return err.to_compile_error().into();
        }
    }
    let variant = variant_ident(kind);
    let frags = lits.iter().map(|lit| quote! { apidoc::DocFragment::#variant(#lit) }).collect();
    emit_many(kind, item_fn, frags)
}

/// Parses a comma-separated list of one or more string literals.
fn litstr_list(args: TokenStream) -> syn::Result<Vec<LitStr>> {
    let list = syn::parse::Parser::parse2(
        Punctuated::<LitStr, Token![,]>::parse_terminated,
        args.into(),
    )?;
    if list.is_empty() {
        Err(syn::Error::new(
            Span::call_site(),
            "expected at least one string literal",
        ))
    } else {
        Ok(list.into_iter().collect())
    }
}

fn validate_lit(kind: &str, lit: &LitStr) -> Option<syn::Error> {
    let v = lit.value();
    match kind {
        "response_status"
            if v.parse::<u16>().map_or(true, |n| !(100..=599).contains(&n)) =>
        {
            Some(syn::Error::new(
                lit.span(),
                format!(
                    "apidoc::response_status must be a numeric HTTP status code 100-599, got `{v}`"
                ),
            ))
        }
        "tag" if v.trim().is_empty() => {
            Some(syn::Error::new(lit.span(), "apidoc::tag must not be empty"))
        }
        _ => None,
    }
}

/// success / error: `code` and `example` are both required.
fn example_fragment(kind: &str, args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as SuccessArgs);
    let item_fn = match parse_item_fn(kind, item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    let Some(code) = args.code.as_ref() else {
        return syn::Error::new(
            Span::call_site(),
            format!("apidoc::{kind} requires `code`"),
        )
        .to_compile_error()
        .into();
    };
    if code.value().parse::<u16>().map_or(true, |n| !(100..=599).contains(&n)) {
        return syn::Error::new(
            code.span(),
            format!(
                "apidoc::{kind} code must be a numeric HTTP status code 100-599, got `{}`",
                code.value()
            ),
        )
        .to_compile_error()
        .into();
    }
    let Some(example) = args.example.as_ref() else {
        return syn::Error::new(
            Span::call_site(),
            format!("apidoc::{kind} requires `example`"),
        )
        .to_compile_error()
        .into();
    };
    let variant = variant_ident(kind);
    let frag = quote! {
        apidoc::DocFragment::#variant(apidoc::DocExample { code: #code, example: #example })
    };
    emit_many(kind, item_fn, vec![frag])
}

