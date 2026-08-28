<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 Rust प्रोसेस मैक्रो (proc-macro) के आधार पर API इंटरफ़ेस दस्तावेज़ उत्पन्न करने वाली सामान्य प्लगइन लाइब्रेरी
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
<a href="README-hi.md"><strong>हिन्दी</strong></a> ·
<a href="README-ar.md">العربية</a> ·
<a href="README-bn.md">বাংলা</a> ·
<a href="README-id.md">Bahasa Indonesia</a> ·
<a href="README-ja.md">日本語</a>
</div>

## परियोजना परिचय

apidoc-rust एक Rust में कार्यान्वित **सामान्य प्लगइन-आधारित API इंटरफ़ेस दस्तावेज़ जनरेटर** है, जो [apidoc-php](https://github.com/erikwang2013/apidoc-php) (PHP 8 attributes के आधार पर API दस्तावेज़ उत्पन्न करने वाला composer एक्सटेंशन) का संदर्भ लेता है और "एनोटेशन ही दस्तावेज़ है" की क्षमता को Rust के मूल तरीके से लागू करता है:

- **संकलन-समय उत्पादन**: दस्तावेज़ संकलन के समय प्रोसेस मैक्रो द्वारा उत्पन्न होते हैं, दस्तावेज़ और कोड कभी भी अतुल्यकालिक नहीं होते;
- **शून्य-लागत संग्रह**: linkme स्थैतिक पंजीकरण, रनटाइम पर एक बार एकत्रित करने पर सभी इंटरफ़ेस दस्तावेज़ प्राप्त हो जाते हैं;
- **सामान्य प्लगइन**: कोर HTTP फ्रेमवर्क से स्वतंत्र है, पतले अडैप्टर (axum / actix-web) के माध्यम से किसी भी फ्रेमवर्क से जोड़ा जा सकता है।

## विशेषताएँ

### कार्यान्वित (M1-M3)

- **एनोटेशन-आधारित दस्तावेज़**: `title` / `desc` / `method` / `url` / `param` / `query` / `returned` सात एट्रिब्यूट मैक्रो, एक-एक करके एनोटेट करें (PHP attributes लेखन शैली के अनुरूप), पैरामीटर `required` / `default` / `desc` / `mock` / `children` नेस्टिंग का समर्थन करते हैं
- **संकलन-समय सत्यापन**: url को `/` से शुरू होना चाहिए, method श्वेतसूची, param name अनिवार्य आदि; अवैध एनोटेशन संकलन के समय त्रुटि देते हैं (सटीक span के साथ)
- **स्वचालित संग्रह**: linkme `distributed_slice` स्थैतिक पंजीकरण, मैन्युअल इंटरफ़ेस सूची की आवश्यकता नहीं; `DocRegistry::collect()` id के आधार पर मर्ज करता है, seq के आधार पर घोषणा क्रम पुनर्स्थापित करता है, क्रॉस-crate स्वचालित संग्रह
- **api.json आउटपुट**: serde एकीकृत दस्तावेज़ डेटा मॉडल (config + endpoints) का क्रमांकन करता है, फ़ील्ड PHP शब्दार्थ के अनुरूप हैं
- **axum अडैप्टर + एम्बेडेड दस्तावेज़ UI**: रूट माउंट करते ही दस्तावेज़ पेज मिलता है, समूहित कैटलॉग ब्राउज़िंग (M2)
- **एनोटेशन पूरा करना**: 12 नए एनोटेशन — `tag` / `group` / `author` / `header` / `route_param` / `response_status` / `success` / `error` / `not_debug` / `md` / `sort` / `ref` (M3)

### कार्यान्वित (M4)

- **ऑनलाइन डिबगिंग**: दस्तावेज़ पेज में «ऑनलाइन डिबगिंग» पैनल बिल्ट-इन है — Base URL `location.origin` से प्री-फिल होकर लक्ष्य सेवा से सीधे क्रॉस-डोमेन जुड़ता है, पैरामीटर फ़ॉर्म mock से प्री-फिल, `{name}` / `:name` रूट प्लेसहोल्डर प्रतिस्थापन, GET/HEAD पैरामीटर query में जोड़े जाते हैं, बाकी method का JSON body बनता है, रिक्वेस्ट हेडर संपादन + कस्टम header, रिस्पॉन्स प्रदर्शन (स्थिति / समय / pretty JSON), CORS विफलता पर पीली चेतावनी
- **मॉक इंजन** (`crates/apidoc-mock`, fake crate पर निर्भर, 15 नियम: name / company / email / phone / url / ip / city / country / text / number / int / float / bool / uuid / date)। नियम प्राथमिकता: `mock="fake:xxx"` fake नियम तालिका से (अज्ञात नाम → डिफ़ॉल्ट मान) → बाकी गैर-खाली mock ज्यों-का-त्यों आउटपुट (जैसे `mock="1"`, `mock="erik"`) → बिना mock के `ty` के अनुसार स्वतः जनरेट (int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`); children रिकर्सिव रूप से नेस्टेड, array में निश्चित 2 आइटम
- **mock इंटरफ़ेस**: axum अडैप्टर में नया `GET /apidoc/mock?url=&method=`, url + method का सटीक मिलान, न मिलने पर 404; डिबग पैनल डिफ़ॉल्ट रूप से `not_debug` एंडपॉइंट छिपाता है, «not_debug इंटरफ़ेस दिखाएँ» चेक करने पर ही दिखते हैं
- **CORS सीधा कनेक्शन**: ऑनलाइन डिबगिंग में ब्राउज़र सीधे लक्ष्य इंटरफ़ेस से जुड़ता है, अडैप्टर का `cors_layer` अनुमति देता है (सर्वर-साइड रिवर्स प्रॉक्सी v2 के लिए)

### नियोजित

- एकाधिक ऐप / एकाधिक संस्करण / एक्सेस पासवर्ड
- निर्यात Markdown / TypeScript / Swagger (OpenAPI3) (M5)
- बहु-फ्रेमवर्क अनुकूलन (apidoc-axum पूर्ण, apidoc-actix शेष)
- v2: कोड जनरेटर, डेटा तालिका फ़ील्ड संदर्भ, साझा लिंक, डिबग इवेंट

## वास्तुकला

<img src="images/hi-architecture.svg" alt="apidoc-rust समग्र वास्तुकला" width="100%">

## कार्यक्षमता

<img src="images/hi-features.svg" alt="apidoc-rust परियोजना कार्यक्षमता" width="100%">

## जीवनचक्र

<img src="images/hi-lifecycle.svg" alt="apidoc-rust दस्तावेज़ जीवनचक्र" width="100%">

## परियोजना संरचना

```
apidoc-rust/
├── Cargo.toml                 # workspace कॉन्फ़िगरेशन (resolver 2)
├── VERSION                    # परियोजना संस्करण (v1.0.0, फ्रेमवर्क संस्करण 0.1.0 से अलग)
├── crates/
│   ├── apidoc/                # रनटाइम कोर (फ्रेमवर्क-स्वतंत्र)
│   │   ├── src/lib.rs         # डेटा मॉडल + DocRegistry एकत्रीकरण + api.json
│   │   ├── tests/             # इंटीग्रेशन टेस्ट (मैक्रो विस्तार/एकत्रीकरण/क्रमांकन/क्रॉस-crate)
│   │   └── examples/demo.rs   # उदाहरण: एनोटेशन + api.json आउटपुट
│   ├── apidoc-macros/         # proc-macro: 19 एट्रिब्यूट मैक्रो
│   │   └── src/lib.rs         # मैक्रो परिभाषाएँ + पैरामीटर पार्सिंग + संकलन-समय सत्यापन
│   ├── apidoc-mock/           # मॉक इंजन (fake नियमों से mock डेटा जनरेट)
│   ├── apidoc-test-fixtures/  # क्रॉस-crate पंजीकरण टेस्ट फिक्स्चर
│   └── apidoc-axum/           # axum अडैप्टर (दस्तावेज़ रूट + cors_layer + /apidoc/mock)
├── .github/
│   └── workflows/release.yml  # रिलीज़ वर्कफ़्लो (VERSION पढ़कर, incremental tag+release बनाता है)
└── docs/
    ├── images/                # वास्तुकला/कार्यक्षमता/जीवनचक्र आरेख (SVG)
    └── i18n/                  # बहुभाषी दस्तावेज़ (12 भाषाएँ)
```

## उपयोग निर्देश

### 1. निर्भरताएँ जोड़ें

```toml
[dependencies]
apidoc = "0.1"        # या path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # मैक्रो विस्तार सीधे linkme पथ का संदर्भ देता है, उपभोक्ता को सीधे निर्भरता चाहिए
serde_json = "1"      # api.json आउटपुट के लिए
```

### 2. एनोटेशन लिखें

handler फ़ंक्शन पर एक-एक करके एनोटेशन लगाएँ, दस्तावेज़ संकलन के समय उत्पन्न हो जाता है:

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

### 3. संग्रह और आउटपुट

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

### 4. उदाहरण चलाएँ

```bash
cargo run --example demo -p apidoc
```

आउटपुट (अंश):

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

### 5. ऑनलाइन डिबगिंग और मॉक (M4)

दस्तावेज़ पेज खोलें → इंटरफ़ेस चुनें → दाईं ओर «ऑनलाइन डिबगिंग» पैनल mock नियमों के अनुसार पैरामीटर प्री-फिल करता है → Base URL को लक्ष्य सेवा के पते पर ले जाएँ (डिफ़ॉल्ट `location.origin`, सीधा क्रॉस-डोमेन कनेक्शन) → भेजें पर क्लिक करें, वास्तविक रिस्पॉन्स मिलता है (स्टेटस कोड / समय / pretty JSON)। डिबग पैनल डिफ़ॉल्ट रूप से `not_debug` एंडपॉइंट छिपाता है, «not_debug इंटरफ़ेस दिखाएँ» चेक करने के बाद ही दिखते हैं।

**CORS आवश्यकता**: ऑनलाइन डिबगिंग में ब्राउज़र सीधे लक्ष्य इंटरफ़ेस से जुड़ता है; लक्ष्य सेवा को अडैप्टर द्वारा प्रदान किया गया `cors_layer` माउंट करना होगा ताकि क्रॉस-डोमेन अनुरोध अनुमत हों; CORS विफल होने पर पैनल पीली चेतावनी दिखाता है।

मॉक नियम सिंटैक्स (तीन प्राथमिकताएँ):

```rust
#[apidoc::param(name = "email", ty = "string", desc = "邮箱", mock = "fake:email")]  // fake नियम से जनरेट
#[apidoc::param(name = "status", ty = "string", desc = "状态", mock = "1")]          // गैर-खाली mock ज्यों-का-त्यों
#[apidoc::param(name = "name", ty = "string", desc = "用户名")]                       // बिना mock: ty के अनुसार स्वतः जनरेट
#[apidoc::returned(
    name = "data",
    ty = "object",
    children = [
        { name = "id", ty = "int", required },       // बिना mock → "1"
        { name = "email", ty = "string", mock = "fake:email" },  // children रिकर्सिव नेस्टिंग
    ]
)]
fn create_user() -> String {
    unimplemented!()
}
```

15 अंतर्निर्मित fake नियम: `name` / `company` / `email` / `phone` / `url` / `ip` / `city` / `country` / `text` / `number` / `int` / `float` / `bool` / `uuid` / `date`; अज्ञात नियम नाम डिफ़ॉल्ट मान पर वापस चला जाता है। बिना mock के स्वतः जनरेशन: int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`; array में निश्चित 2 आइटम।

## विकास योजना

| चरण | विवरण | स्थिति |
|------|------|------|
| M1 | workspace ढाँचा + डेटा मॉडल + मैक्रो MVP + linkme पंजीकरण | ✅ पूर्ण |
| M2 | axum अडैप्टर + एम्बेडेड दस्तावेज़ UI + समूहित कैटलॉग | ✅ पूर्ण |
| M3 | एनोटेशन पूरा करना (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | ✅ पूर्ण |
| M4 | ऑनलाइन डिबगिंग + मॉक इंजन | ✅ पूर्ण |
| M5 | निर्यात markdown / typescript / swagger.json | नियोजित |
| M6 | पासवर्ड प्रमाणीकरण, एकाधिक ऐप और संस्करण, रिलीज़ | नियोजित |

## बहुभाषी दस्तावेज़

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

## समर्थन और दान

यदि यह परियोजना आपके लिए उपयोगी है, तो कृपया ⭐ Star देकर हमारा समर्थन करें, और ओपन-सोर्स के लिए दान का भी स्वागत है!

### 微信支付 (WeChat Pay) / 支付宝 (Alipay)

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

### वैश्विक बैंक हस्तांतरण दान

**【प्राप्तकर्ता जानकारी】**

- प्राप्तकर्ता का नाम: WANG KEXUN
- प्राप्तकर्ता खाता संख्या: 881015918251

**【प्राप्तकर्ता बैंक】**

- ZA Bank SWIFT कोड: AABLHKHHXXX
- बैंक का नाम: ZA Bank Limited
- बैंक कोड: 387
- बैंक का पता: Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【सीमा-पार रेमिटेंस एजेंट बैंक (यदि आवश्यक हो)】**

> कृपया ध्यान दें, यह सीमा-पार रेमिटेंस एजेंट बैंक (मध्यस्थ बैंक) की जानकारी है, प्राप्तकर्ता बैंक की नहीं। कृपया अपने रेमिटेंस बैंक से पूछें कि क्या सीमा-पार रेमिटेंस एजेंट बैंक की जानकारी प्रदान करना आवश्यक है।

- **हाँगकाँग डॉलर, चीनी युआन और अमेरिकी डॉलर के लिए एजेंट बैंक Citibank है:**
  - बैंक का नाम: Citibank N.A. Hong Kong
  - SWIFT कोड: CITIHKHXXXX
  - बैंक कोड: 006
  - शाखा का नाम: Hong Kong Branch
  - शाखा कोड: 391
  - बैंक का पता: Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **अन्य मुद्राओं के लिए एजेंट बैंक BNY Mellon है:**
  - बैंक का नाम: THE BANK OF NEW YORK MELLON
  - SWIFT कोड: IRVTUS3NXXX
  - बैंक का पता: THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## License

[MIT](../../LICENSE)
