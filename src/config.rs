//! Configuration module for IvoryValley proxy
//!
//! Configuration is loaded with the following priority (highest first):
//! 1. Command line arguments
//! 2. Environment variables (prefixed with IVORYVALLEY_)
//! 3. Configuration file (config.toml or config.yaml)
//! 4. Default values

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use config::{ConfigError, Environment, File};
use serde::Deserialize;

/// Default upstream URL
const DEFAULT_UPSTREAM_URL: &str = "https://mastodon.social";
/// Default host to bind to
const DEFAULT_HOST: &str = "0.0.0.0";
/// Default port
const DEFAULT_PORT: u16 = 8080;
/// Default database path
const DEFAULT_DATABASE_PATH: &str = "ivoryvalley.db";
/// Default maximum body size (50MB - allows video uploads)
const DEFAULT_MAX_BODY_SIZE: usize = 50 * 1024 * 1024;
/// Default HTTP connect timeout in seconds
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
/// Default HTTP request timeout in seconds
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
/// Default recording path (None = disabled)
const DEFAULT_RECORD_TRAFFIC_PATH: Option<&str> = None;

/// Command line arguments
#[derive(Parser, Debug)]
#[command(name = "ivoryvalley")]
#[command(about = "A Mastodon proxy server for filtering content")]
pub struct CliArgs {
    /// Upstream Mastodon server URL
    #[arg(long, env = "IVORYVALLEY_UPSTREAM_URL")]
    pub upstream_url: Option<String>,

    /// Host to bind the proxy server to
    #[arg(long, env = "IVORYVALLEY_HOST")]
    pub host: Option<String>,

    /// Port to bind the proxy server to
    #[arg(short, long, env = "IVORYVALLEY_PORT")]
    pub port: Option<u16>,

    /// Path to the SQLite database file
    #[arg(long, env = "IVORYVALLEY_DATABASE_PATH")]
    pub database_path: Option<PathBuf>,

    /// Maximum request body size in bytes (default: 50MB)
    #[arg(long, env = "IVORYVALLEY_MAX_BODY_SIZE")]
    pub max_body_size: Option<usize>,

    /// HTTP client connect timeout in seconds
    #[arg(long, env = "IVORYVALLEY_CONNECT_TIMEOUT_SECS")]
    pub connect_timeout_secs: Option<u64>,

    /// HTTP client request timeout in seconds
    #[arg(long, env = "IVORYVALLEY_REQUEST_TIMEOUT_SECS")]
    pub request_timeout_secs: Option<u64>,

    /// Path to record traffic (JSONL file). If set, all request/response pairs are recorded.
    #[arg(long, env = "IVORYVALLEY_RECORD_TRAFFIC_PATH")]
    pub record_traffic_path: Option<PathBuf>,

    /// Path to configuration file
    #[arg(short, long, env = "IVORYVALLEY_CONFIG")]
    pub config: Option<PathBuf>,
}

/// File-based configuration (for TOML/YAML)
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct FileConfig {
    upstream_url: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    database_path: Option<PathBuf>,
    max_body_size: Option<usize>,
    connect_timeout_secs: Option<u64>,
    request_timeout_secs: Option<u64>,
    record_traffic_path: Option<PathBuf>,
}

/// Configuration for the IvoryValley proxy server
#[derive(Debug, Clone)]
pub struct Config {
    /// Upstream Mastodon server URL (e.g., "https://mastodon.social")
    pub upstream_url: String,

    /// Host to bind the proxy server to
    pub host: String,

    /// Port to bind the proxy server to
    pub port: u16,

    /// Path to the SQLite database file
    pub database_path: PathBuf,

    /// Maximum request body size in bytes (prevents DoS via memory exhaustion)
    pub max_body_size: usize,

    /// HTTP client connect timeout in seconds
    pub connect_timeout_secs: u64,

    /// HTTP client request timeout in seconds
    pub request_timeout_secs: u64,

    /// Path to record traffic (JSONL file). If Some, all traffic is recorded.
    pub record_traffic_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            upstream_url: DEFAULT_UPSTREAM_URL.to_string(),
            host: DEFAULT_HOST.to_string(),
            port: DEFAULT_PORT,
            database_path: PathBuf::from(DEFAULT_DATABASE_PATH),
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            connect_timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
            request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
            record_traffic_path: DEFAULT_RECORD_TRAFFIC_PATH.map(PathBuf::from),
        }
    }
}

impl Config {
    /// Create a new configuration with explicit values (uses default max_body_size and timeouts)
    #[allow(dead_code)] // Used in tests via library crate
    pub fn new(upstream_url: &str, host: &str, port: u16, database_path: PathBuf) -> Self {
        Self {
            upstream_url: upstream_url.to_string(),
            host: host.to_string(),
            port,
            database_path,
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            connect_timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
            request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
            record_traffic_path: None,
        }
    }

    /// Create a new configuration with a custom max body size (for testing)
    #[allow(dead_code)] // Used in tests via library crate
    pub fn with_max_body_size(
        upstream_url: &str,
        host: &str,
        port: u16,
        database_path: PathBuf,
        max_body_size: usize,
    ) -> Self {
        Self {
            upstream_url: upstream_url.to_string(),
            host: host.to_string(),
            port,
            database_path,
            max_body_size,
            connect_timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
            request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
            record_traffic_path: None,
        }
    }

    /// Load configuration from all sources (CLI > env > file > defaults)
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from_args(CliArgs::parse())
    }

    /// Load configuration from provided CLI args (for testing)
    pub fn load_from_args(args: CliArgs) -> Result<Self, ConfigError> {
        // Start with defaults
        let mut config = Config::default();

        // Load from config file if specified or if default exists
        let file_config = Self::load_file_config(&args.config)?;

        // Apply file config (file overrides defaults)
        if let Some(url) = file_config.upstream_url {
            config.upstream_url = url;
        }
        if let Some(h) = file_config.host {
            config.host = h;
        }
        if let Some(p) = file_config.port {
            config.port = p;
        }
        if let Some(db) = file_config.database_path {
            config.database_path = db;
        }
        if let Some(size) = file_config.max_body_size {
            config.max_body_size = size;
        }
        if let Some(ct) = file_config.connect_timeout_secs {
            config.connect_timeout_secs = ct;
        }
        if let Some(rt) = file_config.request_timeout_secs {
            config.request_timeout_secs = rt;
        }
        if let Some(path) = file_config.record_traffic_path {
            config.record_traffic_path = Some(path);
        }

        // Apply CLI args (CLI overrides everything)
        if let Some(url) = args.upstream_url {
            config.upstream_url = url;
        }
        if let Some(h) = args.host {
            config.host = h;
        }
        if let Some(p) = args.port {
            config.port = p;
        }
        if let Some(db) = args.database_path {
            config.database_path = db;
        }
        if let Some(size) = args.max_body_size {
            config.max_body_size = size;
        }
        if let Some(ct) = args.connect_timeout_secs {
            config.connect_timeout_secs = ct;
        }
        if let Some(rt) = args.request_timeout_secs {
            config.request_timeout_secs = rt;
        }
        if let Some(path) = args.record_traffic_path {
            config.record_traffic_path = Some(path);
        }

        Ok(config)
    }

    /// Load configuration from file
    fn load_file_config(config_path: &Option<PathBuf>) -> Result<FileConfig, ConfigError> {
        let mut builder = config::Config::builder();

        // Add config file if specified
        if let Some(path) = config_path {
            builder = builder.add_source(File::from(path.as_path()));
        } else {
            // Try default config files (optional)
            builder = builder
                .add_source(File::with_name("config").required(false))
                .add_source(File::with_name("ivoryvalley").required(false));
        }

        // Add environment variables with IVORYVALLEY_ prefix
        builder = builder.add_source(
            Environment::with_prefix("IVORYVALLEY")
                .separator("_")
                .try_parsing(true),
        );

        let settings = builder.build()?;
        settings.try_deserialize()
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
    pub seen_uri_store: Arc<crate::db::SeenUriStore>,
    pub traffic_recorder: Option<Arc<crate::recording::TrafficRecorder>>,
}

impl AppState {
    /// Create a new application state from configuration and seen URI store.
    ///
    /// The `SeenUriStore` is wrapped in an `Arc` so it can be shared with other
    /// components (e.g., WebSocket handlers) that also need deduplication.
    pub fn new(config: Config, seen_store: Arc<crate::db::SeenUriStore>) -> Self {
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .expect("Failed to create HTTP client");

        // Initialize traffic recorder if configured
        let traffic_recorder = config.record_traffic_path.as_ref().and_then(|path| {
            match crate::recording::TrafficRecorder::new(path.clone()) {
                Ok(recorder) => {
                    tracing::info!("Traffic recording enabled: {}", path.display());
                    Some(Arc::new(recorder))
                }
                Err(e) => {
                    tracing::error!("Failed to initialize traffic recorder: {}", e);
                    None
                }
            }
        });

        Self {
            config: Arc::new(config),
            http_client,
            seen_uri_store: seen_store,
            traffic_recorder,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert_eq!(config.upstream_url, "https://mastodon.social");
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert_eq!(config.database_path, PathBuf::from("ivoryvalley.db"));
        assert_eq!(config.max_body_size, 50 * 1024 * 1024);
        assert_eq!(config.connect_timeout_secs, 10);
        assert_eq!(config.request_timeout_secs, 30);
    }

    #[test]
    fn test_config_new() {
        let config = Config::new(
            "https://example.com",
            "127.0.0.1",
            3000,
            PathBuf::from("/data/test.db"),
        );
        assert_eq!(config.upstream_url, "https://example.com");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 3000);
        assert_eq!(config.database_path, PathBuf::from("/data/test.db"));
    }

    #[test]
    fn test_bind_addr() {
        let config = Config::new(
            "https://mastodon.social",
            "127.0.0.1",
            3000,
            PathBuf::from("test.db"),
        );
        assert_eq!(config.bind_addr(), "127.0.0.1:3000");
    }

    #[test]
    fn test_load_defaults_when_no_config() {
        let args = CliArgs {
            upstream_url: None,
            host: None,
            port: None,
            database_path: None,
            max_body_size: None,
            connect_timeout_secs: None,
            request_timeout_secs: None,
            record_traffic_path: None,
            config: None,
        };
        let config = Config::load_from_args(args).unwrap();
        assert_eq!(config.upstream_url, "https://mastodon.social");
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert_eq!(config.database_path, PathBuf::from("ivoryvalley.db"));
        assert_eq!(config.max_body_size, 50 * 1024 * 1024);
        assert_eq!(config.connect_timeout_secs, 10);
        assert_eq!(config.request_timeout_secs, 30);
        assert_eq!(config.record_traffic_path, None);
    }

    #[test]
    fn test_load_from_cli_args() {
        let args = CliArgs {
            upstream_url: Some("https://cli.example.com".to_string()),
            host: Some("192.168.1.1".to_string()),
            port: Some(9000),
            database_path: Some(PathBuf::from("/cli/path.db")),
            max_body_size: Some(100 * 1024 * 1024),
            connect_timeout_secs: Some(5),
            request_timeout_secs: Some(60),
            record_traffic_path: Some(PathBuf::from("/tmp/traffic.jsonl")),
            config: None,
        };
        let config = Config::load_from_args(args).unwrap();
        assert_eq!(config.upstream_url, "https://cli.example.com");
        assert_eq!(config.host, "192.168.1.1");
        assert_eq!(config.port, 9000);
        assert_eq!(config.database_path, PathBuf::from("/cli/path.db"));
        assert_eq!(config.max_body_size, 100 * 1024 * 1024);
        assert_eq!(config.connect_timeout_secs, 5);
        assert_eq!(config.request_timeout_secs, 60);
        assert_eq!(
            config.record_traffic_path,
            Some(PathBuf::from("/tmp/traffic.jsonl"))
        );
    }

    #[test]
    fn test_load_from_toml_file() {
        let mut file = NamedTempFile::with_suffix(".toml").unwrap();
        writeln!(
            file,
            r#"
upstream_url = "https://toml.example.com"
host = "10.0.0.1"
port = 7000
database_path = "/toml/db.sqlite"
connect_timeout_secs = 15
request_timeout_secs = 45
"#
        )
        .unwrap();

        let args = CliArgs {
            upstream_url: None,
            host: None,
            port: None,
            database_path: None,
            max_body_size: None,
            connect_timeout_secs: None,
            request_timeout_secs: None,
            record_traffic_path: None,
            config: Some(file.path().to_path_buf()),
        };
        let config = Config::load_from_args(args).unwrap();
        assert_eq!(config.upstream_url, "https://toml.example.com");
        assert_eq!(config.host, "10.0.0.1");
        assert_eq!(config.port, 7000);
        assert_eq!(config.database_path, PathBuf::from("/toml/db.sqlite"));
        assert_eq!(config.connect_timeout_secs, 15);
        assert_eq!(config.request_timeout_secs, 45);
    }

    #[test]
    fn test_load_from_yaml_file() {
        let mut file = NamedTempFile::with_suffix(".yaml").unwrap();
        writeln!(
            file,
            r#"
upstream_url: "https://yaml.example.com"
host: "10.0.0.2"
port: 6000
database_path: "/yaml/db.sqlite"
connect_timeout_secs: 20
request_timeout_secs: 120
"#
        )
        .unwrap();

        let args = CliArgs {
            upstream_url: None,
            host: None,
            port: None,
            database_path: None,
            max_body_size: None,
            connect_timeout_secs: None,
            request_timeout_secs: None,
            record_traffic_path: None,
            config: Some(file.path().to_path_buf()),
        };
        let config = Config::load_from_args(args).unwrap();
        assert_eq!(config.upstream_url, "https://yaml.example.com");
        assert_eq!(config.host, "10.0.0.2");
        assert_eq!(config.port, 6000);
        assert_eq!(config.database_path, PathBuf::from("/yaml/db.sqlite"));
        assert_eq!(config.connect_timeout_secs, 20);
        assert_eq!(config.request_timeout_secs, 120);
    }

    #[test]
    fn test_cli_overrides_file() {
        let mut file = NamedTempFile::with_suffix(".toml").unwrap();
        writeln!(
            file,
            r#"
upstream_url = "https://file.example.com"
host = "10.0.0.1"
port = 7000
database_path = "/file/db.sqlite"
connect_timeout_secs = 15
request_timeout_secs = 45
"#
        )
        .unwrap();

        let args = CliArgs {
            upstream_url: Some("https://cli.example.com".to_string()),
            host: None, // Use file value
            port: Some(9999),
            database_path: None, // Use file value
            max_body_size: None,
            connect_timeout_secs: Some(5), // Override file value
            request_timeout_secs: None,    // Use file value
            record_traffic_path: None,
            config: Some(file.path().to_path_buf()),
        };
        let config = Config::load_from_args(args).unwrap();
        assert_eq!(config.upstream_url, "https://cli.example.com"); // CLI
        assert_eq!(config.host, "10.0.0.1"); // File
        assert_eq!(config.port, 9999); // CLI
        assert_eq!(config.database_path, PathBuf::from("/file/db.sqlite")); // File
        assert_eq!(config.connect_timeout_secs, 5); // CLI override
        assert_eq!(config.request_timeout_secs, 45); // File
    }

    #[test]
    fn test_partial_file_config_uses_defaults() {
        let mut file = NamedTempFile::with_suffix(".toml").unwrap();
        writeln!(
            file,
            r#"
upstream_url = "https://partial.example.com"
"#
        )
        .unwrap();

        let args = CliArgs {
            upstream_url: None,
            host: None,
            port: None,
            database_path: None,
            max_body_size: None,
            connect_timeout_secs: None,
            request_timeout_secs: None,
            record_traffic_path: None,
            config: Some(file.path().to_path_buf()),
        };
        let config = Config::load_from_args(args).unwrap();
        assert_eq!(config.upstream_url, "https://partial.example.com"); // From file
        assert_eq!(config.host, "0.0.0.0"); // Default
        assert_eq!(config.port, 8080); // Default
        assert_eq!(config.database_path, PathBuf::from("ivoryvalley.db")); // Default
        assert_eq!(config.max_body_size, 50 * 1024 * 1024); // Default 50MB
        assert_eq!(config.connect_timeout_secs, 10); // Default
        assert_eq!(config.request_timeout_secs, 30); // Default
        assert_eq!(config.record_traffic_path, None); // Default
    }
}
