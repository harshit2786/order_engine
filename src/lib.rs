pub mod api;
pub mod error;
pub mod matching;
pub mod metrics;
pub mod models;
pub mod orderbook;
pub mod state;

use std::net::SocketAddr;

use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::api::routes::create_router;
use crate::state::AppState;

/// Server configuration
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
        }
    }
}

impl ServerConfig {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    pub fn from_env() -> Self {
        let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = std::env::var("PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse()
            .expect("PORT must be a valid number");

        Self { host, port }
    }

    pub fn socket_addr(&self) -> SocketAddr {
        format!("{}:{}", self.host, self.port)
            .parse()
            .expect("Invalid socket address")
    }
}

/// Runs the server with default configuration
pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = ServerConfig::from_env();
    run_with_config(config).await
}

/// Runs the server with custom configuration
pub async fn run_with_config(
    config: ServerConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = AppState::new();
    run_with_state(config, state).await
}

/// Runs the server with custom configuration and state
pub async fn run_with_state(
    config: ServerConfig,
    state: AppState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = create_router(state).layer(TraceLayer::new_for_http());

    let addr = config.socket_addr();
    let listener = TcpListener::bind(addr).await?;

    info!("🚀 Order Matching Engine started");
    info!("📡 Listening on http://{}", addr);
    info!("📊 Metrics available at http://{}/metrics", addr);
    info!("❤️  Health check at http://{}/health", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
    }

    #[test]
    fn test_server_config_new() {
        let config = ServerConfig::new("127.0.0.1".to_string(), 3000);
        
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 3000);
    }

    #[test]
    fn test_server_config_socket_addr() {
        let config = ServerConfig::new("127.0.0.1".to_string(), 8080);
        let addr = config.socket_addr();
        
        assert_eq!(addr.to_string(), "127.0.0.1:8080");
    }
}