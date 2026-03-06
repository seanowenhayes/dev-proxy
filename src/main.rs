use dotenv::dotenv;

#[tokio::main]
async fn main() {
    dotenv().ok();

    proxy_server::start().await;
}
