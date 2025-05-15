use serde_derive::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TiltifyWebhookRequest {
    pub data: Data,
    pub meta: Meta,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Data {
    pub amount: Amount,
    pub campaign_id: String,
    pub cause_id: String,
    pub completed_at: String,
    pub created_at: String,
    pub donation_matches: Vec<Option<serde_json::Value>>,
    pub donor_comment: String,
    pub donor_name: String,
    pub fundraising_event_id: Option<serde_json::Value>,
    pub id: String,
    pub legacy_id: i64,
    pub poll_id: Option<serde_json::Value>,
    pub poll_option_id: Option<serde_json::Value>,
    pub reward_claims: Option<serde_json::Value>,
    pub reward_id: Option<serde_json::Value>,
    pub sustained: bool,
    pub target_id: Option<serde_json::Value>,
    pub team_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Amount {
    pub currency: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub event_type: String,
    pub attempted_at: String,
    pub generated_at: String,
    pub subscription_source_id: String,
    pub subscription_source_type: String,
}
