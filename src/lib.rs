pub mod cdp;
pub mod cdp_server;
pub mod mitm;
pub mod proxy;

use once_cell::sync::Lazy;
use tokio::sync::{broadcast, Mutex};

static STARTED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

/// Start the MITM proxy and the CDP target server.
///
/// Blocks until the proxy listener exits. Returns an error if already running.
pub async fn start() -> Result<(), String> {
    let mut guard = STARTED.lock().await;
    if *guard {
        return Err("already running".into());
    }
    *guard = true;
    drop(guard);

    let (proxy_tx, proxy_rx) = tokio::sync::mpsc::channel::<proxy::ProxyEvent>(256);
    let (cdp_tx, _) = broadcast::channel::<String>(256);

    // Convert proxy events -> CDP Network JSON and broadcast to DevTools clients.
    tokio::spawn(cdp::bridge(proxy_rx, cdp_tx.clone()));

    // CDP target server (chrome://inspect connects here).
    let cdp_port: u16 = std::env::var("CDP_SERVER_PORT")
        .unwrap_or_else(|_| "9222".to_string())
        .parse()
        .unwrap_or(9222);
    tokio::spawn(cdp_server::start(cdp_port, cdp_tx));

    proxy::main(proxy_tx).await;
    Ok(())
}
