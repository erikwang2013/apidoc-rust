#![cfg(feature = "debug-kafka")]
//! P3 Kafka：配置/guard 校验（无 broker）+ `APIDOC_KAFKA_BROKERS` 门控的
//! produce→fetch 端到端测试；未设置环境变量时干净跳过。

use apidoc::debug::{execute, DebugRequest};
use apidoc::DebugConfig;
use rskafka::{
    chrono::{DateTime, Utc},
    client::{
        partition::{Compression, UnknownTopicHandling},
        ClientBuilder,
    },
    record::Record,
};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

fn req(target: &str, opts: &[(&str, &str)]) -> DebugRequest {
    DebugRequest {
        kind: "kafka".into(),
        target: target.into(),
        method: None,
        headers: BTreeMap::new(),
        body: None,
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
async fn kafka_missing_topic_is_reported() {
    let err = execute(&allow_private(), &req("kafka://127.0.0.1:9092", &[]), None)
        .await
        .unwrap_err();
    assert!(err.contains("topic"), "{err}");
}

#[tokio::test]
async fn kafka_bad_partition_is_reported() {
    let err = execute(
        &allow_private(),
        &req(
            "kafka://127.0.0.1:9092",
            &[("topic", "t"), ("partition", "-1")],
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("partition"), "{err}");
}

#[tokio::test]
async fn kafka_tls_target_is_rejected_explicitly() {
    let err = execute(
        &allow_private(),
        &req("kafkas://127.0.0.1:9092", &[("topic", "t")]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("plaintext"), "{err}");
}

#[tokio::test]
async fn kafka_loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("kafka://127.0.0.1:9092", &[("topic", "t")]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}

#[tokio::test]
async fn kafka_target_credentials_are_rejected() {
    let err = execute(
        &allow_private(),
        &req("kafka://user:topsecret@127.0.0.1:9092", &[("topic", "t")]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("via opts"), "{err}");
    assert!(!err.contains("topsecret"), "{err}");
}

#[tokio::test]
async fn kafka_produce_then_fetch() {
    let Ok(brokers) = std::env::var("APIDOC_KAFKA_BROKERS") else {
        eprintln!("skipping: APIDOC_KAFKA_BROKERS not set");
        return;
    };
    let topic = format!(
        "apidoc-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    tokio::time::timeout(Duration::from_secs(30), async {
        let client = ClientBuilder::new(brokers.split(',').map(|v| v.trim().to_string()).collect())
            .build()
            .await
            .expect("connect to APIDOC_KAFKA_BROKERS");
        // Retry 等 broker 自动建 topic；若 broker 关了 auto-create 会报错。
        let partition = client
            .partition_client(&topic, 0, UnknownTopicHandling::Retry)
            .await
            .expect("topic unavailable (auto-create disabled?)");
        partition
            .produce(
                vec![Record {
                    key: None,
                    value: Some(b"hello".to_vec()),
                    headers: BTreeMap::new(),
                    timestamp: DateTime::<Utc>::from(SystemTime::now()),
                }],
                Compression::NoCompression,
            )
            .await
            .expect("produce");

        let res = execute(
            &allow_private(),
            &req(
                &format!("kafka://{brokers}"),
                &[
                    ("topic", &topic),
                    ("offset", "earliest"),
                    ("messages", "1"),
                    ("duration_ms", "5000"),
                ],
            ),
            None,
        )
        .await
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&res.body.data).unwrap();
        let messages = v["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["partition"], 0);
        assert_eq!(messages[0]["payload"]["data"], "hello");
    })
    .await
    .expect("kafka integration timed out");
}
