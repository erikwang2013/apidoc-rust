<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 Biblioteca de plugins universal para gerar documentação de API a partir de macros de procedimento (proc-macro) do Rust
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
<a href="README-pt.md"><strong>Português</strong></a> ·
<a href="README-hi.md">हिन्दी</a> ·
<a href="README-ar.md">العربية</a> ·
<a href="README-bn.md">বাংলা</a> ·
<a href="README-id.md">Bahasa Indonesia</a> ·
<a href="README-ja.md">日本語</a>
</div>

## Introdução

apidoc-rust é um **gerador de documentação de API universal e baseado em plugins** implementado em Rust, inspirado no [apidoc-php](https://github.com/erikwang2013/apidoc-php) (uma extensão do Composer que gera documentação de API a partir dos attributes do PHP 8). Ele concretiza o conceito de "anotações como documentação" na forma nativa do Rust:

- **Geração em tempo de compilação**: a documentação é gerada por macros de procedimento durante a compilação, garantindo que ela nunca fique dessincronizada do código;
- **Coleta de custo zero**: registro estático via linkme; uma única agregação em tempo de execução obtém toda a documentação da API;
- **Plugin universal**: o núcleo é independente do framework HTTP e se conecta a qualquer framework por meio de adaptadores finos (axum / actix-web).

## Recursos

### Implementado (M1)

- **Documentação por anotações**: sete macros de atributo — `title` / `desc` / `method` / `url` / `param` / `query` / `returned` — para anotar item a item (equivalente à sintaxe de attributes do PHP); os parâmetros suportam aninhamento de `required` / `default` / `desc` / `mock` / `children`
- **Validação em tempo de compilação**: url deve começar com `/`, method em lista de permissão, param name obrigatório etc.; anotações inválidas geram erro de compilação (com span preciso)
- **Coleta automática**: registro estático via `distributed_slice` do linkme, sem necessidade de lista manual de endpoints; `DocRegistry::collect()` mescla por id e restaura a ordem de declaração por seq, coletando automaticamente entre crates
- **Saída api.json**: o serde serializa um modelo de dados unificado da documentação (config + endpoints), com campos alinhados à semântica do PHP

### Planejado

- Depuração on-line (conexão direta via CORS do navegador ao endpoint de destino), dados Mock (gerados por regras fake)
- Múltiplos aplicativos / múltiplas versões / senha de acesso
- Exportação Markdown / TypeScript / Swagger (OpenAPI3)
- Adaptação a múltiplos frameworks (apidoc-axum / apidoc-actix)
- v2: gerador de código, referência de campos de tabelas de dados, links de compartilhamento, eventos de depuração

## Arquitetura

<img src="images/pt-architecture.svg" alt="Arquitetura geral do apidoc-rust" width="100%">

## Funcionalidades

<img src="images/pt-features.svg" alt="Recursos do projeto apidoc-rust" width="100%">

## Ciclo de vida

<img src="images/pt-lifecycle.svg" alt="Ciclo de vida da documentação do apidoc-rust" width="100%">

## Estrutura do projeto

```
apidoc-rust/
├── Cargo.toml                 # configuração do workspace (resolver 2)
├── crates/
│   ├── apidoc/                # núcleo em tempo de execução (independente de framework)
│   │   ├── src/lib.rs         # modelo de dados + agregação DocRegistry + api.json
│   │   ├── tests/             # testes de integração (expansão de macros/agregação/serialização/entre crates)
│   │   └── examples/demo.rs   # exemplo: anotações + saída api.json
│   ├── apidoc-macros/         # proc-macro: 7 macros de atributo
│   │   └── src/lib.rs         # definição de macros + análise de parâmetros + validação em tempo de compilação
│   └── apidoc-test-fixtures/  # fixture de teste para registro entre crates
└── docs/
    ├── images/                # diagramas de arquitetura/recursos/ciclo de vida (SVG)
    └── i18n/                  # documentação multilíngue (12 idiomas)
```

## Como usar

### 1. Adicionar dependências

```toml
[dependencies]
apidoc = "0.1"        # ou path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # a expansão de macros referencia diretamente o caminho do linkme; consumidores precisam depender dele diretamente
serde_json = "1"      # usado para gerar o api.json
```

### 2. Escrever anotações

Anexe as anotações item a item às funções handler e a documentação será gerada em tempo de compilação:

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

### 3. Coleta e saída

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

### 4. Executar o exemplo

```bash
cargo run --example demo -p apidoc
```

Saída (trecho):

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

## Roteiro de desenvolvimento

| Fase | Conteúdo | Status |
|------|----------|--------|
| M1 | esqueleto do workspace + modelo de dados + MVP de macros + registro linkme | ✅ Concluído |
| M2 | adaptador axum + UI de documentação embutida + diretório agrupado | ⏳ Planejado |
| M3 | macros de atributo complementares (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | Planejado |
| M4 | depuração on-line + mecanismo Mock | Planejado |
| M5 | exportação markdown / typescript / swagger.json | Planejado |
| M6 | autenticação por senha, múltiplos aplicativos e versões, lançamento | Planejado |

## Documentação multilíngue

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

## Suporte e doações

Se este projeto foi útil para você, considere dar uma ⭐ Star para nos apoiar — também aceitamos doações para apoiar o código aberto!

### WeChat Pay (微信支付) / Alipay (支付宝)

<table>
  <tr>
    <td align="center">
      <img src="../weixinpay.png" width="130" height="130" alt="WeChat Pay (微信支付)" /><br/>
      <strong>WeChat Pay (微信支付)</strong>
    </td>
    <td align="center">
      <img src="../alipay.png" width="130" height="130" alt="Alipay (支付宝)" /><br/>
      <strong>Alipay (支付宝)</strong>
    </td>
  </tr>
</table>

### Doações por transferência internacional

**【Informações do beneficiário】**

- Nome do beneficiário: WANG KEXUN
- Número da conta do beneficiário: 881015918251

**【Banco do beneficiário】**

- Código SWIFT do ZA Bank: AABLHKHHXXX
- Nome do banco: ZA Bank Limited
- Código do banco: 387
- Endereço do banco: Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【Banco intermediário para remessas internacionais (se necessário)】**

> Atenção: estas são informações do banco intermediário (correspondente) para remessas internacionais, e não do banco do beneficiário. Consulte o seu banco remetente para saber se as informações do banco intermediário são necessárias.

- **O banco intermediário para depósitos em dólares de Hong Kong (HKD), RMB e dólares americanos (USD) é o Citibank:**
  - Nome do banco: Citibank N.A. Hong Kong
  - Código SWIFT: CITIHKHXXXX
  - Código do banco: 006
  - Nome da agência: Hong Kong Branch
  - Número da agência: 391
  - Endereço do banco: Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **O banco intermediário para outras moedas é o BNY Mellon:**
  - Nome do banco: THE BANK OF NEW YORK MELLON
  - Código SWIFT: IRVTUS3NXXX
  - Endereço do banco: THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## License

[MIT](../../LICENSE)
