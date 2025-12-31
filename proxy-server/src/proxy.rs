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
    body::{Body, Bytes},
    extract::Request,
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};

use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::upgrade::Upgraded;
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower::Service;
use tower::ServiceExt;

use hyper_util::rt::TokioIo;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
pub async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=trace,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let router_svc = Router::new().route("/", get(|| async { "Hello, World!" }));

    let tower_service = tower::service_fn(move |req: Request<_>| {
        let router_svc = router_svc.clone();
        let req = req.map(Body::new);
        async move {
            if req.method() == Method::CONNECT {
                proxy(req).await
            } else {
                router_svc.oneshot(req).await.map_err(|err| match err {})
            }
        }
    });

    let hyper_service = hyper::service::service_fn(move |request: Request<Incoming>| {
        tower_service.clone().call(request)
    });

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);

    let listener = TcpListener::bind(addr).await.unwrap();
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let io = TokioIo::new(stream);
        let hyper_service = hyper_service.clone();
        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection(io, hyper_service)
                .with_upgrades()
                .await
            {
                println!("Failed to serve connection: {err:?}");
            }
        });
    }
}

async fn proxy(req: Request) -> Result<Response, hyper::Error> {
    tracing::trace!(?req);

    if let Some(host_addr) = req.uri().authority().map(|auth| auth.to_string()) {
        // channel of Bytes (wrapped in Result to satisfy Body::wrap_stream error type)
        let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(64);
        let stream = ReceiverStream::new(rx);
        let body = Body::from_stream(stream);
        let response = Response::new(body);

        let tx_spawn = tx.clone();
        let host_addr_clone = host_addr.clone();
        tokio::task::spawn(async move {
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    if let Err(e) = tunnel(upgraded, host_addr_clone, tx_spawn).await {
                        // try to send the error into the stream for the client as well
                        let _ = tx
                            .send(Ok(Bytes::from(format!("server io error: {}\n", e))))
                            .await;
                        tracing::warn!("server io error: {}", e);
                    };
                }
                Err(e) => {
                    let _ = tx
                        .send(Ok(Bytes::from(format!("upgrade error: {}\n", e))))
                        .await;
                    tracing::warn!("upgrade error: {}", e)
                }
            }
        });

        Ok(response)
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
    mut tx: mpsc::Sender<Result<Bytes, Infallible>>,
) -> std::io::Result<()> {
    // notify client that we're connecting
    let _ = tx
        .send(Ok(Bytes::from(format!("connecting to {}\n", addr))))
        .await;

    let mut server = TcpStream::connect(addr).await?;
    let mut upgraded = TokioIo::new(upgraded);

    let (from_client, from_server) =
        tokio::io::copy_bidirectional(&mut upgraded, &mut server).await?;

    // send a final message into the stream
    let _ = tx
        .send(Ok(Bytes::from(format!(
            "client wrote {} bytes and received {} bytes\n",
            from_client, from_server
        ))))
        .await;

    tracing::debug!(
        "client wrote {} bytes and received {} bytes",
        from_client,
        from_server
    );

    Ok(())
}
