//! P6 Task 1: webhook inbound capture — public POST, guarded GET, HMAC, bounds.
#![cfg(feature = "axum")]

use apidoc::axum::{apidoc_routes, cors_layer, CorsConfig};
use apidoc::webhook::WebhookConfig;
use apidoc::{ApidocConfig, AuthConfig};
use axum::body::{to_bytes, Body};
use axum::http::Request;
use axum::Router;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use tower::ServiceExt;

const ID: &str = "0123456789abcdef0123456789abcdef";
const SECRET: &str = "webhook-test-secret";
const PASSWORD: &str = "pw";

fn cfg() -> ApidocConfig {
    ApidocConfig {
        auth: Some(AuthConfig { enable: true, password: PASSWORD.into(), ..Default::default() }),
        webhooks: vec![WebhookConfig {
            id: ID.into(),
            title: "stripe".into(),
            secret: Some(SECRET.into()),
            max_body: 1024,
            capacity: 2,
        }],
        ..Default::default()
    }
}

fn app(cfg: ApidocConfig) -> Router {
    Router::new().merge(apidoc_routes(cfg)).layer(cors_layer(CorsConfig::default()))
}

struct Resp {
    status: u16,
    body: String,
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> Resp {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let res = app.clone().oneshot(builder.body(Body::from(body)).unwrap()).await.unwrap();
    let status = res.status().as_u16();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    Resp { status, body: String::from_utf8(body.to_vec()).unwrap() }
}

fn sign(body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(body);
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

async fn token(app: &Router) -> String {
    let body = serde_json::json!({ "password": PASSWORD }).to_string();
    let res =
        call(app, "POST", "/apidoc/auth", &[("content-type", "application/json")], body.into())
            .await;
    assert_eq!(res.status, 200, "login failed: {}", res.body);
    let v: Value = serde_json::from_str(&res.body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

async fn post_signed_to(app: &Router, id: &str, body: &[u8]) -> Resp {
    let sig = sign(body);
    call(
        app,
        "POST",
        &format!("/apidoc/webhook/{id}"),
        &[("content-type", "application/json"), ("x-apidoc-signature", &sig)],
        body.to_vec(),
    )
    .await
}

async fn post_signed(app: &Router, body: &[u8]) -> Resp {
    post_signed_to(app, ID, body).await
}

#[tokio::test]
async fn captures_signed_event_and_lists_without_secret() {
    let app = app(cfg());
    let body = br#"{"id":"evt_1"}"#;
    let res = post_signed(&app, body).await;
    assert_eq!(res.status, 202, "body: {}", res.body);
    assert_eq!(serde_json::from_str::<Value>(&res.body).unwrap()["seq"], 1);

    let token = token(&app).await;
    let list = call(&app, "GET", &format!("/apidoc/webhook?token={token}"), &[], vec![]).await;
    assert_eq!(list.status, 200);
    assert!(list.body.contains(ID), "list must expose the id: {}", list.body);
    assert!(list.body.contains("stripe"));
    assert!(!list.body.contains(SECRET), "secret must never be serialized");

    let events =
        call(&app, "GET", &format!("/apidoc/webhook/{ID}?token={token}"), &[], vec![]).await;
    assert_eq!(events.status, 200);
    let v: Value = serde_json::from_str(&events.body).unwrap();
    let e = &v["events"][0];
    assert_eq!(e["seq"], 1);
    assert!(e["ts"].as_u64().unwrap() > 0);
    assert_eq!(e["body"]["encoding"], "utf8");
    assert_eq!(e["body"]["data"], r#"{"id":"evt_1"}"#);
}

#[tokio::test]
async fn stores_binary_body_as_hex() {
    let app = app(cfg());
    let res = post_signed(&app, &[0xff, 0x00, 0x41]).await;
    assert_eq!(res.status, 202);
    let token = token(&app).await;
    let events =
        call(&app, "GET", &format!("/apidoc/webhook/{ID}?token={token}"), &[], vec![]).await;
    let v: Value = serde_json::from_str(&events.body).unwrap();
    assert_eq!(v["events"][0]["body"]["encoding"], "hex");
    assert_eq!(v["events"][0]["body"]["data"], "ff0041");
}

#[tokio::test]
async fn rejects_bad_signature_oversize_unknown_id_and_missing_token() {
    let app = app(cfg());
    let body = b"{}".to_vec();
    let bad = call(
        &app,
        "POST",
        &format!("/apidoc/webhook/{ID}"),
        &[("x-apidoc-signature", "deadbeef")],
        body.clone(),
    )
    .await;
    assert_eq!(bad.status, 401);
    assert!(!bad.body.contains(SECRET));

    let not_hex = call(
        &app,
        "POST",
        &format!("/apidoc/webhook/{ID}"),
        &[("x-apidoc-signature", "zz-not-hex")],
        body.clone(),
    )
    .await;
    assert_eq!(not_hex.status, 401);

    // oversize is rejected before HMAC work (default max_body 1024 here)
    let big = vec![b'a'; 2048];
    let oversized = call(
        &app,
        "POST",
        &format!("/apidoc/webhook/{ID}"),
        &[("x-apidoc-signature", &sign(&big))],
        big,
    )
    .await;
    assert_eq!(oversized.status, 413);

    let unknown = call(&app, "POST", "/apidoc/webhook/nope", &[], b"{}".to_vec()).await;
    assert_eq!(unknown.status, 404);

    // guarded reads: no token -> 401
    let no_token = call(&app, "GET", &format!("/apidoc/webhook/{ID}"), &[], vec![]).await;
    assert_eq!(no_token.status, 401);
    let list_no_token = call(&app, "GET", "/apidoc/webhook", &[], vec![]).await;
    assert_eq!(list_no_token.status, 401);
    assert!(!no_token.body.contains(SECRET));
}

#[tokio::test]
async fn since_filters_by_sequence() {
    let app = app(cfg());
    assert_eq!(post_signed(&app, b"one").await.status, 202);
    assert_eq!(post_signed(&app, b"two").await.status, 202);

    let token = token(&app).await;
    let all = call(&app, "GET", &format!("/apidoc/webhook/{ID}?token={token}"), &[], vec![]).await;
    let v: Value = serde_json::from_str(&all.body).unwrap();
    assert_eq!(v["events"].as_array().unwrap().len(), 2);

    let since =
        call(&app, "GET", &format!("/apidoc/webhook/{ID}?token={token}&since=1"), &[], vec![])
            .await;
    let v: Value = serde_json::from_str(&since.body).unwrap();
    let events = v["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "since=1 must drop the first event: {}", since.body);
    assert_eq!(events[0]["body"]["data"], "two");
}

#[tokio::test]
async fn ring_buffer_keeps_only_capacity() {
    let app = app(cfg());
    for n in 0..3 {
        assert_eq!(post_signed(&app, format!("e{n}").as_bytes()).await.status, 202);
    }
    let token = token(&app).await;
    let all = call(&app, "GET", &format!("/apidoc/webhook/{ID}?token={token}"), &[], vec![]).await;
    let v: Value = serde_json::from_str(&all.body).unwrap();
    let events = v["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "capacity=2 must evict the oldest");
    assert_eq!(events[0]["body"]["data"], "e1");
    assert_eq!(events[1]["body"]["data"], "e2");
}

#[tokio::test]
async fn rate_limits_are_isolated_per_webhook_id() {
    const OTHER: &str = "fedcba9876543210fedcba9876543210";
    let mut config = cfg();
    config.webhooks.push(WebhookConfig {
        id: OTHER.into(),
        title: "github".into(),
        secret: Some(SECRET.into()),
        max_body: 1024,
        capacity: 2,
    });
    let app = app(config);
    for _ in 0..100 {
        assert_eq!(post_signed(&app, b"x").await.status, 202);
    }
    assert_eq!(post_signed(&app, b"x").await.status, 429, "first id hits its own limit");
    assert_eq!(post_signed_to(&app, OTHER, b"x").await.status, 202, "other id has its own bucket");
}

#[tokio::test]
async fn global_cap_bounds_writes_across_ids() {
    let ids: Vec<String> = (0..=10).map(|i| format!("{i:032x}")).collect();
    let config = ApidocConfig {
        webhooks: ids
            .iter()
            .map(|id| WebhookConfig {
                id: id.clone(),
                title: "x".into(),
                secret: None,
                max_body: 16,
                capacity: 1,
            })
            .collect(),
        ..Default::default()
    };
    let app = app(config);
    // The global cap is 1000/min, so ten ids × 100 writes exhausts it.
    for id in &ids[..10] {
        for _ in 0..100 {
            let res =
                call(&app, "POST", &format!("/apidoc/webhook/{id}"), &[], b"x".to_vec()).await;
            assert_eq!(res.status, 202, "id {id}: {}", res.body);
        }
    }
    let res = call(&app, "POST", &format!("/apidoc/webhook/{}", ids[10]), &[], b"x".to_vec()).await;
    assert_eq!(res.status, 429, "global cap must bound total writes across ids");
}

#[tokio::test]
async fn unknown_ids_do_not_consume_the_write_budget() {
    let app = app(cfg());
    for _ in 0..101 {
        let res = call(&app, "POST", "/apidoc/webhook/nope", &[], b"{}".to_vec()).await;
        assert_eq!(res.status, 404);
    }
    assert_eq!(post_signed(&app, b"x").await.status, 202, "unknown ids must not charge the bucket");
}

#[tokio::test]
async fn unauthenticated_writes_are_rate_limited() {
    let app = app(cfg());
    let sig = sign(b"x");
    let mut last = 0;
    // default limit is 100/60s per webhook; config in this test cannot change it.
    for _ in 0..=100 {
        last = call(
            &app,
            "POST",
            &format!("/apidoc/webhook/{ID}"),
            &[("x-apidoc-signature", &sig)],
            b"x".to_vec(),
        )
        .await
        .status;
    }
    assert_eq!(last, 429, "burst of unauthenticated writes must be throttled");
}
