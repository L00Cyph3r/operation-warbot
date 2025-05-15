use crate::routes::tiltify::TiltifyDonation;
use crate::routes::webhook::TiltifyWebhookRequest;
use crate::{Commands, SharedAppState};
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use tracing::info;

pub async fn handler(
    State(state): State<SharedAppState>,
    method: Method,
    headers: HeaderMap,
    Json(json): Json<TiltifyWebhookRequest>,
) -> Result<Json<TiltifyWebhookRequest>, Response> {
    if method != Method::POST {
        return Err(StatusCode::METHOD_NOT_ALLOWED.into_response());
    }
    let cloned = {
        let cloned = state.lock().await;
        cloned
    };
    cloned
        .tx
        .send(Commands::DonationReceived(TiltifyDonation::from(
            json.clone(),
        )))
        .unwrap();
    info!(
        "Tiltify Webhook received with state {:?}",
        cloned.received_donations
    );
    if headers.get("X-Tiltify-Signature").is_none() {
        return Err(StatusCode::UNAUTHORIZED.into_response());
    }

    Ok(Json(json))
}
