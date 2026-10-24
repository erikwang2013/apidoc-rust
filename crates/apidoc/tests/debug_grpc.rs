#![cfg(feature = "debug-grpc")]
//! P2 gRPC：反射/描述符、一元与服务端流。

mod helloworld {
    include!("fixtures/helloworld.rs");
}

use apidoc::debug::{execute, DebugRequest, Payload};
use apidoc::{DebugConfig, DocEndpoint, DocParam};
use base64::Engine;
use helloworld::greeter_server::{Greeter, GreeterServer};
use helloworld::{HelloReply, HelloRequest};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio_stream::wrappers::TcpListenerStream;

struct GreeterSvc;

#[tonic::async_trait]
impl Greeter for GreeterSvc {
    async fn say_hello(
        &self,
        request: tonic::Request<HelloRequest>,
    ) -> Result<tonic::Response<HelloReply>, tonic::Status> {
        let name = request.into_inner().name;
        Ok(tonic::Response::new(HelloReply {
            message: format!("Hello {name}"),
        }))
    }

    type SayHelloStreamStream =
        tokio_stream::wrappers::ReceiverStream<Result<HelloReply, tonic::Status>>;

    async fn say_hello_stream(
        &self,
        request: tonic::Request<HelloRequest>,
    ) -> Result<tonic::Response<Self::SayHelloStreamStream>, tonic::Status> {
        let name = request.into_inner().name;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            for i in 0..3 {
                let msg = HelloReply {
                    message: format!("Hello {name} #{i}"),
                };
                if tx.send(Ok(msg)).await.is_err() {
                    break;
                }
            }
        });
        Ok(tonic::Response::new(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
    }
}

async fn start_server() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(include_bytes!("fixtures/helloworld.desc"))
        .build_v1()
        .unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(reflection)
            .add_service(GreeterServer::new(GreeterSvc))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    addr
}

fn grpc_ep(method: &str) -> DocEndpoint {
    let mut opts = BTreeMap::new();
    opts.insert("service".to_string(), "helloworld.Greeter".to_string());
    opts.insert("method".to_string(), method.to_string());
    DocEndpoint {
        protocol: "grpc".into(),
        protocol_opts: opts,
        params: vec![DocParam {
            name: "name",
            ty: "string",
            required: true,
            default: None,
            desc: None,
            mock: None,
            field: Some("1"),
            children: &[],
        }],
        ..Default::default()
    }
}

fn grpc_req(target: String, opts: &[(&str, &str)], body: &str) -> DebugRequest {
    DebugRequest {
        kind: "grpc".into(),
        target,
        method: None,
        headers: BTreeMap::new(),
        body: Some(Payload {
            encoding: "utf8".into(),
            data: body.into(),
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
async fn unary_via_reflection() {
    let addr = start_server().await;
    let ep = grpc_ep("SayHello");
    let req = grpc_req(format!("http://{addr}"), &[], r#"{"name":"world"}"#);
    let res = execute(&allow_private(), &req, Some(&ep)).await.unwrap();
    let v: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(v["message"], "Hello world");
}

#[tokio::test]
async fn unary_via_descriptor_fallback() {
    let addr = start_server().await;
    let desc = base64::engine::general_purpose::STANDARD
        .encode(include_bytes!("fixtures/helloworld.desc"));
    let ep = grpc_ep("SayHello");
    let req = grpc_req(
        format!("http://{addr}"),
        &[("descriptor", desc.as_str())],
        r#"{"name":"descriptor"}"#,
    );
    let res = execute(&allow_private(), &req, Some(&ep)).await.unwrap();
    let v: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(v["message"], "Hello descriptor");
}

#[tokio::test]
async fn server_streaming_returns_frames() {
    let addr = start_server().await;
    let ep = grpc_ep("SayHelloStream");
    let req = grpc_req(
        format!("http://{addr}"),
        &[("duration_ms", "2000"), ("max_frames", "10")],
        r#"{"name":"stream"}"#,
    );
    let res = execute(&allow_private(), &req, Some(&ep)).await.unwrap();
    assert_eq!(res.frames.len(), 3, "{:?}", res.frames);
    for (i, frame) in res.frames.iter().enumerate() {
        assert_eq!(frame.dir, "in");
        assert!(frame.data.contains(&format!("Hello stream #{i}")), "{}", frame.data);
    }
}

#[tokio::test]
async fn missing_service_is_reported() {
    let req = grpc_req("http://127.0.0.1:1".into(), &[("method", "SayHello")], "{}");
    let err = execute(&allow_private(), &req, None).await.unwrap_err();
    assert!(err.contains("service"), "{err}");
}
