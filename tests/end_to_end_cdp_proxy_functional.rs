//! End-to-end functional test:
//! proxy CONNECT -> ProxyEvent -> cdp bridge -> CDP websocket -> Network.* events.

use std::collections::HashSet;
use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use proxy_server::{
    cdp::{self},
    cdp_server,
    proxy,
};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message;

async fn read_http_response_prefix(stream: &mut tokio::net::TcpStream) -> String {
    let mut buf = vec![0u8; 4096];
    let mut total = 0usize;
    loop {
        let n = stream.read(&mut buf[total..]).await.unwrap();
        assert!(n > 0, "unexpected EOF before end of HTTP headers");
        total += n;
        let slice = &buf[..total];
        if let Some(i) = slice.windows(4).position(|w| w == b"\r\n\r\n") {
            return String::from_utf8_lossy(&slice[..i + 4]).into_owned();
        }
        assert!(total < buf.len(), "headers too large");
    }
}

#[tokio::test]
async fn connect_emits_network_events_over_cdp() {
    // Simple TCP echo server so the tunnel has something to forward.
    let echo_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_addr = echo_listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = echo_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 256];
                loop {
                    let n = match stream.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    if stream.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    // Proxy listener.
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr: SocketAddr = proxy_listener.local_addr().unwrap();
    let (proxy_tx, proxy_rx) = tokio::sync::mpsc::channel::<proxy::ProxyEvent>(256);
    let proxy_handle = tokio::spawn(async move {
        proxy::serve_on_listener(proxy_listener, proxy_tx).await;
    });

    // CDP websocket server.
    let cdp_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cdp_port = cdp_listener.local_addr().unwrap().port();
    let (cdp_tx, _cdp_rx) = broadcast::channel::<String>(256);
    let cdp_tx_for_server = cdp_tx.clone();
    let cdp_handle = tokio::spawn(async move {
        cdp_server::serve(cdp_listener, cdp_tx_for_server).await;
    });

    // Bridge proxy events -> CDP event JSON.
    let bridge_handle = tokio::spawn(async move {
        cdp::bridge(proxy_rx, cdp_tx.clone()).await;
    });

    // Connect DevTools websocket.
    let url = format!(
        "ws://127.0.0.1:{}/devtools/page/{}",
        cdp_port,
        cdp_server::TARGET_ID
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("websocket connect");

    // Enable Network domain.
    let enable_cmd = json!({
        "id": 1,
        "method": "Network.enable",
        "sessionId": "1",
        "params": {}
    });
    ws.send(Message::Text(enable_cmd.to_string().into()))
        .await
        .unwrap();

    // Wait for command reply.
    loop {
        let msg = timeout(Duration::from_secs(2), ws.next())
            .await
            .expect("timeout waiting for websocket reply")
            .expect("websocket closed");
        let msg = match msg {
            Ok(m) => m,
            Err(_) => continue,
        };
        let text = match msg {
            Message::Text(t) => t.to_string(),
            _ => continue,
        };
        let v: Value = serde_json::from_str(&text).unwrap();
        if v.get("id") == Some(&json!(1)) {
            break;
        }
    }

    // Drive a CONNECT tunnel through the proxy.
    let mut stream = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
    let connect_req = format!(
        "CONNECT {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        echo_addr, echo_addr
    );
    stream.write_all(connect_req.as_bytes()).await.unwrap();

    let headers = read_http_response_prefix(&mut stream).await;
    assert!(
        headers.contains("200"),
        "unexpected CONNECT response prefix: {headers:?}"
    );

    // Tunnel some bytes so the tunnel task runs.
    stream.write_all(b"ping").await.unwrap();
    let mut out = [0u8; 4];
    stream.read_exact(&mut out).await.unwrap();
    assert_eq!(&out, b"ping");

    // Dropping the socket should close the tunnel and cause `loadingFinished`.
    drop(stream);

    // Collect CDP Network events.
    let mut seen_methods = HashSet::<String>::new();
    let mut saw_session_scoped_events = false;

    // We expect at least requestWillBeSent and loadingFinished.
    let deadline = timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await {
                Some(Ok(msg)) => {
                    let text = match msg {
                        Message::Text(t) => t.to_string(),
                        _ => continue,
                    };
                    let v: Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
                        if method == "Network.requestWillBeSent" {
                            // Keep test focused on "event delivery".
                        }

                        seen_methods.insert(method.to_string());
                        if let Some(sid) = v.get("sessionId").and_then(|s| s.as_str()) {
                            if sid == "1" && (method == "Network.requestWillBeSent" || method == "Network.loadingFinished") {
                                saw_session_scoped_events = true;
                            }
                        }
                        if seen_methods.contains("Network.requestWillBeSent")
                            && seen_methods.contains("Network.loadingFinished")
                        {
                            break;
                        }
                    }
                }
                Some(Err(_)) => continue,
                None => break,
            }
        }
    })
    .await;
    assert!(deadline.is_ok(), "timed out waiting for Network.* events");

    assert!(
        seen_methods.contains("Network.requestWillBeSent"),
        "did not observe Network.requestWillBeSent"
    );
    assert!(
        seen_methods.contains("Network.loadingFinished"),
        "did not observe Network.loadingFinished"
    );

    assert!(
        saw_session_scoped_events,
        "did not observe sessionId-scoped Network events"
    );

    bridge_handle.abort();
    cdp_handle.abort();
    proxy_handle.abort();
}

