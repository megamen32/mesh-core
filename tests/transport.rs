#![cfg(feature = "transport")]
use mesh_core::transport::{is_direct_connect_error, validated_loopback_proxy_url, PeerTransport};
use serde_json::{json, Value};
use std::{io::{Read, Write}, net::TcpListener, thread};

#[test]
fn approved_proxy_and_no_replay_classification_keep_original_contract() {
    assert!(validated_loopback_proxy_url("http://127.0.0.1:3126/").is_ok());
    for url in ["http://example.org:3126/", "http://127.0.0.1:3126/tunnel", "http://u:p@127.0.0.1:3126/"] {
        assert!(validated_loopback_proxy_url(url).is_err());
    }
    assert!(is_direct_connect_error(&anyhow::anyhow!("send remote request: connect; errno=111")));
    assert!(!is_direct_connect_error(&anyhow::anyhow!("send remote request: timeout")));
    assert!(!is_direct_connect_error(&anyhow::anyhow!("remote HTTP status 503")));
}

// Narrow local transport units. This synthetic peer is not a fleet acceptance proof.
#[test]
fn borrowed_credentials_and_tool_arguments_are_never_retained_between_calls() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let mut received = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
            let mut request = Vec::new();
            let end = loop {
                let mut byte = [0]; stream.read_exact(&mut byte).unwrap(); request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") { break request.len(); }
                assert!(request.len() < 16384);
            };
            let headers = String::from_utf8(request).unwrap();
            let length: usize = headers.lines().find_map(|s| s.to_lowercase().strip_prefix("content-length:").map(|n| n.trim().parse().unwrap())).unwrap();
            let mut body = vec![0; length]; stream.read_exact(&mut body).unwrap();
            let payload: Value = serde_json::from_slice(&body).unwrap();
            received.push((headers, payload));
            let response = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
            let _ = end;
        }
        received
    });
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        for (token, id) in [("unit-caller-a", "original-a"), ("unit-caller-b", "original-b")] {
            let transport = PeerTransport {client: &client, peer_auth_token: Some(token), max_response_bytes: 1024, peer_timeout_ms: 3000};
            assert_eq!(transport.call_remote_direct(&url, "read_text", json!({"request_id": id, "hosts":"local"})).await.unwrap(), json!({"ok":true}));
        }
    });
    let received = server.join().unwrap();
    assert!(received[0].0.contains("Bearer unit-caller-a"));
    assert!(!received[0].0.contains("unit-caller-b"));
    assert!(received[1].0.contains("Bearer unit-caller-b"));
    assert!(!received[1].0.contains("unit-caller-a"));
    assert_eq!(received[0].1["params"]["name"], "read_text");
    assert_eq!(received[0].1["params"]["arguments"]["request_id"], "original-a");
    assert_eq!(received[1].1["params"]["arguments"]["request_id"], "original-b");
}
