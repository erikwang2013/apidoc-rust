![Mascote do Apidoc](../images/apidoc-pet.svg)

# Apidoc (apidoc-rust)

Ferramenta de geração de documentação de API e desenvolvimento de interfaces baseada em macros de procedimento (proc-macro) do Rust, compatível com axum, actix-web e outros frameworks populares

[![License](https://img.shields.io/badge/license-MIT-green)](https://github.com/erikwang2013/apidoc-rust)
[![Stars](https://img.shields.io/github/stars/erikwang2013/apidoc-rust)](https://github.com/erikwang2013/apidoc-rust)

[中文](../../README.md) ·
[English](README-en.md) ·
[한국어](README-ko.md) ·
[Русский](README-ru.md) ·
[Deutsch](README-de.md) ·
[Français](README-fr.md) ·
[Español](README-es.md) ·
**[Português](README-pt.md)** ·
[हिन्दी](README-hi.md) ·
[العربية](README-ar.md) ·
[বাংলা](README-bn.md) ·
[Bahasa Indonesia](README-id.md) ·
[日本語](README-ja.md)

## 📖Introdução

O Apidoc é uma biblioteca de plugins em Rust que gera automaticamente documentação de interfaces de API analisando **macros de procedimento (proc-macro) do Rust**, compatível com frameworks populares como axum e actix-web. Além da geração automática de documentação, integra depuração de interfaces on-line, dados de teste Mock, geração de código Json/TypeScript, gerador de endpoints, gerador de código e outros recursos, cobrindo todo o fluxo de desenvolvimento, depuração e entrega de interfaces, com o objetivo de aumentar a eficiência do desenvolvimento de APIs. Desde a 2.2.0 também expõe o **contrato de backend UIC-1**, consumido diretamente pelo build Vue do apidoc-ui; a UI nativa embutida é mantida e é possível alternar entre as duas por configuração.

Abordagem de implementação do apidoc-rust:

- **Geração em tempo de compilação**: a documentação é gerada por macros de procedimento durante a compilação, garantindo que ela nunca fique dessincronizada do código;
- **Coleta de custo zero**: registro estático via linkme; uma única agregação em tempo de execução obtém toda a documentação da API;
- **Plugin universal**: o núcleo é independente do framework HTTP e se conecta a qualquer framework por meio de adaptadores finos (axum / actix-web).

### ✨Descrição do projeto

- **Pronto para usar**: sem configurações complicadas; após a instalação, basta escrever as anotações conforme a documentação para gerar automaticamente a documentação da API.
- **Escrita simples**: referências de campo `ref` / `table` e nove formatos de definição externos (OpenAPI, AsyncAPI, GraphQL, Proto, WSDL, EDMX, JSON Schema, JSON-RPC, XML-RPC) são suportados via `#[apidoc::definition]`; algumas poucas anotações bastam para completar a definição de todos os campos.
- **Depuração on-line**: depure os endpoints diretamente na página de documentação, com suporte a parâmetros globais, dados Mock e eventos de depuração.
- **Depuração multiprotocolo**: 45 anotações de protocolo (RPC, mensageria, industrial, mídia, IoT e mais) com conexão direta pelo navegador e retransmissão no servidor; as anotações geram tanto a documentação do protocolo quanto um painel de depuração.
- **Contrato UIC-1**: `/apidoc/ui/v1/*` expõe capabilities / config / menus / detail / docs / debug / share / cache / generator / export, de modo que um único build Vue do apidoc-ui atende PHP, Go e Rust. A UI estática Vue é ativada com `ui` / `ui_path` (pacotes de release usam Vue por padrão; o padrão da biblioteca mantém a UI nativa embutida).
- **Múltiplos aplicativos/versões**: projetos com aplicativo único, múltiplos aplicativos e múltiplas versões são configuráveis; os endpoints são exibidos e alternados por aplicativo/versão.
- **Grupos/Tag**: os endpoints suportam agrupamento multinível e marcação por Tag.
- **Documentação Markdown**: a anotação `md` permite montar Markdown como página de documentação.
- **Geração Json/TypeScript**: cada endpoint gera automaticamente exemplos de requisição/resposta Json e definições de tipos TypeScript, prontos para uso direto no frontend.
- **Gerador de código**: com configuração + templates é possível gerar o código de negócio e os arquivos Api do frontend.
- **Compartilhamento de endpoints**: permite gerar links de compartilhamento de um aplicativo/endpoint específico e exportar `swagger.json`.
- **Acesso seguro**: autorização com senha global e senhas independentes por aplicativo/versão; a depuração no servidor fica desligada por padrão e aplica o guard SSRF mais limites de tamanho/frames/timeout; arquivos de definição e protocolos industriais têm limites rígidos; a documentação é materializada na compilação, com respostas locais abaixo de 10 ms p99.

## Recursos

### Implementado (M1-M3)

- **Documentação por anotações**: sete macros de atributo principais — `title` / `desc` / `method` / `url` / `param` / `query` / `returned` — além de `tag` / `group` / `header` / `table` e 45 anotações de protocolo; os parâmetros suportam aninhamento de `required` / `default` / `desc` / `mock` / `children` / `field`
- **Validação em tempo de compilação**: url deve começar com `/`, method em lista de permissão, param name obrigatório etc.; anotações inválidas geram erro de compilação (com span preciso)
- **Coleta automática**: registro estático via `distributed_slice` do linkme, sem necessidade de lista manual de endpoints; `DocRegistry::collect()` mescla por id e restaura a ordem de declaração por seq, coletando automaticamente entre crates
- **Saída api.json**: o serde serializa um modelo de dados unificado da documentação (config + endpoints), com nomes de campos compartilhados entre o modelo e os exportadores
- **Adaptador axum + UI de documentação embutida**: montar a rota já fornece a página de documentação; navegação por diretórios agrupados (M2)
- **Complemento de anotações**: 12 novas anotações — `tag` / `group` / `author` / `header` / `route_param` / `response_status` / `success` / `error` / `not_debug` / `md` / `sort` / `ref` (M3)

### Implementado (M4)

- **Depuração on-line**: a página de documentação inclui o painel «Depuração on-line» — Base URL pré-preenchida com `location.origin` para conexão direta entre domínios ao serviço de destino, parâmetros do formulário pré-preenchidos com mock, substituição de placeholders de rota `{name}` / `:name`, parâmetros GET/HEAD incorporados à query string, corpo JSON montado para os demais métodos, edição de cabeçalhos da requisição + cabeçalhos personalizados, exibição da resposta (status / tempo / JSON bonito), aviso amarelo em caso de falha de CORS
- **Mecanismo Mock** (`crates/apidoc/src/mock.rs`, depende do crate fake, 15 regras: name / company / email / phone / url / ip / city / country / text / number / int / float / bool / uuid / date). Prioridade das regras: `mock="fake:xxx"` usa a tabela de regras fake (nome desconhecido → valor padrão) → demais mock não vazios são emitidos como estão (ex.: `mock="1"`, `mock="erik"`) → sem mock, geração automática conforme `ty` (int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`); children aninhados recursivamente, array fixado em 2 itens
- **Endpoint mock**: o adaptador axum adiciona `GET /apidoc/mock?url=&method=`, correspondência exata de url + method, retorna 404 se não houver correspondência; o painel de depuração oculta por padrão os endpoints `not_debug`, que só aparecem ao marcar «Mostrar endpoints not_debug»
- **Conexão CORS direta**: a depuração on-line conecta o navegador diretamente ao endpoint de destino, liberado pelo `cors_layer` do adaptador; a retransmissão no servidor é fornecida pelos executores P2–P5

### Implementado (M5)

- **Exportação em quatro formatos** (`crates/apidoc/src/export/`): markdown / typescript / swagger (OpenAPI 3.0.0) / AsyncAPI 2.6 (P6); o crate central fornece `export::markdown::render` / `export::typescript::render` / `export::swagger::render` / `export::asyncapi::render`
- **Rotas de exportação**: os adaptadores adicionam `GET /apidoc/export?format=md|ts|swagger|asyncapi`, formato desconhecido → 400; Content-Type: `text/markdown` / `application/typescript` / `application/json`
- **markdown**: índice por grupos + tabela de parâmetros + bloco de resposta; **typescript**: gera os tipos `{Name}Params` / `{Name}Result` por namespace de grupo, endpoints sem grupo caem em `defaultGroup` (`default` é palavra reservada de TS); **swagger**: `info.version` prefere `ApidocConfig.version` e recorre à versão do pacote Cargo (atualmente 2.2.0)
- **Adaptador actix-web** (`crates/apidoc/src/actix.rs`): funcionalidade 1:1 com o adaptador axum — `apidoc_routes(ApidocConfig) -> Scope` monta /apidoc, /apidoc/api.json, /apidoc/mock, /apidoc/export, `cors_layer(CorsConfig)` libera CORS
- **UI compartilhada**: a UI de documentação (`src/ui.html`) sobe para o crate central, exportada como `pub const UI_HTML`; os dois adaptadores referenciam a mesma cópia (seguro para empacotamento de publicação)

### Implementado (M6)

- **Autenticação por senha (M6a)**: com `AuthConfig { enable, password, secret_key, expire }` ativado, o cliente chama `POST /apidoc/auth` (corpo JSON `{"password": "...", "appKey": "..."}`) para obter um token; as rotas de dados `/apidoc/api.json`, `/apidoc/export` e `/apidoc/mock` exigem `?token=xxx` — token ausente/expirado/incorreto retorna 401, e a UI da documentação mostra uma máscara de senha. A senha trafega no corpo da requisição (nunca na URL, nos logs ou no histórico do navegador), é comparada no servidor em tempo constante e nunca é transformada em hash no lado do cliente. O token é um token assinado sem estado `base64url(payload).base64url(HMAC-SHA256(payload))` com payload `{"exp": …}`; a chave de assinatura é derivada de `secret_key` e da credencial, de modo que alterar a senha invalida todos os tokens já emitidos. Os tokens usam apenas caracteres seguros para URL e entram direto nos links de compartilhamento
- **Linhas vermelhas de segurança da autenticação**: `password` / `secret_key` nunca são serializados — a saída do api.json é idêntica byte a byte à versão sem autenticação; com auth desativado, `/apidoc/auth` retorna 404 e as rotas de dados liberam direto; quando um aplicativo define password próprio, a senha do aplicativo tem prioridade sobre a global, e uma senha vazia (global ou do aplicativo) sempre falha de forma fechada tanto na emissão quanto na verificação; `secret_key` cai para uma **chave aleatória por processo** (os tokens morrem ao reiniciar — não existe chave padrão pública), `expire` padrão 86400 segundos; logins repetidamente incorretos recebem backoff exponencial (sem penalidade nas 3 primeiras vezes, depois 1s → 2s → … com teto de 60s, zerado em caso de sucesso); toda resposta de credencial/token leva `Cache-Control: no-store`
- **Múltiplos aplicativos e versões (M6b)**: `ApidocConfig.apps: Vec<AppConfig>` (`key` / `title` / `items` com subversões recursivas / `password`) configura a árvore de aplicativos; `#[apidoc::app("key")]` vincula o endpoint ao key de um aplicativo; endpoints sem key caem no aplicativo padrão; a saída do api.json ganha a árvore `doc.apps`, o seletor de aplicativo/versão aparece no topo da UI, e o token é guardado no localStorage separado por appKey (aplicativos diferentes podem ter senhas independentes)

### Implementado (v2)

- **Referência de campos de tabelas de dados (`table`)**: `#[apidoc::table("user")]` mescla (achatando-os) os campos da tabela configurada pela key em `ApidocConfig.tables` dentro de `returned` (mesma semântica do `ref`; a diferença é que a fonte de dados é uma tabela de configuração e não outro endpoint); os campos suportam `required` / `default` / `desc` / `mock`; uma key não configurada gera apenas um aviso no stderr; sem usar essa anotação, a saída é idêntica byte a byte à da v1
- **Materialização em compilação**: api.json, exportações, templates de codegen e corpos de `/apidoc/mock` são gerados uma vez ao montar as rotas; as requisições apenas consultam uma tabela. `ApidocConfig.cache` é mantido por compatibilidade, mas as respostas mock não precisam mais de memoização na requisição
- **Links de compartilhamento**: `GET /apidoc/share?app=&url=&method=&base=` → link profundo `{"url":"…"}` (`?app=<key>&ep=<url do endpoint>&method=<método>[&token=…]`); na página de documentação cada endpoint tem um botão «Compartilhar» (escreve na área de transferência; em caso de falha, recorre a um campo de texto copiável); abrir o link profundo posiciona no aplicativo/endpoint correspondente; com a autenticação ativada o link inclui o token emitido pelo servidor (a senha independente do aplicativo prevalece sobre a global) e abre sem exigir senha
- **Gerador de código**: `GET /apidoc/generate?template=<name>` → o texto renderizado (template desconhecido → 404); inclui `api.ts` (arquivo Api do frontend), `handler.rs` (esqueleto de endpoint em Rust) e `schema.sql` (instruções de criação de tabela geradas a partir de `tables`); um template de mesmo nome em `ApidocConfig.codegen` substitui o embutido. A sintaxe de template tem apenas duas formas: `{{variável}}` (se não definida, é mantida como está) e `{{#each lista}}…{{/each}}` (aninhável, ex.: `tables` × `fields`)
- **Eventos de depuração**: o painel de depuração on-line ganha os blocos recolhíveis «Script prévio» / «Script posterior» (persistidos no localStorage) — o prévio é executado antes da requisição com `new Function('ctx', code)` e pode alterar `ctx.url/method/headers/body` ou devolver um objeto para mesclagem superficial; o posterior é executado após a resposta e pode ler `status/ms/text/url/method/ep`; se devolver uma string, ela substitui o texto de resposta exibido; erro no script é apenas avisado na área de resultados e não interrompe a requisição

### Implementado (v2.2 · UIC-1)

- **Contrato de backend UIC-1 (Contract 1.0)**: `/apidoc/ui/v1/*` fornece capabilities / config / menus / detail / docMenus / docDetail / verifyAuth / sessões de depuração / registros de compartilhamento / cache / modelos de código / exportação em massa. `capabilities` informa apenas o que este build realmente compilou, para que a UI possa ocultar entradas indisponíveis. O Rust declara `share.actions=false`, `cache.mode=build`, gerador `forms=false` / `templates=true` e somente retransmissão unária (`relay-unary`).
- **Nove formatos de definição externos**: `#[apidoc::definition(file = "...", operation = "...")]` analisa OpenAPI, AsyncAPI, GraphQL, Proto, WSDL, EDMX, JSON Schema, JSON-RPC e XML-RPC em um payload uniforme `{format, source, operation, meta, messages, validation}`; `/apidoc/ui/v1/detail` retorna `definitions` com diagnósticos por item (`path` / `line` / `column` / `severity` / `message`). Definições nunca entram no api.json, então a UI nativa e a saída antiga permanecem inalteradas.
- **UI estática Vue**: `ui: UiMode::Vue` + `ui_path` serve o build `dist/uic1-ui/` do apidoc-ui; `ApidocConfig::release(title, ui_path)` é o ponto de entrada dos pacotes de release (o padrão da biblioteca continua sendo a UI nativa embutida). Inclui fallback SPA, `config.js` / `assets/*`, proteção contra escape de caminho e links simbólicos, e inicialização fail-closed quando `ui_path` ou `index.html` estão ausentes.
- **Relatório completo de capacidades de retransmissão**: `capabilities.debug.relay` lista apenas os executores compilados neste build; `debug-all` agrega todos os 30 ids de protocolo, de modo que a UI nunca oferece um executor que falhe com «not compiled».
- **Correções industriais e limites de recursos**: DNP3 analisa cabeçalhos de objeto após IIN e limita cada resposta a 10.000 pontos; IEC 60870-5-104 limita cada resposta a 10.000 linhas; BACnet/IP envia uma requisição ReadProperty válida; um overflow u16 na contagem de pontos do DNP3 foi corrigido.

## Documentação e depuração multiprotocolo (P1–P6)

![Arquitetura de depuração multiprotocolo do apidoc-rust](../images/multiprotocol-en.svg)

- **45 anotações de protocolo** mais a forma genérica `#[apidoc::protocol("kind", ...)]`; `event` declara mensagens de fluxo e `param(..., field = "1")` declara números de campo do protocolo.
- **Depuração direta no navegador** (sem dependência Rust): SOAP, GraphQL (incl. subscriptions), JSON-RPC, XML-RPC, OData, MCP, A2A, Webhooks, WebSocket (texto/binário), SSE, Socket.IO, STOMP (incl. binário), WebRTC/DataChannel, HLS.
- **Executores no servidor** (feature-gated): HTTP/2, HTTP/3, gRPC, Thrift, Dubbo/Hessian2, ISO 8583, TCP/UDP/Raw/Unix, MQTT, AMQP, Kafka, XMPP, STOMP/TCP, CoAP, LwM2M, Modbus, DNP3, IEC 104, BACnet/IP, OPC UA, RTP/RTCP/RTSP/SIP/RTMP. `debug-all` ativa todos.
- **P6**: captura de webhooks, exportação AsyncAPI 2.6, importação `.proto`/`.thrift` via CLI `apidoc-idl`, registro Dubbo Nacos, exemplos de mock a partir de eventos.
- **Capacidades de retransmissão UIC-1**: `GET /apidoc/ui/v1/capabilities` informa em `features.debug.relay` apenas os executores compilados neste build. O Rust é unário (`modes` contém `relay-unary`, não `relay-stream`); um build `debug-all` informa os 30 ids de protocolo:

| feature | ids de protocolo relay |
|---|---|
| `debug-http` | `http` `http2` |
| `debug-session` | `websocket` |
| `debug-grpc` | `grpc` |
| `debug-thrift` | `thrift` |
| `debug-dubbo` | `dubbo` `hessian` |
| `debug-iso8583` | `iso8583` |
| `debug-mqtt` | `mqtt` |
| `debug-amqp` | `amqp` |
| `debug-kafka` | `kafka` |
| `debug-xmpp` | `xmpp` |
| `debug-sockets` | `stomp` `tcp` `udp` `raw` `unix` |
| `debug-coap` | `coap` `lwm2m` |
| `debug-industrial` | `modbus` `dnp3` `iec104` `bacnet` `opcua` |
| `debug-media` | `rtp` `rtcp` `rtsp` `sip` `rtmp` |
| `debug-http3` | `http3` |

Exceto `websocket`, os executores de streaming ainda não estão ligados a sessões de retransmissão; Kafka / AMQP / XMPP e similares coletam uma resposta unária. `debug-all` apenas agrega as features acima.
- **Segurança**: a depuração no servidor fica desligada por padrão e exige autenticação global; o guard SSRF resolve e valida cada IP e fixa a conexão; redirecionamentos/proxies e metadados de nuvem são recusados; tamanho/frames/timeout têm limites; respostas DNP3 são limitadas a 10.000 pontos e IEC 104 a 10.000 linhas (erro rígido acima disso), para que quadros pequenos não amplifiquem a saída analisada; protocolos sem TLS fixado falham em fail-closed.
- **Desempenho**: documentação, exportações, codegen e corpos mock são materializados na compilação; as rotas locais do apidoc ficam abaixo de 10 ms p99 (`tests/latency.rs`).

## Arquitetura

![Arquitetura geral do apidoc-rust](images/pt-architecture.svg)

## Funcionalidades

![Recursos do projeto apidoc-rust](images/pt-features.svg)

## Ciclo de vida

![Ciclo de vida da documentação do apidoc-rust](images/pt-lifecycle.svg)

## Estrutura do projeto

```
apidoc-rust/
├── Cargo.toml                 # configuração do workspace (resolver 2)
├── VERSION                    # versão do projeto (v2.2.0)
├── crates/
│   ├── apidoc/                # núcleo de execução (agnóstico a framework)
│   │   ├── src/lib.rs         # modelo de dados + agregação DocRegistry + api.json + UI_HTML + UiMode
│   │   ├── src/auth.rs        # autenticação por senha M6a (emissão/verificação de token HMAC + guarda + backoff)
│   │   ├── src/uiapi.rs       # contrato UIC-1: capabilities / menus / detail / docs / rotas de gestão
│   │   ├── src/protocol_defs/ # nove formatos de definição externos (OpenAPI/AsyncAPI/GraphQL/proto/…)
│   │   ├── src/export/        # exportações: markdown / typescript / swagger / asyncapi + codegen
│   │   ├── src/debug/         # executores de servidor com feature-gate (http / grpc / sockets / mqtt / industrial / media / …)
│   │   ├── src/axum.rs        # adaptador axum (docs/debug/stream/webhook/UIC-1/UI estática + CORS)
│   │   ├── src/actix.rs       # adaptador actix-web (feature `actix`, 1:1 com axum)
│   │   ├── src/ui.html        # UI nativa embutida; o build estático Vue é servido via ui/ui_path
│   │   ├── tests/             # testes de integração (macros/agregação/serialização/UIC-1/definições/latência)
│   │   └── examples/demo.rs   # exemplo: anotações + saída api.json
│   ├── apidoc-macros/         # proc-macro: 24 anotações base + 45 aliases de protocolo
│   │   └── src/lib.rs         # definições de macros + parsing de argumentos + validação em compilação
│   ├── apidoc-idl/            # CLI autônoma `.proto` / `.thrift` → anotação apidoc
│   └── apidoc-test-fixtures/  # fixtures de teste de registro entre crates
├── contract/                  # UIC-1 Contract 1.0: schema + fixtures entre backends
├── .github/
│   └── workflows/release.yml  # workflow de release (lê VERSION, cria tag+release incrementalmente)
└── docs/
    ├── images/                # recursos de arquitetura/funcionalidades/ciclo de vida/multiprotocolo/prévia social
    └── i18n/                  # documentação multilíngue (12 idiomas)
```

## Como usar

### 1. Adicionar dependências

```toml
[dependencies]
apidoc-rust = "2.2"        # ou path = "crates/apidoc"

serde_json = "1"      # usado para gerar o api.json
linkme = "0.3"        # obrigatório: o código de registro gerado pela macro o referencia
```

> Adaptador conforme o framework Web: `features = ["axum"]` para axum, `features = ["actix"]` para actix-web (ambos com funcionalidade 1:1). `mock` (mecanismo Mock) é dependência interna do framework, importada automaticamente pelo adaptador; normalmente o consumidor não precisa usá-lo diretamente.

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
    let doc = DocRegistry::collect_doc(ApidocConfig {
        title: "Minha API".to_string(),
        description: None,
        auth: None,    // M6a autenticação por senha, veja «8. Autenticação por senha»
        apps: vec![],  // M6b múltiplos aplicativos e versões, veja «9. Múltiplos aplicativos e versões»
    });
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

### 5. Depuração on-line e Mock (M4)

Abra a página de documentação → selecione um endpoint → o painel «Depuração on-line» à direita pré-preenche os parâmetros conforme as regras de Mock → aponte a Base URL para o endereço do serviço de destino (padrão `location.origin`, conexão direta entre domínios) → clique em Enviar e obtenha a resposta real (código de status / tempo / JSON bonito). O painel de depuração oculta por padrão os endpoints `not_debug`, que só aparecem após marcar «Mostrar endpoints not_debug».

**Requisito de CORS**: a depuração on-line conecta o navegador diretamente ao endpoint de destino; o serviço de destino precisa montar o `cors_layer` fornecido pelo adaptador para liberar requisições entre domínios; se o CORS falhar, o painel exibe um aviso amarelo.

Sintaxe das regras Mock (três prioridades):

```rust
#[apidoc::param(name = "email", ty = "string", desc = "邮箱", mock = "fake:email")]  // gerado pela regra fake
#[apidoc::param(name = "status", ty = "string", desc = "状态", mock = "1")]          // mock não vazio emitido como está
#[apidoc::param(name = "name", ty = "string", desc = "用户名")]                       // sem mock: geração automática conforme ty
#[apidoc::returned(
    name = "data",
    ty = "object",
    children = [
        { name = "id", ty = "int", required },       // sem mock → "1"
        { name = "email", ty = "string", mock = "fake:email" },  // aninhamento recursivo de children
    ]
)]
fn create_user() -> String {
    unimplemented!()
}
```

15 regras fake integradas: `name` / `company` / `email` / `phone` / `url` / `ip` / `city` / `country` / `text` / `number` / `int` / `float` / `bool` / `uuid` / `date`; nomes de regra desconhecidos voltam ao valor padrão. Geração automática sem mock: int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`; array fixado em 2 itens.

### 6. Exportação on-line (M5)

Os adaptadores integram rotas de exportação em quatro formatos, prontas ao conectar (formato desconhecido → 400):

```bash
GET /apidoc/export?format=md        # índice por grupos + tabela de parâmetros + blocos de resposta (text/markdown)
GET /apidoc/export?format=ts        # gera os tipos {Name}Params / {Name}Result por namespace de grupo (application/typescript)
GET /apidoc/export?format=swagger   # arquivo descritivo OpenAPI 3.0.0 (application/json)
GET /apidoc/export?format=asyncapi  # AsyncAPI 2.6 event/streaming spec (application/json)
```

- **markdown**: ideal para colar no Wiki do projeto / notas de versão, índice por grupos, cada endpoint com tabela de parâmetros e bloco de resposta;
- **typescript**: o front pode colar diretamente as definições de tipos; endpoints sem grupo caem no namespace `defaultGroup` (`default` é palavra reservada de TS, não pode ser usado como identificador);
- **swagger**: `info.version` vem de `ApidocConfig.version` (a versão documentada da API); se não definida, usa a versão do pacote apidoc-rust (atualmente 2.2.0); os parâmetros de caminho vêm do template de URL, de modo que a especificação pode ser importada diretamente no Swagger UI ou em um gerador de código.
- **asyncapi**: canais/operações/mensagens para WebSocket, SSE, MQTT, Kafka, STOMP e outros endpoints de eventos (AsyncAPI 2.6).

### 7. Adaptador actix-web

Se o framework Web for actix-web, conecte `features = ["actix"]` (funcionalidade 1:1 com o adaptador axum):

```toml
[dependencies]
apidoc-rust = { version = "2.2", features = ["actix"] }
```

```rust
use actix_web::{App, HttpServer};
use apidoc::actix::{apidoc_routes, cors_layer, CorsConfig};
use apidoc::ApidocConfig;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(apidoc_routes(ApidocConfig {
                title: "Minha API".to_string(),
                description: None,
                auth: None,    // M6a autenticação por senha, veja «8. Autenticação por senha»
                apps: vec![],  // M6b múltiplos aplicativos e versões, veja «9. Múltiplos aplicativos e versões»
            }))
            .wrap(cors_layer(CorsConfig::default()))   // libera CORS para a depuração on-line (M4)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
```

Após montar, ficam acessíveis `/apidoc` (UI de documentação), `/apidoc/api.json` (dados), `/apidoc/mock` (Mock) e `/apidoc/export` (exportação). A configuração CORS vazia — ou `["*"]` — libera literalmente `*` (sem cookies); com lista de permissões `allow_origins`, há correspondência exata refletindo o Origin; nenhum dos modos envia cookies.

### 8. Autenticação por senha (M6a)

Com `auth` ativado, a documentação exige senha para acesso (tokens assinados com HMAC-SHA256, comparação em tempo constante no servidor, backoff de login):

> **Mudança incompatível na 2.0**: o endpoint de login passou de `GET /apidoc/auth?password=…` para `POST /apidoc/auth` (corpo JSON), e os tokens mudaram do envelope anterior para tokens assinados com HMAC-SHA256 — **tokens antigos e formas antigas de chamada deixam de funcionar**, os clientes precisam usar POST. Implantações sem um `secret_key` explícito também perdem os tokens emitidos ao reiniciar (é usada uma chave aleatória por processo; não existe padrão público).

```rust
use apidoc::auth::AuthConfig;

let doc = DocRegistry::collect_doc(ApidocConfig {
    title: "Minha API".to_string(),
    description: None,
    auth: Some(AuthConfig {
        enable: true,
        password: "your-password".to_string(),
        secret_key: "your-secret-key".to_string(), // se não definido, usa uma chave aleatória por processo: reiniciar invalida os tokens; defina explicitamente em implantações multi-instância
        expire: 86400,                             // segundos; padrão 86400
    }),
    apps: vec![],
});
```

**Fluxo**:

1. O cliente chama `POST /apidoc/auth` (corpo JSON `{"password": "...", "appKey": "<key>"}`; `appKey` seleciona um aplicativo com senha própria) para obter um token (sucesso retorna `{"token":"..."}`, senha incorreta retorna 401, corpo malformado retorna 400); a senha trafega no corpo — nunca na URL ou nos logs — e nunca é transformada em hash no cliente. Com auth desativado, essa rota retorna 404 e as rotas de dados liberam direto
2. As rotas de dados `GET /apidoc/api.json`, `/apidoc/export` e `/apidoc/mock` exigem `?token=xxx` (e `&appKey=` quando um aplicativo específico é selecionado); token ausente/expirado/incorreto retorna 401, a UI da documentação abre automaticamente a máscara de senha e, após digitar a senha, o frontend a envia diretamente por POST para trocar por um token
3. O token é `base64url(payload).base64url(HMAC-SHA256(payload))` com payload `{"exp": <segundos unix>}`; a chave de assinatura é HMAC-SHA256(`secret_key`, credencial), então alterar a senha invalida imediatamente os tokens antigos. A verificação compara o MAC em tempo constante e depois valida a expiração; os tokens usam apenas caracteres seguros para URL
4. `password` / `secret_key` nunca são serializados — a saída do api.json é idêntica byte a byte à versão sem autenticação; quando um aplicativo define `password` próprio, a senha do aplicativo tem prioridade sobre a global, e uma senha vazia (global ou do aplicativo) sempre falha de forma fechada
5. Logins repetidamente incorretos recebem backoff exponencial (sem penalidade nas 3 primeiras vezes, depois 1s → 2s → … com teto de 60s, zerado em caso de sucesso); respostas de credencial/token levam `Cache-Control: no-store`

### 9. Múltiplos aplicativos e versões (M6b)

Um projeto pode ser dividido em vários aplicativos/versões, cada um com exibição e controle de acesso independentes:

```rust
#[apidoc::title("获取用户信息")]
#[apidoc::app("demo")]   // vincula ao aplicativo de key="demo"; endpoints sem app caem no aplicativo padrão
fn get_user_info() -> String {
    unimplemented!()
}
```

```rust
let doc = DocRegistry::collect_doc(ApidocConfig {
    title: "Minha API".to_string(),
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
            password: None, // senha de acesso independente do aplicativo, prioridade sobre a global, nunca serializada
        },
    ],
});
```

- `AppConfig { key, title, items, password }`: `key` é o identificador único referenciado pela anotação `#[apidoc::app("key")]`; `items` aninha recursivamente subversões/sub aplicativos; `password` é a senha de acesso independente do aplicativo (com senha própria, apenas o token do aplicativo é validado)
- A saída do api.json ganha a árvore `doc.apps` (key / title / items / endpoints); o seletor de aplicativo/versão aparece no topo da UI — ao alternar, os endpoints são renderizados pelo nó escolhido e os dados são recarregados; o token é guardado no localStorage separado por appKey
- Se a anotação `app` referenciar um key não configurado em `apps`, há um aviso no stderr e o endpoint cai no aplicativo padrão; sem anotação `app` ou sem `apps` configurado, a saída é idêntica byte a byte à do M5

### 10. Referência de campos de tabelas de dados · Cache de documentação (v2)

```rust
ApidocConfig {
    // A estrutura da tabela vem da configuração (o lado Rust não se conecta ao banco de dados); as anotações referenciam por key
    tables: vec![TableDef {
        key: "user".into(),
        title: "用户表".into(),
        fields: vec![
            TableField { name: "id".into(), ty: "int".into(), required: true, desc: Some("用户ID".into()), ..Default::default() },
            TableField { name: "name".into(), ty: "string".into(), mock: Some("erik".into()), ..Default::default() },
        ],
    }],
    // Cache de documentação: a saída de /apidoc/mock é memorizada por (url, method) e reconstruída após ttl segundos (0 = permanente)
    cache: Some(CacheConfig { enable: true, ttl: 60 }),
    ..Default::default()   // title / description / auth / apps / codegen configurados como sempre
}
```

No handler usa-se a mesma forma do `ref`: `#[apidoc::table("user")]`. Os campos são achatados no `returned` daquele endpoint; uma key ausente em `tables` gera apenas um aviso, não um erro.

### 11. Links de compartilhamento · Geração de código (v2)

- `GET /apidoc/share?app=api&url=/api/user/info&method=GET&base=https://example.com` → `{"url":"https://example.com/apidoc?app=api&ep=%2Fapi%2Fuser%2Finfo&method=GET"}` (com a autenticação ativada, o link leva `&token=…` no final e abre sem senha)
- `GET /apidoc/generate?template=api.ts` (ou `handler.rs` / `schema.sql`) → `text/plain; charset=utf-8`; template desconhecido → 404, parâmetro ausente → 400
- Template personalizado (o mesmo nome sobrescreve o embutido):

```rust
ApidocConfig {
    codegen: vec![CodegenTemplate {
        name: "api.ts".into(),
        template: "// {{title}}\n{{#each endpoints}}// {{method}} {{url}}\n{{/each}}".into(),
    }],
    ..Default::default()
}
```

Variáveis disponíveis nos templates: no nível superior `title` / `description`; dentro de `{{#each endpoints}}`, `title` / `url` / `method` / `group` / `desc` / `author`; dentro de `{{#each tables}}`, `key` / `title`, e dentro do seu `{{#each fields}}`, `name` / `ty` / `required` / `default` / `desc` / `mock`; há ainda variáveis derivadas `not_null` / `default_clause` / `comma` / `key_q` / `name_q` (para gerar SQL válido diretamente), além de `url_js` / `method_js` dentro de `{{#each endpoints}}` (literais de string JSON) e `fn_name` (um `{method}_{group}` saneado como identificador para o esqueleto em Rust).

### 12. Scripts de pré/pós-depuração (v2)

Na página de documentação, o painel «Depuração on-line» expande «Script prévio» e «Script posterior» (conteúdo guardado no localStorage):

```js
// Prévio: executado antes do envio da requisição; pode alterar ctx.url / method / headers / body, ou devolver um objeto para mesclagem superficial
ctx.headers['X-Token'] = localStorage.getItem('token') || '';
```

```js
// Posterior: executado quando a resposta chega; ctx = { status, ms, text, url, method, ep } (somente leitura)
// Se devolver uma string, ela substitui o texto exibido na área de resultados
return 'HTTP ' + ctx.status + ' · ' + ctx.ms + 'ms\n' + ctx.text;
```

Erro de script é apenas avisado na área de resultados de depuração e não interrompe a requisição (se o prévio falhar, a requisição é enviada mesmo assim; se o posterior falhar, a resposta original é exibida como está).

### 13. UI: nativa / build estático Vue (UIC-1 Phase 5)

Por padrão, `/apidoc` serve a UI nativa embutida (`UiMode::Native`), então implantações existentes não precisam de mudanças. O build Vue consome o contrato UIC-1 da seção 14 e é compartilhado com PHP e Go (apidoc-ui `npm run package:ui`):

```rust
use apidoc::{ApidocConfig, UiMode};

let cfg = ApidocConfig {
    title: "Minha API".into(),
    ui: UiMode::Vue,
    ui_path: Some("/srv/apidoc".into()), // conteúdo de dist/uic1-ui/: index.html + config.js + assets/
    ..Default::default()
};
```

- Pacotes de release usam Vue por padrão: `ApidocConfig::release("Minha API", "/srv/apidoc")` equivale à configuração explícita acima; **o padrão da biblioteca continua sendo `native`**.
- No modo Vue, `GET /apidoc` retorna um redirecionamento 307 para `/apidoc/` (o build usa URLs relativas `./assets/...`), `/apidoc/`, `config.js` e `assets/*` são lidos de `ui_path`, e caminhos desconhecidos sem extensão caem em `index.html` (fallback SPA).
- As rotas de API mantêm prioridade: `/apidoc/api.json`, `/apidoc/export`, `/apidoc/ui/v1/*`, auth, debug e webhook continuam casando antes dos arquivos estáticos; subcaminhos de API desconhecidos retornam 404 em vez da shell SPA.
- Segurança: os caminhos ficam confinados a `ui_path`; `..`, bypasses de codificação e escapes por links simbólicos retornam 404, diretórios nunca são listados, e um `ui_path` / `index.html` ausente falha na inicialização (fail closed). `ui` / `ui_path` são apenas do servidor e nunca são serializados no api.json.
- Rollback: volte para `ui: UiMode::Native` e reinicie; a UI nativa e todas as rotas antigas permanecem iguais.

### 14. Contrato UIC-1 · definições de protocolos externos

`/apidoc/ui/v1/*` é o contrato estável consumido pelo apidoc-ui (Contract 1.0); respostas bem-sucedidas são `{code:0,message:"ok",data}`:

| Rota | Finalidade |
|---|---|
| `GET /apidoc/ui/v1/capabilities` | Capacidades do backend: contract / backend / version / features (incluindo a lista de protocolos relay compilados) |
| `GET /apidoc/ui/v1/config` | Configuração de inicialização da UI (título, árvore de aplicativos, chaves de auth, …) |
| `POST /apidoc/ui/v1/menus` · `detail` | Catálogo e detalhe de endpoints, incluindo `definitions` e `validation` |
| `GET/POST /apidoc/ui/v1/docMenus` · `POST docDetail` | Catálogo e corpo da documentação Markdown |
| `POST /apidoc/ui/v1/verifyAuth` | Verificar tokens global / aplicativo / compartilhamento |
| `POST /apidoc/ui/v1/debugSend` · `debugStart` · `debugPoll` · `debugStop` | Depuração direta pelo navegador e por retransmissão no servidor |
| `POST /apidoc/ui/v1/addApiShare` · `getApiShareList` · `getApiShareDetail` · `deleteApiShare` · `handleApiShareAction` | CRUD de registros de compartilhamento (Rust `share.actions=false`; sem ações declarativas) |
| `POST /apidoc/ui/v1/cache/clear` · `cache/warm` | Gerenciamento de cache (Rust `cache.mode=build`) |
| `POST /apidoc/ui/v1/renderCodeTemplate` · `exportAllData` | Renderização de modelos de código e exportação em massa (a UI gera md / pdf / openapi / postman / asyncapi / typescript) |

Anexe uma definição externa a um handler com `#[apidoc::definition]`:

```rust
#[apidoc::title("Obter usuário")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
#[apidoc::definition(file = "api/openapi.yaml", operation = "getUser")]
fn get_user_info() -> String { unimplemented!() }
```

- **Nove formatos**: OpenAPI, AsyncAPI, GraphQL, Proto, WSDL, EDMX, JSON Schema, JSON-RPC e XML-RPC. Quando `format` é omitido, ele é detectado pela extensão/conteúdo, ou pode ser definido explicitamente.
- **Argumentos opcionais**: `operation` (selecionar uma operação), `message` / `direction` (selecionar uma mensagem), `sample` / `sample_file` / `sample_encoding` (amostra de validação) e `strict` (padrão `true`: construções não suportadas são erros).
- **Saída**: um payload uniforme `{format, source, operation, meta, messages, validation}`; os diagnósticos carregam `path / line / column / severity / message`. Definições aparecem apenas no detalhe UIC-1 e nunca no api.json.
- **Limites**: os caminhos são relativos ao `CARGO_MANIFEST_DIR` do crate declarante, e o realpath de um link simbólico deve permanecer dentro da raiz do crate; cada arquivo é limitado a 2 MiB / 50.000 linhas, com no máximo 64 imports e profundidade de importação 32.

## Roteiro de desenvolvimento

| Fase | Conteúdo | Status |
|------|----------|--------|
| M1 | esqueleto do workspace + modelo de dados + MVP de macros + registro linkme | ✅ Concluído |
| M2 | adaptador axum + UI de documentação embutida + diretório agrupado | ✅ Concluído |
| M3 | macros de atributo complementares (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | ✅ Concluído |
| M4 | depuração on-line + mecanismo Mock | ✅ Concluído |
| M5 | exportação markdown / typescript / swagger.json (OpenAPI3) | ✅ Concluído |
| —  | adaptador actix-web (funcionalidade 1:1 com axum) | ✅ Concluído |
| M6a | autenticação por senha (token assinado com HMAC-SHA256 + máscara de senha, senha do aplicativo com prioridade) | ✅ Concluído |
| M6b | múltiplos aplicativos e versões (árvore de configuração apps + anotação app + seletor na UI) | ✅ Concluído |
| v2 | referência de campos de tabelas de dados + cache de documentação + links de compartilhamento + gerador de código + eventos de depuração | ✅ Concluído |
| UIC-1 Phase 0–2 | Contract 1.0: capabilities / menus / detail / docs / debug / share / cache / generator / export, alinhados semanticamente entre backends | ✅ Concluído |
| UIC-1 Phase 3–4 | relatório completo de capacidades de relay + nove analisadores de definições externas ligados ao detalhe UIC | ✅ Concluído |
| UIC-1 Phase 5 | UI estática Vue (`ui` / `ui_path`; pacotes de release com Vue por padrão, UI nativa mantida) | ✅ Concluído |

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

| WeChat Pay (微信支付) | Alipay (支付宝) |
|---|---|
| ![WeChat Pay (微信支付)](../weixinpay.png) | ![Alipay (支付宝)](../alipay.png) |

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
