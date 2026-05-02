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
        ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Json},
    routing::get,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

pub const TARGET_ID: &str = "dev-proxy-1";

/// Cap buffered Network events before `Network.enable` so memory stays bounded if DevTools
/// connects late.
const MAX_BUFFERED_NETWORK_EVENTS: usize = 4096;

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
        "attached": false,
        "description": "",
        "devtoolsFrontendUrl":
            format!("https://chrome-devtools-frontend.appspot.com/serve_file/@20230216/devtools.html?ws=127.0.0.1:{}/devtools/page/{}", s.port, TARGET_ID),
        "faviconUrl": "https://www.google.com/favicon.ico",
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
        "attached": false,
        "Browser": "dev-proxy/1.0",
        "Protocol-Version": "1.3",
        "webSocketDebuggerUrl":
            format!("ws://127.0.0.1:{}/devtools/page/{}", s.port, TARGET_ID),
        "devtoolsFrontendUrl":
            format!("https://chrome-devtools-frontend.appspot.com/serve_file/@20230216/devtools.html?ws=127.0.0.1:{}/devtools/page/{}", s.port, TARGET_ID),
    }))
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(s): State<Shared>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle(socket, s.tx.subscribe()))
}

fn build_cdp_command_reply(cmd: &Value) -> (Value, bool) {
    let method = cmd.get("method").and_then(|v| v.as_str());
    let network_just_enabled = method == Some("Network.enable");

    let result = match method {
        Some("Browser.getVersion") => json!({
            "protocolVersion": "1.3",
            "product": "dev-proxy",
            "revision": "1",
            "userAgent": "dev-proxy/1.0",
            "jsVersion": "1.0"
        }),
        Some("Target.attachToTarget") => json!({
            // DevTools expects a sessionId so it can scope subsequent commands/events.
            "sessionId": "1"
        }),
        _ => json!({}),
    };

    let mut reply = json!({
        "id": cmd.get("id").cloned().unwrap_or(Value::Null),
        "result": result,
    });
    if let Some(sid) = cmd.get("sessionId") {
        reply["sessionId"] = sid.clone();
    }

    (reply, network_just_enabled)
}

async fn send_json(socket: &mut WebSocket, value: &Value) -> bool {
    match serde_json::to_string(value) {
        Ok(s) => socket.send(Message::Text(Utf8Bytes::from(s))).await.is_ok(),
        Err(e) => {
            tracing::warn!("CDP reply serialization failed: {e}");
            false
        }
    }
}

async fn flush_buffered_network_events(
    socket: &mut WebSocket,
    buffered: &mut VecDeque<String>,
    active_session_id: &Option<Value>,
) -> bool {
    while let Some(json) = buffered.pop_front() {
        let to_send = if let Some(sid) = active_session_id {
            match serde_json::from_str::<Value>(&json) {
                Ok(mut v) => {
                    if let Some(obj) = v.as_object_mut() {
                        obj.entry("sessionId".to_string()).or_insert(sid.clone());
                    }
                    serde_json::to_string(&v).unwrap_or(json)
                }
                Err(_) => json,
            }
        } else {
            json
        };

        if socket
            .send(Message::Text(Utf8Bytes::from(to_send)))
            .await
            .is_err()
        {
            return false;
        }
    }
    true
}

async fn handle(mut socket: WebSocket, mut rx: broadcast::Receiver<String>) {
    // Chrome only accepts Network domain events after a successful Network.enable.
    let mut network_enabled = false;
    // If DevTools uses a sessionId, events must include it at the top level.
    let mut active_session_id: Option<Value> = None;
    let mut buffered_before_enable: VecDeque<String> = VecDeque::new();

    loop {
        tokio::select! {
            // Always service the WebSocket first when both are ready. Otherwise broadcast
            // traffic can starve command reads and Chrome will stall waiting for command results.
            biased;

            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if !handle_incoming_cdp_text(
                        &mut socket,
                        text.as_str(),
                        &mut network_enabled,
                        &mut buffered_before_enable,
                        &mut active_session_id,
                    )
                    .await
                    {
                        break;
                    }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    let s = String::from_utf8_lossy(&bytes);
                    if !handle_incoming_cdp_text(
                        &mut socket,
                        s.as_ref(),
                        &mut network_enabled,
                        &mut buffered_before_enable,
                        &mut active_session_id,
                    )
                    .await
                    {
                        break;
                    }
                }
                Some(Ok(Message::Ping(payload))) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(e)) => {
                    tracing::debug!("WebSocket recv error: {e}");
                    break;
                }
            },

            event = rx.recv() => match event {
                Ok(json) => {
                    if network_enabled {
                        let to_send = if let Some(sid) = active_session_id.clone() {
                            match serde_json::from_str::<Value>(&json) {
                                Ok(mut v) => {
                                    if let Some(obj) = v.as_object_mut() {
                                        obj.entry("sessionId".to_string())
                                            .or_insert(sid.clone());
                                    }
                                    serde_json::to_string(&v).unwrap_or(json)
                                }
                                Err(_) => json,
                            }
                        } else {
                            json
                        };
                        if socket
                            .send(Message::Text(Utf8Bytes::from(to_send)))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    } else {
                        if buffered_before_enable.len() >= MAX_BUFFERED_NETWORK_EVENTS {
                            buffered_before_enable.pop_front();
                        }
                        buffered_before_enable.push_back(json);
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            },
        }
    }
}

/// Returns `false` if the socket should be closed.
async fn handle_incoming_cdp_text(
    socket: &mut WebSocket,
    text: &str,
    network_enabled: &mut bool,
    buffered_before_enable: &mut VecDeque<String>,
    active_session_id: &mut Option<Value>,
) -> bool {
    let cmd: Value = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("non-JSON WebSocket message (ignored): {e}");
            return true;
        }
    };

    tracing::debug!("CDP command: {}", text);

    let (reply, network_just_enabled) = build_cdp_command_reply(&cmd);
    if !send_json(socket, &reply).await {
        return false;
    }

    let method = cmd.get("method").and_then(|v| v.as_str());
    if method == Some("Target.attachToTarget") {
        *active_session_id = Some(json!("1"));
    }

    if network_just_enabled {
        *network_enabled = true;
        if let Some(sid) = cmd.get("sessionId").cloned() {
            *active_session_id = Some(sid);
        }
        if !flush_buffered_network_events(socket, buffered_before_enable, active_session_id).await {
            return false;
        }
    }

    true
}
