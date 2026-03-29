//! Functional tests: real TCP to the proxy listener (ephemeral port, no env).

use proxy_server::proxy::{self, ProxyEvent};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

async fn read_http_response_prefix(stream: &mut TcpStream) -> String {
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
async fn get_root_returns_hello() {
    let (tx, mut rx) = mpsc::channel::<ProxyEvent>(256);
    let bind = SocketAddr::from(([127, 0, 0, 1], 0));
    let handle = tokio::spawn(async move { proxy::serve_listener(bind, tx).await });

    let addr = match rx.recv().await {
        Some(ProxyEvent::Started(a)) => a,
        other => panic!("expected Started, got {other:?}"),
    };

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(
            b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();

    let mut buf = vec![0u8; 2048];
    let n = stream.read(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf[..n]);
    assert!(text.contains("200"), "response: {text}");
    assert!(text.contains("Hello, World!"));

    handle.abort();
}

#[tokio::test]
async fn connect_tunnels_to_tcp_echo() {
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

    let (tx, mut rx) = mpsc::channel::<ProxyEvent>(256);
    let bind = SocketAddr::from(([127, 0, 0, 1], 0));
    let handle = tokio::spawn(async move { proxy::serve_listener(bind, tx).await });

    let proxy_addr = match rx.recv().await {
        Some(ProxyEvent::Started(a)) => a,
        other => panic!("expected Started, got {other:?}"),
    };

    let mut stream = TcpStream::connect(proxy_addr).await.unwrap();
    let connect_req = format!(
        "CONNECT {echo_addr} HTTP/1.1\r\nHost: {echo_addr}\r\n\r\n",
        echo_addr = echo_addr
    );
    stream.write_all(connect_req.as_bytes()).await.unwrap();

    let headers = read_http_response_prefix(&mut stream).await;
    assert!(
        headers.starts_with("HTTP/1.1 200") || headers.starts_with("HTTP/1.1 200 "),
        "unexpected CONNECT response: {headers:?}"
    );

    stream.write_all(b"ping").await.unwrap();
    let mut out = [0u8; 16];
    let n = stream.read(&mut out).await.unwrap();
    assert_eq!(&out[..n], b"ping");

    handle.abort();
}
