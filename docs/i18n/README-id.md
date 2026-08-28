<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 Perpustakaan plugin umum untuk menghasilkan dokumentasi API berdasarkan makro prosedural (proc-macro) Rust
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
<a href="README-id.md"><strong>Bahasa Indonesia</strong></a> ·
<a href="README-ja.md">日本語</a>
</div>

## Pengenalan Proyek

apidoc-rust adalah **pembuat dokumentasi API plugin-umum** yang diimplementasikan dalam Rust, mengacu pada [apidoc-php](https://github.com/erikwang2013/apidoc-php) (ekstensi composer yang menghasilkan dokumentasi API berdasarkan PHP 8 attributes), mewujudkan kemampuan "anotasi sebagai dokumentasi" secara native di Rust:

- **Dihasilkan saat kompilasi**: dokumentasi dihasilkan oleh makro prosedural pada waktu kompilasi, dokumentasi tidak akan pernah kehilangan sinkronisasi dengan kode;
- **Pengumpulan biaya nol**: registrasi statis linkme, satu kali agregasi saat runtime langsung mendapatkan seluruh dokumentasi API;
- **Plugin umum**: inti tidak terikat kerangka kerja HTTP mana pun, terhubung ke kerangka kerja apa pun melalui adaptor tipis (axum / actix-web).

## Fitur

### Sudah Diimplementasikan (M1–M3)

- **Dokumentasi berbasis anotasi**: tujuh makro atribut `title` / `desc` / `method` / `url` / `param` / `query` / `returned`, anotasi satu per satu (setara dengan penulisan PHP attributes), parameter mendukung nesting `required` / `default` / `desc` / `mock` / `children`
- **Validasi waktu kompilasi**: url harus diawali `/`, whitelist method, param name wajib diisi, dll.; anotasi tidak valid menghasilkan error saat kompilasi (span presisi)
- **Pengumpulan otomatis**: registrasi statis linkme `distributed_slice`, tanpa daftar API manual; `DocRegistry::collect()` menggabungkan berdasarkan id, mengembalikan urutan deklarasi berdasarkan seq, pengumpulan otomatis antar-crate
- **Output api.json**: serde melakukan serialisasi model data dokumentasi terpadu (config + endpoints), field selaras dengan semantik PHP
- **Adaptor axum + UI dokumentasi tertanam**: pasang rute langsung dapat halaman dokumentasi, penelusuran direktori berkelompok (M2)
- **Pelengkapan anotasi**: 12 anotasi baru `tag` / `group` / `author` / `header` / `route_param` / `response_status` / `success` / `error` / `not_debug` / `md` / `sort` / `ref` (M3)

### Sudah Diimplementasikan (M4)

- **Debugging online**: panel «Debugging Online» bawaan di halaman dokumentasi — Base URL terisi otomatis `location.origin` untuk koneksi langsung lintas-domain ke layanan target, form parameter terisi otomatis dengan mock, penggantian placeholder rute `{name}` / `:name`, parameter GET/HEAD digabung ke query, method lainnya dirangkai sebagai JSON body, edit header permintaan + header kustom, tampilan respons (status / durasi / pretty JSON), peringatan kuning saat CORS gagal
- **Mesin Mock** (`crates/apidoc-mock`, bergantung pada crate fake, 15 aturan: name / company / email / phone / url / ip / city / country / text / number / int / float / bool / uuid / date). Prioritas aturan: `mock="fake:xxx"` memakai tabel aturan fake (nama tidak dikenal kembali ke nilai default) ← mock non-kosong lainnya langsung dikeluarkan apa adanya (mis. `mock="1"`, `mock="erik"`) ← tanpa mock dibuat otomatis sesuai `ty` (int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`); children bersarang rekursif, array tetap 2 item
- **Antarmuka mock**: adaptor axum menambah `GET /apidoc/mock?url=&method=`, pencocokan presisi url + method, tidak cocok mengembalikan 404; panel debugging menyembunyikan endpoint `not_debug` secara default, baru tampil setelah mencentang «Tampilkan antarmuka not_debug»
- **Koneksi langsung CORS**: debugging online dijalankan browser langsung ke antarmuka target, `cors_layer` dari adaptor yang mengizinkan (proksi balik sisi server disisakan untuk v2)

### Dalam Rencana

- Multi-aplikasi / multi-versi / kata sandi akses
- Ekspor Markdown / TypeScript / Swagger (OpenAPI3) (M5)
- Adaptasi multi-kerangka kerja (apidoc-axum selesai, apidoc-actix belum dikerjakan)
- v2: generator kode, referensi field tabel data, tautan berbagi, peristiwa debugging

## Arsitektur

<img src="images/id-architecture.svg" alt="Arsitektur keseluruhan apidoc-rust" width="100%">

## Fitur

<img src="images/id-features.svg" alt="Fitur proyek apidoc-rust" width="100%">

## Siklus Hidup

<img src="images/id-lifecycle.svg" alt="Siklus hidup dokumentasi apidoc-rust" width="100%">

## Struktur Proyek

```
apidoc-rust/
├── Cargo.toml                 # Konfigurasi workspace (resolver 2)
├── VERSION                    # Versi proyek (v1.0.0, terpisah dari versi kerangka 0.1.0)
├── crates/
│   ├── apidoc/                # Inti runtime (independen kerangka kerja)
│   │   ├── src/lib.rs         # Model data + agregasi DocRegistry + api.json
│   │   ├── tests/             # Tes integrasi (ekspansi makro/agregasi/serialisasi/antar-crate)
│   │   └── examples/demo.rs   # Contoh: anotasi + output api.json
│   ├── apidoc-macros/         # proc-macro: 19 makro atribut
│   │   └── src/lib.rs         # Definisi makro + parsing parameter + validasi waktu kompilasi
│   ├── apidoc-mock/           # Mesin Mock (pembuatan data mock dengan aturan fake)
│   ├── apidoc-test-fixtures/  # Fixture pengujian registrasi antar-crate
│   └── apidoc-axum/           # Adaptor axum (rute dokumentasi + cors_layer + /apidoc/mock)
├── .github/
│   └── workflows/release.yml  # Workflow rilis (membaca VERSION, membuat tag+release inkremental)
└── docs/
    ├── images/                # Diagram arsitektur/fitur/siklus hidup (SVG)
    └── i18n/                  # Dokumentasi multibahasa (12 bahasa)
```

## Panduan Penggunaan

### 1. Menambahkan Dependensi

```toml
[dependencies]
apidoc = "0.1"        # atau path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # ekspansi makro merujuk langsung ke path linkme, konsumen wajib dependensi langsung
serde_json = "1"      # untuk output api.json
```

> `apidoc-mock` (mesin Mock) adalah dependensi internal kerangka kerja, ditambahkan otomatis melalui adaptor, umumnya konsumen tidak perlu menggunakannya langsung.

### 2. Menulis Anotasi

Pasang anotasi satu per satu pada fungsi handler, dokumentasi langsung dihasilkan saat kompilasi:

```rust
use apidoc::*;

#[apidoc::title("Mendapatkan Info Pengguna")]
#[apidoc::desc("Mengambil detail pengguna berdasarkan ID pengguna")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::param(name = "user_id", ty = "int", required, desc = "ID Pengguna", mock = "1")]
#[apidoc::query(name = "lang", ty = "string", desc = "Bahasa", default = "zh-CN")]
#[apidoc::returned(
    name = "data",
    ty = "object",
    desc = "Data Pengguna",
    children = [
        { name = "id", ty = "int", required, desc = "ID Pengguna" },
        { name = "name", ty = "string", required, desc = "Nama Pengguna", mock = "erik" },
    ]
)]
fn get_user_info() -> String {
    unimplemented!()
}
```

### 3. Mengumpulkan dan Menghasilkan Output

```rust
fn main() {
    let endpoints = DocRegistry::collect();
    let doc = ApiDoc {
        config: ApidocConfig {
            title: "API Saya".to_string(),
            description: None,
        },
        endpoints,
    };
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}
```

### 4. Menjalankan Contoh

```bash
cargo run --example demo -p apidoc
```

Output (cuplikan):

```json
{
  "config": { "title": "demo api" },
  "endpoints": [
    {
      "title": "Mendapatkan Info Pengguna",
      "desc": "Mengambil detail pengguna berdasarkan ID pengguna",
      "url": "/api/user/info",
      "method": "GET",
      "params": [
        { "name": "user_id", "type": "int", "required": true, "desc": "ID Pengguna", "mock": "1" }
      ],
      "querys": [
        { "name": "lang", "type": "string", "required": false, "default": "zh-CN", "desc": "Bahasa" }
      ],
      "returned": [
        {
          "name": "data",
          "type": "object",
          "required": false,
          "desc": "Data Pengguna",
          "children": [
            { "name": "id", "type": "int", "required": true, "desc": "ID Pengguna" },
            { "name": "name", "type": "string", "required": true, "desc": "Nama Pengguna", "mock": "erik" }
          ]
        }
      ]
    }
  ]
}
```

### 5. Debugging Online dan Mock (M4)

Buka halaman dokumentasi → pilih antarmuka → panel «Debugging Online» di kanan terisi otomatis sesuai aturan mock → arahkan Base URL ke alamat layanan target (default `location.origin`, koneksi langsung lintas-domain) → klik Kirim, dapatkan respons asli (kode status / durasi / pretty JSON). Panel debugging menyembunyikan endpoint `not_debug` secara default, baru ditampilkan setelah mencentang «Tampilkan antarmuka not_debug».

**Persyaratan CORS**: debugging online dijalankan browser langsung ke antarmuka target, sehingga layanan target perlu memasang `cors_layer` yang disediakan adaptor untuk mengizinkan permintaan lintas-domain; saat CORS gagal, panel menampilkan peringatan kuning.

Sintaks aturan Mock (tiga prioritas):

```rust
#[apidoc::param(name = "email", ty = "string", desc = "Email", mock = "fake:email")]  // dihasilkan aturan fake
#[apidoc::param(name = "status", ty = "string", desc = "Status", mock = "1")]          // mock non-kosong langsung apa adanya
#[apidoc::param(name = "name", ty = "string", desc = "Nama Pengguna")]                  // tanpa mock: otomatis sesuai ty
#[apidoc::returned(
    name = "data",
    ty = "object",
    children = [
        { name = "id", ty = "int", required },       // tanpa mock → "1"
        { name = "email", ty = "string", mock = "fake:email" },  // children bersarang rekursif
    ]
)]
fn create_user() -> String {
    unimplemented!()
}
```

15 aturan fake bawaan: `name` / `company` / `email` / `phone` / `url` / `ip` / `city` / `country` / `text` / `number` / `int` / `float` / `bool` / `uuid` / `date`; nama tidak dikenal kembali ke nilai default. Aturan pembuatan otomatis tanpa mock: int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`; array tetap 2 item.

## Rencana Pengembangan

| Tahap | Konten | Status |
|------|------|------|
| M1 | Kerangka workspace + model data + MVP makro + registrasi linkme | ✅ Selesai |
| M2 | Adaptor axum + UI dokumentasi tertanam + direktori berkelompok | ✅ Selesai |
| M3 | Melengkapi anotasi (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | ✅ Selesai |
| M4 | Debugging online + mesin Mock | ✅ Selesai |
| M5 | Ekspor markdown / typescript / swagger.json | Dalam rencana |
| M6 | Otentikasi kata sandi, multi-aplikasi multi-versi, rilis | Dalam rencana |

## Dokumentasi Multibahasa

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

## Dukungan dan Donasi

Jika proyek ini bermanfaat bagi Anda, silakan beri ⭐ Star untuk mendukung kami, dan donasi untuk mendukung open source juga sangat diterima!

### 微信支付 / 支付宝 (WeChat Pay / Alipay)

<table>
  <tr>
    <td align="center">
      <img src="../weixinpay.png" width="130" height="130" alt="微信支付 (WeChat Pay)" /><br/>
      <strong>微信支付 (WeChat Pay)</strong>
    </td>
    <td align="center">
      <img src="../alipay.png" width="130" height="130" alt="支付宝 (Alipay)" /><br/>
      <strong>支付宝 (Alipay)</strong>
    </td>
  </tr>
</table>

### Donasi Transfer Global

**【Informasi Penerima】**

- Nama penerima: WANG KEXUN
- Nomor rekening penerima: 881015918251

**【Bank Penerima】**

- SWIFT Code ZA Bank: AABLHKHHXXX
- Nama bank: ZA Bank Limited
- Nomor bank: 387
- Alamat bank: Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【Bank Perantara Transfer Lintas Negara (Jika Diperlukan)】**

> Perlu diperhatikan: ini adalah informasi bank perantara (bank pengirim) untuk transfer lintas negara, bukan informasi bank penerima. Silakan tanyakan kepada bank pengirim apakah perlu menyediakan informasi bank perantara.

- **Bank perantara untuk transfer HKD, CNY, dan USD adalah Citibank:**
  - Nama bank: Citibank N.A. Hong Kong
  - SWIFT Code: CITIHKHXXXX
  - Nomor bank: 006
  - Nama cabang: Hong Kong Branch
  - Nomor cabang: 391
  - Alamat bank: Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **Bank perantara untuk mata uang lainnya adalah BNY Mellon:**
  - Nama bank: THE BANK OF NEW YORK MELLON
  - SWIFT Code: IRVTUS3NXXX
  - Alamat bank: THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## License

[MIT](../../LICENSE)
