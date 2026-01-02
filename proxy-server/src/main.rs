use dotenv::dotenv;

use crate::app::app;

mod app;
mod proxy;

#[tokio::main]
async fn main() {
    dotenv().ok();
    // proxy::main().await;
    let app = app();

    let port: u16 = std::env::var("AXUM_SERVER_PORT")
        .unwrap_or_else(|_| "3030".to_string())
        .parse()
        .expect("Failed to parse axum PORT");

    let address = format!("0.0.0.0:{}", port);
    println!("Address {}", address);
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
