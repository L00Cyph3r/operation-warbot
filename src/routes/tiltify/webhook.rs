use crate::routes::tiltify::TiltifyDonation;
use crate::routes::webhook::TiltifyWebhookRequest;
use crate::{Commands, SharedAppState};
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::WithRejection;
use serde_json::json;
use thiserror::Error;
use tracing::info;

pub async fn handler(
    State(state): State<SharedAppState>,
    method: Method,
    WithRejection(Json(json), _): WithRejection<Json<TiltifyWebhookRequest>, ApiError>,
) -> Result<Json<TiltifyWebhookRequest>, Response> {
    if method != Method::POST {
        return Err(StatusCode::METHOD_NOT_ALLOWED.into_response());
    }
    state
        .lock()
        .await
        .tx
        .send(Commands::DonationReceived(TiltifyDonation::from(
            json.clone(),
        )))
        .expect("Failed to send message");
    info!("Tiltify Webhook received",);

    Ok(Json(json))
}

#[derive(Debug, Error)]
pub enum ApiError {
    // The `#[from]` attribute generates `From<JsonRejection> for ApiError`
    // implementation. See `thiserror` docs for more information
    #[error(transparent)]
    JsonExtractorRejection(#[from] JsonRejection),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let payload = json!({
            "message": self.to_string(),
            "origin": "with_rejection"
        });
        let code = match self {
            ApiError::JsonExtractorRejection(x) => match x {
                JsonRejection::JsonDataError(_) => StatusCode::OK,
                JsonRejection::JsonSyntaxError(_) => StatusCode::BAD_REQUEST,
                JsonRejection::MissingJsonContentType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
        };
        (code, Json(payload)).into_response()
    }
}
