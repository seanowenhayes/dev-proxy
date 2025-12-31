use dotenv::dotenv;

use crate::app::app;

mod app;
mod proxy;

#[tokio::main]
async fn main() {
    dotenv().ok();

    let app = app();

    let port: u16 = std::env::var("AXUM_SERVER_PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .expect("Failed to parse axum PORT");

    let address = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
