<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 مكتبة إضافات عامة لتوليد وثائق واجهات برمجة التطبيقات (API) مبنية على ماكروات Rust الإجرائية (proc-macro)
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
<a href="README-ar.md"><strong>العربية</strong></a> ·
<a href="README-bn.md">বাংলা</a> ·
<a href="README-id.md">Bahasa Indonesia</a> ·
<a href="README-ja.md">日本語</a>
</div>

## مقدمة المشروع

apidoc-rust هو **مولّد وثائق API عام وقابل للتوسع عبر الإضافات** مكتوب بلغة Rust، مستوحى من [apidoc-php](https://github.com/erikwang2013/apidoc-php) (امتداد Composer يولّد وثائق API اعتمادًا على سمات PHP 8 attributes)، وينقل فكرة «التعليقات التوضيحية هي الوثائق» إلى عالم Rust بطريقة أصلية:

- **التوليد في زمن الترجمة**: تُولَّد الوثائق بواسطة الماكرو الإجرائي في زمن الترجمة، فلا تنفصل الوثائق عن الكود أبدًا؛
- **جمع بلا تكلفة**: تسجيل ثابت عبر linkme، وتجميع واحد في زمن التشغيل يكفي للحصول على جميع وثائق الواجهات؛
- **إضافات عامة**: النواة مستقلة عن إطار عمل HTTP، وتتصل بأي إطار عبر محولات رقيقة (axum / actix-web).

## المميزات

### تم تنفيذه (M1)

- **وثائق عبر التعليقات التوضيحية**: سبعة ماكروات سمات `title` / `desc` / `method` / `url` / `param` / `query` / `returned` تُعلَّق واجهةً تلو الأخرى (بما يطابق أسلوب سمات PHP attributes)، مع دعم التداخل في المعاملات: `required` / `default` / `desc` / `mock` / `children`
- **التحقق في زمن الترجمة**: يجب أن يبدأ url بـ `/`، وmethod ضمن قائمة بيضاء، وparam name إلزامي... أي تعليق توضيحي غير صالح يُبلَغ عنه في زمن الترجمة (مع span دقيق)
- **الجمع التلقائي**: تسجيل ثابت عبر `distributed_slice` من linkme دون الحاجة إلى قائمة واجهات يدوية؛ `DocRegistry::collect()` يدمج القطع حسب id ويستعيد ترتيب التعريف حسب seq، مع جمع تلقائي عبر crates متعددة
- **إخراج api.json**: تسلسل serde لنموذج بيانات موحد (config + endpoints)، بحقول مطابقة لدلالات PHP

### قيد التخطيط

- تصحيح أونلاين (اتصال المتصفح المباشر بالواجهة عبر CORS) وبيانات Mock (توليد وفق قواعد fake)
- تطبيقات متعددة / إصدارات متعددة / كلمة مرور للوصول
- تصدير Markdown / TypeScript / Swagger (OpenAPI3)
- دعم أطر عمل متعددة (apidoc-axum / apidoc-actix)
- v2: مولّد كود، مراجع لحقول جداول البيانات، روابط مشاركة، أحداث تصحيح

## البنية

<img src="images/ar-architecture.svg" alt="البنية العامة لـ apidoc-rust" width="100%">

## الوظائف

<img src="images/ar-features.svg" alt="وظائف مشروع apidoc-rust" width="100%">

## دورة الحياة

<img src="images/ar-lifecycle.svg" alt="دورة حياة وثائق apidoc-rust" width="100%">

## بنية المشروع

```
apidoc-rust/
├── Cargo.toml                 # إعداد workspace (resolver 2)
├── crates/
│   ├── apidoc/                # النواة في زمن التشغيل (مستقلة عن الإطار)
│   │   ├── src/lib.rs         # نموذج البيانات + تجميع DocRegistry + api.json
│   │   ├── tests/             # اختبارات التكامل (توسيع الماكرو / التجميع / التسلسل / عبر crates)
│   │   └── examples/demo.rs   # مثال: تعليقات توضيحية + إخراج api.json
│   ├── apidoc-macros/         # proc-macro: 7 ماكروات سمات
│   │   └── src/lib.rs         # تعريفات الماكرو + تحليل المعاملات + التحقق في زمن الترجمة
│   └── apidoc-test-fixtures/  # نماذج اختبار التسجيل عبر crates
└── docs/
    ├── images/                # مخططات البنية / الوظائف / دورة الحياة (SVG)
    └── i18n/                  # وثائق متعددة اللغات (12 لغة)
```

## دليل الاستخدام

### 1. إضافة التبعيات

```toml
[dependencies]
apidoc = "0.1"        # أو path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # توسيع الماكرو يشير مباشرة إلى مسار linkme، لذا يجب أن تعتمد عليه جهة الاستهلاك مباشرة
serde_json = "1"      # لاستخدام إخراج api.json
```

### 2. كتابة التعليقات التوضيحية

علّق على دوال handler واحدًا تلو الآخر، وتُولَّد الوثائق في زمن الترجمة:

```rust
use apidoc::*;

#[apidoc::title("جلب معلومات المستخدم")]
#[apidoc::desc("الاستعلام عن تفاصيل المستخدم حسب معرّف المستخدم")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::param(name = "user_id", ty = "int", required, desc = "معرّف المستخدم", mock = "1")]
#[apidoc::query(name = "lang", ty = "string", desc = "اللغة", default = "zh-CN")]
#[apidoc::returned(
    name = "data",
    ty = "object",
    desc = "بيانات المستخدم",
    children = [
        { name = "id", ty = "int", required, desc = "معرّف المستخدم" },
        { name = "name", ty = "string", required, desc = "اسم المستخدم", mock = "erik" },
    ]
)]
fn get_user_info() -> String {
    unimplemented!()
}
```

### 3. الجمع والإخراج

```rust
fn main() {
    let endpoints = DocRegistry::collect();
    let doc = ApiDoc {
        config: ApidocConfig {
            title: "API الخاص بي".to_string(),
            description: None,
        },
        endpoints,
    };
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}
```

### 4. تشغيل المثال

```bash
cargo run --example demo -p apidoc
```

الإخراج (مقتطف):

```json
{
  "config": { "title": "demo api" },
  "endpoints": [
    {
      "title": "جلب معلومات المستخدم",
      "desc": "الاستعلام عن تفاصيل المستخدم حسب معرّف المستخدم",
      "url": "/api/user/info",
      "method": "GET",
      "params": [
        { "name": "user_id", "type": "int", "required": true, "desc": "معرّف المستخدم", "mock": "1" }
      ],
      "querys": [
        { "name": "lang", "type": "string", "required": false, "default": "zh-CN", "desc": "اللغة" }
      ],
      "returned": [
        {
          "name": "data",
          "type": "object",
          "required": false,
          "desc": "بيانات المستخدم",
          "children": [
            { "name": "id", "type": "int", "required": true, "desc": "معرّف المستخدم" },
            { "name": "name", "type": "string", "required": true, "desc": "اسم المستخدم", "mock": "erik" }
          ]
        }
      ]
    }
  ]
}
```

## خطة التطوير

| المرحلة | المحتوى | الحالة |
|------|------|------|
| M1 | هيكل workspace + نموذج البيانات + ماكرو MVP + تسجيل linkme | ✅ مكتمل |
| M2 | محول axum + واجهة وثائق مدمجة + فهرس مجمّع | ⏳ قيد التخطيط |
| M3 | استكمال التعليقات التوضيحية (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | قيد التخطيط |
| M4 | تصحيح أونلاين + محرك Mock | قيد التخطيط |
| M5 | تصدير markdown / typescript / swagger.json | قيد التخطيط |
| M6 | مصادقة بكلمة مرور، تطبيقات وإصدارات متعددة، إصدار عام | قيد التخطيط |

## وثائق متعددة اللغات

- [中文](../../README.md)
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

## الدعم والتبرعات

إذا كان هذا المشروع مفيدًا لك، فمرحبًا بك لدعمنا بنجمة ⭐، ونسعد أيضًا بأي تبرع لدعم البرمجيات مفتوحة المصدر!

### 微信支付 / 支付宝 (WeChat Pay / Alipay)

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

### التبرعات عبر التحويلات المصرفية العالمية

**【معلومات المستفيد】**

- اسم المستفيد: WANG KEXUN
- رقم حساب المستفيد: 881015918251

**【البنك المستفيد】**

- رمز SWIFT لبنك ZA Bank: AABLHKHHXXX
- اسم البنك: ZA Bank Limited
- رقم البنك: 387
- عنوان البنك: Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【البنك الوكيل للتحويلات العابرة للحدود (عند الحاجة)】**

> يرجى الانتباه: هذه معلومات البنك الوكيل للتحويلات العابرة للحدود (البنك الوسيط)، وليست معلومات البنك المستفيد. استفسر من البنك المُحوِّل عما إذا كانت هناك حاجة لتقديم معلومات البنك الوكيل.

- **البنك الوكيل للتحويلات بالدولار الهونغ كونغي واليوان الصيني والدولار الأمريكي هو Citibank:**
  - اسم البنك: Citibank N.A. Hong Kong
  - رمز SWIFT: CITIHKHXXXX
  - رقم البنك: 006
  - اسم الفرع: Hong Kong Branch
  - رقم الفرع: 391
  - عنوان البنك: Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **أما البنك الوكيل للتحويلات بالعملات الأخرى فهو BNY Mellon:**
  - اسم البنك: THE BANK OF NEW YORK MELLON
  - رمز SWIFT: IRVTUS3NXXX
  - عنوان البنك: THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## الترخيص

[MIT](LICENSE)
