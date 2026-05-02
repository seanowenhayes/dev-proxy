//! Converts `ProxyEvent`s into CDP `Network.*` domain events and broadcasts
//! them as JSON strings to any connected DevTools clients.

use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};

use crate::proxy::ProxyEvent;

static COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_id() -> String {
    COUNTER.fetch_add(1, Ordering::Relaxed).to_string()
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn emit(method: &str, params: Value) -> String {
    serde_json::to_string(&json!({ "method": method, "params": params })).unwrap()
}

/// Reads `ProxyEvent`s and forwards them as CDP Network JSON to `cdp_tx`.
/// Runs until the proxy channel closes.
pub async fn bridge(mut proxy_rx: mpsc::Receiver<ProxyEvent>, cdp_tx: broadcast::Sender<String>) {
    let mut pending: HashMap<String, VecDeque<String>> = HashMap::new();

    while let Some(ev) = proxy_rx.recv().await {
        let msg = match ev {
            ProxyEvent::RequestReceived { method, uri, headers } => {
                let id = next_id();
                let ts = now();
                let url = if method == "CONNECT" {
                    format!("https://{uri}/")
                } else {
                    uri.clone()
                };
                pending.entry(uri).or_default().push_back(id.clone());
                let cdp_headers: Value = headers
                    .into_iter()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect::<serde_json::Map<_, _>>()
                    .into();
                emit(
                    "Network.requestWillBeSent",
                    json!({
                        "requestId": id,
                        "loaderId": id,
                        "documentURL": "",
                        "request": {
                            "url": url,
                            "method": method,
                            "headers": cdp_headers,
                            "initialPriority": "High",
                            "referrerPolicy": "strict-origin-when-cross-origin",
                        },
                        "timestamp": ts,
                        "wallTime": ts,
                        "initiator": { "type": "other" },
                        "type": "Other",
                    }),
                )
            }

            ProxyEvent::Tunnel { addr, from_client, from_server } => {
                let ts = now();
                let id = match pending.get_mut(&addr).and_then(|q| q.pop_front()) {
                    Some(id) => id,
                    None => continue,
                };
                if pending.get(&addr).map_or(false, |q| q.is_empty()) {
                    pending.remove(&addr);
                }
                emit(
                    "Network.loadingFinished",
                    json!({
                        "requestId": id,
                        "timestamp": ts,
                        "encodedDataLength": from_client + from_server,
                    }),
                )
            }

            ProxyEvent::MitmRequest { id, method, url, headers } => {
                let ts = now();
                let cdp_headers: Value = headers
                    .into_iter()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect::<serde_json::Map<_, _>>()
                    .into();
                emit(
                    "Network.requestWillBeSent",
                    json!({
                        "requestId": id,
                        "loaderId": id,
                        "documentURL": url.clone(),
                        "request": {
                            "url": url,
                            "method": method,
                            "headers": cdp_headers,
                            "initialPriority": "High",
                            "referrerPolicy": "strict-origin-when-cross-origin",
                        },
                        "timestamp": ts,
                        "wallTime": ts,
                        "initiator": { "type": "other" },
                        "type": "Other",
                    }),
                )
            }

            ProxyEvent::MitmResponse { id, status, status_text, headers, body_size } => {
                let ts = now();
                let cdp_headers: Value = headers
                    .clone()
                    .into_iter()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect::<serde_json::Map<_, _>>()
                    .into();

                let response_event = emit(
                    "Network.responseReceived",
                    json!({
                        "requestId": id,
                        "loaderId": id,
                        "timestamp": ts,
                        "frameId": id,
                        "type": "Other",
                        "response": {
                            "url": "",
                            "status": status,
                            "statusText": status_text,
                            "headers": cdp_headers,
                            "mimeType": headers.get("content-type").cloned().unwrap_or_else(|| "application/octet-stream".into()),
                            "protocol": "http/1.1",
                        },
                    }),
                );
                let _ = cdp_tx.send(response_event);

                if body_size > 0 {
                    let data_event = emit(
                        "Network.dataReceived",
                        json!({
                            "requestId": id,
                            "timestamp": ts,
                            "dataLength": body_size,
                            "encodedDataLength": body_size,
                        }),
                    );
                    let _ = cdp_tx.send(data_event);
                }

                emit(
                    "Network.loadingFinished",
                    json!({
                        "requestId": id,
                        "timestamp": ts,
                        "encodedDataLength": body_size,
                    }),
                )
            }

            _ => continue,
        };

        let _ = cdp_tx.send(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::ProxyEvent;
    use std::collections::HashMap;
    use tokio::sync::{broadcast, mpsc};

    async fn run(events: Vec<ProxyEvent>) -> Vec<Value> {
        let (proxy_tx, proxy_rx) = mpsc::channel(32);
        let (cdp_tx, mut cdp_rx) = broadcast::channel(32);
        let handle = tokio::spawn(bridge(proxy_rx, cdp_tx));
        for ev in events {
            proxy_tx.send(ev).await.unwrap();
        }
        drop(proxy_tx);
        handle.await.unwrap();
        let mut out = vec![];
        while let Ok(msg) = cdp_rx.try_recv() {
            out.push(serde_json::from_str::<Value>(&msg).unwrap());
        }
        out
    }

    #[tokio::test]
    async fn connect_emits_request_will_be_sent() {
        let events = vec![ProxyEvent::RequestReceived {
            method: "CONNECT".into(),
            uri: "api.example.com:443".into(),
            headers: HashMap::new(),
        }];
        let out = run(events).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["method"], "Network.requestWillBeSent");
        assert_eq!(out[0]["params"]["request"]["method"], "CONNECT");
        assert_eq!(out[0]["params"]["request"]["url"], "https://api.example.com:443/");
    }

    #[tokio::test]
    async fn tunnel_emits_loading_finished_with_matching_id() {
        let events = vec![
            ProxyEvent::RequestReceived {
                method: "CONNECT".into(),
                uri: "api.example.com:443".into(),
                headers: HashMap::new(),
            },
            ProxyEvent::Tunnel {
                addr: "api.example.com:443".into(),
                from_client: 100,
                from_server: 200,
            },
        ];
        let out = run(events).await;
        assert_eq!(out.len(), 2);

        let request_id = out[0]["params"]["requestId"].as_str().unwrap();
        assert_eq!(out[1]["method"], "Network.loadingFinished");
        assert_eq!(out[1]["params"]["requestId"], request_id);
        assert_eq!(out[1]["params"]["encodedDataLength"], 300);
    }

    #[tokio::test]
    async fn concurrent_tunnels_to_same_host_matched_fifo() {
        let events = vec![
            ProxyEvent::RequestReceived {
                method: "CONNECT".into(),
                uri: "api.example.com:443".into(),
                headers: HashMap::new(),
            },
            ProxyEvent::RequestReceived {
                method: "CONNECT".into(),
                uri: "api.example.com:443".into(),
                headers: HashMap::new(),
            },
            ProxyEvent::Tunnel {
                addr: "api.example.com:443".into(),
                from_client: 10,
                from_server: 20,
            },
            ProxyEvent::Tunnel {
                addr: "api.example.com:443".into(),
                from_client: 30,
                from_server: 40,
            },
        ];
        let out = run(events).await;
        assert_eq!(out.len(), 4);

        let id_first = out[0]["params"]["requestId"].as_str().unwrap();
        let id_second = out[1]["params"]["requestId"].as_str().unwrap();
        assert_ne!(id_first, id_second, "each CONNECT must get its own request ID");

        assert_eq!(out[2]["params"]["requestId"], id_first);
        assert_eq!(out[2]["params"]["encodedDataLength"], 30);

        assert_eq!(out[3]["params"]["requestId"], id_second);
        assert_eq!(out[3]["params"]["encodedDataLength"], 70);
    }

    #[tokio::test]
    async fn unmatched_tunnel_produces_no_event() {
        let events = vec![ProxyEvent::Tunnel {
            addr: "ghost.example.com:443".into(),
            from_client: 1,
            from_server: 2,
        }];
        let out = run(events).await;
        assert!(out.is_empty(), "orphan Tunnel must not emit a CDP event");
    }

    #[tokio::test]
    async fn non_request_events_are_ignored() {
        use std::net::SocketAddr;
        let events = vec![
            ProxyEvent::Started("127.0.0.1:3003".parse::<SocketAddr>().unwrap()),
            ProxyEvent::ConnectionAccepted("127.0.0.1:51000".parse::<SocketAddr>().unwrap()),
            ProxyEvent::ConnectionError("something went wrong".into()),
        ];
        let out = run(events).await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn request_headers_are_forwarded() {
        let mut headers = HashMap::new();
        headers.insert("authorization".into(), "Bearer tok".into());
        headers.insert("content-type".into(), "application/json".into());
        let events = vec![ProxyEvent::RequestReceived {
            method: "CONNECT".into(),
            uri: "api.example.com:443".into(),
            headers,
        }];
        let out = run(events).await;
        assert_eq!(out[0]["params"]["request"]["headers"]["authorization"], "Bearer tok");
        assert_eq!(out[0]["params"]["request"]["headers"]["content-type"], "application/json");
    }

    #[tokio::test]
    async fn mitm_request_emits_request_will_be_sent_with_id() {
        let mut headers = HashMap::new();
        headers.insert("host".into(), "api.example.com".into());
        let events = vec![ProxyEvent::MitmRequest {
            id: "mitm-1".into(),
            method: "GET".into(),
            url: "https://api.example.com/v1/users".into(),
            headers,
        }];
        let out = run(events).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["method"], "Network.requestWillBeSent");
        assert_eq!(out[0]["params"]["requestId"], "mitm-1");
        assert_eq!(out[0]["params"]["request"]["method"], "GET");
        assert_eq!(out[0]["params"]["request"]["url"], "https://api.example.com/v1/users");
    }

    #[tokio::test]
    async fn mitm_response_emits_three_events() {
        let events = vec![ProxyEvent::MitmResponse {
            id: "mitm-1".into(),
            status: 200,
            status_text: "OK".into(),
            headers: HashMap::from([("content-type".into(), "application/json".into())]),
            body_size: 1024,
        }];
        let out = run(events).await;
        assert_eq!(out.len(), 3);

        assert_eq!(out[0]["method"], "Network.responseReceived");
        assert_eq!(out[0]["params"]["response"]["status"], 200);
        assert_eq!(out[0]["params"]["response"]["statusText"], "OK");

        assert_eq!(out[1]["method"], "Network.dataReceived");
        assert_eq!(out[1]["params"]["dataLength"], 1024);

        assert_eq!(out[2]["method"], "Network.loadingFinished");
        assert_eq!(out[2]["params"]["encodedDataLength"], 1024);
    }

    #[tokio::test]
    async fn mitm_response_no_body_skips_data_received() {
        let events = vec![ProxyEvent::MitmResponse {
            id: "mitm-1".into(),
            status: 204,
            status_text: "No Content".into(),
            headers: HashMap::new(),
            body_size: 0,
        }];
        let out = run(events).await;
        assert_eq!(out.len(), 2);

        assert_eq!(out[0]["method"], "Network.responseReceived");
        assert_eq!(out[1]["method"], "Network.loadingFinished");
    }
}
