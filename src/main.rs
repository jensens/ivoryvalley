use ivoryvalley::config::Config;
use ivoryvalley::proxy::create_proxy_router;
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

    // Load configuration (for now, use defaults)
    // TODO: Load from config file or environment variables
    let upstream_url =
        std::env::var("UPSTREAM_URL").unwrap_or_else(|_| "https://mastodon.social".to_string());
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let config = Config::new(&upstream_url, &host, port);

    tracing::info!("Starting IvoryValley proxy");
    tracing::info!("  Upstream: {}", config.upstream_url);
    tracing::info!("  Listening on: {}", config.bind_addr());

    // Create the router
    let app = create_proxy_router(config.clone());

    // Bind and serve
    let listener = TcpListener::bind(config.bind_addr())
        .await
        .expect("Failed to bind to address");

    tracing::info!("Proxy server running on http://{}", config.bind_addr());

    axum::serve(listener, app).await.expect("Server error");
}
