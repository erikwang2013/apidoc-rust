<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
 Librería de plugins universal para generar documentación de interfaces de API mediante macros de procedimiento (proc-macro) de Rust
</div>

<div align="center">
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/badge/license-MIT-green"></a>
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/github/stars/erikwang2013/apidoc-rust"></a>
</div>

<div align="center">
<a href="../../README.md"><strong>中文</strong></a> ·
<a href="README-en.md">English</a> ·
<a href="README-ko.md">한국어</a> ·
<a href="README-ru.md">Русский</a> ·
<a href="README-de.md">Deutsch</a> ·
<a href="README-fr.md">Français</a> ·
<strong>Español</strong> ·
<a href="README-pt.md">Português</a> ·
<a href="README-hi.md">हिन्दी</a> ·
<a href="README-ar.md">العربية</a> ·
<a href="README-bn.md">বাংলা</a> ·
<a href="README-id.md">Bahasa Indonesia</a> ·
<a href="README-ja.md">日本語</a>
</div>

## Introducción

apidoc-rust es un **generador de documentación de API universal y basado en plugins** implementado en Rust, inspirado en [apidoc-php](https://github.com/erikwang2013/apidoc-php) (una extensión de composer que genera documentación de API con atributos PHP 8). Lleva la capacidad de "las anotaciones como documentación" al estilo nativo de Rust:

- **Generación en tiempo de compilación**: la documentación se genera mediante macros de procedimiento en tiempo de compilación; la documentación nunca se desincroniza del código;
- **Recolección de costo cero**: registro estático con linkme; una sola agregación en tiempo de ejecución obtiene toda la documentación de las interfaces;
- **Plugins universales**: el núcleo es independiente del framework HTTP; se conecta a cualquier framework mediante adaptadores finos (axum / actix-web).

## Características

### Implementado (M1-M3)

- **Documentación por anotaciones**: siete macros de atributo — `title` / `desc` / `method` / `url` / `param` / `query` / `returned` —, una anotación por entrada (equivalente a la sintaxis de PHP attributes); los parámetros admiten anidación `required` / `default` / `desc` / `mock` / `children`
- **Validación en tiempo de compilación**: la url debe comenzar con `/`, lista blanca de method, param name obligatorio, etc.; las anotaciones inválidas fallan en tiempo de compilación (span preciso)
- **Recolección automática**: registro estático con linkme `distributed_slice`, sin listado manual de interfaces; `DocRegistry::collect()` fusiona por id y restaura el orden de declaración por seq, con recolección automática entre crates
- **Salida api.json**: serde serializa un modelo de datos de documentación unificado (config + endpoints); los campos se alinean con la semántica de PHP
- **Adaptador axum + UI de documentación integrada**: montar la ruta y ya está la página de documentación; navegación por directorios agrupados (M2)
- **Ampliación de anotaciones**: 12 anotaciones nuevas — `tag` / `group` / `author` / `header` / `route_param` / `response_status` / `success` / `error` / `not_debug` / `md` / `sort` / `ref` (M3)

### Implementado (M4)

- **Depuración en línea**: la página de documentación incluye el panel «Depuración en línea» — Base URL prellenada con `location.origin` para conexión directa entre dominios al servicio de destino, parámetros del formulario prellenados con mock, sustitución de marcadores de ruta `{name}` / `:name`, parámetros GET/HEAD incorporados a la query string, cuerpo JSON ensamblado para el resto de métodos, edición de cabeceras de petición + cabeceras personalizadas, visualización de la respuesta (estado / tiempo / JSON bonito), aviso amarillo si falla CORS
- **Motor Mock** (`crates/apidoc-mock`, depende del crate fake, 15 reglas: name / company / email / phone / url / ip / city / country / text / number / int / float / bool / uuid / date). Prioridad de reglas: `mock="fake:xxx"` usa la tabla de reglas fake (nombre desconocido → valor por defecto) → el resto de mock no vacíos se devuelven tal cual (p. ej. `mock="1"`, `mock="erik"`) → sin mock, generación automática según `ty` (int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`); los children se anidan recursivamente, los array fijan 2 elementos
- **Interfaz mock**: el adaptador axum añade `GET /apidoc/mock?url=&method=`, coincidencia exacta de url + method, devuelve 404 si no coincide; el panel de depuración oculta por defecto los endpoints `not_debug`, que solo se muestran al marcar «Mostrar interfaces not_debug»
- **Conexión CORS directa**: la depuración en línea conecta el navegador directamente a la interfaz de destino; el `cors_layer` del adaptador permite el paso (proxy inverso del servidor reservado para v2)

### Planificado

- Múltiples aplicaciones / múltiples versiones / contraseña de acceso
- Exportación a Markdown / TypeScript / Swagger (OpenAPI3) (M5)
- Adaptación a múltiples frameworks (apidoc-axum terminado, apidoc-actix pendiente)
- v2: generador de código, referencias a campos de tablas de datos, enlaces para compartir, eventos de depuración

## Arquitectura

<img src="images/es-architecture.svg" alt="Arquitectura general de apidoc-rust" width="100%">

## Funcionalidades

<img src="images/es-features.svg" alt="Funcionalidades del proyecto apidoc-rust" width="100%">

## Ciclo de vida

<img src="images/es-lifecycle.svg" alt="Ciclo de vida de la documentación de apidoc-rust" width="100%">

## Estructura del proyecto

```
apidoc-rust/
├── Cargo.toml                 # configuración del workspace (resolver 2)
├── VERSION                    # versión del proyecto (v1.0.0, separada de la versión del framework 0.1.0)
├── crates/
│   ├── apidoc/                # núcleo en tiempo de ejecución (independiente del framework)
│   │   ├── src/lib.rs         # modelo de datos + agregación DocRegistry + api.json
│   │   ├── tests/             # pruebas de integración (expansión de macros / agregación / serialización / entre crates)
│   │   └── examples/demo.rs   # ejemplo: anotaciones + salida api.json
│   ├── apidoc-macros/         # proc-macro: 19 macros de atributo
│   │   └── src/lib.rs         # definición de macros + análisis de parámetros + validación en tiempo de compilación
│   ├── apidoc-mock/           # motor Mock (generación de datos Mock por reglas fake)
│   ├── apidoc-test-fixtures/  # accesorios de prueba para registro entre crates
│   └── apidoc-axum/           # adaptador axum (rutas de documentación + cors_layer + /apidoc/mock)
├── .github/
│   └── workflows/release.yml  # workflow de publicación (lee VERSION, crea tag + release de forma incremental)
└── docs/
    ├── images/                # diagramas de arquitectura / funcionalidades / ciclo de vida (SVG)
    └── i18n/                  # documentación multilingüe (12 idiomas)
```

## Instrucciones de uso

### 1. Agregar dependencias

```toml
[dependencies]
apidoc = "0.1"        # o path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # la expansión de macros referencia la ruta de linkme directamente; el consumidor debe depender de ella
serde_json = "1"      # para generar api.json
```

### 2. Escribir anotaciones

Cuelga anotaciones en las funciones handler; la documentación se genera en tiempo de compilación:

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

### 3. Recolección y salida

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

### 4. Ejecutar el ejemplo

```bash
cargo run --example demo -p apidoc
```

Salida (extracto):

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

### 5. Depuración en línea y Mock (M4)

Abra la página de documentación → seleccione una interfaz → el panel «Depuración en línea» de la derecha prellena los parámetros según las reglas Mock → apunte la Base URL a la dirección del servicio de destino (por defecto `location.origin`, conexión directa entre dominios) → haga clic en Enviar y obtendrá la respuesta real (código de estado / tiempo / JSON bonito). El panel de depuración oculta por defecto los endpoints `not_debug`, que solo se muestran tras marcar «Mostrar interfaces not_debug».

**Requisito CORS**: la depuración en línea conecta el navegador directamente a la interfaz de destino; el servicio de destino debe montar el `cors_layer` proporcionado por el adaptador para permitir peticiones entre dominios; si CORS falla, el panel muestra un aviso amarillo.

Sintaxis de las reglas Mock (tres prioridades):

```rust
#[apidoc::param(name = "email", ty = "string", desc = "邮箱", mock = "fake:email")]  // generado por la regla fake
#[apidoc::param(name = "status", ty = "string", desc = "状态", mock = "1")]          // mock no vacío devuelto tal cual
#[apidoc::param(name = "name", ty = "string", desc = "用户名")]                       // sin mock: generación automática según ty
#[apidoc::returned(
    name = "data",
    ty = "object",
    children = [
        { name = "id", ty = "int", required },       // sin mock → "1"
        { name = "email", ty = "string", mock = "fake:email" },  // anidación recursiva de children
    ]
)]
fn create_user() -> String {
    unimplemented!()
}
```

15 reglas fake integradas: `name` / `company` / `email` / `phone` / `url` / `ip` / `city` / `country` / `text` / `number` / `int` / `float` / `bool` / `uuid` / `date`; los nombres de regla desconocidos vuelven al valor por defecto. Generación automática sin mock: int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`; los array fijan 2 elementos.

## Plan de desarrollo

| Fase | Contenido | Estado |
|------|-----------|--------|
| M1 | esqueleto del workspace + modelo de datos + MVP de macros + registro linkme | ✅ Completado |
| M2 | adaptador axum + UI de documentación integrada + directorio por grupos | ✅ Completado |
| M3 | completar anotaciones (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | ✅ Completado |
| M4 | depuración en línea + motor Mock | ✅ Completado |
| M5 | exportar markdown / typescript / swagger.json | Planificado |
| M6 | autenticación con contraseña, múltiples aplicaciones y versiones, publicación | Planificado |

## Documentación multilingüe

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

## Apoyo y donaciones

Si este proyecto le resulta útil, ¡dé una ⭐ Star para apoyarnos! También puede hacer una donación para apoyar el código abierto.

### 微信支付 (WeChat Pay) / 支付宝 (Alipay)

<table>
  <tr>
    <td align="center">
      <img src="../weixinpay.png" width="130" height="130" alt="微信支付" /><br/>
      <strong>微信支付</strong>
    </td>
    <td align="center">
      <img src="../alipay.png" width="130" height="130" alt="支付宝" /><br/>
      <strong>支付宝</strong>
    </td>
  </tr>
</table>

### Donaciones por transferencia internacional

**【Información del beneficiario】**

- Nombre del beneficiario: WANG KEXUN
- Número de cuenta del beneficiario: 881015918251

**【Banco del beneficiario】**

- ZA Bank SWIFT Code：AABLHKHHXXX
- Nombre del banco: ZA Bank Limited
- Código bancario: 387
- Dirección del banco: Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【Banco intermediario para transferencias transfronterizas (si es necesario)】**

> Tenga en cuenta que esta es la información del banco intermediario (banco corresponsal) para transferencias transfronterizas, no la del banco del beneficiario. Consulte con su banco si es necesario proporcionar la información del banco intermediario.

- **El banco intermediario para transferencias en dólares de Hong Kong (HKD), yuanes (CNY) y dólares estadounidenses (USD) es Citibank:**
  - Nombre del banco: Citibank N.A. Hong Kong
  - SWIFT Code：CITIHKHXXXX
  - Código bancario: 006
  - Nombre de la sucursal: Hong Kong Branch
  - Código de sucursal: 391
  - Dirección del banco: Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **El banco intermediario para otras divisas es BNY Mellon:**
  - Nombre del banco: THE BANK OF NEW YORK MELLON
  - SWIFT Code：IRVTUS3NXXX
  - Dirección del banco: THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## License

[MIT](../../LICENSE)
