use crate::{ChannelsState, Commands, SharedAppState};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_derive::Deserialize;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::info;

const MAX_AGE: u64 = 30;
#[derive(Deserialize, Debug, Clone)]
pub struct StatusQueryParams {
    auth: String,
}

#[tracing::instrument(skip(state))]
pub async fn handler(
    State(state): State<SharedAppState>,
    query: Query<StatusQueryParams>,
) -> Result<Json<ChannelsState>, Response> {
    info!("Status request received");
    if query.auth != "test" {
        return Err(StatusCode::UNAUTHORIZED.into_response());
    }

    let now = chrono::Utc::now();
    let last_update = state.clone().lock().await.channels.last_update.clone();

    // If we haven't updated the channels in the last MAX_AGE seconds, update them
    if (now - last_update) > chrono::Duration::seconds(MAX_AGE as i64) {
        {
            let state_guard = state.lock().await;
            state_guard
                .tx
                .send(Commands::UpdateChannels)
                .expect("Failed to send message");
        }

        let started_update = Instant::now();
        loop {
            if started_update.elapsed() > Duration::from_secs(MAX_AGE) {
                return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
            }
            if (now - state.lock().await.channels.last_update) < chrono::Duration::seconds(10) {
                break;
            }

            sleep(Duration::from_millis(100)).await;
        }
    }

    Ok(Json(state.clone().lock().await.channels.clone()))
}
