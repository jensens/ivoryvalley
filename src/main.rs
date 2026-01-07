use std::sync::Arc;

use ivoryvalley::cleanup::spawn_cleanup_task;
use ivoryvalley::config::Config;
use ivoryvalley::db::SeenUriStore;
use ivoryvalley::proxy::create_proxy_router;
use ivoryvalley::shutdown::shutdown_signal;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ivoryvalley=info".into()),
        )
        .init();

    // Load configuration from CLI args, environment variables, and config file
    let config = Config::load().expect("Failed to load configuration");

    tracing::info!("Starting IvoryValley proxy");
    tracing::info!("  Upstream: {}", config.upstream_url);
    tracing::info!("  Listening on: {}", config.bind_addr());
    tracing::info!("  Database: {}", config.database_path.display());

    // Open the seen URI store for deduplication
    let seen_store =
        SeenUriStore::open(&config.database_path).expect("Failed to open seen URI store");
    let seen_store = Arc::new(seen_store);

    // Start the background cleanup task
    let _cleanup_handle = spawn_cleanup_task(
        seen_store.clone(),
        config.cleanup_interval_secs,
        config.cleanup_max_age_secs,
    );

    // Create the router
    let app = create_proxy_router(config.clone(), seen_store);

    // Bind and serve
    let listener = TcpListener::bind(config.bind_addr())
        .await
        .expect("Failed to bind to address");

    tracing::info!("Proxy server running on http://{}", config.bind_addr());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");

    tracing::info!("Server shutdown complete");
}
