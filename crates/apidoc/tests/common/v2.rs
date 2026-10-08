//! v2 适配器路由级集成测试的共享断言体（axum 侧 / actix 侧逐条 1:1 共用）：
//! /apidoc/generate 模板分发、/apidoc/share 深链构造、鉴权守卫一致性（share 按
//! 被分享的应用做守卫、share 链接 token 反向解锁 api.json 的端到端闭环）、缓存
//! 接线不影响输出、v1 api.json 红线、UI 分享按钮与前后置脚本接线。
//! 本模块只测「适配器路由与配置接线」；纯逻辑单测在 codegen.rs / cache.rs /
//! share.rs / m3_table.rs / ui_events.rs，此处不重复。
//! driver（tests/axum_v2_routes.rs / tests/actix_v2_routes.rs）只提供取响应的 get。
#![allow(dead_code)]

use std::future::Future;

use apidoc::auth::{md5_hex, AuthConfig};
use apidoc::{AppConfig, ApidocConfig, CacheConfig, TableDef, TableField};
use serde_json::Value;

use crate::common::{enc, link_of, link_token, Resp};

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

/// /apidoc/auth 换取 token；`app_key` 为 Some 时带 appKey（应用密码优先）。
async fn issue_token<F, Fut>(get: &F, adapter: &str, cfg: ApidocConfig, password: &str, app_key: Option<&str>) -> String
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let ak = app_key.map(|k| format!("&appKey={k}")).unwrap_or_default();
    let uri = format!("/apidoc/auth?password={}{ak}", md5_hex(password));
    let r = get(cfg, uri).await;
    assert_eq!(r.status, 200, "[{adapter}] 换 token 期望 200，实际 {}", r.status);
    serde_json::from_str::<Value>(&r.body).expect("auth 响应必须是合法 JSON")["token"]
        .as_str()
        .expect("auth 响应的 token 必须是字符串")
        .to_string()
}

// ---- 1. GET /apidoc/generate ----

pub async fn assert_generate_route_serves_builtin_templates<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    // 两个逐端点模板：body 必须含本 fixture 的 method + url 组合
    for (tpl, marker) in [("api.ts", "GET /api/user/info"), ("handler.rs", "GET /api/user/info")] {
        let r = get(config(), format!("/apidoc/generate?template={tpl}")).await;
        assert_eq!(r.status, 200, "[{adapter}] {tpl} 期望 200，实际 {}", r.status);
        let ct = r.header("content-type").expect("generate 响应缺少 Content-Type");
        assert_eq!(
            ct, "text/plain; charset=utf-8",
            "[{adapter}] {tpl} Content-Type 期望 text/plain; charset=utf-8，实际 {ct}"
        );
        assert!(
            r.body.contains(marker),
            "[{adapter}] {tpl} 期望含 fixture 接口标记 `{marker}`，实际 {}",
            r.body
        );
    }
    // schema.sql：另建 tables 配置后必须产出建表语句与字段名
    let cfg = ApidocConfig { tables: vec![user_table()], ..config() };
    let r = get(cfg, "/apidoc/generate?template=schema.sql".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] schema.sql 期望 200，实际 {}", r.status);
    let ct = r.header("content-type").unwrap();
    assert_eq!(
        ct, "text/plain; charset=utf-8",
        "[{adapter}] schema.sql Content-Type 期望 text/plain; charset=utf-8，实际 {ct}"
    );
    assert!(r.body.contains("CREATE TABLE"), "[{adapter}] schema.sql 期望含 CREATE TABLE，实际 {}", r.body);
    assert!(
        r.body.contains("id int NOT NULL"),
        "[{adapter}] schema.sql 期望含字段 `id int NOT NULL`，实际 {}",
        r.body
    );
    assert!(r.body.contains("name string"), "[{adapter}] schema.sql 期望含字段 `name string`，实际 {}", r.body);
}

pub async fn assert_generate_route_rejects_unknown_and_missing_template<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc/generate?template=nope.ts".to_string()).await;
    assert_eq!(r.status, 404, "[{adapter}] 未知模板期望 404，实际 {}", r.status);
    let r = get(config(), "/apidoc/generate".to_string()).await;
    assert_eq!(r.status, 400, "[{adapter}] 缺 template 期望 400，实际 {}", r.status);
}

// ---- 2. GET /apidoc/share ----

pub async fn assert_share_route_builds_deep_links<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    // 全参数：base + app + url + method；ep 的 / 必须百分号转义
    let r = get(
        config(),
        "/apidoc/share?base=https%3A%2F%2Fx&app=api&url=%2Fapi%2Fuser%2Finfo&method=GET".to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "[{adapter}] 全参数 share 期望 200，实际 {}", r.status);
    let ct = r.header("content-type").unwrap();
    assert_eq!(ct, "application/json", "[{adapter}] share Content-Type 期望 application/json，实际 {ct}");
    let link = link_of(&r.body);
    assert!(
        link.starts_with("https://x/apidoc?app=api&ep=%2Fapi%2Fuser%2Finfo&method=GET"),
        "[{adapter}] 期望深链以 `https://x/apidoc?app=api&ep=%2Fapi%2Fuser%2Finfo&method=GET` 开头，实际 {link}"
    );

    // 只给 app：链接不含 ep=
    let r = get(config(), "/apidoc/share?app=api".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] 只给 app 期望 200，实际 {}", r.status);
    assert_eq!(
        link_of(&r.body), "/apidoc?app=api",
        "[{adapter}] 只给 app 期望链接恰为 /apidoc?app=api（不含 ep=）"
    );

    // app 与 url 都不给 → 400
    let r = get(config(), "/apidoc/share".to_string()).await;
    assert_eq!(r.status, 400, "[{adapter}] app 与 url 都缺期望 400，实际 {}", r.status);

    // url 不存在 → 仍 200，但链接降级为只定位应用（不含 ep=）
    let r = get(config(), "/apidoc/share?app=api&url=%2Fnope&method=GET".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] 未知 url 期望仍 200，实际 {}", r.status);
    let link = link_of(&r.body);
    assert!(!link.contains("ep="), "[{adapter}] 未知 url 期望链接不含 ep=，实际 {link}");
}

// ---- 3. 鉴权一致性（守卫 + share 链接 token 端到端闭环）----

pub async fn assert_share_token_round_trips_through_api_json_guard<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    // 全局鉴权开启（应用树里 api 未配独立密码 → share 落全局密码签发分支）
    let auth_cfg = || ApidocConfig {
        auth: Some(AuthConfig { enable: true, password: "pw".into(), ..Default::default() }),
        apps: vec![app_of("api", None)],
        ..config()
    };
    // 无 token：share 与 generate 都必须 401
    let r = get(auth_cfg(), "/apidoc/share?app=api&url=%2Fapi%2Fuser%2Finfo&method=GET".to_string()).await;
    assert_eq!(r.status, 401, "[{adapter}] 鉴权开启时 share 无 token 期望 401，实际 {}", r.status);
    let r = get(auth_cfg(), "/apidoc/generate?template=api.ts".to_string()).await;
    assert_eq!(r.status, 401, "[{adapter}] 鉴权开启时 generate 无 token 期望 401，实际 {}", r.status);

    // /apidoc/auth 换 token 后两个路由都放行
    let token = issue_token(&get, adapter, auth_cfg(), "pw", None).await;
    let uri = format!("/apidoc/share?app=api&url=%2Fapi%2Fuser%2Finfo&method=GET&token={}", enc(&token));
    let r = get(auth_cfg(), uri).await;
    assert_eq!(r.status, 200, "[{adapter}] 带 token 的 share 期望 200，实际 {}", r.status);
    let share_body = r.body;
    let r = get(auth_cfg(), format!("/apidoc/generate?template=api.ts&token={}", enc(&token))).await;
    assert_eq!(r.status, 200, "[{adapter}] 带 token 的 generate 期望 200，实际 {}", r.status);

    // 闭环：share 链接中的 token 原样回灌，必须通过 /apidoc/api.json 守卫
    let link = link_of(&share_body);
    assert!(
        link.starts_with("/apidoc?app=api&ep=%2Fapi%2Fuser%2Finfo&method=GET&token="),
        "[{adapter}] 鉴权开启时期望链接尾部带 token，实际 {link}"
    );
    let r = get(auth_cfg(), format!("/apidoc/api.json?token={}", link_token(&link))).await;
    assert_eq!(
        r.status, 200,
        "[{adapter}] share 链接签发的 token 期望解锁 /apidoc/api.json，实际 {}",
        r.status
    );
}

/// share 路由按「被分享的应用」做守卫（接受 `app` / `appKey`）：应用配了独立密码时
/// —— 即便全局鉴权关闭 —— 无 token 也不能换出该应用的 token；全局 token 同样
/// 不构成提权路径，必须持应用 token。
pub async fn assert_share_requires_app_token_when_app_password_configured<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
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
    let r = get(auth_none_app_pw(), share(None)).await;
    assert_eq!(r.status, 401, "[{adapter}] 应用配了密码时 share 无 token 期望 401，实际 {}", r.status);
    // 应用 token → 200，链接带 token，且该 token 对 appKey=api 守卫放行（端到端闭环）
    let app_token = issue_token(&get, adapter, auth_none_app_pw(), "apppw", Some("api")).await;
    let r = get(auth_none_app_pw(), share(Some(&app_token))).await;
    assert_eq!(r.status, 200, "[{adapter}] 带应用 token 的 share 期望 200，实际 {}", r.status);
    let link = link_of(&r.body);
    assert!(
        link.starts_with("/apidoc?app=api&ep=%2Fapi%2Fuser%2Finfo&method=GET&token="),
        "[{adapter}] 期望链接带 token，实际 {link}"
    );
    let uri = format!("/apidoc/api.json?appKey=api&token={}", link_token(&link));
    let r = get(auth_none_app_pw(), uri).await;
    assert_eq!(r.status, 200, "[{adapter}] 链接签发的应用 token 期望解锁 appKey=api，实际 {}", r.status);

    // 全局鉴权开启 + 应用独立密码：全局 token 不得提权分享该应用
    let global_plus_app_pw = || ApidocConfig {
        auth: Some(AuthConfig { enable: true, password: "pw".into(), ..Default::default() }),
        apps: vec![app_of("api", Some("apppw"))],
        ..config()
    };
    let global_token = issue_token(&get, adapter, global_plus_app_pw(), "pw", None).await;
    let r = get(global_plus_app_pw(), share(Some(&global_token))).await;
    assert_eq!(
        r.status, 401,
        "[{adapter}] 全局 token 分享带独立密码应用期望 401（不提权），实际 {}",
        r.status
    );
    let app_token = issue_token(&get, adapter, global_plus_app_pw(), "apppw", Some("api")).await;
    let r = get(global_plus_app_pw(), share(Some(&app_token))).await;
    assert_eq!(r.status, 200, "[{adapter}] 换应用 token 后 share 期望 200，实际 {}", r.status);
}

/// 回归：无任何凭据配置（未配 apps / 应用无密码 + auth: None）时 share 照常 200
/// —— 新守卫不得把无鉴权场景一并挡掉。
pub async fn assert_share_stays_open_without_any_credentials_configured<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    // 未配置 apps 树
    let r = get(config(), "/apidoc/share?app=api&url=%2Fapi%2Fuser%2Finfo&method=GET".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] 未配置任何凭据时期望 share 200，实际 {}", r.status);
    // 应用已配置但无独立密码 + auth: None → 守卫落全局分支，未启用即放行
    let cfg = ApidocConfig { apps: vec![app_of("api", None)], ..config() };
    let r = get(cfg, "/apidoc/share?app=api&url=%2Fapi%2Fuser%2Finfo&method=GET".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] 应用无密码且未开全局鉴权时期望 share 200，实际 {}", r.status);
}

// ---- 4. 缓存接线（cache: Some(enable) vs None，输出必须一致）----

pub async fn assert_cache_wiring_keeps_mock_output_identical<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let uri = "/apidoc/mock?url=%2Fapi%2Fv2%2Fcache&method=GET";
    let cached = || ApidocConfig { cache: Some(CacheConfig { enable: true, ttl: 0 }), ..config() };
    let r1 = get(cached(), uri.to_string()).await;
    let r2 = get(cached(), uri.to_string()).await;
    assert_eq!(r1.status, 200, "[{adapter}] 开启缓存的 mock 期望 200，实际 {}", r1.status);
    assert_eq!(r2.status, 200, "[{adapter}] 开启缓存的 mock 第二次期望 200，实际 {}", r2.status);
    assert_eq!(
        r1.body.as_bytes(), r2.body.as_bytes(),
        "[{adapter}] 开启缓存后两次 mock 期望字节相同，实际 {} vs {}",
        r1.body, r2.body
    );
    assert!(
        r1.body.contains("\"alice\""),
        "[{adapter}] mock 输出期望含 fixture 显式值 alice，实际 {}",
        r1.body
    );

    // cache: None 走原路径，输出必须与开启缓存时逐字节一致（缓存不影响输出）
    let r3 = get(config(), uri.to_string()).await;
    assert_eq!(r3.status, 200, "[{adapter}] cache: None 的 mock 期望 200，实际 {}", r3.status);
    assert_eq!(
        r3.body.as_bytes(), r1.body.as_bytes(),
        "[{adapter}] cache: None 与开启缓存期望输出一致，实际 {} vs {}",
        r3.body, r1.body
    );
}

// ---- 5. v1 红线：api.json 不出现 v2 配置键 ----

pub async fn assert_api_json_has_no_v2_keys_on_v1_config<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let cfg = ApidocConfig { tables: Vec::new(), cache: None, codegen: Vec::new(), ..config() };
    let r = get(cfg, "/apidoc/api.json".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] api.json 期望 200，实际 {}", r.status);
    let v: Value = serde_json::from_str(&r.body).expect("api.json 必须是合法 JSON");
    for key in ["tables", "cache", "codegen"] {
        assert!(
            v["config"].get(key).is_none(),
            "[{adapter}] v1 配置期望 config 不含 {key} 键，实际 {}",
            v["config"]
        );
        assert!(v.get(key).is_none(), "[{adapter}] v1 配置期望根对象不含 {key} 键，实际 {}", v);
    }
    for ep in v["endpoints"].as_array().expect("endpoints 必须是数组") {
        assert!(
            ep.get("table").is_none(),
            "[{adapter}] 未用 table 注解的端点期望不含 table 键，实际 {ep}"
        );
    }
}

// ---- 6. UI 接线（分享按钮 + 前后置脚本标记随 UI 页下发）----

pub async fn assert_ui_page_wires_share_and_pre_post_scripts<F, Fut>(adapter: &str, get: F)
where
    F: Fn(ApidocConfig, String) -> Fut,
    Fut: Future<Output = Resp>,
{
    let r = get(config(), "/apidoc".to_string()).await;
    assert_eq!(r.status, 200, "[{adapter}] /apidoc 期望 200，实际 {}", r.status);
    for marker in ["shareButton", "shareParams", "'share?'", "apidoc_pre_script", "apidoc_post_script"] {
        assert!(r.body.contains(marker), "[{adapter}] UI 页期望含接线标记 `{marker}`，实际缺失");
    }
}
