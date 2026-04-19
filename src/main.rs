mod bot;
mod config;
mod routes;

use crate::bot::Bot;
use crate::bot::auth::{Channels, User, UserError};
use crate::config::Config;
use crate::routes::tiltify::TiltifyDonation;
use axum::Router;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::env::VarError;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast::{Receiver, Sender};
use tokio::sync::{Mutex, broadcast};
use tracing::info;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use twitch_api::HelixClient;

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
            EnvFilter::try_from_default_env()
                .or_else(|_| {
                    EnvFilter::try_new(env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
                })
                .map(|f| {
                    f.add_directive("hyper=error".parse().expect("could not make directive"))
                        .add_directive("h2=error".parse().expect("could not make directive"))
                        .add_directive("rustls=error".parse().expect("could not make directive"))
                        .add_directive(
                            "tungstenite=error"
                                .parse()
                                .expect("could not make directive"),
                        )
                        .add_directive("retainer=info".parse().expect("could not make directive"))
                        .add_directive("want=info".parse().expect("could not make directive"))
                        .add_directive("reqwest=info".parse().expect("could not make directive"))
                        .add_directive("mio=info".parse().expect("could not make directive"))
                        .add_directive(
                            format!("{}=trace", env!("CARGO_CRATE_NAME"))
                                .parse()
                                .expect("could not make directive"),
                        )
                })
                .expect("Failed to parse filter"),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_line_number(true)
                .with_file(true),
        )
        .init();

    let _sentry = match env::var("SENTRY_DSN") {
        Ok(sentry_dsn) => {
            info!("Sentry initialized");
            Some(sentry::init((
                sentry_dsn.as_str(),
                sentry::ClientOptions {
                    release: sentry::release_name!(),
                    traces_sample_rate: 1.0,
                    send_default_pii: true,
                    enable_logs: true,
                    ..Default::default()
                },
            )))
        }
        Err(e) => {
            info!("Sentry not initializing due to missing SENTRY_DSN in environment");
            None
        }
    };

    let config = Config::load("config.toml").expect("Failed to load config");

    let (tx, rx): (Sender<Commands>, Receiver<Commands>) = broadcast::channel(100);
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
                .with_graceful_shutdown(async move {
                    tokio::signal::ctrl_c()
                        .await
                        .expect("failed to install CTRL+C handler");
                    let _ = tx.send(Commands::Shutdown);
                    tracing::info!("received CTRL+C, shutting down");
                })
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
    let mut bot = Bot {
        client: HelixClient::default(),
        token: bot_token.clone(),
        config: config.clone(),
        channels: channels.clone(),
        rx,
    };
    let bot_handle = bot.start();

    // let mut bot = Bot {
    //     client: HelixClient::default(),
    //     token: bot_token.clone(),
    //     config: config.clone(),
    //     broadcaster: UserId::new("Test".to_string()),
    //     channels,
    //     rx: tx.subscribe(),
    // };
    // let bot_handle2 = tokio::spawn(async move {
    //     bot.listen().await
    // });

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
