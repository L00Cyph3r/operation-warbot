use crate::Commands;
use crate::bot::auth::{Channel, Channels, User};
use crate::config::Config;
use eyre::{Report, WrapErr as _};
use reqwest::Error;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::broadcast::error::{TryRecvError};
use tokio::time::sleep;
use tracing::{Instrument, error, info, span, warn};
use twitch_api::eventsub::{Event, Message, Payload};
use twitch_api::extra::AnnouncementColor;
use twitch_api::helix::chat::{
    SendChatAnnouncementBody, SendChatAnnouncementRequest, SendChatMessageBody,
    SendChatMessageRequest,
};
use twitch_api::helix::{ClientRequestError, Request, Response};
use twitch_api::{HelixClient, eventsub};
use twitch_oauth2::{TwitchToken, UserToken};

pub mod auth;
pub mod websocket;

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
    pub broadcaster: twitch_api::types::UserId,
    pub channels: Channels,
    pub rx: tokio::sync::broadcast::Receiver<Commands>,
}

impl Bot {
    //noinspection RsUnreachableCode
    pub async fn start(&mut self) -> Result<(), Report> {
        // To make a connection to the chat we need to use a websocket connection.
        // This is a wrapper for the websocket connection that handles the reconnects and handles all messages from eventsub.

        let refresh_token = async {
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
                info!("Interval ticked, checking token");
                // let mut token = token.lock().await;
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
        };

        let mut rx = self.rx.resubscribe();
        let broadcast_handler = async {
            // We check constantly if the token is valid.
            // We also need to refresh the token if it's about to be expired.
            let span = span!(tracing::Level::INFO, "broadcast_handler");

            loop {
                let _span = span.enter();
                let token = self.token.lock().await;
                match rx.try_recv() {
                    Ok(cmd) => match cmd {
                        Commands::Shutdown => break,
                        Commands::DonationReceived(donation) => {
                            info!("Donation received: {:#?}", donation);

                            let moderated_live_channels = self
                                .channels
                                .clone()
                                .get_moderated_live_channels(&self.client.clone(), &token.clone())
                                .await;
                            info!("Live channels: {:?}", moderated_live_channels);
                            let message = format!("!donation_received {}", donation.amount.value);

                            let announcement = format!(
                                "A donation of ${} has been made by {}!",
                                donation.amount.value, donation.name.unwrap_or_else(|| "an anonymous user".to_string())
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
                                        info!(
                                            "Announcement sent to channel: {}",
                                            live_channel.name
                                        );
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
                        Commands::RaidInitiated(_) => {}
                        Commands::StreamStarted(_) => {}
                        Commands::StreamEnded(_) => {}
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
        };

        tokio::join!(refresh_token, broadcast_handler);
        // let ws = websocket.run(|e, ts| async { self.handle_event(e, ts).await });
        // futures::future::try_join(refresh_token, broadcast_handler).await?;
        Ok(())
    }

    #[tracing::instrument(skip(client))]
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

    #[tracing::instrument(skip(client))]
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
        // self.handle_client_post(res).expect("couldn't send message");
        // info!("Message sent to channel: {}", channel.name);
        // Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn handle_event(
        &self,
        event: Event,
        timestamp: twitch_api::types::Timestamp,
    ) -> Result<(), Report> {
        let token = self.token.lock().await;
        match event {
            Event::ChannelChatMessageV1(Payload {
                message: Message::Notification(payload),
                subscription,
                ..
            }) => {
                println!(
                    "[{}] {}: {}",
                    timestamp, payload.chatter_user_name, payload.message.text
                );
                if let Some(command) = payload.message.text.strip_prefix("!") {
                    let mut split_whitespace = command.split_whitespace();
                    let command = split_whitespace.next().unwrap();
                    let rest = split_whitespace.next();

                    self.command(&payload, &subscription, command, rest, &token)
                        .await?;
                }
            }
            Event::ChannelChatNotificationV1(Payload {
                message: Message::Notification(payload),
                ..
            }) => {
                println!(
                    "[{}] {}: {}",
                    timestamp,
                    match &payload.chatter {
                        eventsub::channel::chat::notification::Chatter::Chatter {
                            chatter_user_name: user,
                            ..
                        } => user.as_str(),
                        _ => "anonymous",
                    },
                    payload.message.text
                );
            }
            _ => {}
        }
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn command(
        &self,
        _payload: &eventsub::channel::ChannelChatMessageV1Payload,
        _subscription: &eventsub::EventSubscriptionInformation<
            eventsub::channel::ChannelChatMessageV1,
        >,
        command: &str,
        _rest: Option<&str>,
        _token: &UserToken,
    ) -> Result<(), Report> {
        info!("Command: {}", command);
        // if let Some(response) = self.config.command.iter().find(|c| c.trigger == command) {
        //     self.client
        //         .send_chat_message_reply(
        //             &subscription.condition.broadcaster_user_id,
        //             &subscription.condition.user_id,
        //             &payload.message_id,
        //             response
        //                 .response
        //                 .replace("{user}", payload.chatter_user_name.as_str())
        //                 .as_str(),
        //             token,
        //         )
        //         .await?;
        // }
        Ok(())
    }

    #[tracing::instrument(skip(self, res))]
    fn handle_client_post<R, D>(
        &self,
        res: Result<Response<R, D>, ClientRequestError<Error>>,
    ) -> Result<(), Report>
    where
        R: Request,
        D: serde::de::DeserializeOwned + PartialEq,
    {
        match res {
            Ok(_) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
