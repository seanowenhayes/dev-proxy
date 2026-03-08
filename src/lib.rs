pub mod proxy;

use once_cell::sync::Lazy;
use tokio::sync::Mutex;

/// Start the proxy, discarding any events.  This mirrors the previous
/// behaviour of the crate and is what the `src/main.rs` binary calls.
///
/// If you want to receive notifications about what the server is doing,
/// use [`start_with_sender`] or `start_once` instead.
pub async fn start() {
    // create a channel and drop events
    let (tx, mut rx) = tokio::sync::mpsc::channel::<proxy::ProxyEvent>(32);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    proxy::main(tx).await;
}

/// Internal helper that takes an explicit sender; used by `start_once` and
/// exposed for callers that care about the events.
pub async fn start_with_sender(event_tx: tokio::sync::mpsc::Sender<proxy::ProxyEvent>) {
    proxy::main(event_tx).await;
}

static STARTED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

/// Convenience: start the proxy if it's not already running and store
/// a flag.  Events are forwarded to the given sender.
pub async fn start_once(
    event_tx: tokio::sync::mpsc::Sender<proxy::ProxyEvent>,
) -> Result<(), String> {
    if *STARTED.lock().await {
        return Err("proxy already running".into());
    }
    start_with_sender(event_tx).await;
    *STARTED.lock().await = true;
    Ok(())
}

/// Check whether the global proxy is running.
pub async fn status() -> bool {
    *STARTED.lock().await
}
