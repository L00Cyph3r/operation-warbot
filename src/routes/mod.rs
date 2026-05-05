use crate::{Commands, SharedAppState};
use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive};
use axum::response::{IntoResponse, Sse};
use axum::routing::{get, post};
use futures_util::stream::{self, Stream};
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tower_http::services::ServeDir;

pub mod oauth2;
pub mod overlays;
pub mod status;
pub mod tiltify;
pub mod webhook;

pub fn router() -> Router<SharedAppState> {
    let assets_dir = "assets";
    let static_files_service = ServeDir::new(assets_dir).append_index_html_on_directories(true);
    Router::new()
        .fallback_service(static_files_service)
        .route("/", get(home_handler))
        .route("/webhook", post(tiltify::webhook::handler))
        .route("/status", get(status::handler))
        .route("/oauth2/redirect", get(crate::routes::oauth2::handler))
        .nest("/tiltify", tiltify::router())
        .route("/sse/tiltify", get(crate::routes::sse_handler))
}

pub async fn home_handler() -> impl IntoResponse {
    "I'm innocent!"
}

pub async fn sse_handler(
    State(state): State<SharedAppState>,
) -> Sse<impl Stream<Item = Result<Event, BroadcastStreamRecvError>>> {
    println!("SSE handler started");

    let state = state.lock().await;
    let rx = state.tx.subscribe();
    let stream = BroadcastStream::new(rx).map(|result| {
        result.map(|cmd| match cmd {
            Commands::DonationReceived(donation) => {
                Event::default().data(serde_json::to_string(&donation).unwrap())
            }
            Commands::TiltifyTeamStatsResponse(stats) => {
                Event::default().data(serde_json::to_string(&stats).unwrap())
            }
            _ => Event::default(),
        })
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive-text"),
    )
}
