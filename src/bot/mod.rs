use crate::bot::auth::{Channel, Channels, User};
use crate::config::Config;
use crate::{Commands, SharedAppState};
use chrono::Utc;
use eyre::{Report, WrapErr as _};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::time::sleep;
use tracing::{Instrument, error, info, instrument, span, warn};
use twitch_api::HelixClient;
use twitch_api::extra::AnnouncementColor;
use twitch_api::helix::chat::{
    SendChatAnnouncementBody, SendChatAnnouncementRequest, SendChatMessageBody,
    SendChatMessageRequest,
};
use twitch_oauth2::{TwitchToken, UserToken};

pub mod auth;
pub mod tiltify;

// pub twitch_id: String,
// pub twitch_name: String,
// pub channels: Vec<UserId>,
// pub refresh_token: Option<RefreshToken>,
// pub access_token: AccessToken,
// pub scopes: Vec<Scope>,
pub struct Bot {
    pub client: HelixClient<'static, reqwest::Client>,
    pub token: Arc<Mutex<UserToken>>,
    pub config: Config,
    pub channels: Channels,
    pub tx: tokio::sync::broadcast::Sender<Commands>,
    pub state: SharedAppState,
}

impl Bot {
    pub async fn start(&mut self) -> Result<(), Report> {
        match tokio::try_join!(self.refresh_token(), self.broadcast_handler()) {
            Ok(_) => {}
            Err(e) => {
                error!("{:?}", e);
            }
        }

        Ok(())
    }

    async fn refresh_token(&self) -> Result<(), Report> {
        // We check constantly if the token is valid.
        // We also need to refresh the token if it's about to be expired.
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        let span = span!(tracing::Level::INFO, "refresh_token");
        loop {
            let mut token_cloned = {
                let token_locked = self.token.lock().await;
                token_locked.clone()
            };
            let _enter = span.enter();

            interval.tick().await;

            let check_token_span = span!(tracing::Level::DEBUG, "check_token");
            {
                let _span = check_token_span.enter();
                info!("Interval ticked, checking token");
                if token_cloned.expires_in() < Duration::from_secs(3600) {
                    info!(
                        "Token expires in {} seconds, refreshing",
                        token_cloned.expires_in().as_secs()
                    );
                    token_cloned
                        .refresh_token(&self.client.clone())
                        .await
                        .wrap_err("couldn't refresh token")
                        .expect("couldn't refresh token");
                    info!(
                        "Token refreshed, new expiration is in {} seconds",
                        token_cloned.expires_in().as_secs()
                    );
                }
            }

            match token_cloned
                .validate_token(&self.client.clone())
                .await
                .wrap_err("couldn't validate token")
            {
                Ok(_) => {
                    info!(
                        "Token {} still valid, expiration is in {} seconds",
                        token_cloned.access_token,
                        token_cloned.expires_in().as_secs(),
                    );
                    *self.token.lock().await = token_cloned.clone();

                    let bot = User::from(token_cloned.clone());
                    bot.save(&self.config.storage.bot)
                        .expect("couldn't save bot");
                }
                Err(_) => {}
            };
        }
    }

    #[instrument(skip(self, client, token))]
    async fn update_channels(
        &self,
        client: &HelixClient<'_, reqwest::Client>,
        token: &UserToken,
    ) -> Result<(), Report> {
        info!("Updating channels");
        info!("Cloned the things");
        let moderated_channels = self.channels.get_moderated_channels(client, &token).await;
        let live_channels = self
            .channels
            .get_live_channels(client, &token, &moderated_channels)
            .await;

        info!("Live channels: {:?}", live_channels);
        info!("Moderated channels: {:?}", moderated_channels);
        {
            let mut state = self.state.lock().await;
            state.channels.last_update = Utc::now();
            state.channels.channels_live = live_channels;
            state.channels.channels_moderated = moderated_channels;
        }
        info!("Channels updated");
        Ok(())
    }

    async fn broadcast_handler(&self) -> Result<(), Report> {
        let mut rx = self.tx.subscribe();
        // We check constantly if the token is valid.
        // We also need to refresh the token if it's about to be expired.
        let span = span!(tracing::Level::INFO, "broadcast_handler");

        loop {
            let _span = span.enter();
            let token = {
                let token_cloned = self.token.lock().await;
                token_cloned
            };
            match rx.try_recv() {
                Ok(cmd) => match cmd {
                    Commands::Shutdown => break,
                    Commands::UpdateChannels => {
                        self.update_channels(&self.client.clone(), &token.clone())
                            .await
                            .wrap_err("couldn't update channels")?;
                    }
                    Commands::DonationReceived(donation) => {
                        info!("Donation received: {:#?}", donation);

                        let moderated_live_channels = self
                            .channels
                            .clone()
                            .get_moderated_live_channels(&self.client.clone(), &token.clone())
                            .await;
                        info!("Live and moderated channels: {:?}", moderated_live_channels);
                        let message = format!(
                            "!donation_received {} {}",
                            donation.amount.currency, donation.amount.value
                        );

                        let announcement = format!(
                            "A donation of {} {} has been made by {}!",
                            donation.amount.value,
                            donation.amount.currency,
                            donation
                                .name
                                .unwrap_or_else(|| "an anonymous user".to_string())
                        );
                        let mut channels_sent_messages_to: Vec<Channel> = Vec::new();
                        for live_channel in &moderated_live_channels {
                            match Self::send_chat_message(
                                self.client.clone(),
                                &token.clone(),
                                live_channel,
                                message.as_str(),
                            )
                            .await
                            {
                                Ok(_) => {
                                    channels_sent_messages_to.push(live_channel.clone());
                                    info!("Announcement sent to channel: {}", live_channel.name);
                                }
                                Err(e) => {
                                    error!("Error sending message: {e:?}");
                                }
                            };
                            match Self::send_chat_announcement(
                                self.client.clone(),
                                &token.clone(),
                                live_channel,
                                announcement.as_str(),
                            )
                            .await
                            {
                                Ok(_) => {
                                    channels_sent_messages_to.push(live_channel.clone());
                                    info!("Message sent to channel: {}", live_channel.name);
                                }
                                Err(e) => {
                                    error!("Error sending message: {e:?}");
                                }
                            };
                        }
                        info!(
                            "Donation message sent to {} channels. Channels were: {:?}",
                            &moderated_live_channels.len(),
                            &moderated_live_channels
                        );
                    }
                    _ => {
                        // info!("Received unknown command: {:?}", cmd);
                    }
                },
                Err(e) => match e {
                    TryRecvError::Closed => {
                        warn!("Broadcast channel closed");
                        break;
                    }
                    TryRecvError::Lagged(_) => {
                        warn!("Broadcast channel lagged");
                        break;
                    }
                    TryRecvError::Empty => {
                        sleep(Duration::from_millis(100)).await;
                    }
                },
            }
        }
        info!("broadcast_handler loop ended");
        Err(Report::msg("broadcast_handler loop ended"))
    }

    #[tracing::instrument(
        skip(client, token),
        fields(
            channel_name = channel.name.as_str(),
            channel_id = channel.user_id.as_str(),
            message = message
        )
    )]
    async fn send_chat_announcement(
        client: HelixClient<'static, reqwest::Client>,
        token: &UserToken,
        channel: &Channel,
        message: &str,
    ) -> Result<(), Report> {
        info!("Sending announcement sent to channel: {}", channel.name);
        let req = SendChatAnnouncementRequest::new(&channel.user_id, &token.user_id);
        let body = SendChatAnnouncementBody::new(message, AnnouncementColor::Orange)?;
        match client
            .req_post(req, body.clone(), token)
            .in_current_span()
            .await
        {
            Ok(r) => {
                info!("SendChatAnnouncement returned: {:?}", r);
                info!("Message sent to channel: {} {r:?}", channel.name);
            }
            Err(e) => {
                error!("Error sending message: {e:?}");
            }
        }

        info!("Announcement sent to channel: {}", channel.name);
        Ok(())
    }

    #[tracing::instrument(
        skip(client, token),
        fields(
            channel_name = channel.name.as_str(),
            channel_id = channel.user_id.as_str(),
            message = message
        )
    )]
    async fn send_chat_message(
        client: HelixClient<'static, reqwest::Client>,
        token: &UserToken,
        channel: &Channel,
        message: &str,
    ) -> Result<(), Report> {
        info!("Sending message to channel: {}", channel.name);
        let req = SendChatMessageRequest::new();
        let body = SendChatMessageBody::new(&channel.user_id, &token.user_id, message);
        match client
            .req_post(req, body.clone(), &token.clone())
            .in_current_span()
            .await
        {
            Ok(r) => {
                info!("SendChatAnnouncement returned: {:?}", r);
                info!("Message sent to channel: {} {r:?}", channel.name);
                Ok(())
            }
            Err(e) => {
                error!("Error sending message: {e:?}");
                Err(e.into())
            }
        }
    }
}
