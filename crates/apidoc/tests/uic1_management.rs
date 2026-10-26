//! UIC-1 Phase 2 (management parity): share CRUD/login/scope, cache build mode,
//! code templates, exportAllData. One test keeps `APIDOC_SHARE_DIR` process-wide
//! and the store lifecycle sequential.
#![cfg(feature = "axum")]

use apidoc::auth::AuthConfig;
use apidoc::axum::apidoc_routes;
use apidoc::{ApidocConfig, AppConfig, CodegenTemplate};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

#[allow(dead_code)]
#[apidoc::app("A")]
#[apidoc::title("List A")]
#[apidoc::url("/a/items")]
#[apidoc::method("GET")]
#[apidoc::md("# List A")]
fn a_items() {}

#[allow(dead_code)]
#[apidoc::app("A")]
#[apidoc::title("Create A")]
#[apidoc::url("/a/items")]
#[apidoc::method("POST")]
fn a_create() {}

#[allow(dead_code)]
#[apidoc::app("B")]
#[apidoc::title("List B")]
#[apidoc::url("/b/items")]
#[apidoc::method("GET")]
#[apidoc::md("# List B")]
fn b_items() {}

fn config() -> ApidocConfig {
    ApidocConfig {
        title: "Management".into(),
        auth: Some(AuthConfig {
            enable: true,
            password: "global-pw".into(),
            secret_key: "management-test-secret".into(),
            expire: 3600,
        }),
        apps: vec![
            AppConfig { key: "A".into(), title: "App A".into(), items: Vec::new(), password: None },
            AppConfig { key: "B".into(), title: "App B".into(), items: Vec::new(), password: None },
        ],
        codegen: vec![CodegenTemplate {
            name: "api.ts".into(),
            template: "{{#each endpoints}}{{title}}|{{method}} {{url}}\n{{/each}}".into(),
        }],
        ..Default::default()
    }
}

async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: &str,
    token: &str,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if !token.is_empty() {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn login_global(app: &axum::Router) -> String {
    let (status, body) = send(
        app,
        "POST",
        "/apidoc/ui/v1/verifyAuth",
        r#"{"password":"global-pw"}"#,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"]["token"].as_str().unwrap().to_string()
}

async fn login_share(app: &axum::Router, share_key: &str, password: &str) -> String {
    let (status, body) = send(
        app,
        "POST",
        "/apidoc/ui/v1/verifyAuth",
        &json!({"password": password, "shareKey": share_key}).to_string(),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"]["token"].as_str().unwrap().to_string()
}

async fn add_share(app: &axum::Router, token: &str, body: Value) {
    let (status, response) =
        send(app, "POST", "/apidoc/ui/v1/addApiShare", &body.to_string(), token).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["data"], true, "{response}");
}

#[tokio::test]
async fn phase2_management_end_to_end() {
    let dir = std::env::temp_dir().join(format!(
        "apidoc-uic1-management-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("APIDOC_SHARE_DIR", &dir);

    let app = apidoc_routes(config());
    let global = login_global(&app).await;

    // ---- capabilities advertise Phase 2 honestly ----
    let (status, caps) = send(&app, "GET", "/apidoc/ui/v1/capabilities", "", "").await;
    assert_eq!(status, StatusCode::OK, "{caps}");
    let features = &caps["data"]["features"];
    assert_eq!(features["share"]["records"], true);
    assert_eq!(features["share"]["actions"], false, "no declarative actions");
    assert_eq!(features["share"]["link"], true);
    assert_eq!(features["cache"]["manage"], true);
    assert_eq!(features["cache"]["mode"], "build");
    assert_eq!(features["generator"]["forms"], false);
    assert_eq!(features["generator"]["templates"], true);
    assert_eq!(features["auth"]["shareToken"], true);
    let exports = features["exports"].as_array().unwrap();
    for name in ["md", "pdf", "openapi", "postman", "asyncapi", "typescript"] {
        assert!(exports.iter().any(|e| e == name), "missing export {name}");
    }

    // ---- seed shares ----
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/addApiShare",
        r#"{"name":"nope","type":"all"}"#,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "owner CRUD requires global auth");

    add_share(
        &app,
        &global,
        json!({"key":"team","name":"Team","type":"app","appKeys":["A"],"password":"team-pw"}),
    )
    .await;
    add_share(
        &app,
        &global,
        json!({"key":"one-api","name":"One API","type":"api","appKeys":["A"],"apiKeys":[]}),
    )
    .await;
    // Public link share: no password field at all.
    add_share(&app, &global, json!({"key":"public","name":"Public","type":"all"})).await;
    add_share(
        &app,
        &global,
        json!({"key":"locked","name":"Locked","type":"all","password":"locked-pw"}),
    )
    .await;

    // ---- owner list/detail never leak credentials ----
    let (status, list) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/getApiShareList",
        r#"{"pageIndex":1}"#,
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["data"]["total"], 4, "{list}");
    let team = list["data"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["key"] == "team")
        .expect("team in list");
    assert_eq!(team["type"], "app");
    assert_eq!(team["appKeys"][0], "A");
    assert!(team["create_at"].is_string());
    assert!(team["create_time"].is_string(), "schema wants a string: {team}");
    for secret in ["secret", "password", "password_hash", "passwordHash"] {
        assert!(team.get(secret).is_none(), "list leaked {secret}");
    }

    let (status, detail) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/getApiShareDetail",
        r#"{"key":"team"}"#,
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["data"]["name"], "Team");
    for secret in ["secret", "password", "password_hash", "passwordHash"] {
        assert!(detail["data"].get(secret).is_none(), "detail leaked {secret}");
    }

    // Edit keeps the old password when none is supplied.
    add_share(
        &app,
        &global,
        json!({"key":"team","name":"Team renamed","type":"app","appKeys":["A"]}),
    )
    .await;
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/verifyAuth",
        r#"{"password":"team-pw","shareKey":"team"}"#,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "edit must keep the password");
    let (_, detail) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/getApiShareDetail",
        r#"{"key":"team"}"#,
        &global,
    )
    .await;
    assert_eq!(detail["data"]["name"], "Team renamed");

    // ---- share login ----
    let (status, body) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/verifyAuth",
        r#"{"password":"wrong","shareKey":"team"}"#,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], 401);

    let team_token = login_share(&app, "team", "team-pw").await;
    let _locked_token = login_share(&app, "locked", "locked-pw").await;
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/verifyAuth",
        r#"{"password":"locked-pw","shareKey":"public"}"#,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "public shares accept any password");

    // ---- config scope ----
    let (status, _) = send(&app, "GET", "/apidoc/ui/v1/config?shareKey=team", "", "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "password share needs its token");
    let (status, _) = send(
        &app,
        "GET",
        "/apidoc/ui/v1/config?shareKey=team",
        "",
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "global token is not a share token");
    let (status, cfg) = send(
        &app,
        "GET",
        "/apidoc/ui/v1/config?shareKey=team",
        "",
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cfg}");
    let keys: Vec<&str> = cfg["data"]["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["A"], "app share must hide app B");
    assert_eq!(cfg["data"]["share"]["enable"], true);
    assert_eq!(cfg["data"]["cache"]["enable"], true);
    assert_eq!(cfg["data"]["code_template"][0]["template"], "api.ts");

    let (status, cfg) = send(
        &app,
        "GET",
        "/apidoc/ui/v1/config?shareKey=public",
        "",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "public share needs no token: {cfg}");
    assert_eq!(cfg["data"]["apps"].as_array().unwrap().len(), 2);

    let (status, _) = send(
        &app,
        "GET",
        "/apidoc/ui/v1/config",
        "",
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "share token must not pass the global guard");

    // ---- menus/detail/doc scope ----
    let (status, menus) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/menus",
        &json!({"appKey":"A","shareKey":"team"}).to_string(),
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{menus}");
    let menu_items: Vec<&Value> = menus["data"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["children"].as_array().unwrap())
        .collect();
    let a_items_id = menu_items
        .iter()
        .find(|item| item["url"] == "/a/items" && item["method"] == "GET")
        .and_then(|item| item["menuKey"].as_str())
        .unwrap()
        .to_string();
    let a_create_id = menu_items
        .iter()
        .find(|item| item["method"] == "POST")
        .and_then(|item| item["menuKey"].as_str())
        .unwrap()
        .to_string();

    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/menus",
        &json!({"appKey":"B","shareKey":"team"}).to_string(),
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "share must not open app B");

    let b_items_id;
    {
        let (status, full_menus) = send(
            &app,
            "POST",
            "/apidoc/ui/v1/menus",
            &json!({"appKey":"B"}).to_string(),
            &global,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{full_menus}");
        b_items_id = full_menus["data"]["data"][0]["children"][0]["menuKey"]
            .as_str()
            .unwrap()
            .to_string();
    }

    let (status, body) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/detail",
        &json!({"path":a_items_id,"shareKey":"team"}).to_string(),
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["title"], "List A");
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/detail",
        &json!({"path":b_items_id,"shareKey":"team"}).to_string(),
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "share must not read app B detail");

    let (status, docs) = send(
        &app,
        "GET",
        "/apidoc/ui/v1/docMenus?appKey=A&shareKey=team",
        "",
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{docs}");
    let doc_paths: Vec<&str> = docs["data"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["children"].as_array().unwrap())
        .map(|item| item["path"].as_str().unwrap())
        .collect();
    assert!(doc_paths.contains(&a_items_id.as_str()));
    assert!(!doc_paths.contains(&b_items_id.as_str()), "docs leaked across apps");
    let (status, _) = send(
        &app,
        "GET",
        "/apidoc/ui/v1/docMenus?appKey=B&shareKey=team",
        "",
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "docMenus must reject apps outside the share");
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/docDetail",
        &json!({"path":b_items_id,"shareKey":"team"}).to_string(),
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, doc) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/docDetail",
        &json!({"path":a_items_id,"shareKey":"team"}).to_string(),
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert!(doc["data"]["content"].as_str().unwrap().contains("# List A"));

    // Share token cross-use is rejected.
    let (status, _) = send(
        &app,
        "GET",
        "/apidoc/ui/v1/config?shareKey=locked",
        "",
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // ---- api share restricts endpoint keys ----
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/addApiShare",
        &json!({"key":"one-api","name":"One API","type":"api","appKeys":["A"],
                "apiKeys":[a_items_id],"password":"api-pw"})
            .to_string(),
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let api_token = login_share(&app, "one-api", "api-pw").await;
    let (status, menus) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/menus",
        &json!({"appKey":"A","shareKey":"one-api"}).to_string(),
        &api_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{menus}");
    let api_items: Vec<&Value> = menus["data"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|group| group["children"].as_array().unwrap())
        .collect();
    assert_eq!(api_items.len(), 1, "api share must only list its apiKeys");
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/detail",
        &json!({"path":a_create_id,"shareKey":"one-api"}).to_string(),
        &api_token,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "api share must hide unselected endpoints");

    // ---- share tokens are global-only on native debug ----
    let (status, _) = send(
        &app,
        "POST",
        &format!("/apidoc/debug?appKey=A&token={team_token}"),
        "{}",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/debugSend",
        &json!({"appKey":"B","path":b_items_id,"shareKey":"team"}).to_string(),
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "debug must honour share scope");

    // ---- cache endpoints report build mode ----
    for path in ["/apidoc/ui/v1/cache/clear", "/apidoc/ui/v1/cache/warm"] {
        let (status, body) = send(&app, "POST", path, "{}", &global).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["ok"], true);
        assert_eq!(body["data"]["mode"], "build");
        assert_eq!(body["data"]["message"], "build-time materialized");
        let (status, _) = send(&app, "POST", path, "{}", "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // ---- code template exposure + rendering ----
    let (status, cfg) = send(
        &app,
        "GET",
        "/apidoc/ui/v1/config?appKey=A",
        "",
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cfg}");
    let template = &cfg["data"]["code_template"][0];
    assert_eq!(template["title"], "api.ts");
    assert_eq!(template["template"], "api.ts");
    assert_eq!(template["language"], "typescript");
    assert!(cfg["data"]["generator"].as_array().unwrap().is_empty());

    let (status, code) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/renderCodeTemplate",
        &json!({"appKey":"A","template":{"template":"api.ts"},"selected":[a_items_id]})
            .to_string(),
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{code}");
    let code = code["data"]["code"].as_str().unwrap();
    assert!(code.contains("List A|GET /a/items"), "{code}");
    assert!(!code.contains("Create A"), "selected must filter endpoints: {code}");
    assert!(!code.contains("List B"), "selected must filter apps endpoints: {code}");

    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/renderCodeTemplate",
        &json!({"appKey":"A","template":"missing.ts"}).to_string(),
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // ---- exportAllData ----
    let (status, exported) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/exportAllData",
        r#"{"key":"team"}"#,
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    let nodes = exported["data"]["apiData"].as_array().unwrap();
    assert_eq!(nodes.len(), 1, "app share exports app A only: {exported}");
    assert_eq!(nodes[0]["appKey"], "A");
    let children = nodes[0]["children"].as_array().unwrap();
    assert_eq!(children.len(), 2);
    assert!(children.iter().any(|c| c["title"] == "List A"));
    assert!(children.iter().all(|c| c["appKey"].is_null() || c["appKey"] == "A"));

    let (status, exported) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/exportAllData",
        &json!({"appKey":"A"}).to_string(),
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    assert_eq!(exported["data"]["apiData"].as_array().unwrap().len(), 1);

    let (status, exported) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/exportAllData",
        &json!({"shareKey":"team"}).to_string(),
        &team_token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    assert_eq!(exported["data"]["apiData"][0]["appKey"], "A");

    let (status, exported) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/exportAllData",
        r#"{"shareKey":"public"}"#,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    assert_eq!(
        exported["data"]["apiData"].as_array().unwrap().len(),
        2,
        "public all share exports both apps"
    );

    let (status, exported) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/exportAllData",
        r#"{"key":"one-api"}"#,
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{exported}");
    assert_eq!(
        exported["data"]["apiData"][0]["children"].as_array().unwrap().len(),
        1,
        "api share export keeps only selected endpoints"
    );

    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/exportAllData",
        r#"{"appKey":"missing"}"#,
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/exportAllData",
        r#"{"key":"team"}"#,
        "",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "share record export is owner-only");

    // ---- native AsyncAPI endpoint already materialises the document ----
    let (status, asyncapi) = send(
        &app,
        "GET",
        &format!("/apidoc/export?format=asyncapi&token={global}"),
        "",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{asyncapi}");
    assert_eq!(asyncapi["asyncapi"], "2.6.0");

    // ---- delete + share actions ----
    let (status, action) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/handleApiShareAction",
        r#"{"key":"team","index":0}"#,
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{action}");
    assert_eq!(action["data"], false, "Rust declares no share actions");
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/handleApiShareAction",
        r#"{"key":"missing","index":0}"#,
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    add_share(&app, &global, json!({"key":"trash","name":"Trash","type":"all"})).await;
    let (status, deleted) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/deleteApiShare",
        r#"{"key":"trash"}"#,
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted["data"], true);
    let (status, _) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/getApiShareDetail",
        r#"{"key":"trash"}"#,
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, deleted) = send(
        &app,
        "POST",
        "/apidoc/ui/v1/deleteApiShare",
        r#"{"key":"trash"}"#,
        &global,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete is idempotent: {deleted}");

    let _ = std::fs::remove_dir_all(&dir);
}
