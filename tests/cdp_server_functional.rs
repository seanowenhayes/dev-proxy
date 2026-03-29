//! Functional tests for the CDP discovery HTTP API and WebSocket command replies.

use futures_util::StreamExt;
use proxy_server::cdp_server::{self, TARGET_ID};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

fn json_body_after_headers(raw: &str) -> &str {
    raw.find("\r\n\r\n")
        .map(|i| &raw[i + 4..])
        .expect("HTTP response with body")
}

async fn http_get_path(port: u16, path: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut buf)
        .await
        .unwrap();
    String::from_utf8(buf).unwrap()
}

#[tokio::test]
async fn json_and_version_endpoints() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, _rx) = broadcast::channel::<String>(256);
    let handle = tokio::spawn(async move { cdp_server::serve(listener, tx).await });

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    let targets_raw = http_get_path(port, "/json").await;
    let targets: Value = serde_json::from_str(json_body_after_headers(&targets_raw)).unwrap();
    let arr = targets.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], TARGET_ID);
    assert_eq!(arr[0]["title"], "dev-proxy");
    let ws_url = arr[0]["webSocketDebuggerUrl"].as_str().unwrap();
    assert!(ws_url.contains(&format!("/devtools/page/{TARGET_ID}")));

    let ver_raw = http_get_path(port, "/json/version").await;
    let ver: Value = serde_json::from_str(json_body_after_headers(&ver_raw)).unwrap();
    assert_eq!(ver["Protocol-Version"], "1.3");
    assert!(ver["webSocketDebuggerUrl"].as_str().unwrap().contains("ws://"));

    handle.abort();
}

#[tokio::test]
async fn websocket_replies_to_command_with_result() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, _rx) = broadcast::channel::<String>(256);
    let handle = tokio::spawn(async move { cdp_server::serve(listener, tx).await });

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    let url = format!("ws://127.0.0.1:{port}/devtools/page/{TARGET_ID}");
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("websocket connect");

    let cmd = r#"{"id":42,"method":"Network.enable","params":{}}"#;
    ws.send(Message::Text(cmd.into())).await.unwrap();

    let msg = ws.next().await.expect("message").unwrap();
    let text = match msg {
        Message::Text(t) => t.to_string(),
        other => panic!("expected Text, got {other:?}"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["id"], 42);
    assert_eq!(v["result"], json!({}));

    handle.abort();
}
