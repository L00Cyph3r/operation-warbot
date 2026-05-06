mod bot;
mod config;
mod routes;

use crate::bot::tiltify::{TiltifyTeamResponse, TiltifyUser};
use crate::{
    bot::Bot,
    bot::auth::{Channel, Channels, User, UserError},
    config::Config,
    routes::tiltify::TiltifyDonation,
};
use axum::Router;
use chrono::DateTime;
use oauth2::basic::BasicTokenType;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::{env, path::Path, sync::Arc};
use tokio::{
    sync::broadcast::{Receiver, Sender},
    sync::{Mutex, broadcast},
};
use tracing::{debug, info};
use tracing_subscriber::{
    EnvFilter, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt,
};
use twitch_api::HelixClient;

pub type SharedAppState = Arc<Mutex<AppState>>;
pub struct AppState {
    tx: Sender<Commands>,
    pub channels: ChannelsState,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ChannelsState {
    pub last_update: DateTime<chrono::Utc>,
    pub channels_live: Vec<Channel>,
    pub channels_moderated: Vec<Channel>,
}

impl ChannelsState {
    pub fn set_channels_live(&mut self, channels: Vec<Channel>) {
        self.channels_live = channels;
        self.last_update = chrono::Utc::now();
    }
    pub fn set_channels_moderated(&mut self, channels: Vec<Channel>) {
        self.channels_moderated = channels;
        self.last_update = chrono::Utc::now();
    }
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
                .with_file(true)
                .with_span_events(FmtSpan::CLOSE),
        )
        .init();

    // Set a panic hook that will exit the process on when any thread panics
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        std::process::exit(1);
    }));

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
        Err(_) => {
            info!("Sentry not initializing due to missing SENTRY_DSN in environment");
            None
        }
    };

    let config = Config::load("config.toml").expect("Failed to load config");

    let (tx, rx): (Sender<Commands>, Receiver<Commands>) = broadcast::channel(100);

    let mut tiltify_user = TiltifyUser::load("./tiltify.json").unwrap_or_else(|_| {
        let auth_url = crate::bot::tiltify::authorize_url();
        info!("Please authenticate Tiltify using this URL: {}", auth_url.0);

        TiltifyUser {
            access_token: None,
            refresh_token: None,
            expires_in: Duration::default(),
            token_type: BasicTokenType::Bearer,
        }
    });

    let mut tiltify_client = bot::tiltify::TiltifyClient {
        user: tiltify_user,
        tx: tx.clone(),
    };

    let tiltify_client_handle = tiltify_client.start(tx.clone());

    let app_state: SharedAppState = Arc::new(Mutex::new(AppState {
        tx: tx.clone(),
        channels: ChannelsState::default(),
    }));

    let http_server = {
        let config = config.clone();
        let app_state = app_state.clone();
        let tx = tx.clone();
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
                    info!("received CTRL+C, shutting down");
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
        tx: tx.clone(),
        state: app_state.clone(),
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

    let refresher = async {
        debug!("Refresher started");
        let tx = tx.clone();
        loop {
            debug!("Refresher loop start");
            tokio::time::sleep(Duration::from_secs(30)).await;
            tx.send(Commands::TiltifyAuthRefresh).unwrap();
            debug!("Refresher loop end");
        }
    };
    let _ = tokio::join!(http_handle, bot_handle, tiltify_client_handle, refresher);
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Commands {
    Shutdown,
    DonationReceived(TiltifyDonation),
    RaidInitiated(String),
    StreamStarted(String),
    StreamEnded(String),
    UpdateChannels,
    OAuthResponse(String),
    TiltifyAuthRefresh,
    TiltifyTeamCampaignsRequest,
    TiltifyTeamStatsRequest,
    TiltifyTeamStatsResponse(TiltifyTeamResponse),
}
