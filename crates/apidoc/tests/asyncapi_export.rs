//! P6 Task 6: AsyncAPI 2.6 export — only non-HTTP streaming protocols.
#![allow(dead_code)]

use apidoc::{ApiDoc, ApidocConfig, DocEndpoint, DocEvent, DocParam};
use serde_json::json;

fn param(name: &'static str, ty: &'static str) -> DocParam {
    DocParam {
        name,
        ty,
        required: false,
        default: None,
        desc: None,
        mock: None,
        field: None,
        children: &[],
    }
}

fn event(direction: &'static str, name: &'static str, ty: &'static str) -> DocEvent {
    DocEvent { direction, name, ty, desc: None, mock: None }
}

fn ep(
    title: &str,
    url: &str,
    protocol: &str,
    opts: &[(&str, &str)],
    params: Vec<DocParam>,
    returned: Vec<DocParam>,
    events: Vec<DocEvent>,
) -> DocEndpoint {
    DocEndpoint {
        title: title.into(),
        url: url.into(),
        protocol: protocol.into(),
        protocol_opts: opts.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        params,
        returned,
        events,
        ..Default::default()
    }
}

fn fixture() -> ApiDoc {
    ApiDoc {
        config: ApidocConfig { title: "Async API".into(), ..Default::default() },
        endpoints: vec![
            ep(
                "Chat",
                "/chat",
                "websocket",
                &[("target", "wss://example.com/chat")],
                vec![param("room", "string")],
                vec![param("text", "string")],
                vec![event("out", "join", "json"), event("in", "message", "json")],
            ),
            ep(
                "Sensor",
                "/sensors",
                "mqtt",
                &[("target", "mqtt://broker:1883"), ("topic", "sensors/temp")],
                vec![],
                vec![],
                vec![event("out", "publish", "json"), event("in", "reading", "json")],
            ),
            // Two out events -> AsyncAPI `messages` array.
            ep(
                "Multi",
                "/multi",
                "ws-binary",
                &[("target", "wss://example.com/multi")],
                vec![],
                vec![],
                vec![event("out", "a", "text"), event("out", "b", "binary")],
            ),
            // HTTP and non-streaming protocols must stay out of the export.
            ep("Plain", "/api/plain", "http", &[], vec![], vec![param("ok", "bool")], vec![]),
            ep("RPC", "/rpc", "grpc", &[("service", "x.Y")], vec![], vec![], vec![]),
        ],
        apps: Vec::new(),
    }
}

#[test]
fn renders_asyncapi_2_6_only_for_streaming_protocols() {
    let v = apidoc::export::asyncapi::render(&fixture(), "2.0.0");
    let expected = json!({
        "asyncapi": "2.6.0",
        "info": { "title": "Async API", "version": "2.0.0" },
        "channels": {
            "wss://example.com/chat": {
                "publish": {
                    "operationId": "wss_example_com_chat_publish",
                    "message": {
                        "name": "join",
                        "payload": { "type": "object", "properties": { "room": { "type": "string" } } }
                    }
                },
                "subscribe": {
                    "operationId": "wss_example_com_chat_subscribe",
                    "message": {
                        "name": "message",
                        "payload": { "type": "object", "properties": { "text": { "type": "string" } } }
                    }
                }
            },
            "sensors/temp": {
                "publish": {
                    "operationId": "sensors_temp_publish",
                    "message": { "name": "publish", "payload": { "type": "object" } }
                },
                "subscribe": {
                    "operationId": "sensors_temp_subscribe",
                    "message": { "name": "reading", "payload": { "type": "object" } }
                }
            },
            "wss://example.com/multi": {
                "publish": {
                    "operationId": "wss_example_com_multi_publish",
                    "messages": [
                        { "name": "a", "payload": { "type": "string" } },
                        { "name": "b", "payload": { "type": "string", "format": "binary" } }
                    ]
                }
            }
        }
    });
    assert_eq!(v, expected);
    let channels = v["channels"].as_object().unwrap();
    assert!(!channels.contains_key("/api/plain"), "HTTP endpoints must not be exported");
    assert!(!channels.contains_key("/rpc"), "non-streaming protocols must not be exported");
}

#[test]
fn no_events_falls_back_to_returned_subscribe() {
    let doc = ApiDoc {
        config: ApidocConfig { title: "Hook".into(), ..Default::default() },
        endpoints: vec![ep(
            "Hook",
            "/hook",
            "webhook",
            &[("webhook_id", "abc")],
            vec![],
            vec![param("event_id", "string")],
            vec![],
        )],
        apps: Vec::new(),
    };
    let v = apidoc::export::asyncapi::render(&doc, "9.9.9");
    let hook = &v["channels"]["/hook"];
    assert!(hook["publish"].is_null());
    assert_eq!(hook["subscribe"]["message"]["payload"]["properties"]["event_id"]["type"], "string");
}

#[cfg(feature = "axum")]
mod route {
    use super::*;
    use apidoc::axum::{apidoc_routes, CorsConfig};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use axum::Router;
    use tower::ServiceExt;

    #[tokio::test]
    async fn export_route_serves_asyncapi_json() {
        let router = Router::new()
            .merge(apidoc_routes(ApidocConfig::default()))
            .layer(apidoc::axum::cors_layer(CorsConfig::default()));
        let res = router
            .oneshot(
                Request::builder()
                    .uri("/apidoc/export?format=asyncapi")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(res.headers()["content-type"], "application/json");
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["asyncapi"], "2.6.0");
    }
}
