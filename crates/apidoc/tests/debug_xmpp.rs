#![cfg(feature = "debug-xmpp")]
//! P3 XMPP：配置校验/guard（不连网）+ `APIDOC_XMPP_*` 门控的真实登录
//! 测试；未设置环境变量时干净跳过。MSRV spike 结论：tokio-xmpp 4.0.0
//! 可编译并完成 STARTTLS+SASL，无需降级。

use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::DebugConfig;
use std::collections::BTreeMap;

fn req(target: &str, opts: &[(&str, &str)], body: Option<&str>) -> DebugRequest {
    DebugRequest {
        kind: "xmpp".into(),
        target: target.into(),
        method: None,
        headers: BTreeMap::new(),
        body: body.map(|s| Payload {
            encoding: "utf8".into(),
            data: s.into(),
        }),
        opts: opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        endpoint: None,
        timeout_ms: Some(5000),
    }
}

fn allow_private() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn xmpp_missing_jid_is_reported() {
    let err = execute(
        &allow_private(),
        &req("", &[("password", "secret")], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("jid"), "{err}");
    assert!(!err.contains("secret"), "{err}");
}

#[tokio::test]
async fn xmpp_missing_password_is_reported() {
    let err = execute(
        &allow_private(),
        &req("", &[("jid", "user@example.com")], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("password"), "{err}");
}

#[tokio::test]
async fn xmpp_invalid_jid_is_reported() {
    let err = execute(
        &allow_private(),
        &req("", &[("jid", "not a jid"), ("password", "secret")], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("jid"), "{err}");
    assert!(!err.contains("secret"), "{err}");
}

#[tokio::test]
async fn xmpp_invalid_type_is_reported() {
    let err = execute(
        &allow_private(),
        &req(
            "",
            &[
                ("jid", "user@example.com"),
                ("password", "secret"),
                ("type", "bogus"),
            ],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("type"), "{err}");
}

#[tokio::test]
async fn xmpp_body_without_to_is_reported() {
    let err = execute(
        &allow_private(),
        &req(
            "xmpp://127.0.0.1:5222",
            &[("jid", "user@example.com"), ("password", "secret")],
            Some("hi"),
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("opts.to"), "{err}");
}

#[tokio::test]
async fn xmpp_loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req(
            "xmpp://127.0.0.1:5222",
            &[("jid", "user@example.com"), ("password", "secret")],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}

#[tokio::test]
async fn xmpp_login() {
    let (Ok(target), Ok(jid), Ok(password)) = (
        std::env::var("APIDOC_XMPP_TARGET"),
        std::env::var("APIDOC_XMPP_JID"),
        std::env::var("APIDOC_XMPP_PASSWORD"),
    ) else {
        eprintln!("skipping: APIDOC_XMPP_TARGET/JID/PASSWORD not set");
        return;
    };
    let res = execute(
        &allow_private(),
        &req(
            &target,
            &[
                ("jid", &jid),
                ("password", &password),
                ("duration_ms", "500"),
            ],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&res.body.data).unwrap();
    assert!(v["messages"].as_array().is_some());
}
