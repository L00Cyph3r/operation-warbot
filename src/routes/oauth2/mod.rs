use crate::routes::status::StatusQueryParams;
use crate::{Commands, SharedAppState};
use axum::extract::{Query, State};
use axum::response::Response;
use tracing::info;

#[derive(Clone, serde::Deserialize)]
pub struct QueryAxumCallback {
    pub code: String,
    pub state: String,
}

pub async fn handler(
    State(state): State<SharedAppState>,
    query: Query<QueryAxumCallback>,
) -> Result<String, Response> {
    state
        .lock()
        .await
        .tx
        .send(Commands::OAuthResponse(query.code.clone()))
        .expect("Failed to send message");
    info!("OAuth2 callback received: {:?}", query.code.clone());
    Ok("Authentication done".to_string())
}
