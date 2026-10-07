#![cfg(feature = "axum")]
//! v2 适配器路由级集成测试（axum 侧）：/apidoc/generate 模板分发、/apidoc/share
//! 深链构造、鉴权守卫一致性（share 按被分享的应用做守卫、share 链接 token
//! 反向解锁 api.json 的端到端闭环）、缓存接线不影响输出、v1 api.json 红线、
//! UI 分享按钮与前后置脚本接线。
//! 与 actix_v2_routes.rs 逐条 1:1 对齐（同断言、同期望）。
//! 本文件只测「适配器路由与配置接线」；纯逻辑单测在 codegen.rs / cache.rs /
//! share.rs / m3_table.rs / ui_events.rs，此处不重复。

use apidoc::auth::{md5_hex, AuthConfig};
use apidoc::axum::apidoc_routes;
use apidoc::{AppConfig, ApidocConfig, CacheConfig, TableDef, TableField};
use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;

// ---- fixtures（真实注解；linkme 收集整个测试二进制的 fixture，断言只针对本文件的 url）----

/// 分享 / 代码生成共用的目标接口（url 唯一，断言可精确锁定）。
#[allow(dead_code)]
#[apidoc::title("获取用户信息")]
#[apidoc::url("/api/user/info")]
#[apidoc::method("GET")]
fn v2_share_info() {}

/// 缓存接线用接口：mock 全为显式常量（无 fake: 随机规则），输出字节级确定。
#[allow(dead_code)]
#[apidoc::title("缓存接线")]
#[apidoc::url("/api/v2/cache")]
#[apidoc::method("GET")]
#[apidoc::param(name = "name", ty = "string", mock = "alice")]
#[apidoc::query(name = "page", ty = "int", mock = "1")]
fn v2_cache_ep() {}

// ---- 公共脚手架 ----

/// v1 基线配置：v2 三个服务端字段全为空（红线形态），各用例用结构体更新覆盖单项。
fn config() -> ApidocConfig {
    ApidocConfig {
        title: "v2 测试".into(),
        description: Some("路由级集成".into()),
        auth: None,
        tables: Vec::new(),
        cache: None,
        codegen: Vec::new(),
        apps: Vec::new(),
    }
}

/// schema.sql 的配置源：数据表来自 config.tables，与端点注解无关。
fn user_table() -> TableDef {
    TableDef {
        key: "user".into(),
        title: "用户表".into(),
        fields: vec![
            TableField {
                name: "id".into(),
                ty: "int".into(),
                required: true,
                default: None,
                desc: Some("用户ID".into()),
                mock: None,
            },
            TableField {
                name: "name".into(),
                ty: "string".into(),
                required: true,
                default: None,
                desc: None,
                mock: None,
            },
        ],
    }
}

fn app_of(key: &str, password: Option<&str>) -> AppConfig {
    AppConfig {
        key: key.into(),
        title: key.into(),
        items: Vec::new(),
        password: password.map(String::from),
    }
}

async fn get(cfg: ApidocConfig, uri: &str) -> (StatusCode, HeaderMap, String) {
    let res = Router::new()
        .merge(apidoc_routes(cfg))
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

/// 解析 share 响应的 {"url": ...}。
fn link_of(body: &str) -> String {
    serde_json::from_str::<Value>(body).expect("share 响应必须是合法 JSON")["url"]
        .as_str()
        .expect("share 响应的 url 必须是字符串")
        .to_string()
}

/// 取 share 深链里的 token 参数原样编码值（build_url 保证 token 是最后一个参数）。
/// 原样回灌请求 URL 即为「链接开箱可用」的端到端验证，不需要再解码/重编码。
fn link_token(link: &str) -> String {
    link.split("token=")
        .nth(1)
        .unwrap_or_else(|| panic!("分享链接缺少 token 参数: {link}"))
        .to_string()
}

/// token 含 base64 的 + / 字符，作为 query 传值须百分号编码（同 ui.js 的 encodeURIComponent）。
fn enc(s: &str) -> String {
    s.replace('+', "%2B").replace('/', "%2F")
}

/// /apidoc/auth 换取 token；`app_key` 为 Some 时带 appKey（应用密码优先）。
async fn issue_token(cfg: ApidocConfig, password: &str, app_key: Option<&str>) -> String {
    let ak = app_key.map(|k| format!("&appKey={k}")).unwrap_or_default();
    let (status, _, body) = get(cfg, &format!("/apidoc/auth?password={}{ak}", md5_hex(password))).await;
    assert_eq!(status, StatusCode::OK, "换 token 期望 200，实际 {status}");
    serde_json::from_str::<Value>(&body).expect("auth 响应必须是合法 JSON")["token"]
        .as_str()
        .expect("auth 响应的 token 必须是字符串")
        .to_string()
}

// ---- 1. GET /apidoc/generate ----

#[tokio::test]
async fn generate_route_serves_builtin_templates() {
    // 两个逐端点模板：body 必须含本 fixture 的 method + url 组合
    for (tpl, marker) in [("api.ts", "GET /api/user/info"), ("handler.rs", "GET /api/user/info")] {
        let (status, headers, body) = get(config(), &format!("/apidoc/generate?template={tpl}")).await;
        assert_eq!(status, StatusCode::OK, "{tpl} 期望 200，实际 {status}");
        let ct = headers
            .get(header::CONTENT_TYPE)
            .expect("generate 响应缺少 Content-Type")
            .to_str()
            .unwrap();
        assert_eq!(
            ct, "text/plain; charset=utf-8",
            "{tpl} Content-Type 期望 text/plain; charset=utf-8，实际 {ct}"
        );
        assert!(body.contains(marker), "{tpl} 期望含 fixture 接口标记 `{marker}`，实际 {body}");
    }
    // schema.sql：另建 tables 配置后必须产出建表语句与字段名
    let cfg = ApidocConfig { tables: vec![user_table()], ..config() };
    let (status, headers, body) = get(cfg, "/apidoc/generate?template=schema.sql").await;
    assert_eq!(status, StatusCode::OK, "schema.sql 期望 200，实际 {status}");
    let ct = headers.get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
    assert_eq!(
        ct, "text/plain; charset=utf-8",
        "schema.sql Content-Type 期望 text/plain; charset=utf-8，实际 {ct}"
    );
    assert!(body.contains("CREATE TABLE"), "schema.sql 期望含 CREATE TABLE，实际 {body}");
    assert!(body.contains("id int NOT NULL"), "schema.sql 期望含字段 `id int NOT NULL`，实际 {body}");
    assert!(body.contains("name string"), "schema.sql 期望含字段 `name string`，实际 {body}");
}

#[tokio::test]
async fn generate_route_rejects_unknown_and_missing_template() {
    let (status, _, _) = get(config(), "/apidoc/generate?template=nope.ts").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "未知模板期望 404，实际 {status}");
    let (status, _, _) = get(config(), "/apidoc/generate").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "缺 template 期望 400，实际 {status}");
}

// ---- 2. GET /apidoc/share ----

#[tokio::test]
async fn share_route_builds_deep_links() {
    // 全参数：base + app + url + method；ep 的 / 必须百分号转义
    let (status, headers, body) =
        get(config(), "/apidoc/share?base=https%3A%2F%2Fx&app=api&url=%2Fapi%2Fuser%2Finfo&method=GET").await;
    assert_eq!(status, StatusCode::OK, "全参数 share 期望 200，实际 {status}");
    let ct = headers.get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
    assert_eq!(ct, "application/json", "share Content-Type 期望 application/json，实际 {ct}");
    let link = link_of(&body);
    assert!(
        link.starts_with("https://x/apidoc?app=api&ep=%2Fapi%2Fuser%2Finfo&method=GET"),
        "期望深链以 `https://x/apidoc?app=api&ep=%2Fapi%2Fuser%2Finfo&method=GET` 开头，实际 {link}"
    );

    // 只给 app：链接不含 ep=
    let (status, _, body) = get(config(), "/apidoc/share?app=api").await;
    assert_eq!(status, StatusCode::OK, "只给 app 期望 200，实际 {status}");
    assert_eq!(
        link_of(&body), "/apidoc?app=api",
        "只给 app 期望链接恰为 /apidoc?app=api（不含 ep=）"
    );

    // app 与 url 都不给 → 400
    let (status, _, _) = get(config(), "/apidoc/share").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "app 与 url 都缺期望 400，实际 {status}");

    // url 不存在 → 仍 200，但链接降级为只定位应用（不含 ep=）
    let (status, _, body) = get(config(), "/apidoc/share?app=api&url=%2Fnope&method=GET").await;
    assert_eq!(status, StatusCode::OK, "未知 url 期望仍 200，实际 {status}");
    let link = link_of(&body);
    assert!(!link.contains("ep="), "未知 url 期望链接不含 ep=，实际 {link}");
}

// ---- 3. 鉴权一致性（守卫 + share 链接 token 端到端闭环）----

#[tokio::test]
async fn share_token_round_trips_through_api_json_guard() {
    // 全局鉴权开启（应用树里 api 未配独立密码 → share 落全局密码签发分支）
    let auth_cfg = || ApidocConfig {
        auth: Some(AuthConfig { enable: true, password: "pw".into(), ..Default::default() }),
        apps: vec![app_of("api", None)],
        ..config()
    };
    // 无 token：share 与 generate 都必须 401
    let (status, _, _) = get(auth_cfg(), "/apidoc/share?app=api&url=%2Fapi%2Fuser%2Finfo&method=GET").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "鉴权开启时 share 无 token 期望 401，实际 {status}");
    let (status, _, _) = get(auth_cfg(), "/apidoc/generate?template=api.ts").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "鉴权开启时 generate 无 token 期望 401，实际 {status}");

    // /apidoc/auth 换 token 后两个路由都放行
    let token = issue_token(auth_cfg(), "pw", None).await;
    let uri = format!("/apidoc/share?app=api&url=%2Fapi%2Fuser%2Finfo&method=GET&token={}", enc(&token));
    let (status, _, body) = get(auth_cfg(), &uri).await;
    assert_eq!(status, StatusCode::OK, "带 token 的 share 期望 200，实际 {status}");
    let (status, _, _) = get(auth_cfg(), &format!("/apidoc/generate?template=api.ts&token={}", enc(&token))).await;
    assert_eq!(status, StatusCode::OK, "带 token 的 generate 期望 200，实际 {status}");

    // 闭环：share 链接中的 token 原样回灌，必须通过 /apidoc/api.json 守卫
    let link = link_of(&body);
    assert!(
        link.starts_with("/apidoc?app=api&ep=%2Fapi%2Fuser%2Finfo&method=GET&token="),
        "鉴权开启时期望链接尾部带 token，实际 {link}"
    );
    let (status, _, _) = get(auth_cfg(), &format!("/apidoc/api.json?token={}", link_token(&link))).await;
    assert_eq!(status, StatusCode::OK, "share 链接签发的 token 期望解锁 /apidoc/api.json，实际 {status}");
}

/// share 路由按「被分享的应用」做守卫（接受 `app` / `appKey`）：应用配了独立密码时
/// —— 即便全局鉴权关闭 —— 无 token 也不能换出该应用的 token；全局 token 同样
/// 不构成提权路径，必须持应用 token。
#[tokio::test]
async fn share_requires_app_token_when_app_password_configured() {
    let share = |token: Option<&str>| match token {
        Some(t) => format!("/apidoc/share?app=api&url=%2Fapi%2Fuser%2Finfo&method=GET&token={}", enc(t)),
        None => "/apidoc/share?app=api&url=%2Fapi%2Fuser%2Finfo&method=GET".to_string(),
    };
    // auth: None + 应用独立密码：无 token → 401（修复前这里 200 且无凭据换出应用 token）
    let auth_none_app_pw = || ApidocConfig {
        auth: None,
        apps: vec![app_of("api", Some("apppw"))],
        ..config()
    };
    let (status, _, _) = get(auth_none_app_pw(), &share(None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "应用配了密码时 share 无 token 期望 401，实际 {status}");
    // 应用 token → 200，链接带 token，且该 token 对 appKey=api 守卫放行（端到端闭环）
    let app_token = issue_token(auth_none_app_pw(), "apppw", Some("api")).await;
    let (status, _, body) = get(auth_none_app_pw(), &share(Some(&app_token))).await;
    assert_eq!(status, StatusCode::OK, "带应用 token 的 share 期望 200，实际 {status}");
    let link = link_of(&body);
    assert!(
        link.starts_with("/apidoc?app=api&ep=%2Fapi%2Fuser%2Finfo&method=GET&token="),
        "期望链接带 token，实际 {link}"
    );
    let uri = format!("/apidoc/api.json?appKey=api&token={}", link_token(&link));
    let (status, _, _) = get(auth_none_app_pw(), &uri).await;
    assert_eq!(status, StatusCode::OK, "链接签发的应用 token 期望解锁 appKey=api，实际 {status}");

    // 全局鉴权开启 + 应用独立密码：全局 token 不得提权分享该应用
    let global_plus_app_pw = || ApidocConfig {
        auth: Some(AuthConfig { enable: true, password: "pw".into(), ..Default::default() }),
        apps: vec![app_of("api", Some("apppw"))],
        ..config()
    };
    let global_token = issue_token(global_plus_app_pw(), "pw", None).await;
    let (status, _, _) = get(global_plus_app_pw(), &share(Some(&global_token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "全局 token 分享带独立密码应用期望 401（不提权），实际 {status}");
    let app_token = issue_token(global_plus_app_pw(), "apppw", Some("api")).await;
    let (status, _, _) = get(global_plus_app_pw(), &share(Some(&app_token))).await;
    assert_eq!(status, StatusCode::OK, "换应用 token 后 share 期望 200，实际 {status}");
}

/// 回归：无任何凭据配置（未配 apps / 应用无密码 + auth: None）时 share 照常 200
/// —— 新守卫不得把无鉴权场景一并挡掉。
#[tokio::test]
async fn share_stays_open_without_any_credentials_configured() {
    // 未配置 apps 树
    let (status, _, _) = get(config(), "/apidoc/share?app=api&url=%2Fapi%2Fuser%2Finfo&method=GET").await;
    assert_eq!(status, StatusCode::OK, "未配置任何凭据时期望 share 200，实际 {status}");
    // 应用已配置但无独立密码 + auth: None → 守卫落全局分支，未启用即放行
    let cfg = ApidocConfig { apps: vec![app_of("api", None)], ..config() };
    let (status, _, _) = get(cfg, "/apidoc/share?app=api&url=%2Fapi%2Fuser%2Finfo&method=GET").await;
    assert_eq!(status, StatusCode::OK, "应用无密码且未开全局鉴权时期望 share 200，实际 {status}");
}

// ---- 4. 缓存接线（cache: Some(enable) vs None，输出必须一致）----

#[tokio::test]
async fn cache_wiring_keeps_mock_output_identical() {
    let uri = "/apidoc/mock?url=%2Fapi%2Fv2%2Fcache&method=GET";
    let cached = || ApidocConfig { cache: Some(CacheConfig { enable: true, ttl: 0 }), ..config() };
    let (s1, _, b1) = get(cached(), uri).await;
    let (s2, _, b2) = get(cached(), uri).await;
    assert_eq!(s1, StatusCode::OK, "开启缓存的 mock 期望 200，实际 {s1}");
    assert_eq!(s2, StatusCode::OK, "开启缓存的 mock 第二次期望 200，实际 {s2}");
    assert_eq!(b1.as_bytes(), b2.as_bytes(), "开启缓存后两次 mock 期望字节相同，实际 {b1} vs {b2}");
    assert!(b1.contains("\"alice\""), "mock 输出期望含 fixture 显式值 alice，实际 {b1}");

    // cache: None 走原路径，输出必须与开启缓存时逐字节一致（缓存不影响输出）
    let (s3, _, b3) = get(config(), uri).await;
    assert_eq!(s3, StatusCode::OK, "cache: None 的 mock 期望 200，实际 {s3}");
    assert_eq!(b3.as_bytes(), b1.as_bytes(), "cache: None 与开启缓存期望输出一致，实际 {b3} vs {b1}");
}

// ---- 5. v1 红线：api.json 不出现 v2 配置键 ----

#[tokio::test]
async fn api_json_has_no_v2_keys_on_v1_config() {
    let cfg = ApidocConfig { tables: Vec::new(), cache: None, codegen: Vec::new(), ..config() };
    let (status, _, body) = get(cfg, "/apidoc/api.json").await;
    assert_eq!(status, StatusCode::OK, "api.json 期望 200，实际 {status}");
    let v: Value = serde_json::from_str(&body).expect("api.json 必须是合法 JSON");
    for key in ["tables", "cache", "codegen"] {
        assert!(
            v["config"].get(key).is_none(),
            "v1 配置期望 config 不含 {key} 键，实际 {}",
            v["config"]
        );
        assert!(v.get(key).is_none(), "v1 配置期望根对象不含 {key} 键，实际 {}", v);
    }
    for ep in v["endpoints"].as_array().expect("endpoints 必须是数组") {
        assert!(ep.get("table").is_none(), "未用 table 注解的端点期望不含 table 键，实际 {ep}");
    }
}

// ---- 6. UI 接线（分享按钮 + 前后置脚本标记随 UI 页下发）----

#[tokio::test]
async fn ui_page_wires_share_and_pre_post_scripts() {
    let (status, _, html) = get(config(), "/apidoc").await;
    assert_eq!(status, StatusCode::OK, "/apidoc 期望 200，实际 {status}");
    for marker in ["shareButton", "shareParams", "'share?'", "apidoc_pre_script", "apidoc_post_script"] {
        assert!(html.contains(marker), "UI 页期望含接线标记 `{marker}`，实际缺失");
    }
}
