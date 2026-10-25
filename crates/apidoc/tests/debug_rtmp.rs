#![cfg(feature = "debug-media")]
//! P5 RTMP：握手字节、AMF0 黄金向量、chunk 分块与本地 connect/play 会话。

use apidoc::debug::{execute, rtmp, DebugRequest};
use apidoc::DebugConfig;
use rtmp::Amf0;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn req(target: String, opts: &[(&str, &str)]) -> DebugRequest {
    DebugRequest {
        kind: "rtmp".into(),
        target,
        method: None,
        headers: BTreeMap::new(),
        body: None,
        opts: opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        endpoint: None,
        timeout_ms: Some(3000),
    }
}

fn cfg() -> DebugConfig {
    DebugConfig {
        allow_private_networks: true,
        ..Default::default()
    }
}

fn command(values: &[Amf0]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in values {
        value.write(&mut out);
    }
    out
}

fn object(fields: &[(&str, &str)]) -> Amf0 {
    Amf0::Object(
        fields
            .iter()
            .map(|(k, v)| (k.to_string(), Amf0::String(v.to_string())))
            .collect(),
    )
}

#[test]
fn amf0_golden_vectors() {
    assert_eq!(
        rtmp::encode_string("connect"),
        vec![0x02, 0x00, 0x07, b'c', b'o', b'n', b'n', b'e', b'c', b't']
    );
    assert_eq!(
        rtmp::encode_number(1.0),
        vec![0x00, 0x3f, 0xf0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
    );
    let encoded = rtmp::encode_object(&[("app", Amf0::String("live".into()))]);
    assert_eq!(
        encoded,
        vec![
            0x03, 0x00, 0x03, b'a', b'p', b'p', 0x02, 0x00, 0x04, b'l', b'i', b'v', b'e', 0x00,
            0x00, 0x09
        ]
    );
    let decoded = rtmp::decode_all(&encoded).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(
        decoded[0],
        Amf0::Object(BTreeMap::from([(
            "app".into(),
            Amf0::String("live".into())
        )]))
    );
    assert!(rtmp::decode_all(&[0x99]).is_err());
}

#[test]
fn chunk_writer_splits_large_payload_with_fmt3_continuation() {
    let payload = vec![0xab; 130];
    let mut out = Vec::new();
    rtmp::write_message(&mut out, 3, 20, 1, &payload, 128).unwrap();
    // fmt0 头 12 字节 + 首块 128 字节 + fmt3 续块头 1 字节 + 剩余 2 字节
    assert_eq!(out.len(), 12 + 128 + 1 + 2);
    assert_eq!(out[12], 0xab);
    assert_eq!(out[12 + 128], 0xc3);
    assert_eq!(&out[12 + 128 + 1..], &[0xab, 0xab]);
    assert!(rtmp::write_message(&mut Vec::new(), 1, 20, 0, &[], 128).is_err());
}

#[tokio::test]
async fn handshake_and_connect_command() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        server_handshake(&mut stream).await;
        let mut reader = rtmp::ChunkReader::new(1 << 20);
        let (type_id, stream_id, payload) = reader.read_message(&mut stream).await.unwrap();
        assert_eq!(type_id, 20);
        assert_eq!(stream_id, 0);
        let values = rtmp::decode_all(&payload).unwrap();
        assert_eq!(values[0], Amf0::String("connect".into()));
        assert_eq!(values[1], Amf0::Number(1.0));
        match &values[2] {
            Amf0::Object(fields) => {
                assert_eq!(fields.get("app"), Some(&Amf0::String("live".into())));
                assert!(fields.contains_key("tcUrl"), "{fields:?}");
            }
            other => panic!("connect properties must be an object, got {other:?}"),
        }

        let reply = command(&[
            Amf0::String("_result".into()),
            Amf0::Number(1.0),
            Amf0::Null,
            object(&[
                ("level", "status"),
                ("code", "NetConnection.Connect.Success"),
                ("description", "ok"),
            ]),
        ]);
        let mut out = Vec::new();
        rtmp::write_message(&mut out, 3, 20, 0, &reply, 128).unwrap();
        stream.write_all(&out).await.unwrap();
    });

    let res = execute(
        &cfg(),
        &req(format!("rtmp://{addr}/live"), &[("app", "live")]),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(value["connected"], true);
    assert_eq!(value["operation"], "connect");
}

#[tokio::test]
async fn play_creates_stream_and_reads_on_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        server_handshake(&mut stream).await;
        let mut reader = rtmp::ChunkReader::new(1 << 20);

        let (_, _, payload) = reader.read_message(&mut stream).await.unwrap();
        assert_eq!(
            rtmp::decode_all(&payload).unwrap()[0],
            Amf0::String("connect".into())
        );
        send_command(
            &mut stream,
            3,
            0,
            &[
                Amf0::String("_result".into()),
                Amf0::Number(1.0),
                Amf0::Null,
                object(&[("code", "NetConnection.Connect.Success")]),
            ],
        )
        .await;

        let (_, _, payload) = reader.read_message(&mut stream).await.unwrap();
        assert_eq!(
            rtmp::decode_all(&payload).unwrap()[0],
            Amf0::String("createStream".into())
        );
        send_command(
            &mut stream,
            3,
            0,
            &[
                Amf0::String("_result".into()),
                Amf0::Number(2.0),
                Amf0::Null,
                Amf0::Number(1.0),
            ],
        )
        .await;

        let (_, stream_id, payload) = reader.read_message(&mut stream).await.unwrap();
        assert_eq!(stream_id, 1);
        let values = rtmp::decode_all(&payload).unwrap();
        assert_eq!(values[0], Amf0::String("play".into()));
        assert_eq!(values[3], Amf0::String("live".into()));
        send_command(
            &mut stream,
            5,
            1,
            &[
                Amf0::String("onStatus".into()),
                Amf0::Number(0.0),
                Amf0::Null,
                object(&[("level", "status"), ("code", "NetStream.Play.Start")]),
            ],
        )
        .await;
    });

    let res = execute(
        &cfg(),
        &req(
            format!("rtmp://{addr}/live"),
            &[("app", "live"), ("operation", "play"), ("stream", "live")],
        ),
        None,
    )
    .await
    .unwrap();
    let value: Value = serde_json::from_str(&res.body.data).unwrap();
    assert_eq!(value["connected"], true);
    assert_eq!(value["stream"], "live");
    assert_eq!(value["status"], "NetStream.Play.Start");
}

#[tokio::test]
async fn loopback_is_denied_by_default() {
    let err = execute(
        &DebugConfig::default(),
        &req("rtmp://127.0.0.1:1935/live".into(), &[]),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("private/loopback"), "{err}");
}

/// 简单握手服务端：校验 C0/C1 与 C2=S1。
async fn server_handshake(stream: &mut TcpStream) {
    let mut c0 = [0u8; 1];
    stream.read_exact(&mut c0).await.unwrap();
    assert_eq!(c0[0], 3, "C0 version");
    let mut c1 = vec![0u8; 1536];
    stream.read_exact(&mut c1).await.unwrap();
    assert_eq!(&c1[..8], &[0u8; 8], "C1 time+zero");

    let s1 = vec![0x5au8; 1536];
    let mut out = vec![3u8];
    out.extend_from_slice(&s1);
    out.extend_from_slice(&vec![0u8; 1536]);
    stream.write_all(&out).await.unwrap();

    let mut c2 = vec![0u8; 1536];
    stream.read_exact(&mut c2).await.unwrap();
    assert_eq!(c2, s1, "C2 must echo S1");
}

async fn send_command(stream: &mut TcpStream, csid: u8, stream_id: u32, values: &[Amf0]) {
    let payload = command(values);
    let mut out = Vec::new();
    rtmp::write_message(&mut out, csid, 20, stream_id, &payload, 128).unwrap();
    stream.write_all(&out).await.unwrap();
}
