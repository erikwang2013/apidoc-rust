<h1 align="center">Apidoc (apidoc-rust)</h1>

<div align="center">
  Универсальная плагинная библиотека для генерации документации API на основе процедурных макросов Rust (proc-macro)
</div>

<div align="center">
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/badge/license-MIT-green"></a>
<a href="https://github.com/erikwang2013/apidoc-rust"><img src="https://img.shields.io/github/stars/erikwang2013/apidoc-rust"></a>
</div>

<div align="center">
<a href="../../README.md">中文</a> ·
<a href="README-en.md">English</a> ·
<a href="README-ko.md">한국어</a> ·
<a href="README-ru.md"><strong>Русский</strong></a> ·
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

## О проекте

apidoc-rust — это **универсальный плагинный генератор документации API**, реализованный на Rust, по образцу [apidoc-php](https://github.com/erikwang2013/apidoc-php) (composer-расширение, генерирующее документацию API на основе PHP 8 attributes). Он воплощает принцип «аннотации как документация» нативным для Rust способом:

- **Генерация на этапе компиляции**: документация создаётся процедурными макросами во время компиляции — документация никогда не расходится с кодом;
- **Сбор с нулевой стоимостью**: статическая регистрация через linkme — один проход агрегации в рантайме даёт всю документацию API;
- **Универсальные плагины**: ядро не зависит от HTTP-фреймворка; любой фреймворк подключается через тонкие адаптеры (axum / actix-web).

## Возможности

### Реализовано (M1-M3)

- **Аннотации как документация**: семь атрибутных макросов `title` / `desc` / `method` / `url` / `param` / `query` / `returned`, по одной аннотации на элемент (аналог PHP attributes); параметры поддерживают вложенность `required` / `default` / `desc` / `mock` / `children`
- **Проверка на этапе компиляции**: url обязан начинаться с `/`, method — по белому списку, param name обязателен и т.д.; недопустимые аннотации вызывают ошибку компиляции (с точным span)
- **Автоматический сбор**: статическая регистрация linkme `distributed_slice` — ручной список интерфейсов не нужен; `DocRegistry::collect()` объединяет фрагменты по id и восстанавливает порядок объявления по seq, с автоматическим сбором между crates
- **Вывод api.json**: serde сериализует единую модель данных документации (config + endpoints), поля выровнены по семантике PHP
- **Адаптер axum + встроенный UI документации**: монтируете маршруты — получаете страницу документации с группировкой по разделам (M2)
- **Дополнение аннотаций**: 12 новых аннотаций `tag` / `group` / `author` / `header` / `route_param` / `response_status` / `success` / `error` / `not_debug` / `md` / `sort` / `ref` (M3)

### Реализовано (M4)

- **Онлайн-отладка**: на странице документации встроена панель «Онлайн-отладка» — Base URL предзаполняется из `location.origin` для кросс-доменного прямого подключения к целевому сервису, форма параметров предзаполняется по mock, подстановка плейсхолдеров `{name}` / `:name` в маршруте, параметры GET/HEAD уходят в query, остальные method собираются в JSON body, редактирование заголовков запроса + свои заголовки, показ ответа (статус / время / pretty JSON), жёлтая подсказка при сбое CORS
- **Движок Mock** (`crates/apidoc-mock`, зависит от crate fake, 15 правил: name / company / email / phone / url / ip / city / country / text / number / int / float / bool / uuid / date). Приоритет правил: `mock="fake:xxx"` — через таблицу правил fake (неизвестные имена откатываются к значениям по умолчанию) → остальные непустые mock выводятся как есть (например, `mock="1"`, `mock="erik"`) → без mock автоматически по `ty` (int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`); children рекурсивно вкладываются, array всегда из 2 элементов
- **Mock-интерфейс**: в адаптере axum добавлен `GET /apidoc/mock?url=&method=` — точное совпадение url + method, иначе 404; панель отладки по умолчанию скрывает эндпоинты `not_debug`, они появляются после установки флажка «Показать интерфейсы not_debug»
- **Прямое подключение через CORS**: онлайн-отладка выполняется браузером напрямую к целевому API, `cors_layer` адаптера отвечает за разрешение (серверный обратный прокси — в v2)

### В планах

- Несколько приложений / несколько версий / пароль доступа
- Экспорт в Markdown / TypeScript / Swagger (OpenAPI3) (M5)
- Адаптеры для разных фреймворков (apidoc-axum готов, apidoc-actix не сделан)
- v2: генератор кода, ссылки на поля таблиц БД, ссылки для совместного доступа, события отладки

## Архитектура

<img src="images/ru-architecture.svg" alt="Общая архитектура apidoc-rust" width="100%">

## Возможности

<img src="images/ru-features.svg" alt="Возможности проекта apidoc-rust" width="100%">

## Жизненный цикл

<img src="images/ru-lifecycle.svg" alt="Жизненный цикл документации apidoc-rust" width="100%">

## Структура проекта

```
apidoc-rust/
├── Cargo.toml                 # конфигурация workspace (resolver 2)
├── VERSION                    # версия проекта (v1.0.0, отделена от версии фреймворка 0.1.0)
├── crates/
│   ├── apidoc/                # ядро рантайма (не зависит от фреймворка)
│   │   ├── src/lib.rs         # модель данных + агрегация DocRegistry + api.json
│   │   ├── tests/             # интеграционные тесты (раскрытие макросов/агрегация/сериализация/между crates)
│   │   └── examples/demo.rs   # пример: аннотации + вывод api.json
│   ├── apidoc-macros/         # proc-macro: 19 атрибутных макросов
│   │   └── src/lib.rs         # определения макросов + разбор параметров + проверка на этапе компиляции
│   ├── apidoc-mock/           # движок Mock (генерация mock-данных по правилам fake)
│   ├── apidoc-test-fixtures/  # тестовые фикстуры для регистрации между crates
│   └── apidoc-axum/           # адаптер axum (маршруты документации + cors_layer + /apidoc/mock)
├── .github/
│   └── workflows/release.yml  # рабочий процесс релиза (читает VERSION, инкрементально создаёт tag+release)
└── docs/
    ├── images/                # схемы: архитектура/возможности/жизненный цикл (SVG)
    └── i18n/                  # многоязычная документация (12 языков)
```

## Использование

### 1. Добавление зависимостей

```toml
[dependencies]
apidoc = "0.1"        # или path = "crates/apidoc"
apidoc-macros = "0.1"
linkme = "0.3"        # раскрытие макросов напрямую ссылается на linkme — потребителю нужна прямая зависимость
serde_json = "1"      # для вывода api.json
```

> `apidoc-mock` (движок Mock) — внутренняя зависимость фреймворка, подключается адаптером автоматически; обычно потребителю не нужно использовать её напрямую.

### 2. Написание аннотаций

Аннотации навешиваются на функции-handler по одной — документация генерируется на этапе компиляции:

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

### 3. Сбор и вывод

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

### 4. Запуск примера

```bash
cargo run --example demo -p apidoc
```

Вывод (фрагмент):

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

### 5. Онлайн-отладка и Mock (M4)

Откройте страницу документации → выберите интерфейс → панель «Онлайн-отладка» справа предзаполнит параметры по правилам mock → укажите Base URL целевого сервиса (по умолчанию `location.origin`, прямое кросс-доменное подключение) → нажмите «Отправить» и получите реальный ответ (код статуса / время / pretty JSON). Панель отладки по умолчанию скрывает эндпоинты `not_debug` — они появляются только после установки флажка «Показать интерфейсы not_debug».

**Требование CORS**: онлайн-отладка выполняется браузером напрямую к целевому API, поэтому целевой сервис должен подключить `cors_layer` из адаптера, чтобы разрешить кросс-доменные запросы; при сбое CORS панель показывает жёлтую подсказку.

Синтаксис правил mock (три уровня приоритета):

```rust
#[apidoc::param(name = "email", ty = "string", desc = "邮箱", mock = "fake:email")]  // генерация по правилу fake
#[apidoc::param(name = "status", ty = "string", desc = "状态", mock = "1")]          // непустой mock выводится как есть
#[apidoc::param(name = "name", ty = "string", desc = "用户名")]                       // без mock: автогенерация по ty
#[apidoc::returned(
    name = "data",
    ty = "object",
    children = [
        { name = "id", ty = "int", required },       // без mock → "1"
        { name = "email", ty = "string", mock = "fake:email" },  // children рекурсивно вложены
    ]
)]
fn create_user() -> String {
    unimplemented!()
}
```

15 встроенных правил fake: `name` / `company` / `email` / `phone` / `url` / `ip` / `city` / `country` / `text` / `number` / `int` / `float` / `bool` / `uuid` / `date`; неизвестные имена правил откатываются к значениям по умолчанию. Автогенерация без mock: int→`"1"`, float→`"0.5"`, bool→`"true"`, object→`"{}"`, string→`"string"`; массив всегда из 2 элементов.

## План разработки

| Этап | Содержание | Статус |
|------|------------|--------|
| M1 | Скелет workspace + модель данных + MVP макросов + регистрация linkme | ✅ Завершено |
| M2 | Адаптер axum + встроенный UI документации + группировка по разделам | ✅ Завершено |
| M3 | Дополнение аннотаций (tag/group/author/header/route_param/response_status/success/error/not_debug/md/sort/ref) | ✅ Завершено |
| M4 | Онлайн-отладка + движок Mock | ✅ Завершено |
| M5 | Экспорт в markdown / typescript / swagger.json | В планах |
| M6 | Парольная аутентификация, несколько приложений и версий, релиз | В планах |

## Многоязычная документация

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

## Поддержка и пожертвования

Если этот проект оказался вам полезен, поставьте нам ⭐ Star — и вы также можете поддержать открытый исходный код пожертвованием!

### 微信 / 支付宝 (WeChat Pay / Alipay)

<table>
  <tr>
    <td align="center">
      <img src="../../docs/weixinpay.png" width="130" height="130" alt="微信支付 (WeChat Pay)" /><br/>
      <strong>微信支付</strong>
    </td>
    <td align="center">
      <img src="../../docs/alipay.png" width="130" height="130" alt="支付宝 (Alipay)" /><br/>
      <strong>支付宝</strong>
    </td>
  </tr>
</table>

### Пожертвования международным переводом

**【Информация о получателе】**

- Имя получателя: WANG KEXUN
- Номер счёта получателя: 881015918251

**【Банк получателя】**

- ZA Bank SWIFT Code: AABLHKHHXXX
- Название банка: ZA Bank Limited
- Банковский код: 387
- Адрес банка: Core F, Cyberport 3, 100 Cyberport Road, Hong Kong

**【Банк-посредник для международных переводов (при необходимости)】**

> Обратите внимание: это информация о банке-посреднике (транзитном банке) для международных переводов, а не о банке получателя. Уточните в банке, из которого делаете перевод, требуется ли предоставлять данные банка-посредника.

- **Банк-посредник для переводов в гонконгских долларах, юанях и долларах США — Citibank:**
  - Название банка: Citibank N.A. Hong Kong
  - SWIFT Code: CITIHKHXXXX
  - Банковский код: 006
  - Отделение: Hong Kong Branch
  - Код отделения: 391
  - Адрес банка: Citibank Tower, Citibank Plaza, 3 Garden Road, Central, Hong Kong
- **Банк-посредник для переводов в других валютах — BNY Mellon:**
  - Название банка: THE BANK OF NEW YORK MELLON
  - SWIFT Code: IRVTUS3NXXX
  - Адрес банка: THE BANK OF NEW YORK MELLON, 240 GREENWICH STREET, NEW YORK, United States

## License

[MIT](../../LICENSE)
