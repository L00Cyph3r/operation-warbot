use axum::response::IntoResponse;
use axum::Router;
use axum::routing::{get, post};
use crate::SharedAppState;

pub mod webhook;
pub mod tiltify;
pub mod twitch;

pub fn router() -> Router<SharedAppState> {
    Router::new()
        .route("/", get(home_handler))
        .route("/webhook", post(tiltify::webhook::handler))
        .nest("/tiltify", tiltify::router())
}

pub async fn home_handler() -> impl IntoResponse {
    "Hello, world!"
}