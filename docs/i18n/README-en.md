<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 A general-purpose plugin library that generates API documentation from Rust procedural macros (proc-macro)
</div>

<div align="center">
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/badge/license-MIT-green"></a>
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/github/stars/erikwang2013/apidoc-rust"></a>
</div>

<div align="center">
<a href="../../README.md">中文</a> ·
<a href="README-en.md"><strong>English</strong></a> ·
<a href="README-ko.md">한국어</a> ·
<a href="README-ru.md">Русский</a> ·
<a href="README-de.md">Deutsch</a> ·
<a href="README-fr.md">Français</a> ·
<a href="README-es.md">Español</a> ·
<a href="README-pt.md">Português</a> ·
<a href="README-hi.md">हिन्दी</a> ·
<a href="README-ar.md">العربية</a> ·
<a href="README-bn.md">বাংলা</a> ·
<a href="README-id.md">Bahasa Indonesia</a> ·
<a href="README-ja.md">日本語</a>
</div>

## Introduction

apidoc-rust is a **general-purpose pluggable API documentation generator** implemented in Rust, inspired by [apidoc-php](https://github.com/erikwang2013/apidoc-php) (a composer extension that generates API documentation from PHP 8 attributes). It brings "annotations as documentation" to the Rust ecosystem the native way:

- **Generated at compile time**: documentation is produced by procedural macros during compilation, so the docs can never drift out of sync with the code;
- **Zero-cost collection**: static registration via linkme, a single pass at runtime aggregates all endpoint documentation;
- **Generic plugin core**: the core is HTTP-framework-agnostic and plugs into any framework through thin adapters (axum / actix-web).

## Features

### Implemented (M1)

- **Annotation-based documentation**: seven attribute macros — `title` / `desc` / `method` / `url` / `param` / `query` / `returned` — annotated one by one (mirroring the PHP attributes style); parameters support nested `required` / `default` / `desc` / `mock` / `children`
- **Compile-time validation**: url must start with `/`, method whitelist, param name is required, etc.; invalid annotations fail at compile time with precise spans
- **Automatic collection**: static registration via linkme `distributed_slice`, no manual endpoint manifest needed; `DocRegistry::collect()` merges fragments by id and restores declaration order by seq, collecting automatically across crates
- **api.json output**: serde serializes the unified document data model (config + endpoints), with fields aligned to PHP semantics

### Planned

- Online debugging (browser CORS direct connection to target endpoints), Mock data (fake-rule generation)
- Multi-app / multi-version / access password
- Export to Markdown / TypeScript / Swagger (OpenAPI3)
- Multi-framework adapters (apidoc-axum / apidoc-actix)
- v2: code generator, data-table field references, share links, debug events

## Architecture

<img src="images/en-architecture.svg" alt="apidoc-rust overall architecture" width="100%">

## Features

<img src="images/en-features.svg" alt="apidoc-rust project features" width="100%">

## Lifecycle

<img src="images/en-lifecycle.svg" alt="apidoc-rust documentation lifecycle" width="100%">

## Project Structure

```
apidoc-rust/
├── Cargo.toml                 # workspace config (resolver 2)
├── crates/
│   ├── apidoc/                # runtime core (framework-agnostic)
│   │   ├── src/lib.rs         # data model + DocRegistry aggregation + api.json
│   │   ├── tests/             # integration tests (macro expansion/aggregation/serialization/cross-crate)
│   │   └── examples/demo.rs   # example: annotations + api.json output
│   ├── apidoc-macros/         # proc-macro: 7 attribute macros
│   │   └── src/lib.rs         # macro definitions + argument parsing + compile-time validation
│   └── apidoc-test-fixtures/  # cross-crate registration test fixtures
└── docs/
    ├── images/                # architecture/features/lifecycle diagrams (SVG)
    └── i18n/                  # multilingual documentation (12 languages)
```

## Usage

### 1. Add the dependency

```toml
[dependencies]
apidoc = "0.1"        # or path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # macro expansion references the linkme path directly; consumers must depend on it
serde_json = "1"      # for api.json output
```

### 2. Write annotations

Attach annotations to handler functions one by one, and the documentation is generated at compile time:

```rust
use apidoc::*;

#[apidoc::title("获取用户信息")]
#[apidoc::desc("根据用户 ID 查询用户详情")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::param(name = "user_id", ty = "int", required, desc = "用户ID", mock = "1")]
#[apidoc::query(name = "lang", ty = "string", desc = "语言", default = "zh-CN")]
#[apidoc::returned(
    name = "data",
    ty = "object",
    desc = "用户数据",
    children = [
        { name = "id", ty = "int", required, desc = "用户ID" },
        { name = "name", ty = "string", required, desc = "用户名", mock = "erik" },
    ]
)]
fn get_user_info() -> String {
    unimplemented!()
}
```

### 3. Collect and output

```rust
fn main() {
    let endpoints = DocRegistry::collect();
    let doc = ApiDoc {
        config: ApidocConfig {
            title: "我的 API".to_string(),
            description: None,
        },
        endpoints,
    };
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}
```

### 4. Run the example

```bash
cargo run --example demo -p apidoc
```

Output (excerpt):

```json
{
  "config": { "title": "demo api" },
  "endpoints": [
    {
      "title": "获取用户信息",
      "desc": "根据用户 ID 查询用户详情",
      "url": "/api/user/info",
      "method": "GET",
      "params": [
        { "name": "user_id", "type": "int", "required": true, "desc": "用户ID", "mock": "1" }
      ],
      "querys": [
        { "name": "lang", "type": "string", "required": false, "default": "zh-CN", "desc": "语言" }
      ],
      "returned": [
        {
          "name": "data",
          "type": "object",
          "required": false,
          "desc": "用户数据",
          "children": [
            { "name": "id", "type": "int", "required": true, "desc": "用户ID" },
            { "name": "name", "type": "string", "required": true, "desc": "用户名", "mock": "erik" }
          ]
        }
      ]
    }
  ]
}
```

## Roadmap

| Phase | Content | Status |
|-------|---------|--------|
| M1 | workspace skeleton + data model + macro MVP + linkme registration | ✅ Done |
| M2 | axum adapter + embedded docs UI + grouped catalog | ⏳ Planned |
| M3 | annotation completion (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | Planned |
| M4 | online debugging + Mock engine | Planned |
| M5 | export markdown / typescript / swagger.json | Planned |
| M6 | password auth, multi-app multi-version, release | Planned |

## Multilingual Documentation

- [English](README-en.md)
- [한국어](README-ko.md)
- [Русский](README-ru.md)
- [Deutsch](README-de.md)
- [Français](README-fr.md)
- [Español](README-es.md)
- [Português](README-pt.md)
- [हिन्दी](README-hi.md)
- [العربية](README-ar.md)
- [বাংলা](README-bn.md)
- [Bahasa Indonesia](README-id.md)
- [日本語](README-ja.md)

## Support & Donations

If this project helps you, feel free to give us a ⭐ Star — donations to support open source are also welcome!

### 微信支付 (WeChat Pay) / 支付宝 (Alipay)

<table>
  <tr>
    <td align="center">
      <img src="../../docs/weixinpay.png" width="130" height="130" alt="微信支付 (WeChat Pay)" /><br/>
      <strong>微信支付 (WeChat Pay)</strong>
    </td>
    <td align="center">
      <img src="../../docs/alipay.png" width="130" height="130" alt="支付宝 (Alipay)" /><br/>
      <strong>支付宝 (Alipay)</strong>
    </td>
  </tr>
</table>

### Global Bank Transfer Donation

**Recipient Information**

- Recipient name: WANG KEXUN
- Recipient account number: 881015918251

**Receiving Bank**

- ZA Bank SWIFT Code: AABLHKHHXXX
- Bank name: ZA Bank Limited
- Bank code: 387
- Bank address: Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**Correspondent Bank for Cross-Border Remittance (if required)**

> Please note that the following is the correspondent (intermediary) bank for cross-border remittance, not the receiving bank. Please check with your remitting bank whether correspondent bank information is required.

- **Citibank is the correspondent bank for HKD, CNY and USD remittances:**
  - Bank name: Citibank N.A. Hong Kong
  - SWIFT Code: CITIHKHXXXX
  - Bank code: 006
  - Branch name: Hong Kong Branch
  - Branch code: 391
  - Bank address: Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **BNY Mellon is the correspondent bank for remittances in other currencies:**
  - Bank name: THE BANK OF NEW YORK MELLON
  - SWIFT Code: IRVTUS3NXXX
  - Bank address: THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## License

[MIT](../../LICENSE)
