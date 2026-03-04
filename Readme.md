# dev-proxy

A desktop GUI application for monitoring and controlling an HTTP/HTTPS proxy server. Built with Rust (Tauri backend), React (shadcn/Tailwind frontend), and TanStack libraries for state management.

## Project Overview

This project monitors API calls flowing through a backend service (e.g., a BFF) by intercepting and logging HTTP traffic. The desktop app provides a UI to start/stop the proxy and view connection logs in real-time.

## Architecture

### High-Level Flow

```
┌─────────────────────────────────────┐
│  Frontend (React + shadcn)          │
│  - Start/stop controls              │
│  - Status display                   │
│  - Tauri API integration            │
└──────────────┬──────────────────────┘
               │ Tauri Commands
               ▼
┌─────────────────────────────────────┐
│  Tauri Backend (Rust)               │
│  - Exposes start_proxy command      │
│  - Calls into proxy-server library  │
│  - Manages app lifecycle            │
└──────────────┬──────────────────────┘
               │ Library API
               ▼
┌─────────────────────────────────────┐
│  proxy-server Library (Rust)        │
│  - HTTP proxy (CONNECT tunneling)   │
│  - Axum server for SSE logs         │
│  - Global start/stop controls       │
└─────────────────────────────────────┘
```

### Folder Structure

```
dev-proxy/
├── src/                          # Rust proxy library + CLI binary
│   ├── lib.rs                    # Public API: start_once(), stop_if_running(), status()
│   ├── main.rs                   # CLI entry point
│   ├── proxy.rs                  # HTTP proxy logic (CONNECT tunneling)
│   └── app.rs                    # Axum server for SSE + REST endpoints
├── src-tauri/                    # Tauri desktop backend
│   ├── src/main.rs               # Tauri app entry, command handlers
│   ├── tauri.conf.json           # Tauri config (v2 schema)
│   └── Cargo.toml                # Tauri dependencies
├── frontend/                     # React frontend
│   ├── src/
│   │   ├── App.tsx               # Main component (start/stop UI)
│   │   ├── main.tsx              # React DOM entry
│   │   ├── index.css             # Tailwind directives
│   │   ├── components/ui/
│   │   │   └── Button.tsx        # shadcn Button component
│   │   └── utils.ts              # Utility functions (cn())
│   ├── index.html                # Entry HTML
│   ├── vite.config.ts            # Vite + React plugin config
│   └── package.json              # Frontend dependencies
├── Cargo.toml                    # Root Rust workspace
├── package.json                  # Root scripts
└── .gitignore                    # Ignore node_modules, /target
```

### Key Components

**proxy-server (Rust Library)**
- `start_once()` — Spawns proxy and Axum app in background tasks, stores handles globally
- `stop_if_running()` — Stops running proxy and app, cleans up handles
- `status()` — Returns whether proxy is currently running
- Proxy binds to port 3003 (CONNECT tunneling)
- Axum server binds to port 3030 (SSE logs, REST endpoints)

**Tauri Backend (src-tauri)**
- Exposes three commands:
  - `start_proxy` — Calls `proxy_server::start_once()`
  - `stop_proxy` — Calls `proxy_server::stop_if_running()`
  - `status_proxy` — Calls `proxy_server::status()`
- Routes commands to frontend via IPC

**Frontend (React + shadcn)**
- React hooks for state management (useState)
- TanStack React Query for async operations (in place for future expansion)
- Shadcn Button component (customizable, styled with Tailwind)
- Calls Tauri commands via `@tauri-apps/api`
- Displays proxy status and start/stop button

## Quick Start

### 1. Install Dependencies

```bash
# Install Node dependencies for frontend
cd frontend
npm install
cd ..

# Install Tauri CLI globally (one-time); match the major version used in the Rust
# dependency (v2 at the time of writing):
#   npm install -g @tauri-apps/cli@^2
npm install -g @tauri-apps/cli
```

### 2. Start Development

Open **two terminal windows**:

**Terminal 1: Frontend dev server** (runs hot-reload on port **5174**)
```bash
npm run frontend:dev
```

**Terminal 2: Tauri desktop app** (connects to Terminal 1)
```bash
npm run tauri:dev
```

The desktop window will open automatically. Click **Start** to run the proxy, **Stop** to shut it down.

### 3. Build for macOS Distribution

```bash
# Build frontend and package as .app / .dmg
npm run frontend:build
npm run tauri:build
```

Output: `src-tauri/target/release/bundle/`

### 4. (Optional) Run Proxy CLI Without Desktop UI

```bash
cargo run
# Starts proxy on localhost:3003
# Axum server on localhost:3030
```

## Development

### Prerequisites

- Node.js 18+
- Rust 1.70+
- Xcode Command Line Tools (macOS)
- Tauri CLI: `npm install -g @tauri-apps/cli`

### Setup & Running

See **Quick Start** above for the fastest way to get up and running.

For detailed reference:

**Frontend setup**
```bash
cd frontend && npm install && cd ..
```

**Running development servers**
```bash
# Terminal 1: Frontend
npm run frontend:dev        # Runs on http://localhost:5173

# Terminal 2: Tauri
npm run tauri:dev           # Connects to frontend, opens window with hot-reload
```

**Building & packaging**
```bash
npm run frontend:build      # Vite production build -> frontend/dist
npm run tauri:build         # Creates .app and .dmg for macOS -> src-tauri/target/release/bundle
```

**Running proxy without desktop UI**
```bash
cargo run                   # CLI-only: proxy on 3003, Axum on 3030
```

## Configuration

### Environment Variables

In `.env` at repo root:
```
PROXY_SERVER_PORT=3003       # HTTP proxy listen port
AXUM_SERVER_PORT=3030        # REST/SSE server listen port
```

### Tauri Config

`src-tauri/tauri.conf.json`:
- `devUrl`: Points to frontend dev server (port 5173)
- `frontendDist`: Path to built frontend output (`../frontend/dist`)
- `identifier`: Bundle identifier (`com.proxy.app`)
- Window dimensions: 800x600

## Architecture Rationale

**Why separate `src/` and `src-tauri/`?**
- Keeps concerns isolated: library vs. desktop app
- Allows running proxy as standalone CLI or via Tauri
- Simplifies dependency management (Tauri deps don't bloat the library)
- Standard Tauri monorepo pattern

**Why Tauri over Electron?**
- Smaller bundle size (~10x smaller)
- Native Rust backend (no Node.js runtime)
- OS-level integrations cheaper
- Full Rust ecosystem for proxy logic

**Why shadcn + Tailwind?**
- Component library pattern (copy-paste, fully customizable)
- Tailwind for rapid styling
- Low overhead compared to Material or Bootstrap

## Future Enhancements

- Real-time log streaming via SSE or WebSocket
- Request/response filtering and search
- Export logs as HAR or JSON
- Settings panel (port configuration, TLS certificate pinning)
- Dark mode theme toggle
- Cross-platform native installers (Windows .msi, Linux .deb)

## Notes

- The proxy uses HTTP CONNECT method for HTTPS tunneling
- Port 3003 (proxy) binds to localhost only; adjust in `src/proxy.rs` if needed
- Tauri v2 architecture (stable as of March 2026)
- Frontend built with Vite v4 and esbuild for fast HMR
