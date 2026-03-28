# dev-proxy

A fast, headless Rust MITM (man-in-the-middle) proxy for inspecting HTTP/HTTPS traffic. Designed for developers who want to observe what a BFF (Backend for Frontend) or any HTTP client is actually sending — without a custom UI. Traffic is surfaced directly in browser DevTools via the Chrome DevTools Protocol (CDP).

## Goal

Intercept and inspect outbound HTTP/HTTPS requests from a local service. Instead of building a bespoke UI, the proxy speaks CDP so you can use the browser's built-in **Network tab** — filterable, searchable, HAR-exportable, already familiar.

## Architecture

```
Your BFF / HTTP client
        │  (configured to use localhost:3003 as proxy)
        ▼
┌───────────────────────┐
│   dev-proxy (Rust)    │
│   MITM proxy          │
│   - HTTP CONNECT      │
│   - Tunnel forwarding │
│   - Event emission    │
└──────────┬────────────┘
           │  CDP (Chrome DevTools Protocol)
           ▼
  Browser DevTools Network tab
```

## Folder Structure

```
dev-proxy/
├── src/
│   ├── lib.rs       # Public API: start_once(), start_with_sender(), status()
│   ├── main.rs      # CLI entry point
│   └── proxy.rs     # HTTP proxy logic (CONNECT tunneling, event emission)
├── Cargo.toml
└── .env
```

## Quick Start

### Prerequisites

- Rust 1.70+

### Run

```bash
cargo run
# Proxy starts on localhost:3003
```

Configure your HTTP client or BFF to use `http://127.0.0.1:3003` as its proxy.

### Environment Variables

```
PROXY_SERVER_PORT=3003   # Port the proxy listens on (default: 3003)
```

## Events

The proxy emits structured events via a tokio channel as it runs:

| Event | Description |
|---|---|
| `Started(addr)` | Proxy is listening |
| `ConnectionAccepted(addr)` | Client connected |
| `RequestReceived { method, uri, headers }` | HTTP CONNECT request intercepted |
| `Tunnel { addr, from_client, from_server }` | Tunnel closed, bytes transferred |
| `ConnectionError(msg)` | Connection or upgrade failed |

Use `start_with_sender(tx)` to receive these in your own code.

## Roadmap

- CDP integration — expose intercepted requests in browser DevTools Network tab
- Request/response body capture (requires TLS termination / MITM cert)
- Request filtering and rewriting
- HAR export
