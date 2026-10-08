![Mascota de Apidoc](../images/apidoc-pet.svg)

# Apidoc (apidoc-rust)

Herramienta de generación de documentación de API y desarrollo de interfaces basada en macros de procedimiento (proc-macro) de Rust, compatible con axum, actix-web y otros frameworks populares

[![License](https://img.shields.io/badge/license-MIT-green)](https://github.com/erikwang2013/apidoc-rust)
[![Stars](https://img.shields.io/github/stars/erikwang2013/apidoc-rust)](https://github.com/erikwang2013/apidoc-rust)

**[中文](../../README.md)** ·
[English](README-en.md) ·
[한국어](README-ko.md) ·
[Русский](README-ru.md) ·
[Deutsch](README-de.md) ·
[Français](README-fr.md) ·
**Español** ·
[Português](README-pt.md) ·
[हिन्दी](README-hi.md) ·
[العربية](README-ar.md) ·
[বাংলা](README-bn.md) ·
[Bahasa Indonesia](README-id.md) ·
[日本語](README-ja.md)

## 📖Introducción

Apidoc es una librería de plugins de Rust que genera automáticamente documentación de interfaces de API analizando **macros de procedimiento (proc-macro) de Rust**, y es compatible con frameworks populares como axum y actix-web. Además de la generación automática de documentación, integra depuración de interfaces en línea, datos de prueba Mock, generación de código Json/TypeScript, generador de interfaces, generador de código y otras capacidades, cubriendo todo el flujo de desarrollo, depuración y entrega de interfaces, con el objetivo de mejorar la eficiencia del desarrollo de API.

> **Origen del proyecto**: este proyecto toma como referencia [apidoc-php](https://github.com/erikwang2013/apidoc-php) (una extensión de composer que genera documentación de API con atributos PHP 8), llevando la capacidad de «las anotaciones como documentación» al estilo nativo de Rust, con mantenimiento y ampliación continuos por parte de erikwang2013.

Enfoque de implementación de apidoc-rust:

- **Generación en tiempo de compilación**: la documentación se genera mediante macros de procedimiento en tiempo de compilación; la documentación nunca se desincroniza del código;
- **Recolección de costo cero**: registro estático con linkme; una sola agregación en tiempo de ejecución obtiene toda la documentación de las interfaces;
- **Plugins universales**: el núcleo es independiente del framework HTTP; se conecta a cualquier framework mediante adaptadores finos (axum / actix-web).

### ✨Descripción del proyecto

- **Listo para usar**: sin configuraciones complicadas; tras la instalación, basta con escribir las anotaciones según la documentación para generar automáticamente la documentación de la API.
- **Escritura sencilla**: admite definiciones generales (definitions) y referencias `ref` de campos; con unas pocas anotaciones se completa la definición de todos los campos.
- **Depuración en línea**: depura las interfaces directamente desde la página de documentación, con soporte para parámetros globales, datos Mock y eventos de depuración.
- **Múltiples aplicaciones/versiones**: admite proyectos de aplicación única, múltiples aplicaciones y múltiples versiones; las interfaces se agrupan y alternan por aplicación/versión.
- **Grupos/Tag**: las interfaces admiten agrupación multinivel y marcado con Tag.
- **Documentación Markdown**: la anotación `md` permite montar Markdown como página de documentación.
- **Generación Json/TypeScript**: cada interfaz genera automáticamente ejemplos de petición/respuesta Json y definiciones de tipos TypeScript, listos para usar directamente en el frontend.
- **Generador de código**: con configuración + plantillas se genera el código de negocio y los archivos Api del frontend.
- **Compartir interfaces**: permite generar enlaces para compartir de una aplicación/interfaz concreta y exportar `swagger.json`.
- **Acceso seguro**: admite autorización con contraseña global y contraseñas independientes por aplicación/versión, y permite activar la caché de documentación.

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
- **Motor Mock** (`crates/apidoc/src/mock.rs`, depende del crate fake, 15 reglas: name / company / email / phone / url / ip / city / country / text / number / int / float / bool / uuid / date). Prioridad de reglas: `mock="fake:xxx"` usa la tabla de reglas fake (nombre desconocido → valor por defecto) → el resto de mock no vacíos se devuelven tal cual (p. ej. `mock="1"`, `mock="erik"`) → sin mock, generación automática según `ty` (int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`); los children se anidan recursivamente, los array fijan 2 elementos
- **Interfaz mock**: el adaptador axum añade `GET /apidoc/mock?url=&method=`, coincidencia exacta de url + method, devuelve 404 si no coincide; el panel de depuración oculta por defecto los endpoints `not_debug`, que solo se muestran al marcar «Mostrar interfaces not_debug»
- **Conexión CORS directa**: la depuración en línea conecta el navegador directamente a la interfaz de destino; el `cors_layer` del adaptador permite el paso (proxy inverso del servidor reservado para v2)

### Implementado (M5)

- **Exportación en tres formatos** (`crates/apidoc/src/export/`): markdown / typescript / swagger (OpenAPI 3.0.0), el crate central proporciona `export::markdown::render` / `export::typescript::render` / `export::swagger::render`
- **Rutas de exportación**: los adaptadores añaden `GET /apidoc/export?format=md|ts|swagger`, formato desconocido → 400; Content-Type: `text/markdown` / `application/typescript` / `application/json`
- **markdown**: índice por grupos + tabla de parámetros + bloque de respuesta; **typescript**: genera los tipos `{Name}Params` / `{Name}Result` por namespace de grupo, las interfaces sin grupo caen en `defaultGroup` (`default` es palabra reservada de TS); **swagger**: `info.version` toma la versión del paquete de Cargo
- **Adaptador actix-web** (`crates/apidoc/src/actix.rs`): funcionalidad 1:1 con el adaptador axum — `apidoc_routes(ApidocConfig) -> Scope` monta /apidoc, /apidoc/api.json, /apidoc/mock, /apidoc/export, `cors_layer(CorsConfig)` permite CORS
- **UI compartida**: la UI de documentación (`src/ui.html`) sube al crate central, exportada como `pub const UI_HTML`, ambos adaptadores referencian la misma copia (seguro para el empaquetado de publicación)

### Implementado (M6)

- **Autenticación con contraseña (M6a)**: con `AuthConfig { enable, password, secret_key, expire }` activado, el cliente usa `GET /apidoc/auth?password=<md5(contraseña)>&appKey=<key>` para obtener un token; las rutas de datos `/apidoc/api.json`, `/apidoc/export`, `/apidoc/mock` requieren `?token=xxx`; token ausente/expirado/incorrecto → 401 y la UI de documentación muestra una máscara de contraseña; el token se emite con el cifrado authcode (portado línea a línea del authcode de Discuz: variante RC4 + checksum md5 + base64 sin padding), con payload `{key: md5(md5(contraseña original)), expire: now+expire}` y comparación MAC en tiempo constante
- **Línea roja de seguridad de la autenticación**: `password` / `secret_key` nunca se serializan; la salida api.json es byte a byte idéntica a la de autenticación desactivada; con auth desactivado, `/apidoc/auth` devuelve 404 y las rutas de datos pasan directamente; si una aplicación configura su propio `password`, la contraseña de la aplicación prevalece sobre la global; `secret_key` por defecto `"apidoc#hgcode"` (advertencia stderr una vez si está activado sin configurar) y `expire` por defecto 86400 segundos
- **Múltiples aplicaciones y versiones (M6b)**: `ApidocConfig.apps: Vec<AppConfig>` (`key` / `title` / `items` subversiones recursivas / `password`) configura el árbol de aplicaciones; `#[apidoc::app("key")]` cuelga la interfaz en la aplicación de esa key y las interfaces sin key caen en la aplicación por defecto; la salida api.json añade el árbol `doc.apps`; aparece un selector de aplicación/versión en la parte superior de la UI y los tokens se guardan en localStorage separados por appKey (distintas aplicaciones pueden tener contraseñas independientes)

### Implementado (v2)

- **Referencia de campos de tablas de datos (`table`)**: `#[apidoc::table("user")]` fusiona (aplanándolos) los campos de la tabla configurada por su key en `ApidocConfig.tables` dentro de `returned` (misma semántica que `ref`; la diferencia es que la fuente de datos es una tabla de configuración y no otro endpoint); los campos admiten `required` / `default` / `desc` / `mock`; una key no configurada solo avisa por stderr; sin usar esta anotación, la salida es byte a byte idéntica a la de v1
- **Caché de documentación**: `ApidocConfig.cache = Some(CacheConfig { enable, ttl })` — la salida de `/apidoc/mock` se memoriza en el proceso por `(url, method)` y se reconstruye tras `ttl` segundos; `ttl = 0` significa permanente; si no se configura o no se activa, no se pasa por la ruta de caché (cero cambios en la salida). api.json / export ya se construyen una vez al montar las rutas, lo que equivale a una caché permanente
- **Enlaces para compartir**: `GET /apidoc/share?app=&url=&method=&base=` → enlace profundo `{"url":"…"}` (`?app=<key>&ep=<url de la interfaz>&method=<método>[&token=…]`); en la página de documentación cada interfaz tiene un botón «Compartir» (escribe en el portapapeles; si falla, recurre a un campo de texto copiable); abrir el enlace profundo lleva a esa aplicación/interfaz; con la autenticación activada el enlace incluye el token emitido por el servidor (la contraseña independiente de la aplicación prevalece sobre la global) y se abre sin contraseña
- **Generador de código**: `GET /apidoc/generate?template=<name>` → el texto renderizado (plantilla desconocida → 404); integra `api.ts` (archivo Api del frontend), `handler.rs` (esqueleto de interfaz en Rust) y `schema.sql` (sentencias de creación de tablas generadas a partir de `tables`); una plantilla con el mismo nombre en `ApidocConfig.codegen` sobrescribe la integrada. La sintaxis de plantillas solo tiene dos formas: `{{variable}}` (si no está definida, se conserva tal cual) y `{{#each lista}}…{{/each}}` (anidable, p. ej. `tables` × `fields`)
- **Eventos de depuración**: el panel de depuración en línea añade los bloques plegables «Script previo» / «Script posterior» (persistidos en localStorage) — el previo se ejecuta antes de la petición con `new Function('ctx', code)` y puede modificar `ctx.url/method/headers/body` o devolver un objeto para una fusión superficial; el posterior se ejecuta tras la respuesta y puede leer `status/ms/text/url/method/ep`; si devuelve una cadena, esta reemplaza el texto de respuesta mostrado; un error del script solo se avisa en el área de resultados y no interrumpe la petición

## Arquitectura

![Arquitectura general de apidoc-rust](images/es-architecture.svg)

## Funcionalidades

![Funcionalidades del proyecto apidoc-rust](images/es-features.svg)

## Ciclo de vida

![Ciclo de vida de la documentación de apidoc-rust](images/es-lifecycle.svg)

## Estructura del proyecto

```
apidoc-rust/
├── Cargo.toml                 # configuración del workspace (resolver 2)
├── VERSION                    # versión del proyecto (v1.6.1)
├── crates/
│   ├── apidoc/                # núcleo en tiempo de ejecución (independiente del framework)
│   │   ├── src/lib.rs         # modelo de datos + agregación DocRegistry + api.json + UI_HTML
│   │   ├── src/auth.rs        # autenticación M6a (emisión/validación de token authcode + guardia de rutas)
│   │   ├── src/export/        # exportación M5: markdown / typescript / swagger
│   │   ├── src/ui.html        # UI de documentación compartida (exportada por el crate central, referenciada por ambos adaptadores)
│   │   ├── tests/             # pruebas de integración (expansión de macros / agregación / serialización / entre crates)
│   │   └── examples/demo.rs   # ejemplo: anotaciones + salida api.json
│   ├── apidoc-macros/         # proc-macro: 20 macros de atributo
│   │   └── src/lib.rs         # definición de macros + análisis de parámetros + validación en tiempo de compilación

│   ├── apidoc-test-fixtures/  # accesorios de prueba para registro entre crates

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
apidoc-rust = "1.6"        # o path = "crates/apidoc"

serde_json = "1"      # para generar api.json
```

> Adaptador según el framework Web: `features = ["axum"]` para axum, `features = ["actix"]` para actix-web (ambos con funcionalidad 1:1). `mock` (motor Mock) es dependencia interna del framework, la importa automáticamente el adaptador; el consumidor normalmente no necesita usarlo directamente.

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
    let doc = DocRegistry::collect_doc(ApidocConfig {
        title: "我的 API".to_string(),
        description: None,
        auth: None,    // autenticación M6a, ver «8. Autenticación con contraseña»
        apps: vec![],  // M6b multi-aplicaciones y versiones, ver «9. Multi-aplicaciones y versiones»
    });
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

### 6. Exportación en línea (M5)

Los adaptadores integran rutas de exportación en tres formatos, listas al conectar (formato desconocido → 400):

```bash
GET /apidoc/export?format=md        # índice por grupos + tabla de parámetros + bloques de respuesta (text/markdown)
GET /apidoc/export?format=ts        # genera los tipos {Name}Params / {Name}Result por namespace de grupo (application/typescript)
GET /apidoc/export?format=swagger   # archivo descriptivo OpenAPI 3.0.0 (application/json)
```

- **markdown**: ideal para pegar en el Wiki del proyecto / notas de versión, índice por grupos, cada interfaz con tabla de parámetros y bloque de respuesta;
- **typescript**: el front puede pegar directamente las definiciones de tipos; las interfaces sin grupo caen en el namespace `defaultGroup` (`default` es palabra reservada de TS, no puede usarse como identificador);
- **swagger**: `info.version` toma la versión del paquete de Cargo (actualmente 1.6.1), importable directamente en Swagger UI o en un generador de código.

### 7. Adaptador actix-web

Si el framework Web es actix-web, conecte `features = ["actix"]` (funcionalidad 1:1 con el adaptador axum):

```toml
[dependencies]
apidoc-rust = { version = "1.6", features = ["actix"] }
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
                title: "我的 API".to_string(),
                description: None,
                auth: None,    // autenticación M6a, ver «8. Autenticación con contraseña»
                apps: vec![],  // M6b multi-aplicaciones y versiones, ver «9. Multi-aplicaciones y versiones»
            }))
            .wrap(cors_layer(CorsConfig::default()))   // permite CORS para la depuración en línea (M4)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
```

Una vez montados, son accesibles `/apidoc` (UI de documentación), `/apidoc/api.json` (datos), `/apidoc/mock` (Mock) y `/apidoc/export` (exportación). La configuración CORS vacía permite literalmente `*` (sin cookies); con lista blanca `allow_origins`, hace coincidencia exacta reflejando el Origin; ningún modo envía cookies.

### 8. Autenticación con contraseña (M6a)

Al activar `auth`, la documentación exige contraseña para acceder (alineado con Auth.php de apidoc-php; el token es un portado línea a línea del cifrado authcode de Discuz):

```rust
use apidoc::auth::AuthConfig;

let doc = DocRegistry::collect_doc(ApidocConfig {
    title: "我的 API".to_string(),
    description: None,
    auth: Some(AuthConfig {
        enable: true,
        password: "your-password".to_string(),
        secret_key: "your-secret-key".to_string(), // por defecto "apidoc#hgcode" (advertencia stderr única si activado sin configurar)
        expire: 86400,                             // segundos; por defecto 86400
    }),
    apps: vec![],
});
```

**Flujo**:

1. El cliente llama `GET /apidoc/auth?password=<md5(contraseña)>&appKey=<key>` para obtener el token (éxito → `{"token":"..."}`, contraseña incorrecta → 401); si auth no está activado, esta ruta devuelve 404 y las rutas de datos pasan directamente
2. Las rutas de datos `GET /apidoc/api.json`, `/apidoc/export`, `/apidoc/mock` requieren `?token=xxx` (y `&appKey=` a la vez si se ha elegido una aplicación concreta); token ausente/vencido/incorrecto → 401 y la UI de documentación muestra automáticamente la máscara de contraseña; al introducir la contraseña, el front calcula el md5 localmente y lo envía para obtener el token
3. El payload del token es `{key: md5(md5(contraseña original)), expire: now+expire}`, cifrado por `secret_key` mediante authcode (variante RC4 + checksum md5 + base64 sin padding, comparación MAC en tiempo constante contra canales laterales de tiempo)
4. `password` / `secret_key` nunca se serializan; la salida api.json es byte a byte idéntica a la de autenticación desactivada; si una aplicación configura su propio `password`, la contraseña de la aplicación prevalece sobre la global

### 9. Múltiples aplicaciones y versiones (M6b)

Un proyecto puede dividirse en varias aplicaciones/versiones, cada una con su propia visualización y control de acceso:

```rust
#[apidoc::title("获取用户信息")]
#[apidoc::app("demo")]   // cuelga en la aplicación con key="demo"; las interfaces sin app caen en la aplicación por defecto
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
            password: None, // contraseña de acceso independiente de la aplicación, prevalece sobre la global, nunca se serializa
        },
    ],
});
```

- `AppConfig { key, title, items, password }`: `key` es el identificador único referenciado por la anotación `#[apidoc::app("key")]`; `items` anida recursivamente subversiones/sub-aplicaciones; `password` es la contraseña de acceso independiente de la aplicación (con contraseña independiente solo se valida el token de la aplicación)
- La salida api.json añade el árbol `doc.apps` (key / title / items / endpoints); aparece un selector de aplicación/versión en la parte superior de la UI; al cambiar, las interfaces se renderizan según ese nodo y se recargan los datos; los tokens se guardan en localStorage separados por appKey
- Si la anotación `app` referencia una key no configurada en `apps`, aviso en stderr y cae en la aplicación por defecto; sin anotación `app` ni `apps` configurado, la salida es byte a byte idéntica a M5

### 10. Referencia de campos de tablas de datos · Caché de documentación (v2)

```rust
ApidocConfig {
    // La estructura de la tabla la aporta la configuración (el lado de Rust no se conecta a la base de datos); las anotaciones la referencian por key
    tables: vec![TableDef {
        key: "user".into(),
        title: "用户表".into(),
        fields: vec![
            TableField { name: "id".into(), ty: "int".into(), required: true, desc: Some("用户ID".into()), ..Default::default() },
            TableField { name: "name".into(), ty: "string".into(), mock: Some("erik".into()), ..Default::default() },
        ],
    }],
    // Caché de documentación: la salida de /apidoc/mock se memoriza por (url, method) y se reconstruye tras ttl segundos (0 = permanente)
    cache: Some(CacheConfig { enable: true, ttl: 60 }),
    ..Default::default()   // title / description / auth / apps / codegen se configuran como siempre
}
```

En el handler se usa la misma forma que con `ref`: `#[apidoc::table("user")]`. Los campos se aplanan dentro del `returned` de esa interfaz; una key ausente en `tables` solo genera un aviso, no un error.

### 11. Enlaces para compartir · Generación de código (v2)

- `GET /apidoc/share?app=api&url=/api/user/info&method=GET&base=https://example.com` → `{"url":"https://example.com/apidoc?app=api&ep=%2Fapi%2Fuser%2Finfo&method=GET"}` (con la autenticación activada, el enlace lleva `&token=…` al final y se abre sin contraseña)
- `GET /apidoc/generate?template=api.ts` (o `handler.rs` / `schema.sql`) → `text/plain; charset=utf-8`; plantilla desconocida → 404, parámetro faltante → 400
- Plantilla personalizada (el mismo nombre sobrescribe la integrada):

```rust
ApidocConfig {
    codegen: vec![CodegenTemplate {
        name: "api.ts".into(),
        template: "// {{title}}\n{{#each endpoints}}// {{method}} {{url}}\n{{/each}}".into(),
    }],
    ..Default::default()
}
```

Variables disponibles en las plantillas: en el nivel superior `title` / `description`; dentro de `{{#each endpoints}}`, `title` / `url` / `method` / `group` / `desc` / `author`; dentro de `{{#each tables}}`, `key` / `title`, y dentro de su `{{#each fields}}`, `name` / `ty` / `required` / `default` / `desc` / `mock`; además hay variables derivadas `not_null` / `default_clause` / `comma` (para generar SQL válido directamente).

### 12. Scripts previo/posterior de depuración (v2)

En la página de documentación, el panel «Depuración en línea» despliega «Script previo» y «Script posterior» (el contenido se guarda en localStorage):

```js
// Previo: se ejecuta antes de enviar la petición; puede modificar ctx.url / method / headers / body, o devolver un objeto para fusión superficial
ctx.headers['X-Token'] = localStorage.getItem('token') || '';
```

```js
// Posterior: se ejecuta al volver la respuesta; ctx = { status, ms, text, url, method, ep } (solo lectura)
// Si devuelve una cadena, reemplaza el texto mostrado en el área de resultados
return 'HTTP ' + ctx.status + ' · ' + ctx.ms + 'ms\n' + ctx.text;
```

Un error de script solo se avisa en el área de resultados de depuración y no interrumpe la petición (si falla el previo, la petición se envía igualmente; si falla el posterior, se muestra la respuesta original tal cual).

## Plan de desarrollo

| Fase | Contenido | Estado |
|------|-----------|--------|
| M1 | esqueleto del workspace + modelo de datos + MVP de macros + registro linkme | ✅ Completado |
| M2 | adaptador axum + UI de documentación integrada + directorio por grupos | ✅ Completado |
| M3 | completar anotaciones (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | ✅ Completado |
| M4 | depuración en línea + motor Mock | ✅ Completado |
| M5 | exportar markdown / typescript / swagger.json (OpenAPI3) | ✅ Completado |
| —  | adaptador actix-web (funcionalidad 1:1 con axum) | ✅ Completado |
| M6a | Autenticación con contraseña (token authcode + máscara de contraseña, contraseña de la aplicación prevalece) | ✅ Completado |
| M6b | Múltiples aplicaciones y versiones (árbol de configuración apps + anotación app + selector de UI) | ✅ Completado |
| v2 | referencia de campos de tablas de datos + caché de documentación + enlaces para compartir + generador de código + eventos de depuración | ✅ Completado |

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

| 微信支付 | 支付宝 |
|---|---|
| ![微信支付](../weixinpay.png) | ![支付宝](../alipay.png) |

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
