use dotenv::dotenv;

mod app;
mod proxy;

#[tokio::main]
async fn main() {
    dotenv().ok();

    let (_proxy, _app) = tokio::join!(proxy::main(), app::main());
}
