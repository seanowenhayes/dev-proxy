pub mod proxy;

use once_cell::sync::Lazy;
use tokio::sync::Mutex;

/// Start the proxy
pub async fn start() {
    proxy::main().await;
}

static STARTED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

/// Convenience: start the proxy if it's not already running and store
pub async fn start_once() -> Result<(), String> {
    if *STARTED.lock().await {
        return Err("proxy already running".into());
    }
    start().await;
    *STARTED.lock().await = true;
    Ok(())
}

/// Check whether the global proxy is running.
pub async fn status() -> bool {
    *STARTED.lock().await
}
