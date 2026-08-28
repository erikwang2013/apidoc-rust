<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 Rust の手続きマクロ（proc-macro）で API インターフェースドキュメントを生成する汎用プラグインライブラリ
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
<a href="README-bn.md">বাংলা</a> ·
<a href="README-id.md">Bahasa Indonesia</a> ·
<a href="README-ja.md"><strong>日本語</strong></a>
</div>

## プロジェクト紹介

apidoc-rust は Rust で実装された**汎用プラグイン型 API インターフェースドキュメントジェネレータ**です。[apidoc-php](https://github.com/erikwang2013/apidoc-php)（PHP 8 の attributes で API ドキュメントを生成する composer 拡張）を参考に、「注釈＝ドキュメント」という能力を Rust ネイティブな形で実現します：

- **コンパイル期生成**：ドキュメントは手続きマクロによりコンパイル期に生成され、ドキュメントとコードの同期が常に保たれます；
- **ゼロコスト収集**：linkme による静的登録で、実行時に一度集約するだけで全インターフェースドキュメントを取得できます；
- **汎用プラグイン**：コアは HTTP フレームワークに依存せず、薄いアダプタ（axum / actix-web）経由で任意のフレームワークに接続できます。

## 特徴

### 実装済み（M1）

- **注釈式ドキュメント**：`title` / `desc` / `method` / `url` / `param` / `query` / `returned` の 7 つの属性マクロで項目ごとに注釈します（PHP attributes の書き方に対応）。パラメータは `required` / `default` / `desc` / `mock` / `children` のネストに対応
- **コンパイル期検証**：url は `/` で始まる必要があり、method はホワイトリスト、param の name は必須など、不正な注釈はコンパイル期にエラーになります（span 単位で正確）
- **自動収集**：linkme の `distributed_slice` による静的登録で、手動のインターフェース一覧は不要。`DocRegistry::collect()` が id でマージし、seq で宣言順を復元、crate をまたいで自動収集します
- **api.json 出力**：serde で統一ドキュメントデータモデル（config + endpoints）をシリアライズし、フィールドは PHP のセマンティクスに合わせます

### 計画中

- オンラインデバッグ（ブラウザから CORS で対象インターフェースに直接接続）、Mock データ（fake ルールによる生成）
- 複数アプリ / 複数バージョン / アクセスパスワード
- Markdown / TypeScript / Swagger（OpenAPI3）のエクスポート
- 複数フレームワーク対応（apidoc-axum / apidoc-actix）
- v2：コードジェネレータ、データテーブルフィールド参照、共有リンク、デバッグイベント

## アーキテクチャ

<img src="images/ja-architecture.svg" alt="apidoc-rust 全体アーキテクチャ" width="100%">

## 機能

<img src="images/ja-features.svg" alt="apidoc-rust プロジェクト機能" width="100%">

## ライフサイクル

<img src="images/ja-lifecycle.svg" alt="apidoc-rust ドキュメントライフサイクル" width="100%">

## プロジェクト構成

```
apidoc-rust/
├── Cargo.toml                 # workspace 設定（resolver 2）
├── crates/
│   ├── apidoc/                # ランタイムコア（フレームワーク非依存）
│   │   ├── src/lib.rs         # データモデル + DocRegistry 集約 + api.json
│   │   ├── tests/             # 統合テスト（マクロ展開/集約/シリアライズ/クロス crate）
│   │   └── examples/demo.rs   # サンプル：注釈 + api.json 出力
│   ├── apidoc-macros/         # proc-macro：7 つの属性マクロ
│   │   └── src/lib.rs         # マクロ定義 + パラメータ解析 + コンパイル期検証
│   └── apidoc-test-fixtures/  # クロス crate 登録テストフィクスチャ
└── docs/
    ├── images/                # アーキテクチャ/機能/ライフサイクル図（SVG）
    └── i18n/                  # 多言語ドキュメント（12 言語）
```

## 使い方

### 1. 依存関係の追加

```toml
[dependencies]
apidoc = "0.1"        # または path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # マクロ展開が linkme のパスを直接参照するため、利用側は直接依存が必要
serde_json = "1"      # api.json 出力用
```

### 2. 注釈の記述

handler 関数に注釈を 1 つずつ付けると、ドキュメントはコンパイル期に生成されます：

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

### 3. 収集と出力

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

### 4. サンプルの実行

```bash
cargo run --example demo -p apidoc
```

出力（抜粋）：

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

## 開発ロードマップ

| フェーズ | 内容 | ステータス |
|------|------|------|
| M1 | workspace の骨組み + データモデル + マクロ MVP + linkme 登録 | ✅ 完了 |
| M2 | axum アダプタ + 組み込みドキュメント UI + グループ別ディレクトリ | ⏳ 計画中 |
| M3 | 注釈の拡充（tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref） | 計画中 |
| M4 | オンラインデバッグ + Mock エンジン | 計画中 |
| M5 | markdown / typescript / swagger.json のエクスポート | 計画中 |
| M6 | パスワード認証、複数アプリ・複数バージョン、リリース | 計画中 |

## 多言語ドキュメント

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

## サポートと寄付

本プロジェクトがお役に立ちましたら、ぜひ ⭐ Star でのサポートをお願いします。オープンソースへの寄付も大歓迎です！

### 微信 / 支付宝

<table>
  <tr>
    <td align="center">
      <img src="../../docs/weixinpay.png" width="130" height="130" alt="微信支付（WeChat Pay）" /><br/>
      <strong>微信支付（WeChat Pay）</strong>
    </td>
    <td align="center">
      <img src="../../docs/alipay.png" width="130" height="130" alt="支付宝（Alipay）" /><br/>
      <strong>支付宝（Alipay）</strong>
    </td>
  </tr>
</table>

### 海外送金での寄付

**【受取人情報】**

- 受取人名：WANG KEXUN
- 受取口座番号：881015918251

**【受取銀行】**

- ZA Bank SWIFT Code：AABLHKHHXXX
- 銀行名：ZA Bank Limited
- 銀行番号：387
- 銀行住所：Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【クロスボーダー送金代理銀行（必要な場合）】**

> ご注意：これはクロスボーダー送金の代理銀行（中継銀行）情報であり、受取銀行の情報ではありません。送金銀行に代理銀行情報の提供が必要かどうかをご確認ください。

- **香港ドル・人民元・米ドルでの送金時の代理銀行は Citibank です：**
  - 銀行名：Citibank N.A. Hong Kong
  - SWIFT Code：CITIHKHXXXX
  - 銀行番号：006
  - 支店名：Hong Kong Branch
  - 支店番号：391
  - 銀行住所：Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **その他の通貨での送金時の代理銀行は BNY Mellon です：**
  - 銀行名：THE BANK OF NEW YORK MELLON
  - SWIFT Code：IRVTUS3NXXX
  - 銀行住所：THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## License

[MIT](../../LICENSE)
