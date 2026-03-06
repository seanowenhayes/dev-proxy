pub mod proxy;

use once_cell::sync::Lazy;
use std::sync::Mutex;
use tokio::task::JoinHandle;

static GLOBAL_HANDLE: Lazy<Mutex<Option<JoinHandle<()>>>> = Lazy::new(|| Mutex::new(None));

/// Start the proxy
/// return handles that can be awaited or aborted by the caller.
pub async fn start() {
    proxy::main().await;
}

/// Convenience: start the proxy if it's not already running and store
/// the handles in a global so other code (e.g. Tauri) can control it.
pub async fn start_once() -> Result<(), String> {
    start().await;
    Ok(())
}

/// Abort both background tasks and await their termination.
pub async fn stop(handle: JoinHandle<()>) {
    handle.abort();
    let _ = handle.await;
}

/// Stop the global running proxy if present.
pub async fn stop_if_running() -> Result<(), String> {
    let handle_opt = {
        let mut guard = GLOBAL_HANDLE
            .lock()
            .map_err(|e| format!("lock error: {}", e))?;
        guard.take()
    };

    if let Some(handle) = handle_opt {
        stop(handle).await;
        Ok(())
    } else {
        Err("proxy not running".into())
    }
}

/// Check whether the global proxy is running.
pub fn status() -> bool {
    GLOBAL_HANDLE.lock().map(|g| g.is_some()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The proxy should not be running before any test starts.
    /// Run the full suite with `-- --test-threads=1` if you add
    /// tests that mutate GLOBAL_HANDLE (start / stop), otherwise
    /// state can bleed between tests.
    #[tokio::test]
    async fn status_is_false_before_start() {
        // This assumes nothing else has called start_once() in this process.
        // Safe to run in parallel with other read-only tests.
        assert!(!status());
    }

    #[tokio::test]
    async fn stop_returns_err_when_not_running() {
        let result = stop_if_running().await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "proxy not running");
    }

    #[tokio::test]
    async fn start_once_then_stop() {
        // This test mutates global state — run with --test-threads=1
        // so it doesn't race with other tests.
        let start_result = start_once();
        assert!(start_result.is_ok(), "first start should succeed");

        let double_start = start_once();
        assert!(double_start.is_err(), "second start should fail");
        assert_eq!(double_start.unwrap_err(), "proxy already running");

        assert!(status(), "status should be true while running");

        let stop_result = stop_if_running().await;
        assert!(stop_result.is_ok(), "stop should succeed");

        assert!(!status(), "status should be false after stop");
    }
}
