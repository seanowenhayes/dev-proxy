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
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
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
}

pub async fn main(event_tx: mpsc::Sender<ProxyEvent>) {
    println!("Starting proxy...");

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

    let port: u16 = std::env::var("PROXY_SERVER_PORT")
        .unwrap_or_else(|_| "3003".to_string())
        .parse()
        .expect("Failed to parse proxy PORT");

    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    tracing::debug!("listening on {}", addr);

    let listener = TcpListener::bind(addr).await.unwrap();
    let bound = listener.local_addr().expect("bound socket has address");
    // notify caller that we've started listening (include actual port)
    let _ = event_tx.send(ProxyEvent::Started(bound)).await;
    loop {
        let (stream, peer) = listener.accept().await.unwrap();
        let io = TokioIo::new(stream);
        let hyper_service = hyper_service.clone();
        let tx = event_tx.clone();
        // report accepted connection
        let _ = tx.send(ProxyEvent::ConnectionAccepted(peer)).await;
        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection(io, hyper_service)
                .with_upgrades()
                .await
            {
                let _ = tx
                    .send(ProxyEvent::ConnectionError(format!("{err:?}")))
                    .await;
            }
        });
    }
}

async fn proxy(req: Request, event_tx: mpsc::Sender<ProxyEvent>) -> Result<Response, hyper::Error> {
    tracing::trace!(?req);

    if let Some(host_addr) = req.uri().authority().map(|auth| auth.to_string()) {
        let tx = event_tx.clone();
        tokio::task::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    if let Err(e) = tunnel(upgraded, host_addr.clone(), tx.clone()).await {
                        let _ = tx.send(ProxyEvent::ConnectionError(e.to_string())).await;
                        tracing::warn!("server io error: {}", e);
                    };
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
