use crate::Commands;
use crate::routes::webhook::Amount;
use eyre::{Context, Report};
use oauth2::basic::{BasicErrorResponse, BasicTokenResponse, BasicTokenType};
use oauth2::http::Error;
use oauth2::url::Url;
use oauth2::{
    AccessToken, AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken,
    EndpointNotSet, EndpointSet, HttpClientError, RedirectUrl, RefreshToken, RefreshTokenRequest,
    RequestTokenError, Scope, TokenResponse, TokenUrl,
};
use reqwest::header::AUTHORIZATION;
use serde_derive::{Deserialize, Serialize};
use std::env;
use std::fmt::Debug;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

type BasicClient = oauth2::basic::BasicClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TiltifyUser {
    pub access_token: Option<AccessToken>,
    pub refresh_token: Option<RefreshToken>,
    pub expires_in: Duration,
    pub token_type: BasicTokenType,
}

impl TiltifyUser {
    pub fn scopes() -> Vec<Scope> {
        vec![Scope::new("public".to_string())]
    }

    //noinspection DuplicatedCode
    #[tracing::instrument(skip(self))]
    pub fn save(&self, path: impl AsRef<Path> + Debug) -> Result<(), Report> {
        let mut file = std::fs::File::create(path)?;
        let contents = serde_json::to_string(&self)?;

        Ok(file.write_all(contents.as_bytes())?)
    }

    //noinspection DuplicatedCode
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
}

pub struct TiltifyClient {
    pub user: TiltifyUser,
    pub tx: Sender<Commands>,
}
impl TiltifyClient {
    pub async fn start(&mut self, tx: Sender<Commands>) -> Result<(), Report> {
        debug!("Starting Tiltify client");
        // if self.user.access_token.is_none() {
        //     self.new_user_token().await;
        // }

        // let tx = self.tx.clone();
        let refresh_token_timer = refresh_token_timer(tx);
        let listener = self.listener();

        let _ = tokio::join!(refresh_token_timer, listener);

        Ok(())
    }

    pub async fn listener(&mut self) {
        debug!("Starting listener");
        let tx = self.tx.clone();
        let mut rx = tx.subscribe();
        debug!("listener started");
        loop {
            match rx.try_recv() {
                Ok(cmd) => match cmd {
                    Commands::Shutdown => {
                        break;
                    }
                    Commands::OAuthResponse(code) => {
                        let client = oauth_client().expect("failed to create OAuth client");
                        let (authorize_url, csrf_state) = client
                            .authorize_url(CsrfToken::new_random)
                            // This example is requesting access to the user's public repos and email.
                            .add_scope(Scope::new("public".to_string()))
                            .url();

                        warn!("Open this URL in your browser:\n{authorize_url}\n");
                        let http_client = reqwest::ClientBuilder::new()
                            .redirect(reqwest::redirect::Policy::none())
                            .build()
                            .expect("failed to create HTTP client");
                        match client
                            .exchange_code(AuthorizationCode::new(code))
                            .request_async(&http_client)
                            .await
                        {
                            Ok(response) => {
                                self.set_user_from_response(&response);
                                break;
                            }
                            Err(e) => {
                                error!("Error exchanging AuthorizationCode: {:?}", e);
                                break;
                            }
                        }
                    }
                    Commands::TiltifyAuthRefresh => {
                        debug!("Received TiltifyAuthRefresh command");
                        match self.refresh_tokens().await {
                            Ok(_) => {
                                info!("Tokens refreshed successfully");
                            }
                            Err(_) => {
                                error!("Failed to refresh tokens");
                            }
                        }
                    }
                    Commands::TiltifyTeamStatsRequest => match self.api_get_team_stats().await {
                        Ok(response) => {
                            let _ = response.save().await;
                            self.tx
                                .send(Commands::TiltifyTeamStatsResponse(response))
                                .expect("Failed to send message");
                        }
                        Err(e) => {
                            error!("Error getting team stats: {:?}", e);
                        }
                    },
                    _ => {}
                },
                Err(TryRecvError::Empty) => {
                    sleep(Duration::from_millis(100)).await;
                }
                Err(TryRecvError::Closed) => {
                    warn!("Broadcast channel closed");
                    break;
                }

                Err(TryRecvError::Lagged(_)) => {
                    warn!("Broadcast channel lagged");
                    break;
                }
            }
        }
    }

    async fn refresh_tokens(&mut self) -> Result<(), Report> {
        let client = oauth_client().expect("failed to create OAuth client");
        let http_client = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to create HTTP client");
        if self.user.refresh_token.is_none() {
            return Err(Report::msg("No refresh token available"));
        }
        let refresh_token = &self.user.refresh_token.clone().unwrap();
        match client
            .exchange_refresh_token(refresh_token)
            .request_async(&http_client)
            .await
        {
            Ok(res) => {
                self.set_user_from_response(&res);
                Ok(())
            }
            Err(e) => {
                error!("Failed to refresh tokens: {}", e);
                Err(e.into())
            }
        }
    }

    async fn api_get_team_stats(&self) -> Result<TiltifyTeamResponse, Report> {
        let mut headers = reqwest::header::HeaderMap::new();
        let token = self.user.access_token.clone().unwrap();
        headers.insert(AUTHORIZATION, format!("Bearer {}", token.secret()).parse()?);

        let response = reqwest::Client::new()
            .get("https://v5api.tiltify.com/api/public/teams/a32931bf-2f89-4a66-9f16-07d980cb9165")
            .headers(headers)
            .send()
            .await?;
        let response_text = response.text().await?;
        let response: TiltifyTeamResponse = serde_json::from_str(&response_text)?;
        Ok(response)
    }

    fn set_user_from_response(&mut self, response: &BasicTokenResponse) {
        self.user.access_token = Some(response.access_token().clone());
        self.user.refresh_token = Some(response.refresh_token().unwrap().clone());
        self.user.expires_in = response.expires_in().unwrap();
        self.user.token_type = response.token_type().clone();
        self.user
            .save("./tiltify.json")
            .expect("couldn't save tiltify user");

        info!("Tiltify user updated and saved");
    }
}

pub async fn refresh_token_timer(tx: Sender<Commands>) {
    debug!("Starting refresh token timer");
    let mut interval = tokio::time::interval(Duration::from_secs(1800));
    loop {
        interval.tick().await;
        {
            debug!("Sending TiltifyAuthRefresh command");
            tx.send(Commands::TiltifyAuthRefresh)
                .expect("Failed to send message");
            debug!("Sent TiltifyAuthRefresh command");
        }
    }
}

fn oauth_client() -> Result<BasicClient, Report> {
    let client_id = env::var("TILTIFY_CLIENT_ID").context("Missing TILTIFY_CLIENT_ID!")?;
    let client_secret =
        env::var("TILTIFY_CLIENT_SECRET").context("Missing TILTIFY_CLIENT_SECRET!")?;
    let redirect_url = env::var("TILTIFY_REDIRECT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:28257/oauth2/redirect".to_string());

    let auth_url = env::var("TILTIFY_AUTH_URL").unwrap_or_else(|_| {
        "https://v5api.tiltify.com/oauth/authorize?response_type=code".to_string()
    });

    let token_url = env::var("TILTIFY_TOKEN_URL")
        .unwrap_or_else(|_| "https://v5api.tiltify.com/oauth/token".to_string());

    Ok(oauth2::basic::BasicClient::new(ClientId::new(client_id))
        .set_client_secret(ClientSecret::new(client_secret))
        .set_auth_uri(
            AuthUrl::new(auth_url).context("failed to create new authorization server URL")?,
        )
        .set_token_uri(TokenUrl::new(token_url).context("failed to create new token endpoint URL")?)
        .set_redirect_uri(
            RedirectUrl::new(redirect_url).context("failed to create new redirection URL")?,
        )
        .set_auth_type(AuthType::RequestBody))
}

pub fn authorize_url() -> (Url, CsrfToken) {
    let client = oauth_client().expect("failed to create OAuth client");
    client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("public".to_string()))
        .url()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TiltifyTeamResponse {
    data: TiltifyTeamResponseData
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TiltifyTeamResponseData {
    total_amount_raised: Amount,
}

impl TiltifyTeamResponse {
    pub async fn save(&self) -> Result<(), Report> {
        let mut file =
            std::fs::File::create("./assets/tiltify_team_stats.json").expect("Failed to create file");
        let contents = serde_json::to_string(&self)?;

        Ok(file.write_all(contents.as_bytes())?)
    }
}
