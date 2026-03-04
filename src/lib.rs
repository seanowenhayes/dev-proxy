pub mod app;
pub mod proxy;

use once_cell::sync::Lazy;
use std::sync::Mutex;
use tokio::task::JoinHandle;

pub struct ProxyHandle {
    pub proxy: JoinHandle<()>,
    pub app: JoinHandle<()>,
}

static GLOBAL_HANDLE: Lazy<Mutex<Option<ProxyHandle>>> = Lazy::new(|| Mutex::new(None));

/// Start the proxy and the axum app in background tasks and
/// return handles that can be awaited or aborted by the caller.
pub fn start() -> ProxyHandle {
    let proxy_handle = tokio::spawn(async { proxy::main().await });
    let app_handle = tokio::spawn(async { app::main().await });

    ProxyHandle {
        proxy: proxy_handle,
        app: app_handle,
    }
}

/// Convenience: start the proxy if it's not already running and store
/// the handles in a global so other code (e.g. Tauri) can control it.
pub fn start_once() -> Result<(), String> {
    let mut guard = GLOBAL_HANDLE.lock().map_err(|e| format!("lock error: {}", e))?;
    if guard.is_some() {
        return Err("proxy already running".into());
    }

    let handles = start();
    *guard = Some(handles);
    Ok(())
}

/// Abort both background tasks and await their termination.
pub async fn stop(handle: ProxyHandle) {
    handle.proxy.abort();
    handle.app.abort();

    let _ = handle.proxy.await;
    let _ = handle.app.await;
}

/// Stop the global running proxy if present.
pub async fn stop_if_running() -> Result<(), String> {
    let handle_opt = {
        let mut guard = GLOBAL_HANDLE.lock().map_err(|e| format!("lock error: {}", e))?;
        guard.take()
    };

    if let Some(handle) = handle_opt {
        handle.proxy.abort();
        handle.app.abort();

        let _ = handle.proxy.await;
        let _ = handle.app.await;
        Ok(())
    } else {
        Err("proxy not running".into())
    }
}

/// Check whether the global proxy is running.
pub fn status() -> bool {
    GLOBAL_HANDLE
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false)
}
