mod bot;
mod config;
mod routes;

use crate::bot::Bot;
use crate::bot::auth::{Channels, User, UserError};
use crate::config::Config;
use crate::routes::tiltify::TiltifyDonation;
use axum::{
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::sync::{Mutex, broadcast};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use twitch_api::HelixClient;
use twitch_api::types::UserId;

pub type SharedAppState = Arc<Mutex<AppState>>;
pub struct AppState {
    received_donations: HashSet<TiltifyDonation>,
    tx: Sender<Commands>,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(sentry::integrations::tracing::layer())
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!(
                    "info,{}=trace,hyper_util=debug,axum_serve=debug,tungstenite=debug",
                    env!("CARGO_CRATE_NAME")
                )
                .into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let _sentry = sentry::init((
        "https://bccb4a945782b51898cbf804aba13f19@o4507225498845184.ingest.de.sentry.io/4509317367332944",
        sentry::ClientOptions {
            release: sentry::release_name!(),
            traces_sample_rate: 1.0,
            // Capture user IPs and potentially sensitive headers when using HTTP server integrations
            // see https://docs.sentry.io/platforms/rust/data-management/data-collected for more info
            send_default_pii: true,
            ..Default::default()
        },
    ));

    let config = Config::load("config.toml").expect("Failed to load config");

    let (tx, rx): (Sender<Commands>, Receiver<Commands>) = broadcast::channel(8);
    let app_state: SharedAppState = Arc::new(Mutex::new(AppState {
        received_donations: HashSet::new(),
        tx: tx.clone(),
    }));

    let http_server = {
        let config = config.clone();
        let app_state = app_state.clone();
        async move {
            let listener = tokio::net::TcpListener::bind(&config.server.to_socket_addrs())
                .await
                .unwrap();
            tracing::debug!("listening on {}", listener.local_addr().unwrap());

            let app = Router::new().merge(routes::router()).with_state(app_state);

            axum::serve(listener, app)
                // .with_graceful_shutdown(async move {
                //     tokio::signal::ctrl_c()
                //         .await
                //         .expect("failed to install CTRL+C handler");
                //     let _ = tx.send(Commands::Shutdown);
                //     tracing::info!("received CTRL+C, shutting down");
                // })
                .await
                .unwrap();
        }
    };

    let http_handle = tokio::spawn(http_server);

    let helix_client = HelixClient::default();
    let mut bot_user = User::load(&config.storage.bot).unwrap_or_else(|_| User {
        user_id: env::var("BOT_USER_ID")
            .expect("BOT_USER_ID not in environment")
            .into(),
        twitch_name: env::var("BOT_USER_NAME")
            .expect("BOT_USER_NAME not in environment")
            .into(),
        user_token: None,
        expires_in: None,
        refresh_token: None,
        access_token: None,
    });

    match bot_user.ensure_token(&helix_client).await {
        Ok(_) => {}
        Err(e) => {
            match e {
                UserError::NoTokens => bot_user.new_user_token(&helix_client).await.unwrap(),
                UserError::TokenError(_) => bot_user.new_user_token(&helix_client).await.unwrap(),
            };
            bot_user.ensure_token(&helix_client).await.unwrap();
        }
    }
    bot_user.save(Path::new(&config.storage.bot)).unwrap();
    let bot_token = Arc::new(Mutex::new(
        bot_user
            .user_token
            .clone()
            .expect("Failed to load bot token"),
    ));

    let channels = Channels::load(&config.storage.channels).unwrap_or_default();
    channels.save(&config.storage.channels).unwrap();
    let bot = Bot {
        client: HelixClient::default(),
        token: bot_token.clone(),
        config: config.clone(),
        broadcaster: UserId::new("Test".to_string()),
        channels,
        rx,
    };

    let bot_handle = bot.start();

    let _ = tokio::join!(http_handle, bot_handle);
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Commands {
    Shutdown,
    DonationReceived(TiltifyDonation),
    RaidInitiated(String),
    StreamStarted(String),
    StreamEnded(String),
}
