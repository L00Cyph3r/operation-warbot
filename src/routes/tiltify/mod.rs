use crate::SharedAppState;
use crate::routes::webhook::{Amount, TiltifyWebhookRequest};
use axum::Router;
use axum::routing::post;
use serde_derive::{Deserialize, Serialize};
use crate::routes::tiltify::TiltifyEventType::Other;

pub mod webhook;

pub fn router() -> Router<SharedAppState> {
    Router::new().route("/webhook", post(webhook::handler))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TiltifyEventType {
    DonationUpdated,
    Other
}

impl From<String> for TiltifyEventType {
    fn from(value: String) -> Self {
        match value.as_str() {
            "public:direct:donation_updated" => TiltifyEventType::DonationUpdated,
            "private:direct:donation_updated" => TiltifyEventType::DonationUpdated,
            "public:indirect:donation_updated" => TiltifyEventType::DonationUpdated,
            "private:indirect:donation_updated" => TiltifyEventType::DonationUpdated,
            _ => Other
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TiltifyDonation {
    pub event_type: TiltifyEventType,
    pub amount: Amount,
    pub name: Option<String>,
    pub message: Option<String>,
}

impl From<TiltifyWebhookRequest> for TiltifyDonation {
    fn from(value: TiltifyWebhookRequest) -> Self {
        Self {
            event_type: TiltifyEventType::from(value.meta.event_type),
            amount: value.data.amount,
            name: value.data.donor_name,
            message: value.data.donor_comment,
        }
    }
}
