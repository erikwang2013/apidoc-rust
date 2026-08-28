<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 基于 Rust 过程宏（proc-macro）生成 API 接口文档的通用插件库
</div>

<div align="center">
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/badge/license-MIT-green"></a>
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/github/stars/erikwang2013/apidoc-rust"></a>
</div>

<div align="center">
<a href="README.md"><strong>中文</strong></a> ·
<a href="docs/i18n/README-en.md">English</a> ·
<a href="docs/i18n/README-ko.md">한국어</a> ·
<a href="docs/i18n/README-ru.md">Русский</a> ·
<a href="docs/i18n/README-de.md">Deutsch</a> ·
<a href="docs/i18n/README-fr.md">Français</a> ·
<a href="docs/i18n/README-es.md">Español</a> ·
<a href="docs/i18n/README-pt.md">Português</a> ·
<a href="docs/i18n/README-hi.md">हिन्दी</a> ·
<a href="docs/i18n/README-ar.md">العربية</a> ·
<a href="docs/i18n/README-bn.md">বাংলা</a> ·
<a href="docs/i18n/README-id.md">Bahasa Indonesia</a> ·
<a href="docs/i18n/README-ja.md">日本語</a>
</div>

## 项目介绍

apidoc-rust 是一个用 Rust 实现的**通用插件式 API 接口文档生成器**，参考 [apidoc-php](https://github.com/erikwang2013/apidoc-php)（基于 PHP 8 attributes 生成 API 文档的 composer 扩展），把"注解即文档"的能力以 Rust 原生方式落地：

- **编译期生成**：文档由过程宏在编译期生成，文档与代码永不失同步；
- **零成本收集**：linkme 静态注册，运行期一次聚合即得全部接口文档；
- **通用插件**：核心与 HTTP 框架无关，通过薄适配器（axum / actix-web）接入任意框架。

## 特性

### 已实现（M1）

- **注解式文档**：`title` / `desc` / `method` / `url` / `param` / `query` / `returned` 七个属性宏，逐条注解（对应 PHP attributes 写法），参数支持 `required` / `default` / `desc` / `mock` / `children` 嵌套
- **编译期校验**：url 必须以 `/` 开头、method 白名单、param name 必填等，非法注解编译期报错（span 精确）
- **自动收集**：linkme `distributed_slice` 静态注册，无需手动接口清单；`DocRegistry::collect()` 按 id 合并、按 seq 恢复声明顺序，跨 crate 自动收集
- **api.json 输出**：serde 序列化统一文档数据模型（config + endpoints），字段对齐 PHP 语义

### 规划中

- 在线调试（浏览器 CORS 直连目标接口）、Mock 数据（fake 规则生成）
- 多应用 / 多版本 / 访问密码
- 导出 Markdown / TypeScript / Swagger（OpenAPI3）
- 多框架适配（apidoc-axum / apidoc-actix）
- v2：代码生成器、数据表字段引用、分享链接、调试事件

## 架构

<img src="docs/images/architecture.svg" alt="apidoc-rust 总体架构" width="100%">

## 功能

<img src="docs/images/features.svg" alt="apidoc-rust 项目功能" width="100%">

## 生命周期

<img src="docs/images/lifecycle.svg" alt="apidoc-rust 文档生命周期" width="100%">

## 项目结构

```
apidoc-rust/
├── Cargo.toml                 # workspace 配置（resolver 2）
├── crates/
│   ├── apidoc/                # 运行时核心（框架无关）
│   │   ├── src/lib.rs         # 数据模型 + DocRegistry 聚合 + api.json
│   │   ├── tests/             # 集成测试（宏展开/聚合/序列化/跨 crate）
│   │   └── examples/demo.rs   # 示例：注解 + 输出 api.json
│   ├── apidoc-macros/         # proc-macro：7 个属性宏
│   │   └── src/lib.rs         # 宏定义 + 参数解析 + 编译期校验
│   └── apidoc-test-fixtures/  # 跨 crate 注册测试夹具
└── docs/
    ├── images/                # 架构/功能/生命周期图（SVG）
    └── i18n/                  # 多语言文档（12 种语言）
```

## 使用说明

### 1. 添加依赖

```toml
[dependencies]
apidoc = "0.1"        # 或 path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # 宏展开直接引用 linkme 路径，消费方需直接依赖
serde_json = "1"      # 输出 api.json 用
```

### 2. 编写注解

在 handler 函数上逐条挂注解，文档即在编译期生成：

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

### 3. 收集与输出

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

### 4. 运行示例

```bash
cargo run --example demo -p apidoc
```

输出（节选）：

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

## 开发计划

| 阶段 | 内容 | 状态 |
|------|------|------|
| M1 | workspace 骨架 + 数据模型 + 宏 MVP + linkme 注册 | ✅ 已完成 |
| M2 | axum 适配器 + 内嵌文档 UI + 分组目录 | ✅ 已完成 |
| M3 | 注解补齐（tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref） | 规划中 |
| M4 | 在线调试 + Mock 引擎 | 规划中 |
| M5 | 导出 markdown / typescript / swagger.json | 规划中 |
| M6 | 密码鉴权、多应用多版本、发布 | 规划中 |

## 多语言文档

- [English](docs/i18n/README-en.md)
- [한국어](docs/i18n/README-ko.md)
- [Русский](docs/i18n/README-ru.md)
- [Deutsch](docs/i18n/README-de.md)
- [Français](docs/i18n/README-fr.md)
- [Español](docs/i18n/README-es.md)
- [Português](docs/i18n/README-pt.md)
- [हिन्दी](docs/i18n/README-hi.md)
- [العربية](docs/i18n/README-ar.md)
- [বাংলা](docs/i18n/README-bn.md)
- [Bahasa Indonesia](docs/i18n/README-id.md)
- [日本語](docs/i18n/README-ja.md)

## 支持与打赏

如果本项目对您有所帮助，欢迎点个 ⭐ Star 支持我们，也欢迎打赏支持开源！

### 微信 / 支付宝

<table>
  <tr>
    <td align="center">
      <img src="docs/weixinpay.png" width="130" height="130" alt="微信支付" /><br/>
      <strong>微信支付</strong>
    </td>
    <td align="center">
      <img src="docs/alipay.png" width="130" height="130" alt="支付宝" /><br/>
      <strong>支付宝</strong>
    </td>
  </tr>
</table>

### 全球转账打赏

**【收款人信息】**

- 收款人姓名：WANG KEXUN
- 收款账户号码：881015918251

**【收款银行】**

- ZA Bank SWIFT Code：AABLHKHHXXX
- 银行名称：ZA Bank Limited
- 银行编号：387
- 银行地址：Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【跨境汇款代理银行（如需）】**

> 请留意，此为跨境汇款代理银行（中转银行）信息，非收款银行信息。请向汇款银行查询是否需要提供跨境汇款代理银行信息。

- **汇入港元、人民币及美元的代理银行为 Citibank：**
  - 银行名称：Citibank N.A. Hong Kong
  - SWIFT Code：CITIHKHXXXX
  - 银行编号：006
  - 分行名称：Hong Kong Branch
  - 分行编号：391
  - 银行地址：Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **汇入其他币种时的代理银行为 BNY Mellon：**
  - 银行名称：THE BANK OF NEW YORK MELLON
  - SWIFT Code：IRVTUS3NXXX
  - 银行地址：THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## License

[MIT](LICENSE)
