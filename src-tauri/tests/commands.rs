/// Integration tests for Tauri commands.
///
/// These use `tauri::test::mock_builder()` to create a headless Tauri app
/// (no real window, no Webview) and invoke commands exactly as the frontend
/// would — going through the full serialisation / dispatch stack.
///
/// Run with:
///   cd src-tauri && cargo test -- --test-threads=1
///
/// The `--test-threads=1` flag is important because the proxy-server keeps its
/// state in a process-level static; parallel tests would race on that state.
use tauri::test::{mock_builder, mock_context, noop_assets};

/// Re-export the commands so we can register them with the mock app.
/// The functions are defined in src/main.rs but are not pub — we use
/// tauri::generate_handler! which only needs them in scope at the call site.
#[path = "../src/main.rs"]
#[allow(dead_code)]
mod app;

fn build_app() -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .invoke_handler(tauri::generate_handler![
            app::start_proxy,
            app::stop_proxy,
            app::status_proxy,
        ])
        .build(mock_context(noop_assets()))
        .expect("failed to build mock app")
}

#[tokio::test]
async fn status_proxy_returns_false_on_fresh_app() {
    let app = build_app();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();

    let result = tauri::test::get_ipc_response::<bool>(
        &webview,
        tauri::webview::InvokeRequest {
            cmd: "status_proxy".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    );
    assert!(result.is_ok());
    assert!(!result.unwrap(), "proxy should not be running on startup");
}

#[tokio::test]
async fn stop_proxy_errors_when_not_running() {
    let app = build_app();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();

    // stop_proxy is an async command; invoking it when nothing is running
    // should return an Err string.
    let result = tauri::test::get_ipc_response::<String>(
        &webview,
        tauri::webview::InvokeRequest {
            cmd: "stop_proxy".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    );
    // The command returns Err("proxy not running") which Tauri serialises
    // as a rejection — get_ipc_response will itself be Err.
    assert!(result.is_err(), "stopping when not running should error");
}
