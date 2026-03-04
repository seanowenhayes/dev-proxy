use dotenv::dotenv;

#[tokio::main]
async fn main() {
    dotenv().ok();

    let handles = proxy_server::start();
    let (_proxy_res, _app_res) = tokio::join!(handles.proxy, handles.app);
}
