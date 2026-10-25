#![cfg(feature = "axum")]
//! 本地响应延迟门槛：apidoc 自身路由 p99 < 10ms（不含服务端调试的目标网络耗时）。
//!
//! 用 in-process `ServiceExt::oneshot` 测量路由 + 序列化本地开销；预热后取 p99。
//! 外部依赖/调试目标的耗时不在本测试范围内。

use apidoc::auth::AuthConfig;
use apidoc::webhook::WebhookConfig;
use apidoc::ApidocConfig;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use axum::Router;
use serde_json::Value;
use std::hint::black_box;
use std::time::{Duration, Instant};
use tower::ServiceExt;

#[allow(dead_code)]
#[apidoc::title("List items")]
#[apidoc::url("/api/items")]
#[apidoc::method("GET")]
#[apidoc::param(name = "id", ty = "int", mock = "1")]
#[apidoc::returned(name = "name", ty = "string")]
fn list() {}

fn config() -> ApidocConfig {
    ApidocConfig {
        title: "perf".into(),
        auth: Some(AuthConfig {
            enable: true,
            password: "pw".into(),
            secret_key: "perf-test-key".into(),
            expire: 0,
        }),
        webhooks: vec![WebhookConfig {
            id: "hook".into(),
            title: "Hook".into(),
            secret: None,
            max_body: 0,
            capacity: 0,
        }],
        ..Default::default()
    }
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn cpu_time() -> Duration {
    #[cfg(target_os = "linux")]
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_THREAD, &mut usage) == 0 {
            let secs = usage.ru_utime.tv_sec as u64 + usage.ru_stime.tv_sec as u64;
            let micros = usage.ru_utime.tv_usec as u64 + usage.ru_stime.tv_usec as u64;
            return Duration::from_secs(secs) + Duration::from_micros(micros);
        }
    }
    Duration::ZERO
}

fn system_overloaded() -> bool {
    let load = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok());
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    load.is_some_and(|l| l > cpus)
}

/// 返回 (wall p99, CPU p99)。CPU 时间排除调度抢占，适合共享 CI 机器。
async fn p99(app: &Router, make: impl Fn() -> Request<Body>, iterations: usize) -> (Duration, Duration) {
    for _ in 0..10 {
        let _ = app.clone().oneshot(make()).await.unwrap();
    }
    let mut wall = Vec::with_capacity(iterations);
    let mut cpu = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let cpu0 = cpu_time();
        let started = Instant::now();
        let res = app.clone().oneshot(make()).await.unwrap();
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        wall.push(started.elapsed());
        cpu.push(cpu_time().saturating_sub(cpu0));
        black_box(&body);
    }
    wall.sort();
    cpu.sort();
    let idx = (iterations - 1) * 99 / 100;
    (wall[idx], cpu[idx])
}

/// p99 < 10ms；CPU 时间始终强制，wall 时间仅在系统未过载时强制。
async fn assert_p99_under_10ms(
    app: &Router,
    make: impl Fn() -> Request<Body> + Copy,
    label: &str,
) {
    let overloaded = system_overloaded();
    for attempt in 0..3 {
        let (wall, cpu) = p99(app, make, 100).await;
        if cpu < Duration::from_millis(10) && (overloaded || wall < Duration::from_millis(10)) {
            if overloaded && wall >= Duration::from_millis(10) {
                eprintln!("latency: {label} wall p99 {wall:?} under system load (cpu {cpu:?})");
            }
            return;
        }
        if attempt == 2 {
            panic!(
                "{label} p99 exceeds 10ms after 3 attempts (wall {wall:?}, cpu {cpu:?}, overloaded={overloaded})"
            );
        }
    }
}

async fn login(app: &Router) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/apidoc/auth")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"pw"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["token"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "current_thread")]
async fn all_apidoc_routes_p99_under_10ms() {
    let app = apidoc::axum::apidoc_routes(config());
    let token = login(&app).await;
    let tok = format!("token={token}");

    let uris = [
        "/apidoc".to_string(),
        format!("/apidoc/api.json?{tok}"),
        format!("/apidoc/export?format=md&{tok}"),
        format!("/apidoc/export?format=ts&{tok}"),
        format!("/apidoc/export?format=swagger&{tok}"),
        format!("/apidoc/export?format=asyncapi&{tok}"),
        format!("/apidoc/mock?url=%2Fapi%2Fitems&method=GET&{tok}"),
        format!("/apidoc/generate?template=api.ts&{tok}"),
        format!("/apidoc/share?url=%2Fapi%2Fitems&method=GET&{tok}"),
        format!("/apidoc/webhook?{tok}"),
        format!("/apidoc/webhook/hook?{tok}"),
    ];
    for uri in uris {
        assert_p99_under_10ms(&app, || get(&uri), &uri).await;
    }

    // 登录与关闭状态下的 debug 路由（本地 401/403，不含目标网络）。
    let auth_req = || {
        Request::builder()
            .method("POST")
            .uri("/apidoc/auth")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"password":"pw"}"#))
            .unwrap()
    };
    assert_p99_under_10ms(&app, auth_req, "auth").await;

    let debug_req = || {
        Request::builder()
            .method("POST")
            .uri(format!("/apidoc/debug?{tok}"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"kind":"http","target":"http://127.0.0.1:1"}"#))
            .unwrap()
    };
    assert_p99_under_10ms(&app, debug_req, "debug 403").await;

    // 装满 webhook 事件（40 × 8KiB ≈ 320KiB，触发 256KiB 响应上限），再测 GET。
    for i in 0..40 {
        let body = format!(r#"{{"n":{i},"pad":"{}"}}"#, "x".repeat(8192));
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/apidoc/webhook/hook")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 202);
    }
    assert_p99_under_10ms(
        &app,
        || get(&format!("/apidoc/webhook/hook?{tok}")),
        "webhook capped events",
    )
    .await;
}
