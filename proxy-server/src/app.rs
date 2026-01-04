use axum::{
    Router,
    response::sse::{Event, Sse},
    routing::get,
};
use axum_extra::TypedHeader;
use futures_util::stream::{self, Stream};
use std::{convert::Infallible, time::Duration};
use tokio_stream::StreamExt as _;

use tower_http::cors::{Any, CorsLayer};

fn app() -> Router {
    Router::new()
        .route("/", get(|| async { "Hello, Axums!" }))
        .route("/sse", get(sse_handler))
        .layer(
            // see https://docs.rs/tower-http/latest/tower_http/cors/index.html
            // for more details
            //
            // pay attention that for some request types like posting content-type: application/json
            // it is required to add ".allow_headers([http::header::CONTENT_TYPE])"
            // or see this issue https://github.com/tokio-rs/axum/issues/849
            CorsLayer::new()
                .allow_origin(Any) // or specific origins
                .allow_methods(Any)
                .allow_headers(Any),
        )
}

async fn sse_handler(
    TypedHeader(user_agent): TypedHeader<headers::UserAgent>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    println!("`{}` connected", user_agent.as_str());

    // A `Stream` that repeats an event every second
    //
    // You can also create streams from tokio channels using the wrappers in
    // https://docs.rs/tokio-stream
    let stream = stream::repeat_with(|| Event::default().data("{\"message\": \"hi!\"}"))
        .map(Ok)
        .throttle(Duration::from_secs(1));

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(1))
            .text("keep-alive-text"),
    )
}

pub async fn main() {
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
