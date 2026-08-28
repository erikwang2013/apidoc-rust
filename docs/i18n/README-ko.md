<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 Rust 프로시저 매크로(proc-macro)로 API 인터페이스 문서를 생성하는 범용 플러그인 라이브러리
</div>

<div align="center">
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/badge/license-MIT-green"></a>
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/github/stars/erikwang2013/apidoc-rust"></a>
</div>

<div align="center">
<a href="../../README.md">中文</a> ·
<a href="README-en.md">English</a> ·
<a href="README-ko.md"><strong>한국어</strong></a> ·
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

## 프로젝트 소개

apidoc-rust는 Rust로 구현된 **범용 플러그형 API 인터페이스 문서 생성기**로, [apidoc-php](https://github.com/erikwang2013/apidoc-php)(PHP 8 attributes 기반으로 API 문서를 생성하는 composer 확장)를 참고해 "주석이 곧 문서"라는 능력을 Rust 네이티브 방식으로 구현했습니다:

- **컴파일 타임 생성**: 문서는 프로시저 매크로에 의해 컴파일 타임에 생성되어, 문서와 코드가 결코 동기화되지 않을 일이 없습니다;
- **제로 비용 수집**: linkme 정적 등록으로 런타임에 한 번의 집계로 모든 인터페이스 문서를 얻습니다;
- **범용 플러그인**: 코어는 HTTP 프레임워크와 무관하며, 얇은 어댑터(axum / actix-web)로 어떤 프레임워크에도 연결됩니다.

## 기능

### 구현됨 (M1-M3)

- **주석 기반 문서**: `title` / `desc` / `method` / `url` / `param` / `query` / `returned` 7개 속성 매크로로 개별 주석 작성(PHP attributes 방식에 대응), 파라미터는 `required` / `default` / `desc` / `mock` / `children` 중첩 지원
- **컴파일 타임 검증**: url은 반드시 `/`로 시작, method 화이트리스트, param name 필수 등, 잘못된 주석은 컴파일 타임에 오류 발생(span 정밀)
- **자동 수집**: linkme `distributed_slice` 정적 등록으로 수동 인터페이스 목록 불필요; `DocRegistry::collect()`가 id별로 병합하고 seq로 선언 순서를 복원하며, 크로스 crate 자동 수집
- **api.json 출력**: serde 직렬화로 통일된 문서 데이터 모델(config + endpoints) 생성, 필드는 PHP 의미론에 정렬
- **axum 어댑터 + 내장 문서 UI**: 라우트를 마운트하면 문서 페이지가 제공되며, 그룹 목차 탐색 지원(M2)
- **주석 보강**: `tag` / `group` / `author` / `header` / `route_param` / `response_status` / `success` / `error` / `not_debug` / `md` / `sort` / `ref` 12개 신규 주석(M3)

### 구현됨 (M4)

- **온라인 디버깅**: 문서 페이지에 내장된 「온라인 디버깅」 패널 — Base URL을 `location.origin`으로 미리 채워 크로스 오리진으로 대상 서비스에 직접 연결, 파라미터 폼에 mock 미리 채움, `{name}` / `:name` 라우트 플레이스홀더 치환, GET/HEAD 파라미터는 query로 병합, 나머지 method는 JSON body로 조립, 요청 헤더 편집 + 커스텀 헤더, 응답 표시(상태 / 소요 시간 / pretty JSON), CORS 실패 시 노란색 힌트
- **Mock 엔진**(`crates/apidoc-mock`, fake crate 의존, 15개 규칙: name / company / email / phone / url / ip / city / country / text / number / int / float / bool / uuid / date). 규칙 우선순위: `mock="fake:xxx"`는 fake 규칙 테이블로 처리(미지정명은 기본값으로 폴백) → 그 외 비어 있지 않은 mock은 그대로 출력(예: `mock="1"`, `mock="erik"`) → mock 없으면 `ty`로 자동 생성(int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`); children은 재귀 중첩, array는 고정 2항목
- **mock 인터페이스**: axum 어댑터에 `GET /apidoc/mock?url=&method=` 추가, url + method 정확 일치, 불일치 시 404 반환; 디버깅 패널은 기본적으로 `not_debug` 엔드포인트를 숨기고, 「not_debug 인터페이스 표시」를 체크해야만 표시
- **CORS 직접 연결**: 온라인 디버깅은 브라우저가 대상 인터페이스에 직접 연결하며, 어댑터의 `cors_layer`가 허용 처리(서버 측 리버스 프록시는 v2에 예정)

### 계획 중

- 다중 앱 / 다중 버전 / 접근 비밀번호
- Markdown / TypeScript / Swagger(OpenAPI3) 내보내기(M5)
- 다중 프레임워크 어댑터(apidoc-axum 완료, apidoc-actix 미구현)
- v2: 코드 생성기, 데이터 테이블 필드 참조, 공유 링크, 디버깅 이벤트

## 아키텍처

<img src="images/ko-architecture.svg" alt="apidoc-rust 전체 아키텍처" width="100%">

## 기능

<img src="images/ko-features.svg" alt="apidoc-rust 프로젝트 기능" width="100%">

## 수명주기

<img src="images/ko-lifecycle.svg" alt="apidoc-rust 문서 수명주기" width="100%">

## 프로젝트 구조

```
apidoc-rust/
├── Cargo.toml                 # workspace 설정(resolver 2)
├── VERSION                    # 프로젝트 버전(v1.0.0, 프레임워크 버전 0.1.0과 분리)
├── crates/
│   ├── apidoc/                # 런타임 코어(프레임워크 무관)
│   │   ├── src/lib.rs         # 데이터 모델 + DocRegistry 집계 + api.json
│   │   ├── tests/             # 통합 테스트(매크로 확장/집계/직렬화/크로스 crate)
│   │   └── examples/demo.rs   # 예제: 주석 + api.json 출력
│   ├── apidoc-macros/         # proc-macro: 19개 속성 매크로
│   │   └── src/lib.rs         # 매크로 정의 + 파라미터 파싱 + 컴파일 타임 검증
│   ├── apidoc-mock/           # Mock 엔진(fake 규칙으로 mock 데이터 생성)
│   ├── apidoc-test-fixtures/  # 크로스 crate 등록 테스트 픽스처
│   └── apidoc-axum/           # axum 어댑터(문서 라우트 + cors_layer + /apidoc/mock)
├── .github/
│   └── workflows/release.yml  # 릴리스 워크플로(VERSION 읽기, 태그+릴리스 증분 생성)
└── docs/
    ├── images/                # 아키텍처/기능/수명주기 다이어그램(SVG)
    └── i18n/                  # 다국어 문서(12개 언어)
```

## 사용 방법

### 1. 의존성 추가

```toml
[dependencies]
apidoc = "0.1"        # 또는 path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # 매크로 확장이 linkme 경로를 직접 참조하므로 소비 측은 직접 의존해야 함
serde_json = "1"      # api.json 출력용
```

> `apidoc-mock`(Mock 엔진)은 프레임워크 내부 의존성으로 어댑터가 자동으로 가져오므로, 일반적인 소비 측은 직접 사용할 필요가 없습니다.

### 2. 주석 작성

handler 함수에 개별 주석을 달면 문서가 컴파일 타임에 생성됩니다:

```rust
use apidoc::*;

#[apidoc::title("사용자 정보 조회")]
#[apidoc::desc("사용자 ID로 사용자 상세 조회")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::param(name = "user_id", ty = "int", required, desc = "사용자 ID", mock = "1")]
#[apidoc::query(name = "lang", ty = "string", desc = "언어", default = "zh-CN")]
#[apidoc::returned(
    name = "data",
    ty = "object",
    desc = "사용자 데이터",
    children = [
        { name = "id", ty = "int", required, desc = "사용자 ID" },
        { name = "name", ty = "string", required, desc = "사용자 이름", mock = "erik" },
    ]
)]
fn get_user_info() -> String {
    unimplemented!()
}
```

### 3. 수집과 출력

```rust
fn main() {
    let endpoints = DocRegistry::collect();
    let doc = ApiDoc {
        config: ApidocConfig {
            title: "내 API".to_string(),
            description: None,
        },
        endpoints,
    };
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}
```

### 4. 예제 실행

```bash
cargo run --example demo -p apidoc
```

출력(발췌):

```json
{
  "config": { "title": "demo api" },
  "endpoints": [
    {
      "title": "사용자 정보 조회",
      "desc": "사용자 ID로 사용자 상세 조회",
      "url": "/api/user/info",
      "method": "GET",
      "params": [
        { "name": "user_id", "type": "int", "required": true, "desc": "사용자 ID", "mock": "1" }
      ],
      "querys": [
        { "name": "lang", "type": "string", "required": false, "default": "zh-CN", "desc": "언어" }
      ],
      "returned": [
        {
          "name": "data",
          "type": "object",
          "required": false,
          "desc": "사용자 데이터",
          "children": [
            { "name": "id", "type": "int", "required": true, "desc": "사용자 ID" },
            { "name": "name", "type": "string", "required": true, "desc": "사용자 이름", "mock": "erik" }
          ]
        }
      ]
    }
  ]
}
```

### 5. 온라인 디버깅과 Mock(M4)

문서 페이지를 열고 → 인터페이스를 선택하면 → 오른쪽 「온라인 디버깅」 패널에 mock 규칙대로 파라미터가 미리 채워집니다 → Base URL을 대상 서비스 주소로 지정하고(기본 `location.origin`, 크로스 오리진 직접 연결) → 보내기를 누르면 실제 응답(상태 코드 / 소요 시간 / pretty JSON)을 얻습니다. 디버깅 패널은 기본적으로 `not_debug` 엔드포인트를 숨기며, 「not_debug 인터페이스 표시」를 체크한 후에만 보여줍니다.

**CORS 요구 사항**: 온라인 디버깅은 브라우저가 대상 인터페이스에 직접 연결하므로, 대상 서비스는 어댑터가 제공하는 `cors_layer`를 마운트해 크로스 오리진 요청을 허용해야 합니다. CORS 실패 시 패널에 노란색 힌트가 표시됩니다.

Mock 규칙 문법(3단계 우선순위):

```rust
#[apidoc::param(name = "email", ty = "string", desc = "邮箱", mock = "fake:email")]  // fake 규칙 생성
#[apidoc::param(name = "status", ty = "string", desc = "状态", mock = "1")]          // 비어 있지 않은 mock은 그대로 출력
#[apidoc::param(name = "name", ty = "string", desc = "用户名")]                       // mock 없음: ty로 자동 생성
#[apidoc::returned(
    name = "data",
    ty = "object",
    children = [
        { name = "id", ty = "int", required },       // mock 없음 → "1"
        { name = "email", ty = "string", mock = "fake:email" },  // children 재귀 중첩
    ]
)]
fn create_user() -> String {
    unimplemented!()
}
```

내장 fake 규칙 15개: `name` / `company` / `email` / `phone` / `url` / `ip` / `city` / `country` / `text` / `number` / `int` / `float` / `bool` / `uuid` / `date`; 알 수 없는 규칙명은 기본값으로 폴백됩니다. mock 없는 자동 생성 규칙: int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`; array는 고정 2항목.

## 개발 계획

| 단계 | 내용 | 상태 |
|------|------|------|
| M1 | workspace 뼈대 + 데이터 모델 + 매크로 MVP + linkme 등록 | ✅ 완료 |
| M2 | axum 어댑터 + 내장 문서 UI + 그룹 목차 | ✅ 완료 |
| M3 | 주석 보강(tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | ✅ 완료 |
| M4 | 온라인 디버깅 + Mock 엔진 | ✅ 완료 |
| M5 | markdown / typescript / swagger.json 내보내기 | 계획 중 |
| M6 | 비밀번호 인증, 다중 앱·다중 버전, 릴리스 | 계획 중 |

## 다국어 문서

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

## 후원과 기부

이 프로젝트가 도움이 되셨다면 ⭐ Star로 응원해 주시고, 오픈소스 후원 기부도 환영합니다!

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

### 글로벌 송금 기부

**【수취인 정보】**

- 수취인 이름: WANG KEXUN
- 수취 계좌 번호: 881015918251

**【수취 은행】**

- ZA Bank SWIFT Code: AABLHKHHXXX
- 은행 이름: ZA Bank Limited
- 은행 번호: 387
- 은행 주소: Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【해외 송금 중개 은행(필요 시)】**

> 참고: 이는 해외 송금 중개 은행(중개 은행) 정보이며 수취 은행 정보가 아닙니다. 송금 은행에 중개 은행 정보 제공이 필요한지 문의하시기 바랍니다.

- **홍콩 달러, 위안화, 미국 달러 송금 시 중개 은행은 Citibank:**
  - 은행 이름: Citibank N.A. Hong Kong
  - SWIFT Code: CITIHKHXXXX
  - 은행 번호: 006
  - 지점 이름: Hong Kong Branch
  - 지점 번호: 391
  - 은행 주소: Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **기타 통화 송금 시 중개 은행은 BNY Mellon:**
  - 은행 이름: THE BANK OF NEW YORK MELLON
  - SWIFT Code: IRVTUS3NXXX
  - 은행 주소: THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## License

[MIT](LICENSE)
