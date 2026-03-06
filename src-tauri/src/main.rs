#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tauri::command]
async fn start_proxy() -> Result<String, String> {
    proxy_server::start_once().map(|_| "started".to_string())
}

#[tauri::command]
async fn stop_proxy() -> Result<String, String> {
    proxy_server::stop_if_running()
        .await
        .map(|_| "stopped".to_string())
}

#[tauri::command]
fn status_proxy() -> bool {
    proxy_server::status()
}

fn main() {
    // This should be called as early in the execution of the app as possible
    #[cfg(debug_assertions)] // only enable instrumentation in development builds
    let devtools = tauri_plugin_devtools::init();

    let mut builder = tauri::Builder::default();
    #[cfg(debug_assertions)]
    {
        builder = builder.plugin(devtools);
    }
    builder
        .invoke_handler(tauri::generate_handler![
            start_proxy,
            stop_proxy,
            status_proxy
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
