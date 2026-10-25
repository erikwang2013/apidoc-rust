#![cfg(feature = "debug-amqp")]
//! P3 AMQP：配置校验（无 broker）+ `APIDOC_RABBITMQ_URL` 门控的
//! publish/declare/consume 端到端测试；未设置环境变量时干净跳过。

use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::DebugConfig;
use lapin::{
    options::{QueueDeclareOptions, QueueDeleteOptions},
    types::FieldTable,
    uri::{AMQPScheme, AMQPUri},
    Connection, ConnectionProperties,
};
use std::collections::BTreeMap;

fn req(target: &str, opts: &[(&str, &str)], body: Option<&str>) -> DebugRequest {
    DebugRequest {
        kind: "amqp".into(),
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

fn pairs<'a>(items: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    items.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

#[tokio::test]
async fn amqp_consume_without_queue_is_reported() {
    let err = execute(
        &allow_private(),
        &req("amqp://127.0.0.1:1", &[("consume", "true")], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("queue"), "{err}");
}

#[tokio::test]
async fn amqp_publish_without_routing_key_is_reported() {
    let err = execute(
        &allow_private(),
        &req("amqp://127.0.0.1:1", &[], Some("x")),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("routing_key"), "{err}");
}

#[tokio::test]
async fn amqp_target_credentials_are_rejected() {
    let err = execute(
        &allow_private(),
        &req(
            "amqp://user:topsecret@127.0.0.1:5672",
            &[("routing_key", "q")],
            Some("x"),
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("via opts"), "{err}");
    assert!(!err.contains("topsecret"), "{err}");
}

#[tokio::test]
async fn amqp_loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("amqp://127.0.0.1:5672", &[("routing_key", "q")], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}

#[tokio::test]
async fn amqp_publish_and_consume() {
    let Ok(url) = std::env::var("APIDOC_RABBITMQ_URL") else {
        eprintln!("skipping: APIDOC_RABBITMQ_URL not set");
        return;
    };
    let parsed: AMQPUri = url.parse().expect("invalid APIDOC_RABBITMQ_URL");
    let scheme = match parsed.scheme {
        AMQPScheme::AMQPS => "amqps",
        AMQPScheme::AMQP => "amqp",
    };
    let target = format!(
        "{scheme}://{}:{}",
        parsed.authority.host, parsed.authority.port
    );
    let queue = format!(
        "apidoc-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let connection = Connection::connect_uri(parsed.clone(), ConnectionProperties::default())
        .await
        .expect("connect to APIDOC_RABBITMQ_URL");
    let channel = connection.create_channel().await.unwrap();
    channel
        .queue_declare(
            &queue,
            QueueDeclareOptions::default(),
            FieldTable::default(),
        )
        .await
        .unwrap();

    let auth = [
        ("username", parsed.authority.userinfo.username.clone()),
        ("password", parsed.authority.userinfo.password.clone()),
        ("vhost", parsed.vhost.clone()),
        ("queue", queue.clone()),
    ];
    let publish_opts = [
        ("consume", "false"),
        ("routing_key", queue.as_str()),
        ("username", auth[0].1.as_str()),
        ("password", auth[1].1.as_str()),
        ("vhost", auth[2].1.as_str()),
    ];
    let published = execute(
        &allow_private(),
        &req(&target, &publish_opts, Some("hello")),
        None,
    )
    .await
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&published.body.data).unwrap();
    assert_eq!(v["published"], true);

    let mut consume_opts = pairs(&auth);
    consume_opts.push(("consume", "true"));
    consume_opts.push(("messages", "1"));
    consume_opts.push(("duration_ms", "5000"));
    let consumed = execute(&allow_private(), &req(&target, &consume_opts, None), None)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&consumed.body.data).unwrap();
    assert_eq!(v["delivery"]["payload"]["data"], "hello");
    assert_eq!(v["delivery"]["routing_key"], queue);
    assert_eq!(v["delivery"]["exchange"], "");

    let _ = channel
        .queue_delete(&queue, QueueDeleteOptions::default())
        .await;
    let _ = channel.close(200, "done").await;
    let _ = connection.close(200, "done").await;
}
