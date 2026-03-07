#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use tauri::ipc::Channel;

#[derive(Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum ProxyEvent {
    Started { addr: String },
    Stopped,
    Error { message: String },
}

#[tauri::command]
async fn start_proxy(on_event: Channel<ProxyEvent>) {
    println!("start_proxy called");
    let message = proxy_server::start_once()
        .await
        .map_err(|e| ProxyEvent::Error { message: e })
        .map(|_| ProxyEvent::Started {
            addr: "127.0.0.1:3003".to_string(),
        });
    match message {
        Ok(event) => on_event.send(event).unwrap(),
        Err(event) => on_event.send(event).unwrap(),
    }
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
        .invoke_handler(tauri::generate_handler![start_proxy])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
