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
│   ├── lib.rs       # Public API: start()
│   ├── main.rs      # CLI entry point
│   ├── proxy.rs     # HTTP proxy logic (CONNECT tunneling, event emission)
│   ├── mitm.rs      # TLS termination, CA cert generation, HTTP parsing
│   ├── cdp.rs       # Bridge: ProxyEvent → CDP Network.* JSON
│   └── cdp_server.rs# CDP target server (chrome://inspect connects here)
├── Cargo.toml
└── .env
```

## Quick Start

### Prerequisites

- Rust 1.70+

### Run (blind tunneling — default)

```bash
cargo run
# Proxy starts on localhost:3003
```

Configure your HTTP client or BFF to use `http://127.0.0.1:3003` as its proxy.

In blind mode, CONNECT tunnels pipe raw bytes — you can see the target host:port but not the actual HTTP requests inside.

### Run (MITM mode — full HTTP visibility)

```bash
MITM_MODE=true cargo run
```

In MITM mode, the proxy terminates TLS with a dynamically generated certificate, parses the decrypted HTTP requests/responses, and re-encrypts to the real server. This gives full request/response visibility in DevTools.

**You must trust the generated CA certificate** in your client to avoid certificate warnings. The CA is generated at startup with CN `dev-proxy-mitm-ca`.

### Environment Variables

```
PROXY_SERVER_PORT=3003   # Port the proxy listens on (default: 3003)
MITM_MODE=true           # Enable full TLS termination / HTTP parsing (default: false)
CDP_SERVER_PORT=9222     # Port the CDP target server listens on (default: 9222)
```

## Events

The proxy emits structured events via a tokio channel as it runs:

| Event | Description |
|---|---|
| `Started(addr)` | Proxy is listening |
| `ConnectionAccepted(addr)` | Client connected |
| `RequestReceived { method, uri, headers }` | HTTP CONNECT request intercepted |
| `Tunnel { addr, from_client, from_server }` | Tunnel closed, bytes transferred (blind mode) |
| `MitmRequest { id, method, url, headers }` | HTTP request parsed inside TLS tunnel (MITM mode) |
| `MitmResponse { id, status, status_text, headers, body_size }` | HTTP response parsed inside TLS tunnel (MITM mode) |
| `ConnectionError(msg)` | Connection or upgrade failed |

Use `start_with_sender(tx)` to receive these in your own code.

## Roadmap

- ~~CDP integration — expose intercepted requests in browser DevTools Network tab~~
- ~~Request/response visibility (TLS termination / MITM cert)~~
- Request filtering and rewriting
- HAR export
- HTTP/2 support
