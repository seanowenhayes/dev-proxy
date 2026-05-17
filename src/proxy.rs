//! Run with
//!
//! ```not_rust
//! $ cargo run -p example-http-proxy
//! ```
//!
//! In another terminal:
//!
//! ```not_rust
//! $ curl -v -x "127.0.0.1:3000" https://tokio.rs
//! ```
//!
//! Example is based on <https://github.com/hyperium/hyper/blob/master/examples/http_proxy.rs>

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};

use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::upgrade::Upgraded;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::io::AsyncReadExt;
use tower::Service;
use tower::ServiceExt;

use hyper_util::rt::TokioIo;

/// Events emitted by the proxy server.  The caller passes a sender so these can
/// be received asynchronously.
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum ProxyEvent {
    Started(SocketAddr),
    ConnectionAccepted(SocketAddr),
    ConnectionError(String),
    Tunnel {
        addr: String,
        from_client: u64,
        from_server: u64,
    },
    RequestReceived {
        method: String,
        uri: String,
        headers: HashMap<String, String>,
    },
    MitmRequest {
        id: String,
        method: String,
        url: String,
        headers: HashMap<String, String>,
    },
    MitmResponse {
        id: String,
        status: u16,
        status_text: String,
        headers: HashMap<String, String>,
        body_size: u64,
    },
}

pub async fn main(event_tx: mpsc::Sender<ProxyEvent>) {
    let port: u16 = std::env::var("PROXY_SERVER_PORT")
        .unwrap_or_else(|_| "3003".to_string())
        .parse()
        .expect("Failed to parse proxy PORT");

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    serve_listener(addr, event_tx).await
}

/// HTTP proxy listener: `CONNECT` tunnels and plain HTTP hits the inner router (`GET /` → hello).
///
/// Use [`SocketAddr`] with port `0` to bind an ephemeral port (integration tests). This function
/// never returns.
pub async fn serve_listener(addr: SocketAddr, event_tx: mpsc::Sender<ProxyEvent>) -> ! {
    println!("Starting proxy...");
    tracing::debug!("listening on {}", addr);
    let listener = TcpListener::bind(addr).await.unwrap();
    serve_on_listener(listener, event_tx).await
}

/// Like [`serve_listener`] but binds an already-created listener.
/// Useful for integration tests so the test can bind on port `0` and learn
/// the selected port without racing on events.
pub async fn serve_on_listener(listener: TcpListener, event_tx: mpsc::Sender<ProxyEvent>) -> ! {
    let bound = listener.local_addr().expect("bound socket has address");
    // notify caller that we've started listening (include actual port)
    let _ = event_tx.send(ProxyEvent::Started(bound)).await;
    loop {
        let (stream, peer) = listener.accept().await.unwrap();
        tracing::info!("TCP connection from {}", peer);
        let tx = event_tx.clone();
        // report accepted connection
        let _ = tx.send(ProxyEvent::ConnectionAccepted(peer)).await;
        tokio::task::spawn(async move {
            if let Err(err) = handle_connection(stream, &peer, tx).await {
                tracing::error!("Connection error from {}: {:?}", peer, err);
            }
        });
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    peer: &SocketAddr,
    event_tx: mpsc::Sender<ProxyEvent>,
) -> Result<(), String> {
    // Read first bytes to detect TLS vs HTTP
    let mut probe = [0u8; 3];
    let n = stream
        .peek(&mut probe)
        .await
        .map_err(|e| format!("peek: {e}"))?;

    // TLS ClientHello starts with 0x16 0x03
    if n >= 3 && probe[0] == 0x16 && probe[1] == 0x03 {
        // Read the full TLS record
        let mut header = [0u8; 5];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|e| format!("read tls header: {e}"))?;
        let record_len = ((header[3] as usize) << 8) | (header[4] as usize);
        let mut record = vec![0u8; 5 + record_len];
        record[..5].copy_from_slice(&header);
        stream
            .read_exact(&mut record[5..])
            .await
            .map_err(|e| format!("read tls record: {e}"))?;

        // Use rustls Acceptor to parse ClientHello
        let mut acceptor = rustls::server::Acceptor::default();
        acceptor
            .read_tls(&mut std::io::Cursor::new(&record))
            .map_err(|e| format!("read tls: {e}"))?;
        let accepted = acceptor
            .accept()
            .map_err(|e| format!("accept: {e:?}"))?
            .ok_or("no client hello")?;
        let client_hello = accepted.client_hello();
        let host: String = match client_hello.server_name() {
            Some(name) => name.to_string(),
            None => {
                tracing::debug!("TLS connection from {} without SNI - dropping", peer);
                return Ok(());
            }
        };

        tracing::info!("TLS connection from {} -> SNI: {}", peer, host);

        if is_mitm_mode() {
            mitm_tunnel_from_tls(stream, host, record, &event_tx).await?;
        } else {
            return Err("Raw TLS connections require MITM_MODE=true".into());
        }
        return Ok(());
    }

    // Normal HTTP - let hyper parse it
    let router_svc = Router::new().route("/", get(|| async { "Hello, World!" }));
    let event_tx_clone = event_tx.clone();
    let tower_service = tower::service_fn(move |req: Request<_>| {
        let router_svc = router_svc.clone();
        let tx = event_tx_clone.clone();
        let req = req.map(Body::new);
        async move {
            if req.method() == Method::CONNECT {
                proxy(req, tx.clone()).await
            } else {
                router_svc.oneshot(req).await.map_err(|err| match err {})
            }
        }
    });

    let hyper_service = hyper::service::service_fn(move |request: Request<Incoming>| {
        tower_service.clone().call(request)
    });

    let io = TokioIo::new(stream);
    http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .serve_connection(io, hyper_service)
        .with_upgrades()
        .await
        .map_err(|e| format!("http: {e}"))?;
    Ok(())
}

async fn mitm_tunnel_from_tls(
    stream: TcpStream,
    host: String,
    client_hello: Vec<u8>,
    event_tx: &mpsc::Sender<ProxyEvent>,
) -> Result<(), String> {
    let id = next_mitm_id();
    crate::mitm::mitm_handler_from_buffered(stream, client_hello, &host, &id, event_tx.clone()).await
}

fn is_mitm_mode() -> bool {
    std::env::var("MITM_MODE")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

async fn proxy(req: Request, event_tx: mpsc::Sender<ProxyEvent>) -> Result<Response, hyper::Error> {
    tracing::trace!(?req);
    let method = req.method().to_string();
    let uri = req.uri().to_string();
    let headers = req.headers().clone();
    let _ = event_tx
        .send(ProxyEvent::RequestReceived {
            method,
            uri,
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect(),
        })
        .await;
    if let Some(host_addr) = req.uri().authority().map(|auth| auth.to_string()) {
        let tx = event_tx.clone();
        tokio::task::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    if is_mitm_mode() {
                        if let Err(e) = mitm_tunnel(upgraded, host_addr.clone(), tx.clone()).await {
                            let _ = tx.send(ProxyEvent::ConnectionError(e.to_string())).await;
                            tracing::warn!("mitm error: {}", e);
                        };
                    } else {
                        if let Err(e) = tunnel(upgraded, host_addr.clone(), tx.clone()).await {
                            let _ = tx.send(ProxyEvent::ConnectionError(e.to_string())).await;
                            tracing::warn!("server io error: {}", e);
                        };
                    }
                }
                Err(e) => {
                    let _ = tx.send(ProxyEvent::ConnectionError(e.to_string())).await;
                    tracing::warn!("upgrade error: {}", e)
                }
            }
        });

        Ok(Response::new(Body::empty()))
    } else {
        tracing::warn!("CONNECT host is not socket addr: {:?}", req.uri());
        Ok((
            StatusCode::BAD_REQUEST,
            "CONNECT must be to a socket address",
        )
            .into_response())
    }
}

async fn tunnel(
    upgraded: Upgraded,
    addr: String,
    event_tx: mpsc::Sender<ProxyEvent>,
) -> std::io::Result<()> {
    let mut server = TcpStream::connect(addr.clone()).await?;
    let mut upgraded = TokioIo::new(upgraded);

    let (from_client, from_server) =
        tokio::io::copy_bidirectional(&mut upgraded, &mut server).await?;

    tracing::debug!(
        "client wrote {} bytes and received {} bytes",
        from_client,
        from_server
    );

    let _ = event_tx
        .send(ProxyEvent::Tunnel {
            addr,
            from_client,
            from_server,
        })
        .await;

    Ok(())
}

static MITM_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_mitm_id() -> String {
    MITM_COUNTER
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .to_string()
}

async fn mitm_tunnel(
    upgraded: Upgraded,
    addr: String,
    event_tx: mpsc::Sender<ProxyEvent>,
) -> std::io::Result<()> {
    let id = next_mitm_id();

    let client_io = TokioIo::new(upgraded);

    match crate::mitm::mitm_handler(client_io, &addr, &id, event_tx.clone()).await {
        Ok(_) => Ok(()),
        Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::ProxyEvent;
    use std::collections::HashMap;
    use std::net::SocketAddr;

    #[test]
    fn proxy_event_serde_roundtrip() {
        let started: SocketAddr = "127.0.0.1:3003".parse().unwrap();
        let peer: SocketAddr = "127.0.0.1:51000".parse().unwrap();
        let cases = vec![
            ProxyEvent::Started(started),
            ProxyEvent::ConnectionAccepted(peer),
            ProxyEvent::ConnectionError("eof".into()),
            ProxyEvent::Tunnel {
                addr: "example.com:443".into(),
                from_client: 10,
                from_server: 20,
            },
            ProxyEvent::RequestReceived {
                method: "GET".into(),
                uri: "http://example.com/path".into(),
                headers: HashMap::from([("host".into(), "example.com".into())]),
            },
        ];
        for ev in cases {
            let json = serde_json::to_string(&ev).unwrap();
            let back: ProxyEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(
                serde_json::to_value(&ev).unwrap(),
                serde_json::to_value(&back).unwrap()
            );
        }
    }
}
