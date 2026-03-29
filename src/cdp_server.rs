//! Minimal CDP (Chrome DevTools Protocol) target server.
//!
//! Exposes:
//!   GET  /json              — target discovery list
//!   GET  /json/version      — protocol version info
//!   WS   /devtools/page/:id — DevTools WebSocket connection
//!
//! Connect via chrome://inspect → Configure → add `localhost:<CDP_PORT>`.

use axum::{
    Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Json},
    routing::get,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

pub const TARGET_ID: &str = "dev-proxy-1";

#[derive(Clone)]
struct Shared {
    tx: broadcast::Sender<String>,
    port: u16,
}

/// Bind to `port` and serve. Called by the binary at startup.
pub async fn start(port: u16, cdp_tx: broadcast::Sender<String>) {
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tracing::info!(
        "CDP server on {addr} — open chrome://inspect, click Configure, add localhost:{port}"
    );
    serve(listener, cdp_tx).await;
}

/// Serve on an already-bound listener. Used directly by integration tests so
/// they can bind on port 0 and learn the actual port before connecting.
pub async fn serve(listener: TcpListener, cdp_tx: broadcast::Sender<String>) {
    let port = listener.local_addr().unwrap().port();
    let state = Shared { tx: cdp_tx, port };
    let app = Router::new()
        .route("/json", get(targets))
        .route("/json/version", get(version))
        .route("/devtools/page/{id}", get(ws_upgrade))
        .with_state(state);
    axum::serve(listener, app).await.unwrap();
}

async fn targets(State(s): State<Shared>) -> impl IntoResponse {
    Json(json!([{
        "description": "",
        "id": TARGET_ID,
        "title": "dev-proxy",
        "type": "page",
        "url": "http://dev-proxy/",
        "webSocketDebuggerUrl":
            format!("ws://127.0.0.1:{}/devtools/page/{}", s.port, TARGET_ID),
    }]))
}

async fn version(State(s): State<Shared>) -> impl IntoResponse {
    Json(json!({
        "Browser": "dev-proxy/1.0",
        "Protocol-Version": "1.3",
        "webSocketDebuggerUrl":
            format!("ws://127.0.0.1:{}/devtools/page/{}", s.port, TARGET_ID),
    }))
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(s): State<Shared>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle(socket, s.tx.subscribe()))
}

async fn handle(mut socket: WebSocket, mut rx: broadcast::Receiver<String>) {
    loop {
        tokio::select! {
            // Commands arriving from DevTools (Network.enable, etc.)
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    // Respond to every command with an empty result so DevTools
                    // doesn't stall waiting for an acknowledgement.
                    if let Ok(cmd) = serde_json::from_str::<Value>(&text) {
                        let id = cmd.get("id").cloned().unwrap_or(Value::Null);
                        let reply =
                            serde_json::to_string(&json!({ "id": id, "result": {} })).unwrap();
                        if socket.send(Message::Text(reply.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                _ => {}
            },

            // CDP Network events produced by the proxy bridge
            event = rx.recv() => match event {
                Ok(json) => {
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            },
        }
    }
}
