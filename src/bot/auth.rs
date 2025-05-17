use eyre::Report;
use serde_derive::{Deserialize, Serialize};
use std::env;
use std::fmt::Debug;
use std::io::{Read, Write};
use std::path::Path;
use tracing::{error, info, Instrument};
use twitch_api::client::CompatError;
use twitch_api::helix::streams::StreamType;
use twitch_api::types::{UserId, UserName};
use twitch_api::{HelixClient, TwitchClient};
use twitch_oauth2::{AccessToken, TwitchToken};
use twitch_oauth2::AppAccessToken;
use twitch_oauth2::ClientId;
use twitch_oauth2::ClientSecret;
use twitch_oauth2::RefreshToken;
use twitch_oauth2::Scope;
use twitch_oauth2::UserToken;
use twitch_oauth2::tokens::errors::{RefreshTokenError, RetrieveTokenError, ValidationError};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Channels(pub Vec<Channel>);

impl Channels {
    #[tracing::instrument(skip(path))]

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), Report> {
        let mut file = std::fs::File::create(path)?;
        let contents = serde_json::to_string(&self)?;

        Ok(file.write_all(contents.as_bytes())?)
    }

    #[tracing::instrument(skip(path))]
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Report> {
        let mut file = std::fs::File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        match serde_json::from_str(&contents) {
            Ok(s) => Ok(s),
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(skip(self, client))]
    pub async fn get_live_channels(
        &self,
        client: &HelixClient<'_, reqwest::Client>,
        token: &UserToken,
    ) -> Vec<Channel> {
        let all: Vec<UserId> = self.0.iter().map(|c| c.user_id.clone()).collect();
        let req = twitch_api::helix::streams::get_streams::GetStreamsRequest::user_ids(all);
        match client.req_get(req, token).in_current_span().await {
            Ok(res) => res
                .data
                .iter()
                .filter(|s| s.type_ == StreamType::Live)
                .map(|s| Channel {
                    name: s.user_login.clone(),
                    user_id: s.user_id.clone(),
                })
                .collect::<Vec<Channel>>(),
            Err(e) => {
                error!("{e:?}");
                Vec::new()
            }
        }
    }

    #[tracing::instrument(skip(self, client))]
    pub async fn get_moderated_channels(
        &self,
        client: &HelixClient<'_, reqwest::Client>,
        token: &UserToken,
    ) -> Vec<Channel> {
        let req = twitch_api::helix::moderation::get_moderated_channels::GetModeratedChannelsRequest::user_id(&token.user_id);
        match client.req_get(req, token).in_current_span().await {
            Ok(res) => res
                .data
                .iter()
                .map(|s| Channel {
                    name: s.broadcaster_login.clone(),
                    user_id: s.broadcaster_id.clone(),
                })
                .collect::<Vec<Channel>>(),
            Err(e) => {
                error!("{e:?}");
                Vec::new()
            }
        }
    }
    
    pub async fn get_moderated_live_channels(
        &self,
        client: &HelixClient<'_, reqwest::Client>,
        token: &UserToken,
    ) -> Vec<Channel> {
        let (live, moderated) = tokio::join!(self.get_live_channels(client, token), self.get_moderated_channels(client, token));
        info!("Found {} live and {} moderated channels", live.len(), moderated.len());
        info!("Live: {}", live.iter().map(|c| c.name.clone().to_string()).collect::<Vec<String>>().join(" "));
        info!("Moderated: {}", moderated.iter().map(|c| c.name.clone().to_string()).collect::<Vec<String>>().join(" "));
        let mut channels: Vec<Channel> = Vec::new();
        for channel in moderated {
            if live.iter().any(|live| live.user_id.to_string() == channel.user_id.to_string()) {
                channels.push(channel);
            }
        }
        channels
    }
}

impl Into<Vec<UserId>> for Channels {
    fn into(self) -> Vec<UserId> {
        let mut channels: Vec<UserId> = Vec::new();
        for channel in self.0 {
            channels.push(channel.into());
        }
        channels
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Channel {
    pub user_id: UserId,
    pub name: UserName,
}

impl Into<UserId> for Channel {
    fn into(self) -> UserId {
        self.user_id
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Streamers(pub Vec<User>);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct User {
    pub user_id: UserId,
    pub twitch_name: UserName,
    pub access_token: Option<AccessToken>,
    pub refresh_token: Option<RefreshToken>,
    pub expires_in: Option<u64>,
    #[serde(skip)]
    pub user_token: Option<UserToken>,
}

#[derive(thiserror::Error, Debug)]
pub enum UserError {
    #[error("No tokens")]
    NoTokens,
    #[error(transparent)]
    TokenError(RetrieveTokenError<CompatError<reqwest::Error>>),
}

impl User {
    #[allow(unused)]
    pub fn user_scopes() -> Vec<Scope> {
        vec![Scope::ChannelBot, Scope::ModeratorManageAnnouncements]
    }

    #[allow(unused)]
    pub fn bot_scopes() -> Vec<Scope> {
        vec![
            Scope::UserReadChat,
            Scope::UserWriteChat,
            Scope::ChatRead,
            Scope::ChatEdit,
            Scope::ModeratorManageAnnouncements,
            Scope::ModeratorManageChatMessages,
        ]
    }

    #[tracing::instrument(skip(self))]
    pub fn save(&self, path: impl AsRef<Path> + Debug) -> Result<(), Report> {
        let mut file = std::fs::File::create(path)?;
        let contents = serde_json::to_string(&self)?;

        Ok(file.write_all(contents.as_bytes())?)
    }

    #[tracing::instrument]
    pub fn load(path: impl AsRef<Path> + Debug) -> Result<Self, Report> {
        let mut file = std::fs::File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        match serde_json::from_str(&contents) {
            Ok(s) => Ok(s),
            Err(e) => Err(e.into()),
        }
    }

    #[tracing::instrument(skip(self, client))]
    pub async fn new_user_token(
        &mut self,
        client: &HelixClient<'_, reqwest::Client>,
    ) -> Result<UserToken, Report> {
        let client_id = env::var("CLIENT_ID").expect("CLIENT_ID not set");

        let mut builder = twitch_oauth2::tokens::DeviceUserTokenBuilder::new(
            client_id.clone(),
            vec![
                Scope::UserBot,
                Scope::ChannelBot,
                Scope::UserReadChat,
                Scope::UserWriteChat,
                Scope::ModeratorManageAnnouncements,
                Scope::UserReadModeratedChannels,
            ],
        );
        let code = builder
            .start(client)
            .await
            .expect("Couldn't start DeviceCodeResponse builder");

        println!("Please go to: {}", code.verification_uri);
        let token = builder
            .wait_for_code(client, tokio::time::sleep)
            .await
            .expect("Could not get UserToken from Twitch");

        self.access_token = Some(token.access_token.clone());
        self.refresh_token = token.refresh_token.clone();
        Ok(token)
    }

    #[tracing::instrument(skip(self, client))]
    pub async fn ensure_token(
        &mut self,
        client: &HelixClient<'_, reqwest::Client>,
    ) -> Result<(), UserError> {
        let client_id = env::var("CLIENT_ID").expect("CLIENT_ID not set");
        let client_secret = env::var("CLIENT_SECRET").expect("CLIENT_SECRET not set");
        if self.access_token.is_none() && self.refresh_token.is_none() {
            return Err(UserError::NoTokens);
        }
        if self.user_token.is_none() {
            return match UserToken::from_existing_or_refresh_token(
                client,
                self.access_token.clone().unwrap(),
                self.refresh_token.clone().unwrap(),
                ClientId::new(client_id),
                ClientSecret::new(client_secret),
            )
            .await
            {
                Ok(token) => {
                    self.user_token = Some(token);
                    Ok(())
                }
                Err(rte) => {
                    match &rte {
                        RetrieveTokenError::ValidationError(ve) => match ve {
                            ValidationError::NotAuthorized => {}
                            ValidationError::RequestParseError(_) => {}
                            ValidationError::Request(_) => {}
                            ValidationError::InvalidToken(_) => {}
                            _ => {}
                        },
                        RetrieveTokenError::RefreshTokenError(re) => match re {
                            RefreshTokenError::RequestError(_) => {}
                            RefreshTokenError::RequestParseError(_) => {}
                            RefreshTokenError::NoClientSecretFound => {}
                            RefreshTokenError::NoRefreshToken => {}
                            RefreshTokenError::NoExpiration => {}
                            _ => {}
                        },
                        _ => {}
                    }
                    error!("Error refreshing token: {}", rte);
                    Err(UserError::TokenError(rte))
                }
            };
        }
        Err(UserError::NoTokens)
    }
}

impl From<UserToken> for User {
    #[tracing::instrument]
    fn from(token: UserToken) -> Self {
        Self {
            user_id: token.user_id.clone(),
            twitch_name: token.login.clone(),
            access_token: Some(token.access_token.clone()),
            refresh_token: token.refresh_token.clone(),
            expires_in: Some(token.expires_in().as_secs()),
            user_token: Some(token),
        }
    }
}

#[allow(unused)]
pub async fn get_app_token_request(
    client: &TwitchClient<'_, reqwest::Client>,
    client_id: ClientId,
    client_secret: ClientSecret,
) -> Result<AppAccessToken, ()> {
    let token =
        // here we can use the TwitchClient as a client for twitch_oauth2
        AppAccessToken::get_app_access_token(client, client_id, client_secret, Scope::all())
            .await
            .unwrap();

    Ok(token)
}
