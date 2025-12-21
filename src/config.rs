//! Configuration module for IvoryValley proxy

use std::sync::Arc;

/// Configuration for the IvoryValley proxy server
#[derive(Debug, Clone)]
pub struct Config {
    /// Upstream Mastodon server URL (e.g., "https://mastodon.social")
    pub upstream_url: String,

    /// Host to bind the proxy server to
    pub host: String,

    /// Port to bind the proxy server to
    pub port: u16,
}

impl Config {
    /// Create a new configuration
    pub fn new(upstream_url: &str, host: &str, port: u16) -> Self {
        Self {
            upstream_url: upstream_url.to_string(),
            host: host.to_string(),
            port,
        }
    }

    /// Get the socket address for binding
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Shared application state containing configuration
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub http_client: reqwest::Client,
}

impl AppState {
    /// Create a new application state from configuration
    pub fn new(config: Config) -> Self {
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config: Arc::new(config),
            http_client,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_new() {
        let config = Config::new("https://mastodon.social", "0.0.0.0", 8080);
        assert_eq!(config.upstream_url, "https://mastodon.social");
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
    }

    #[test]
    fn test_bind_addr() {
        let config = Config::new("https://mastodon.social", "127.0.0.1", 3000);
        assert_eq!(config.bind_addr(), "127.0.0.1:3000");
    }
}
