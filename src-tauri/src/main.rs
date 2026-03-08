#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use proxy_server::proxy::ProxyEvent;
use tauri::ipc::Channel;

#[tauri::command]
async fn start_proxy(on_event: Channel<ProxyEvent>) {
    println!("start_proxy called");

    // create a channel for the proxy library to send us events
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);

    // forward any received library events to the frontend channel
    let mut on_event_clone = on_event.clone();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            // ignore send errors (frontend might have disconnected)
            let _ = on_event_clone.send(ev);
        }
    });

    // start the server; errors are reported separately below
    if let Err(e) = proxy_server::start_once(tx).await {
        let _ = on_event.send(ProxyEvent::ConnectionError(e.to_string()));
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
