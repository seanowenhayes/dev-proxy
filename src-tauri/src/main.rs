#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tauri::command]
fn start_proxy() -> Result<String, String> {
    proxy_server::start_once().map(|_| "started".to_string())
}

#[tauri::command]
async fn stop_proxy() -> Result<String, String> {
    proxy_server::stop_if_running().await.map(|_| "stopped".to_string())
}

#[tauri::command]
fn status_proxy() -> bool {
    proxy_server::status()
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![start_proxy, stop_proxy, status_proxy])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
