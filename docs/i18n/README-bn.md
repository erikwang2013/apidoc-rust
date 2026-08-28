<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 রাস্ট প্রসেস-ম্যাক্রো (proc-macro) দিয়ে API ইন্টারফেস ডকুমেন্টেশন তৈরি করার সাধারণ প্লাগইন লাইব্রেরি
</div>

<div align="center">
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/badge/license-MIT-green"></a>
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/github/stars/erikwang2013/apidoc-rust"></a>
</div>

<div align="center">
<a href="../../README.md">中文</a> ·
<a href="README-en.md">English</a> ·
<a href="README-ko.md">한국어</a> ·
<a href="README-ru.md">Русский</a> ·
<a href="README-de.md">Deutsch</a> ·
<a href="README-fr.md">Français</a> ·
<a href="README-es.md">Español</a> ·
<a href="README-pt.md">Português</a> ·
<a href="README-hi.md">हिन्दी</a> ·
<a href="README-ar.md">العربية</a> ·
<a href="README-bn.md"><strong>বাংলা</strong></a> ·
<a href="README-id.md">Bahasa Indonesia</a> ·
<a href="README-ja.md">日本語</a>
</div>

## প্রকল্প পরিচিতি

apidoc-rust হল একটি রাস্টে বাস্তবায়িত **সাধারণ প্লাগইন-ভিত্তিক API ইন্টারফেস ডকুমেন্টেশন জেনারেটর**, যা [apidoc-php](https://github.com/erikwang2013/apidoc-php) (PHP 8 attributes-এর ভিত্তিতে API ডকুমেন্টেশন তৈরি করা composer এক্সটেনশন)-কে অনুসরণ করে, "অ্যানোটেশনই ডকুমেন্টেশন" ধারণাটিকে রাস্টের নেটিভ উপায়ে বাস্তবায়ন করে:

- **কম্পাইল-টাইম জেনারেশন**: প্রসেস-ম্যাক্রো কম্পাইল-টাইমে ডকুমেন্টেশন তৈরি করে, ডকুমেন্টেশন ও কোড কখনোই সিঙ্কের বাইরে যায় না;
- **শূন্য-খরচ সংগ্রহ**: linkme স্ট্যাটিক রেজিস্ট্রেশনের মাধ্যমে, রানটাইমে একবার অ্যাগ্রিগেশন করলেই সব ইন্টারফেস ডকুমেন্টেশন পাওয়া যায়;
- **সাধারণ প্লাগইন**: কোর HTTP ফ্রেমওয়ার্ক-নিরপেক্ষ, পাতলা অ্যাডাপ্টার (axum / actix-web)-এর মাধ্যমে যেকোনো ফ্রেমওয়ার্কে সংযুক্ত হয়।

## বৈশিষ্ট্য

### বাস্তবায়িত (M1)

- **অ্যানোটেশন-ভিত্তিক ডকুমেন্টেশন**: `title` / `desc` / `method` / `url` / `param` / `query` / `returned` সাতটি অ্যাট্রিবিউট ম্যাক্রো, একেকটি করে অ্যানোটেশন (PHP attributes লেখার সাথে সামঞ্জস্যপূর্ণ), প্যারামিটারগুলো `required` / `default` / `desc` / `mock` / `children` নেস্টিং সমর্থন করে
- **কম্পাইল-টাইম ভ্যালিডেশন**: url অবশ্যই `/` দিয়ে শুরু হতে হবে, method হোয়াইটলিস্ট, param name বাধ্যতামূলক ইত্যাদি — অবৈধ অ্যানোটেশনে কম্পাইল-টাইম এরর (span সঠিক)
- **স্বয়ংক্রিয় সংগ্রহ**: linkme `distributed_slice` স্ট্যাটিক রেজিস্ট্রেশন, ম্যানুয়াল ইন্টারফেস তালিকার দরকার নেই; `DocRegistry::collect()` id অনুযায়ী মার্জ করে, seq অনুযায়ী ডিক্লারেশন ক্রম পুনরুদ্ধার করে, ক্রস-crate স্বয়ংক্রিয় সংগ্রহ
- **api.json আউটপুট**: serde সিরিয়ালাইজেশনের মাধ্যমে ইউনিফাইড ডকুমেন্ট ডেটা মডেল (config + endpoints), ফিল্ডগুলো PHP সেমান্টিকসের সাথে সামঞ্জস্যপূর্ণ

### পরিকল্পনাধীন

- অনলাইন ডিবাগিং (ব্রাউজার থেকে CORS-এ সরাসরি টার্গেট ইন্টারফেস), Mock ডেটা (fake রুল দিয়ে জেনারেশন)
- মাল্টি-অ্যাপ / মাল্টি-ভার্সন / অ্যাক্সেস পাসওয়ার্ড
- Markdown / TypeScript / Swagger (OpenAPI3) এক্সপোর্ট
- মাল্টি-ফ্রেমওয়ার্ক অ্যাডাপ্টেশন (apidoc-axum / apidoc-actix)
- v2: কোড জেনারেটর, ডেটা টেবিল ফিল্ড রেফারেন্স, শেয়ার লিংক, ডিবাগ ইভেন্ট

## আর্কিটেকচার

<img src="images/bn-architecture.svg" alt="apidoc-rust সামগ্রিক আর্কিটেকচার" width="100%">

## কার্যকারিতা

<img src="images/bn-features.svg" alt="apidoc-rust প্রজেক্ট কার্যকারিতা" width="100%">

## জীবনচক্র

<img src="images/bn-lifecycle.svg" alt="apidoc-rust ডকুমেন্টেশন জীবনচক্র" width="100%">

## প্রজেক্ট কাঠামো

```
apidoc-rust/
├── Cargo.toml                 # workspace কনফিগারেশন (resolver 2)
├── crates/
│   ├── apidoc/                # রানটাইম কোর (ফ্রেমওয়ার্ক-নিরপেক্ষ)
│   │   ├── src/lib.rs         # ডেটা মডেল + DocRegistry অ্যাগ্রিগেশন + api.json
│   │   ├── tests/             # ইন্টিগ্রেশন টেস্ট (ম্যাক্রো এক্সপানশন/অ্যাগ্রিগেশন/সিরিয়ালাইজেশন/ক্রস-crate)
│   │   └── examples/demo.rs   # উদাহরণ: অ্যানোটেশন + api.json আউটপুট
│   ├── apidoc-macros/         # proc-macro: ৭টি অ্যাট্রিবিউট ম্যাক্রো
│   │   └── src/lib.rs         # ম্যাক্রো সংজ্ঞা + প্যারামিটার পার্সিং + কম্পাইল-টাইম ভ্যালিডেশন
│   └── apidoc-test-fixtures/  # ক্রস-crate রেজিস্ট্রেশন টেস্ট ফিক্সচার
└── docs/
    ├── images/                # আর্কিটেকচার/কার্যকারিতা/জীবনচক্র চিত্র (SVG)
    └── i18n/                  # বহুভাষিক ডকুমেন্টেশন (১২টি ভাষা)
```

## ব্যবহার নির্দেশিকা

### ১. ডিপেন্ডেন্সি যোগ করা

```toml
[dependencies]
apidoc = "0.1"        # অথবা path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # ম্যাক্রো এক্সপানশন সরাসরি linkme পাথ রেফার করে, ভোক্তাকে সরাসরি ডিপেন্ডেন্সি দিতে হবে
serde_json = "1"      # api.json আউটপুটের জন্য
```

### ২. অ্যানোটেশন লেখা

handler ফাংশনে একেকটি করে অ্যানোটেশন লাগান, ডকুমেন্টেশন কম্পাইল-টাইমেই তৈরি হবে:

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

### ৩. সংগ্রহ ও আউটপুট

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

### ৪. উদাহরণ চালানো

```bash
cargo run --example demo -p apidoc
```

আউটপুট (নির্বাচিত অংশ):

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

## উন্নয়ন পরিকল্পনা

| পর্যায় | বিষয়বস্তু | অবস্থা |
|------|------|------|
| M1 | workspace কঙ্কাল + ডেটা মডেল + ম্যাক্রো MVP + linkme রেজিস্ট্রেশন | ✅ সম্পন্ন |
| M2 | axum অ্যাডাপ্টার + এমবেডেড ডকুমেন্ট UI + গ্রুপড ডিরেক্টরি | ⏳ পরিকল্পনাধীন |
| M3 | অ্যানোটেশন সম্পূর্ণকরণ (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | পরিকল্পনাধীন |
| M4 | অনলাইন ডিবাগিং + Mock ইঞ্জিন | পরিকল্পনাধীন |
| M5 | markdown / typescript / swagger.json এক্সপোর্ট | পরিকল্পনাধীন |
| M6 | পাসওয়ার্ড অথেনটিকেশন, মাল্টি-অ্যাপ মাল্টি-ভার্সন, রিলিজ | পরিকল্পনাধীন |

## বহুভাষিক ডকুমেন্টেশন

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

## সমর্থন ও দান

এই প্রজেক্ট যদি আপনার কাজে লাগে, তাহলে ⭐ Star দিয়ে আমাদের সমর্থন করুন, এবং ওপেন-সোর্স সমর্থনে দানও স্বাগতম!

### 微信支付 / 支付宝 (WeChat Pay / Alipay)

<table>
  <tr>
    <td align="center">
      <img src="../../docs/weixinpay.png" width="130" height="130" alt="微信支付" /><br/>
      <strong>微信支付</strong>
    </td>
    <td align="center">
      <img src="../../docs/alipay.png" width="130" height="130" alt="支付宝" /><br/>
      <strong>支付宝</strong>
    </td>
  </tr>
</table>

### বৈশ্বিক ব্যাংক ট্রান্সফার দান

**【প্রাপকের তথ্য】**

- প্রাপকের নাম: WANG KEXUN
- প্রাপক অ্যাকাউন্ট নম্বর: 881015918251

**【প্রাপক ব্যাংক】**

- ZA Bank SWIFT Code: AABLHKHHXXX
- ব্যাংকের নাম: ZA Bank Limited
- ব্যাংক কোড: 387
- ব্যাংকের ঠিকানা: Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【ক্রস-বর্ডার রেমিট্যান্স করেসপন্ডেন্ট ব্যাংক (যদি প্রয়োজন হয়)】**

> দয়া করে লক্ষ্য করুন, এটি ক্রস-বর্ডার রেমিট্যান্স করেসপন্ডেন্ট (মধ্যস্থ ব্যাংক) এর তথ্য, প্রাপক ব্যাংকের তথ্য নয়। রেমিট্যান্স পাঠানোর ব্যাংককে জিজ্ঞাসা করুন ক্রস-বর্ডার করেসপন্ডেন্ট ব্যাংকের তথ্য প্রদান প্রয়োজন কিনা।

- **হংকং ডলার, চাইনিজ ইউয়ান ও মার্কিন ডলার জমা হলে করেসপন্ডেন্ট ব্যাংক Citibank:**
  - ব্যাংকের নাম: Citibank N.A. Hong Kong
  - SWIFT Code: CITIHKHXXXX
  - ব্যাংক কোড: 006
  - শাখার নাম: Hong Kong Branch
  - শাখা কোড: 391
  - ব্যাংকের ঠিকানা: Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **অন্যান্য মুদ্রা জমা হলে করেসপন্ডেন্ট ব্যাংক BNY Mellon:**
  - ব্যাংকের নাম: THE BANK OF NEW YORK MELLON
  - SWIFT Code: IRVTUS3NXXX
  - ব্যাংকের ঠিকানা: THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## লাইসেন্স

[MIT](../../LICENSE)
