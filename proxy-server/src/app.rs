use axum::{Router, routing::get};

pub fn app() -> Router {
    Router::new().route("/", get(|| async { "Hello, Axums!" }))
}
