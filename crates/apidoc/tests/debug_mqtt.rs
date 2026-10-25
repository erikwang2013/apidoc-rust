#![cfg(feature = "debug-mqtt")]
//! P3 MQTT：本机 rumqttd broker 上验证订阅+发布、收集窗口、认证失败
//! 不回显密码；配置与 guard 校验不需要 broker。

use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::DebugConfig;
use rumqttd::{Broker, Config, ConnectionSettings, RouterConfig, ServerSettings};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

fn req(target: &str, opts: &[(&str, &str)], body: Option<&str>) -> DebugRequest {
    DebugRequest {
        kind: "mqtt".into(),
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

fn unique_topic() -> String {
    format!(
        "test/{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

/// 起一个本机 rumqttd，返回监听地址；线程随测试进程退出。
fn start_broker(auth: Option<HashMap<String, String>>) -> String {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    let mut v4 = HashMap::new();
    v4.insert(
        "1".to_string(),
        ServerSettings {
            name: "v4-1".to_string(),
            listen: addr,
            tls: None,
            next_connection_delay_ms: 0,
            connections: ConnectionSettings {
                connection_timeout_ms: 5000,
                max_payload_size: 4096,
                max_inflight_count: 100,
                auth,
                external_auth: None,
                dynamic_filters: false,
            },
        },
    );
    let config = Config {
        // RouterConfig::default() 的 max_connections 是 0，会拒绝所有连接。
        router: RouterConfig {
            max_connections: 100,
            max_outgoing_packet_count: 200,
            max_segment_size: 1_048_576,
            max_segment_count: 10,
            ..RouterConfig::default()
        },
        v4: Some(v4),
        ..Config::default()
    };
    std::thread::spawn(move || {
        let mut broker = Broker::new(config);
        broker.start().unwrap();
    });
    // 等 broker 起监听；实测用 TCP 探测连接（不发 CONNECT）会让 rumqttd
    // 0.20 的 v4 连接任务异常，因此只做固定等待。
    std::thread::sleep(Duration::from_millis(300));
    addr.to_string()
}

#[tokio::test]
async fn mqtt_subscribe_publish_and_receive() {
    let addr = start_broker(Some(HashMap::from([(
        "demo".to_string(),
        "secret".to_string(),
    )])));
    let topic = unique_topic();
    let res = execute(
        &allow_private(),
        &req(
            &format!("mqtt://{addr}"),
            &[
                ("topic", &topic),
                ("username", "demo"),
                ("password", "secret"),
                ("messages", "1"),
                ("duration_ms", "3000"),
            ],
            Some("hello"),
        ),
        None,
    )
    .await
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&res.body.data).unwrap();
    let messages = v["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["topic"], topic);
    assert_eq!(messages[0]["payload"]["data"], "hello");
    assert_eq!(messages[0]["qos"], 0);
}

#[tokio::test]
async fn mqtt_collection_window_returns_empty() {
    let addr = start_broker(None);
    let res = execute(
        &allow_private(),
        &req(
            &format!("mqtt://{addr}"),
            &[
                ("topic", "test/never"),
                ("duration_ms", "100"),
                ("messages", "1"),
            ],
            None,
        ),
        None,
    )
    .await
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&res.body.data).unwrap();
    assert!(v["messages"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn mqtt_auth_failure_does_not_echo_password() {
    let addr = start_broker(Some(HashMap::from([(
        "demo".to_string(),
        "secret".to_string(),
    )])));
    let err = execute(
        &allow_private(),
        &req(
            &format!("mqtt://{addr}"),
            &[
                ("topic", "test/auth"),
                ("username", "demo"),
                ("password", "wrongpass"),
                ("duration_ms", "1000"),
            ],
            None,
        ),
        None,
    )
    .await
    .unwrap_err();
    assert!(!err.contains("wrongpass"), "{err}");
    assert!(err.contains("mqtt"), "{err}");
}

#[tokio::test]
async fn mqtt_missing_topic_is_reported() {
    let err = execute(
        &allow_private(),
        &req("mqtt://127.0.0.1:1", &[], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("topic"), "{err}");
}

#[tokio::test]
async fn mqtt_bad_qos_is_reported() {
    let err = execute(
        &allow_private(),
        &req("mqtt://127.0.0.1:1", &[("topic", "t"), ("qos", "9")], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("qos"), "{err}");
}

#[tokio::test]
async fn mqtt_loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("mqtt://127.0.0.1:1883", &[("topic", "t")], None),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}
