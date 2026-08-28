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

### বাস্তবায়িত (M1–M3)

- **অ্যানোটেশন-ভিত্তিক ডকুমেন্টেশন**: `title` / `desc` / `method` / `url` / `param` / `query` / `returned` সাতটি অ্যাট্রিবিউট ম্যাক্রো, একেকটি করে অ্যানোটেশন (PHP attributes লেখার সাথে সামঞ্জস্যপূর্ণ), প্যারামিটারগুলো `required` / `default` / `desc` / `mock` / `children` নেস্টিং সমর্থন করে
- **কম্পাইল-টাইম ভ্যালিডেশন**: url অবশ্যই `/` দিয়ে শুরু হতে হবে, method হোয়াইটলিস্ট, param name বাধ্যতামূলক ইত্যাদি — অবৈধ অ্যানোটেশনে কম্পাইল-টাইম এরর (span সঠিক)
- **স্বয়ংক্রিয় সংগ্রহ**: linkme `distributed_slice` স্ট্যাটিক রেজিস্ট্রেশন, ম্যানুয়াল ইন্টারফেস তালিকার দরকার নেই; `DocRegistry::collect()` id অনুযায়ী মার্জ করে, seq অনুযায়ী ডিক্লারেশন ক্রম পুনরুদ্ধার করে, ক্রস-crate স্বয়ংক্রিয় সংগ্রহ
- **api.json আউটপুট**: serde সিরিয়ালাইজেশনের মাধ্যমে ইউনিফাইড ডকুমেন্ট ডেটা মডেল (config + endpoints), ফিল্ডগুলো PHP সেমান্টিকসের সাথে সামঞ্জস্যপূর্ণ
- **axum অ্যাডাপ্টার + এমবেডেড ডকুমেন্ট UI**: রাউট মাউন্ট করলেই ডকুমেন্টেশন পেজ পাওয়া যায়, গ্রুপড ডিরেক্টরি ব্রাউজিং (M2)
- **অ্যানোটেশন সম্পূর্ণকরণ**: `tag` / `group` / `author` / `header` / `route_param` / `response_status` / `success` / `error` / `not_debug` / `md` / `sort` / `ref` ১২টি নতুন অ্যানোটেশন (M3)

### বাস্তবায়িত (M4)

- **অনলাইন ডিবাগিং**: ডকুমেন্ট পেজে বিল্ট-ইন «অনলাইন ডিবাগিং» প্যানেল — Base URL প্রি-ফিল হয় `location.origin` দিয়ে ক্রস-অরিজিনে সরাসরি টার্গেট সার্ভিসে, প্যারামিটার ফর্ম mock দিয়ে প্রি-ফিল, `{name}` / `:name` রাউট প্লেসহোল্ডার রিপ্লেসমেন্ট, GET/HEAD প্যারামিটার query-তে, বাকি methods JSON body হিসেবে, রিকোয়েস্ট হেডার এডিট + কাস্টম header, রেসপন্স ডিসপ্লে (স্ট্যাটাস / সময় / pretty JSON), CORS ফেল হলে হলুদ সতর্কতা
- **Mock ইঞ্জিন** (`crates/apidoc-mock`, fake crate-এর উপর নির্ভরশীল, ১৫টি রুল: name / company / email / phone / url / ip / city / country / text / number / int / float / bool / uuid / date)। রুল প্রায়োরিটি: `mock="fake:xxx"` fake রুল টেবিলে যায় (অজানা নাম ডিফল্ট ভ্যালুতে ফলব্যাক) ← বাকি নন-খালি mock অপরিবর্তিত আউটপুট (যেমন `mock="1"`, `mock="erik"`) ← mock না থাকলে `ty` অনুযায়ী অটো-জেনারেশন (int←`"1"`, float←`"0.5"`, bool←`"true"`, object←`"{}"`, string←`"string"`)؛ children রিকার্সিভ নেস্টিং, array ফিক্সড ২টি আইটেম
- **mock ইন্টারফেস**: axum অ্যাডাপ্টারে নতুন `GET /apidoc/mock?url=&method=`, url + method এক্সাক্ট ম্যাচ, ম্যাচ না হলে 404; ডিবাগ প্যানেল ডিফল্টে `not_debug` এন্ডপয়েন্ট লুকিয়ে রাখে, «not_debug ইন্টারফেস দেখান» টিক দিলে দেখায়
- **CORS ডাইরেক্ট**: অনলাইন ডিবাগিং ব্রাউজার থেকে সরাসরি টার্গেট ইন্টারফেসে সংযোগ করে, অ্যাডাপ্টারের `cors_layer` পারমিশন দেয় (সার্ভার-সাইড রিভার্স প্রক্সি v2-তে)

### বাস্তবায়িত (M5)

- **তিন ফরম্যাটে এক্সপোর্ট** (`crates/apidoc/src/export/`): markdown / typescript / swagger (OpenAPI 3.0.0), কোর ক্রেট `export::markdown::render` / `export::typescript::render` / `export::swagger::render` সরবরাহ করে
- **এক্সপোর্ট রাউট**: অ্যাডাপ্টারে নতুন `GET /apidoc/export?format=md|ts|swagger`, অজানা format-এ 400; Content-Type যথাক্রমে `text/markdown` / `application/typescript` / `application/json`
- **markdown**: গ্রুপড ডিরেক্টরি + প্যারামিটার টেবিল + রেসপন্স ব্লক; **typescript**: group নেমস্পেস অনুযায়ী `{Name}Params` / `{Name}Result` টাইপ তৈরি, গ্রুপবিহীন ইন্টারফেস `defaultGroup`-এ পড়ে (`default` TS রিজার্ভড শব্দ); **swagger**: `info.version` রুটের `VERSION` ফাইলের কনটেন্ট থেকে নেওয়া
- **actix-web অ্যাডাপ্টার** (`crates/apidoc-actix`): axum অ্যাডাপ্টারের সাথে 1:1 ফাংশনালিটি—`apidoc_routes(ApidocConfig) -> Scope` /apidoc, /apidoc/api.json, /apidoc/mock, /apidoc/export মাউন্ট করে, `cors_layer(CorsConfig)` ক্রস-অরিজিন পারমিশন দেয়
- **UI শেয়ারিং**: ডকুমেন্ট UI (`src/ui.html`) কোর ক্রেটে স্থানান্তরিত, `pub const UI_HTML` এক্সপোর্ট হয়, দুই অ্যাডাপ্টারই একই কপি রেফার করে (রিলিজ প্যাকেজিংয়ে নিরাপদ)

### বাস্তবায়িত (M6)

- **পাসওয়ার্ড অথেনটিকেশন (M6a)**: `AuthConfig { enable, password, secret_key, expire }` চালু করলে ক্লায়েন্ট `GET /apidoc/auth?password=<md5(পাসওয়ার্ড)>&appKey=<key>` দিয়ে token নেয়; ডেটা রাউট `/apidoc/api.json`, `/apidoc/export`, `/apidoc/mock`-এ `?token=xxx` লাগে, token না থাকলে/মেয়াদ শেষ হলে/ভুল হলে 401 রিটার্ন হয় এবং ডকুমেন্ট UI পাসওয়ার্ড মাস্ক দেখায়; token authcode এনক্রিপশন দিয়ে ইস্যু হয় (Discuz authcode লাইন বাই লাইন পোর্ট: RC4 ভ্যারিয়েন্ট + md5 চেকসাম + প্যাডিংবিহীন base64), পেলোড `{key: md5(md5(আসল পাসওয়ার্ড)), expire: now+expire}`, MAC তুলনা ধ্রুবক সময়ে
- **অথেনটিকেশন নিরাপত্তা লালরেখা**: `password` / `secret_key` কখনো সিরিয়ালাইজ হয় না — api.json আউটপুট অথেনটিকেশন বন্ধ অবস্থার সাথে বাইট-মিল; অথেনটিকেশন বন্ধ থাকলে `/apidoc/auth` 404 রিটার্ন করে এবং ডেটা রাউট সরাসরি পাস করে; কোনো অ্যাপ কনফিগে আলাদা পাসওয়ার্ড থাকলে অ্যাপ পাসওয়ার্ড গ্লোবালের চেয়ে প্রাধান্য পায়; `secret_key` ডিফল্ট `"apidoc#hgcode"` (চালু থাকা অবস্থায় কনফিগ না থাকলে একবার stderr সতর্কতা), `expire` ডিফল্ট ৮৬৪০০ সেকেন্ড
- **মাল্টি-অ্যাপ মাল্টি-ভার্সন (M6b)**: `ApidocConfig.apps: Vec<AppConfig>` (`key` / `title` / `items` রিকার্সিভ সাব-ভার্সন / `password`) অ্যাপ ট্রি কনফিগার করে, `#[apidoc::app("key")]` ইন্টারফেসগুলো নির্দিষ্ট অ্যাপ key-তে ঝুলিয়ে দেয়, key ছাড়া ইন্টারফেস ডিফল্ট অ্যাপে পড়ে; api.json আউটপুটে নতুন `doc.apps` ট্রি, UI-র উপরে অ্যাপ/ভার্সন সিলেক্টর, token প্রতি appKey আলাদা করে localStorage-এ (ভিন্ন অ্যাপের আলাদা পাসওয়ার্ড থাকতে পারে)

### পরিকল্পনাধীন (v2)

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
├── VERSION                    # প্রজেক্ট ভার্সন (v1.1.0, ফ্রেমওয়ার্ক ভার্সন 0.1.0 থেকে আলাদা)
├── crates/
│   ├── apidoc/                # রানটাইম কোর (ফ্রেমওয়ার্ক-নিরপেক্ষ)
│   │   ├── src/lib.rs         # ডেটা মডেল + DocRegistry অ্যাগ্রিগেশন + api.json + UI_HTML
│   │   ├── src/auth.rs        # M6a পাসওয়ার্ড অথেনটিকেশন (authcode token ইস্যু/যাচাই + রাউট গার্ড)
│   │   ├── src/export/        # M5 এক্সপোর্ট: markdown / typescript / swagger
│   │   ├── src/ui.html        # শেয়ারড ডকুমেন্ট UI (কোর ক্রেট এক্সপোর্ট, দুই অ্যাডাপ্টার রেফার করে)
│   │   ├── tests/             # ইন্টিগ্রেশন টেস্ট (ম্যাক্রো এক্সপানশন/অ্যাগ্রিগেশন/সিরিয়ালাইজেশন/ক্রস-crate)
│   │   └── examples/demo.rs   # উদাহরণ: অ্যানোটেশন + api.json আউটপুট
│   ├── apidoc-macros/         # proc-macro: ২০টি অ্যাট্রিবিউট ম্যাক্রো
│   │   └── src/lib.rs         # ম্যাক্রো সংজ্ঞা + প্যারামিটার পার্সিং + কম্পাইল-টাইম ভ্যালিডেশন
│   ├── apidoc-mock/           # Mock ইঞ্জিন (fake রুল দিয়ে mock ডেটা জেনারেশন)
│   ├── apidoc-test-fixtures/  # ক্রস-crate রেজিস্ট্রেশন টেস্ট ফিক্সচার
│   ├── apidoc-axum/           # axum অ্যাডাপ্টার (ডকুমেন্ট রাউট + cors_layer + mock + export)
│   └── apidoc-actix/          # actix-web অ্যাডাপ্টার (axum-এর সাথে 1:1 ফাংশনালিটি)
├── .github/
│   └── workflows/release.yml  # রিলিজ ওয়ার্কফ্লো (VERSION পড়ে, ইনক্রিমেন্টাল tag+release তৈরি)
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

> অ্যাডাপ্টার ওয়েব ফ্রেমওয়ার্ক অনুযায়ী একটি বেছে নিন: axum-এ `apidoc-axum`, actix-web-এ `apidoc-actix` (দুটোর ফাংশনালিটি 1:1)। `apidoc-mock` (Mock ইঞ্জিন) ফ্রেমওয়ার্কের অভ্যন্তরীণ ডিপেন্ডেন্সি, অ্যাডাপ্টারের মাধ্যমে স্বয়ংক্রিয়ভাবে যুক্ত হয়, সাধারণত ভোক্তাকে সরাসরি ব্যবহার করতে হয় না।

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
    let doc = DocRegistry::collect_doc(ApidocConfig {
        title: "我的 API".to_string(),
        description: None,
        auth: None,    // M6a পাসওয়ার্ড অথেনটিকেশন, দেখুন «৮. পাসওয়ার্ড অথেনটিকেশন»
        apps: vec![],  // M6b মাল্টি-অ্যাপ মাল্টি-ভার্সন, দেখুন «৯. মাল্টি-অ্যাপ মাল্টি-ভার্সন»
    });
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

### ৫. অনলাইন ডিবাগিং ও Mock (M4)

ডকুমেন্টেশন পেজ খুলুন ← ইন্টারফেস নির্বাচন করুন ← ডান পাশের «অনলাইন ডিবাগিং» প্যানেল mock রুল অনুযায়ী প্যারামিটার প্রি-ফিল করে ← Base URL টার্গেট সার্ভিসের ঠিকানায় দিন (ডিফল্ট `location.origin`, ক্রস-অরিজিনে সরাসরি সংযোগ) ← সেন্ড চাপলে আসল রেসপন্স পাবেন (স্ট্যাটাস কোড / সময় / pretty JSON)। ডিবাগ প্যানেল ডিফল্টে `not_debug` এন্ডপয়েন্ট লুকিয়ে রাখে, «not_debug ইন্টারফেস দেখান» টিক দিলে দেখায়।

**CORS প্রয়োজনীয়তা**: অনলাইন ডিবাগিং ব্রাউজার থেকে সরাসরি টার্গেট ইন্টারফেসে সংযোগ করে, তাই টার্গেট সার্ভিসকে অ্যাডাপ্টারের দেওয়া `cors_layer` মাউন্ট করতে হবে যাতে ক্রস-অরিজিন রিকোয়েস্ট অনুমোদিত হয়; CORS ব্যর্থ হলে প্যানেল হলুদ সতর্কতা দেখায়।

Mock রুল সিনট্যাক্স (তিনটি প্রায়োরিটি):

```rust
#[apidoc::param(name = "email", ty = "string", desc = "邮箱", mock = "fake:email")]  // fake 规则生成
#[apidoc::param(name = "status", ty = "string", desc = "状态", mock = "1")]          // 非空 mock 原样直出
#[apidoc::param(name = "name", ty = "string", desc = "用户名")]                       // 无 mock：按 ty 自动生成
#[apidoc::returned(
    name = "data",
    ty = "object",
    children = [
        { name = "id", ty = "int", required },       // 无 mock → "1"
        { name = "email", ty = "string", mock = "fake:email" },  // children 递归嵌套
    ]
)]
fn create_user() -> String {
    unimplemented!()
}
```

বিল্ট-ইন ১৫টি fake রুল: `name` / `company` / `email` / `phone` / `url` / `ip` / `city` / `country` / `text` / `number` / `int` / `float` / `bool` / `uuid` / `date`; অজানা নাম ডিফল্ট ভ্যালুতে ফলব্যাক। mock ছাড়া অটো-জেনারেশন রুল: int←`"1"`, float←`"0.5"`, bool←`"true"`, object←`"{}"`, string←`"string"`; array ফিক্সড ২টি আইটেম।

### ৬. অনলাইন এক্সপোর্ট (M5)

অ্যাডাপ্টারে তিন ফরম্যাটের এক্সপোর্ট ইন্টারফেস বিল্ট-ইন, মাউন্ট করলেই ব্যবহার করা যায় (অজানা `format`-এ 400):

```bash
GET /apidoc/export?format=md        # গ্রুপড ডিরেক্টরি + প্যারামিটার টেবিল + রেসপন্স ব্লক (text/markdown)
GET /apidoc/export?format=ts        # group নেমস্পেস অনুযায়ী {Name}Params / {Name}Result টাইপ (application/typescript)
GET /apidoc/export?format=swagger   # OpenAPI 3.0.0 বর্ণনা ফাইল (application/json)
```

- **markdown**: প্রজেক্ট Wiki / রিলিজ নোটে পেস্ট করার উপযুক্ত, গ্রুপ অনুযায়ী ডিরেক্টরি আউটপুট, প্রতিটি ইন্টারফেসে প্যারামিটার টেবিল ও রেসপন্স ব্লক;
- **typescript**: ফ্রন্টএন্ড সরাসরি টাইপ ডেফিনিশন হিসেবে পেস্ট করতে পারে; গ্রুপবিহীন ইন্টারফেস `defaultGroup` নেমস্পেসে পড়ে (`default` TS রিজার্ভড শব্দ, আইডেন্টিফায়ার হিসেবে ব্যবহার করা যাবে না);
- **swagger**: `info.version` রুটের `VERSION` ফাইলের কনটেন্ট থেকে নেওয়া (বর্তমানে 1.1.0), সরাসরি Swagger UI বা কোড জেনারেটরে ইমপোর্ট করা যায়।

### ৭. actix-web অ্যাডাপ্টার

ওয়েব ফ্রেমওয়ার্ক হিসেবে actix-web ব্যবহার করলে `apidoc-actix` যুক্ত করুন (axum অ্যাডাপ্টারের সাথে 1:1 ফাংশনালিটি):

```toml
[dependencies]
apidoc-actix = "0.1"     # অথবা path = "crates/apidoc-actix"
```

```rust
use actix_web::{App, HttpServer};
use apidoc_actix::{apidoc_routes, cors_layer, ApidocConfig, CorsConfig};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(apidoc_routes(ApidocConfig {
                title: "我的 API".to_string(),
                description: None,
                auth: None,    // M6a পাসওয়ার্ড অথেনটিকেশন, দেখুন «৮. পাসওয়ার্ড অথেনটিকেশন»
                apps: vec![],  // M6b মাল্টি-অ্যাপ মাল্টি-ভার্সন, দেখুন «৯. মাল্টি-অ্যাপ মাল্টি-ভার্সন»
            }))
            .wrap(cors_layer(CorsConfig::default()))   // M4 在线调试跨域放行
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
```

মাউন্ট করার পর `/apidoc` (ডকুমেন্ট UI), `/apidoc/api.json` (ডেটা), `/apidoc/mock` (Mock), `/apidoc/export` (এক্সপোর্ট) অ্যাক্সেস করা যায়। CORS খালি কনফিগ লিটারাল `*` পারমিশন দেয় (ক্রেডেনশিয়াল ছাড়া), `allow_origins` হোয়াইটলিস্ট কনফিগ করলে রিফ্লেক্টেড Origin-কে এক্সাক্ট ম্যাচ করে, দুই মোডেই ক্রেডেনশিয়াল খোলা হয় না।

### ৮. পাসওয়ার্ড অথেনটিকেশন (M6a)

`auth` চালু করলে ডকুমেন্টেশনে পাসওয়ার্ড ছাড়া প্রবেশ করা যায় না (আপস্ট্রিম apidoc-php-এর Auth.php-এর সাথে সামঞ্জস্যপূর্ণ, token হলো Discuz authcode এনক্রিপশনের লাইন বাই লাইন পোর্ট):

```rust
use apidoc::auth::AuthConfig;

let doc = DocRegistry::collect_doc(ApidocConfig {
    title: "我的 API".to_string(),
    description: None,
    auth: Some(AuthConfig {
        enable: true,
        password: "your-password".to_string(),
        secret_key: "your-secret-key".to_string(), // ডিফল্ট "apidoc#hgcode" (চালু থাকা অবস্থায় কনফিগ না থাকলে একবার stderr সতর্কতা)
        expire: 86400,                             // সেকেন্ড; ডিফল্ট 86400
    }),
    apps: vec![],
});
```

**প্রক্রিয়া**:

১. ক্লায়েন্ট `GET /apidoc/auth?password=<md5(পাসওয়ার্ড)>&appKey=<key>` কল করে token নেয় (সফল হলে `{"token":"..."}` রিটার্ন, পাসওয়ার্ড ভুল হলে 401); অথেনটিকেশন বন্ধ থাকলে এই রাউট 404 রিটার্ন করে এবং ডেটা রাউট সরাসরি পাস করে
২. ডেটা রাউট `GET /apidoc/api.json`, `/apidoc/export`, `/apidoc/mock`-এ `?token=xxx` লাগে (নির্দিষ্ট অ্যাপ নির্বাচন করলে সাথে `&appKey=`); token না থাকলে/মেয়াদ শেষ হলে/ভুল হলে 401 রিটার্ন হয়, ডকুমেন্ট UI স্বয়ংক্রিয়ভাবে পাসওয়ার্ড মাস্ক খোলে, এবং পাসওয়ার্ড দেওয়ার পর ফ্রন্টএন্ড লোকালি md5 হ্যাশ করে token নেয়
৩. token পেলোড `{key: md5(md5(আসল পাসওয়ার্ড)), expire: now+expire}`, `secret_key` দিয়ে authcode-তে এনক্রিপ্ট (RC4 ভ্যারিয়েন্ট + md5 চেকসাম + প্যাডিংবিহীন base64, টাইমিং সাইড-চ্যানেল রোধে ধ্রুবক সময়ে MAC তুলনা)
৪. `password` / `secret_key` কখনো সিরিয়ালাইজ হয় না — api.json আউটপুট অথেনটিকেশন বন্ধ অবস্থার সাথে বাইট-মিল; কোনো অ্যাপ কনফিগে নিজস্ব `password` থাকলে অ্যাপ পাসওয়ার্ড গ্লোবালের চেয়ে প্রাধান্য পায়

### ৯. মাল্টি-অ্যাপ মাল্টি-ভার্সন (M6b)

একটি প্রজেক্টকে একাধিক অ্যাপ/ভার্সনে ভাগ করা যায়, প্রতিটির নিজস্ব ডিসপ্লে ও অ্যাক্সেস নিয়ন্ত্রণ:

```rust
#[apidoc::title("获取用户信息")]
#[apidoc::app("demo")]   // key="demo" অ্যাপে ঝুলানো হয়; app অ্যানোটেশন ছাড়া ইন্টারফেস ডিফল্ট অ্যাপে পড়ে
fn get_user_info() -> String {
    unimplemented!()
}
```

```rust
let doc = DocRegistry::collect_doc(ApidocConfig {
    title: "我的 API".to_string(),
    description: None,
    auth: None,
    apps: vec![
        AppConfig {
            key: "demo".to_string(),
            title: "演示应用".to_string(),
            items: vec![AppConfig {
                key: "v1".to_string(),
                title: "v1".to_string(),
                items: vec![],
                password: None,
            }],
            password: None, // অ্যাপের স্বাধীন অ্যাক্সেস পাসওয়ার্ড, গ্লোবালের চেয়ে প্রাধান্য পায়, কখনো সিরিয়ালাইজ হয় না
        },
    ],
});
```

- `AppConfig { key, title, items, password }`: `key` হলো সেই ইউনিক আইডেন্টিফায়ার যাকে `#[apidoc::app("key")]` অ্যানোটেশন রেফার করে, `items` রিকার্সিভভাবে সাব-ভার্সন/সাব-অ্যাপ নেস্ট করে, `password` হলো অ্যাপের স্বাধীন অ্যাক্সেস পাসওয়ার্ড (স্বাধীন পাসওয়ার্ড থাকলে শুধু অ্যাপ token যাচাই হয়)
- api.json আউটপুটে নতুন `doc.apps` ট্রি (key / title / items / endpoints); UI-র উপরে অ্যাপ/ভার্সন সিলেক্টর — স্যুইচ করলে সেই নোডের ইন্টারফেস রেন্ডার হয় ও ডেটা নতুন করে আনা হয়, token প্রতি appKey আলাদা করে localStorage-এ থাকে
- `app` অ্যানোটেশন `apps`-এ কনফিগ না করা key রেফার করলে stderr সতর্কতা দিয়ে ইন্টারফেসটি ডিফল্ট অ্যাপে পড়ে; `app` অ্যানোটেশন ছাড়া বা `apps` কনফিগ না থাকলে আউটপুট M5-এর সাথে বাইট-মিল

## উন্নয়ন পরিকল্পনা

| পর্যায় | বিষয়বস্তু | অবস্থা |
|------|------|------|
| M1 | workspace কঙ্কাল + ডেটা মডেল + ম্যাক্রো MVP + linkme রেজিস্ট্রেশন | ✅ সম্পন্ন |
| M2 | axum অ্যাডাপ্টার + এমবেডেড ডকুমেন্ট UI + গ্রুপড ডিরেক্টরি | ✅ সম্পন্ন |
| M3 | অ্যানোটেশন সম্পূর্ণকরণ (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | ✅ সম্পন্ন |
| M4 | অনলাইন ডিবাগিং + Mock ইঞ্জিন | ✅ সম্পন্ন |
| M5 | markdown / typescript / swagger.json (OpenAPI3) এক্সপোর্ট | ✅ সম্পন্ন |
| —  | actix-web অ্যাডাপ্টার (axum-এর সাথে 1:1 ফাংশনালিটি) | ✅ সম্পন্ন |
| M6a | পাসওয়ার্ড অথেনটিকেশন (authcode token + পাসওয়ার্ড মাস্ক, অ্যাপ পাসওয়ার্ডের প্রাধান্য) | ✅ সম্পন্ন |
| M6b | মাল্টি-অ্যাপ মাল্টি-ভার্সন (apps কনফিগ ট্রি + app অ্যানোটেশন + UI সিলেক্টর) | ✅ সম্পন্ন |

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
