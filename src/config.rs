use eyre::{Context, Report};
use serde_derive::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub storage: StorageConfig,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StorageConfig {
    pub tokens: String,
    pub bot: String,
    pub channels: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServerConfig {
    pub host: Ipv4Addr,
    pub port: u16,
}

impl ServerConfig {
    pub fn to_socket_addrs(&self) -> SocketAddr {
        SocketAddr::new(self.host.into(), self.port)
    }
}


impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Report> {
        let config = std::fs::read_to_string(path)?;
        toml::from_str(&config).wrap_err("Failed to parse config")
    }
}
