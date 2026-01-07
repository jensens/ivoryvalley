//! Integration tests for the proxy functionality.
//!
//! These tests verify the end-to-end behavior of the IvoryValley proxy.

mod common;

use axum::{
    body::Body,
    extract::Request,
    http::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE},
    response::Response,
    routing::{get, post},
    Router,
};
use common::{create_temp_dir, TestConfig};
use ivoryvalley::{config::Config, db::SeenUriStore, proxy::create_proxy_router};
use std::net::SocketAddr;
use tokio::net::TcpListener;

/// Mock upstream server for testing
struct MockUpstream {
    pub addr: SocketAddr,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MockUpstream {
    async fn start() -> Self {
        let app = Router::new()
            .route("/api/v1/timelines/home", get(mock_timeline_handler))
            .route(
                "/api/v1/accounts/verify_credentials",
                get(mock_verify_credentials),
            )
            .route("/api/v1/statuses", post(mock_post_status))
            .route("/oauth/token", post(mock_oauth_token))
            .fallback(mock_fallback_handler);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for MockUpstream {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

async fn mock_timeline_handler(req: Request<Body>) -> Response<Body> {
    let auth = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if auth.is_empty() {
        return Response::builder()
            .status(401)
            .body(Body::from(r#"{"error":"unauthorized"}"#))
            .unwrap();
    }

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"[{"id":"1","content":"Hello"}]"#))
        .unwrap()
}

async fn mock_verify_credentials(req: Request<Body>) -> Response<Body> {
    let auth = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if auth.is_empty() {
        return Response::builder()
            .status(401)
            .body(Body::from(r#"{"error":"unauthorized"}"#))
            .unwrap();
    }

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"id":"12345","username":"testuser"}"#))
        .unwrap()
}

async fn mock_post_status(req: Request<Body>) -> Response<Body> {
    let auth = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if auth.is_empty() {
        return Response::builder()
            .status(401)
            .body(Body::from(r#"{"error":"unauthorized"}"#))
            .unwrap();
    }

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"id":"999","content":"Posted!"}"#))
        .unwrap()
}

async fn mock_oauth_token() -> Response<Body> {
    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"access_token":"test_token","token_type":"Bearer"}"#,
        ))
        .unwrap()
}

async fn mock_fallback_handler(req: Request<Body>) -> Response<Body> {
    let path = req.uri().path().to_string();
    let method = req.method().to_string();

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(format!(
            r#"{{"path":"{}","method":"{}"}}"#,
            path, method
        )))
        .unwrap()
}

/// Test that the proxy can be configured with test defaults.
#[test]
fn test_proxy_config_creation() {
    let temp_dir = create_temp_dir();
    let config = TestConfig::new()
        .with_db_path(temp_dir.path().join("test.db"))
        .with_upstream_url("http://mastodon.example.com");

    assert!(config.db_path.is_some());
    assert_eq!(config.upstream_url, "http://mastodon.example.com");
}

/// Test that requests are forwarded to the upstream server.
#[tokio::test]
async fn test_proxy_forwards_get_request() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("Hello"));
}

/// Test that Authorization header is passed through to upstream.
#[tokio::test]
async fn test_proxy_passes_auth_header() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Without auth header, should get 401
    let response = client.get("/api/v1/timelines/home").await;
    response.assert_status_unauthorized();

    // With auth header, should succeed
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
}

/// Test that POST requests are forwarded (passthrough for actions).
#[tokio::test]
async fn test_proxy_forwards_post_request() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .post("/api/v1/statuses")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .add_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .text(r#"{"status":"Hello world"}"#)
        .await;

    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("Posted!"));
}

/// Test that OAuth endpoints pass through unchanged.
#[tokio::test]
async fn test_proxy_oauth_passthrough() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .post("/oauth/token")
        .add_header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        )
        .text("grant_type=authorization_code&code=test")
        .await;

    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("access_token"));
}

/// Test that account endpoints pass through.
#[tokio::test]
async fn test_proxy_account_passthrough() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .get("/api/v1/accounts/verify_credentials")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("testuser"));
}

/// Test that arbitrary endpoints pass through (fallback).
#[tokio::test]
async fn test_proxy_fallback_passthrough() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .get("/api/v1/some/unknown/endpoint")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body = response.text();
    assert!(body.contains("/api/v1/some/unknown/endpoint"));
    assert!(body.contains("GET"));
}

// =============================================================================
// Timeline filtering tests
// =============================================================================

/// Mock upstream server that returns timeline with statuses
struct MockTimelineUpstream {
    pub addr: SocketAddr,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MockTimelineUpstream {
    async fn start_with_statuses(statuses_json: &'static str) -> Self {
        let app = Router::new()
            .route(
                "/api/v1/timelines/home",
                get(move || async move {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(statuses_json))
                        .unwrap()
                }),
            )
            .route(
                "/api/v1/timelines/public",
                get(move || async move {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(statuses_json))
                        .unwrap()
                }),
            )
            .route(
                "/api/v1/timelines/list/{list_id}",
                get(move || async move {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(statuses_json))
                        .unwrap()
                }),
            )
            .route(
                "/api/v1/timelines/tag/{hashtag}",
                get(move || async move {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(statuses_json))
                        .unwrap()
                }),
            )
            .route(
                "/api/v1/timelines/link",
                get(move || async move {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(statuses_json))
                        .unwrap()
                }),
            )
            .route(
                "/api/v1/trends/statuses",
                get(move || async move {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(statuses_json))
                        .unwrap()
                }),
            );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for MockTimelineUpstream {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Test that first-time statuses pass through the filter.
#[tokio::test]
async fn test_timeline_first_status_passes_through() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Hello</p>"},
        {"id": "2", "uri": "https://example.com/statuses/2", "content": "<p>World</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let statuses = body.as_array().unwrap();

    // Both statuses should pass through on first request
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0]["id"], "1");
    assert_eq!(statuses[1]["id"], "2");
}

/// Test that duplicate statuses are filtered out on subsequent requests.
#[tokio::test]
async fn test_timeline_duplicates_are_filtered() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Hello</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // First request - status should pass through
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Second request - status should be filtered (already seen)
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 0);
}

/// Test that boosts are deduplicated based on the original content URI.
#[tokio::test]
async fn test_timeline_boost_deduplication() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    // Create and pre-populate the store with the original URI
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    seen_store
        .mark_seen("https://original.com/statuses/1")
        .unwrap();

    // Now test with a boost of the same content
    let boost_statuses = r#"[
        {
            "id": "2",
            "uri": "https://booster.com/statuses/2",
            "content": "",
            "reblog": {
                "id": "1",
                "uri": "https://original.com/statuses/1",
                "content": "<p>Original</p>"
            }
        }
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(boost_statuses).await;
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    // Boost should be filtered because we already saw the original
    assert_eq!(body.as_array().unwrap().len(), 0);
}

/// Test that filtering works for public timeline endpoint.
#[tokio::test]
async fn test_timeline_public_filtering() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Hello</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // First request
    let response = client.get("/api/v1/timelines/public").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Second request - should be filtered
    let response = client.get("/api/v1/timelines/public").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 0);
}

/// Test that filtering works for list timeline endpoint.
#[tokio::test]
async fn test_timeline_list_filtering() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Hello</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // First request
    let response = client.get("/api/v1/timelines/list/12345").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Second request - should be filtered
    let response = client.get("/api/v1/timelines/list/12345").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 0);
}

/// Test that filtering works for hashtag timeline endpoint.
#[tokio::test]
async fn test_timeline_hashtag_filtering() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Hello</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // First request
    let response = client.get("/api/v1/timelines/tag/rust").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Second request - should be filtered
    let response = client.get("/api/v1/timelines/tag/rust").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 0);
}

/// Test that statuses without URI field are passed through (not filtered).
#[tokio::test]
async fn test_timeline_status_without_uri_passes_through() {
    let statuses = r#"[
        {"id": "1", "content": "<p>No URI field</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // First request
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Second request - should still pass through because no URI to deduplicate
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);
}

// =============================================================================
// Body size limit tests
// =============================================================================

/// Test that requests within the body size limit are processed normally.
#[tokio::test]
async fn test_body_within_limit_succeeds() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    // Use a small limit (1KB) for testing
    let config = Config::with_max_body_size(&upstream.url(), "0.0.0.0", 0, db_path, 1024);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Send a small body (under 1KB)
    let small_body = "x".repeat(500);
    let response = client
        .post("/api/v1/statuses")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .add_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .text(small_body)
        .await;

    response.assert_status_ok();
}

/// Test that requests exceeding the body size limit return 413 Payload Too Large.
#[tokio::test]
async fn test_body_exceeding_limit_returns_413() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    // Use a small limit (1KB) for testing
    let config = Config::with_max_body_size(&upstream.url(), "0.0.0.0", 0, db_path, 1024);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Send a large body (over 1KB)
    let large_body = "x".repeat(2000);
    let response = client
        .post("/api/v1/statuses")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .add_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .text(large_body)
        .await;

    // Should return 413 Payload Too Large
    response.assert_status(axum::http::StatusCode::PAYLOAD_TOO_LARGE);
}

/// Test that the default body size limit allows reasonable requests.
#[tokio::test]
async fn test_default_body_limit_allows_normal_requests() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    // Use default config (should have 50MB limit)
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Send a reasonably sized body - should succeed with default 50MB limit
    let normal_body = r#"{"status":"Hello world with some content"}"#;
    let response = client
        .post("/api/v1/statuses")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .add_header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .text(normal_body)
        .await;

    response.assert_status_ok();
}

// =============================================================================
// New endpoint filtering tests (Issue #61)
// =============================================================================

/// Test that filtering works for link timeline endpoint (trending articles).
#[tokio::test]
async fn test_timeline_link_filtering() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Hello</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // First request
    let response = client.get("/api/v1/timelines/link").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Second request - should be filtered
    let response = client.get("/api/v1/timelines/link").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 0);
}

/// Test that filtering works for trends/statuses endpoint (trending statuses).
#[tokio::test]
async fn test_trends_statuses_filtering() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Hello</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // First request
    let response = client.get("/api/v1/trends/statuses").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Second request - should be filtered
    let response = client.get("/api/v1/trends/statuses").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 0);
}

/// Test that the proxy strips Accept-Encoding header to prevent gzip responses.
///
/// This is critical for deduplication - the proxy must parse JSON responses to
/// filter duplicates. If upstream returns gzip-compressed data, parsing fails
/// and deduplication silently breaks.
#[tokio::test]
async fn test_accept_encoding_stripped_prevents_gzip() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // Track whether upstream received Accept-Encoding header
    let received_accept_encoding = Arc::new(AtomicBool::new(false));
    let received_accept_encoding_clone = received_accept_encoding.clone();

    // Create a mock that checks for Accept-Encoding and returns accordingly
    let app = Router::new().route(
        "/api/v1/timelines/home",
        get(move |headers: axum::http::HeaderMap| async move {
            let has_accept_encoding = headers.get("accept-encoding").is_some();
            received_accept_encoding_clone.store(has_accept_encoding, Ordering::SeqCst);

            // Return uncompressed JSON (proxy should never send accept-encoding)
            axum::Json(serde_json::json!([
                {"id": "1", "uri": "https://example.com/1", "content": "test"}
            ]))
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Create proxy
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&format!("http://{}", addr), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let proxy_app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(proxy_app).unwrap();

    // Send request WITH Accept-Encoding header (like a real browser would)
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(
            axum::http::header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br"),
        )
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test"))
        .await;

    response.assert_status_ok();

    // The proxy should have stripped the Accept-Encoding header
    assert!(
        !received_accept_encoding.load(Ordering::SeqCst),
        "Proxy must strip Accept-Encoding header to prevent gzip responses"
    );

    // Verify response is valid JSON (deduplication worked)
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);
}

// =============================================================================
// Legitimate message tests (Issue #20)
// These tests verify that deduplication doesn't drop valid content.
// =============================================================================

/// Test that same content from different authors is NOT deduplicated.
/// Each author's post has a unique URI, so both should pass through.
#[tokio::test]
async fn test_same_content_different_authors_not_deduplicated() {
    // Two posts with identical content but different URIs (different authors)
    let statuses = r#"[
        {"id": "1", "uri": "https://alice.example/statuses/1", "content": "<p>Hello World</p>"},
        {"id": "2", "uri": "https://bob.example/statuses/2", "content": "<p>Hello World</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let statuses = body.as_array().unwrap();

    // Both statuses should pass through - same content, but different URIs
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0]["id"], "1");
    assert_eq!(statuses[1]["id"], "2");
}

/// Test that a reply passes through even when its parent was already seen.
/// Replies have their own URI, independent of the parent.
#[tokio::test]
async fn test_reply_passes_through_with_seen_parent() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    // Pre-mark the parent as seen
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    seen_store
        .mark_seen("https://example.com/statuses/1")
        .unwrap();

    // Reply has its own URI
    let statuses = r#"[
        {"id": "2", "uri": "https://example.com/statuses/2", "in_reply_to_id": "1", "content": "<p>This is a reply</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();

    // Reply should pass through - it has its own URI
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], "2");
}

/// Test that all statuses in a thread pass through.
/// Each status in a thread has its own unique URI.
#[tokio::test]
async fn test_thread_all_statuses_pass_through() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Original post</p>"},
        {"id": "2", "uri": "https://example.com/statuses/2", "in_reply_to_id": "1", "content": "<p>First reply</p>"},
        {"id": "3", "uri": "https://example.com/statuses/3", "in_reply_to_id": "2", "content": "<p>Reply to reply</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let statuses = body.as_array().unwrap();

    // All three statuses should pass through
    assert_eq!(statuses.len(), 3);
    assert_eq!(statuses[0]["id"], "1");
    assert_eq!(statuses[1]["id"], "2");
    assert_eq!(statuses[2]["id"], "3");
}

/// Test that posts from different instances with the same ID are not deduplicated.
/// Deduplication is based on URI, not ID. Different instances can have the same ID.
#[tokio::test]
async fn test_cross_instance_same_id_different_uri() {
    // Same ID but different URIs (from different instances)
    let statuses = r#"[
        {"id": "123", "uri": "https://instance-a.social/statuses/123", "content": "<p>From instance A</p>"},
        {"id": "123", "uri": "https://instance-b.social/statuses/123", "content": "<p>From instance B</p>"}
    ]"#;

    let upstream = MockTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let statuses = body.as_array().unwrap();

    // Both should pass through - same ID but different URIs
    assert_eq!(statuses.len(), 2);
}

// =============================================================================
// Non-filtered endpoint tests (Issue #20)
// These tests verify that certain endpoints are NOT filtered.
// =============================================================================

/// Mock upstream server for non-timeline endpoints
struct MockNonTimelineUpstream {
    pub addr: SocketAddr,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MockNonTimelineUpstream {
    async fn start_with_statuses(statuses_json: &'static str) -> Self {
        let app = Router::new()
            .route(
                "/api/v1/accounts/{account_id}/statuses",
                get(move || async move {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(statuses_json))
                        .unwrap()
                }),
            )
            .route(
                "/api/v1/statuses/{status_id}/context",
                get(move || async move {
                    // Context returns ancestors and descendants
                    let context =
                        format!(r#"{{"ancestors": {}, "descendants": []}}"#, statuses_json);
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(context))
                        .unwrap()
                }),
            )
            .route(
                "/api/v1/bookmarks",
                get(move || async move {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(statuses_json))
                        .unwrap()
                }),
            )
            .route(
                "/api/v1/favourites",
                get(move || async move {
                    Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(Body::from(statuses_json))
                        .unwrap()
                }),
            );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for MockNonTimelineUpstream {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Test that account statuses are NOT filtered, even if already seen.
/// Users should always see all statuses when viewing a profile.
#[tokio::test]
async fn test_account_statuses_not_filtered() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Hello</p>"}
    ]"#;

    let upstream = MockNonTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);

    // Pre-mark the status as seen
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    seen_store
        .mark_seen("https://example.com/statuses/1")
        .unwrap();

    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // First request - should pass through (no filtering on account statuses)
    let response = client
        .get("/api/v1/accounts/12345/statuses")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();

    // Status should appear even though it was pre-marked as seen
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], "1");

    // Second request - should still pass through
    let response = client
        .get("/api/v1/accounts/12345/statuses")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);
}

/// Test that thread context is NOT filtered, even if statuses were already seen.
/// Users should always see the full thread when viewing context.
#[tokio::test]
async fn test_thread_context_not_filtered() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Parent</p>"}
    ]"#;

    let upstream = MockNonTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);

    // Pre-mark the status as seen
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    seen_store
        .mark_seen("https://example.com/statuses/1")
        .unwrap();

    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Request thread context - should not be filtered
    let response = client
        .get("/api/v1/statuses/999/context")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();

    // Ancestors should contain the status even though it was pre-marked as seen
    let ancestors = body["ancestors"].as_array().unwrap();
    assert_eq!(ancestors.len(), 1);
    assert_eq!(ancestors[0]["id"], "1");
}

/// Test that bookmarks are NOT filtered, even if already seen.
/// Users should always see all their bookmarks.
#[tokio::test]
async fn test_bookmarks_not_filtered() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Bookmarked</p>"}
    ]"#;

    let upstream = MockNonTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);

    // Pre-mark the status as seen
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    seen_store
        .mark_seen("https://example.com/statuses/1")
        .unwrap();

    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Request bookmarks - should not be filtered
    let response = client
        .get("/api/v1/bookmarks")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();

    // Bookmark should appear even though it was pre-marked as seen
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Second request - should still appear
    let response = client
        .get("/api/v1/bookmarks")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 1);
}

/// Test that favourites are NOT filtered, even if already seen.
/// Users should always see all their favourites.
#[tokio::test]
async fn test_favourites_not_filtered() {
    let statuses = r#"[
        {"id": "1", "uri": "https://example.com/statuses/1", "content": "<p>Favourited</p>"}
    ]"#;

    let upstream = MockNonTimelineUpstream::start_with_statuses(statuses).await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);

    // Pre-mark the status as seen
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    seen_store
        .mark_seen("https://example.com/statuses/1")
        .unwrap();

    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Request favourites - should not be filtered
    let response = client
        .get("/api/v1/favourites")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();

    // Favourite should appear even though it was pre-marked as seen
    assert_eq!(body.as_array().unwrap().len(), 1);
}

// =============================================================================
// Health check endpoint tests
// =============================================================================

/// Test that /health returns 200 OK with status and version.
#[tokio::test]
async fn test_health_endpoint_returns_ok() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client.get("/health").await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "healthy");
    assert!(body["version"].is_string());
}

/// Test that /health does not require authentication.
#[tokio::test]
async fn test_health_endpoint_no_auth_required() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    // No auth header
    let response = client.get("/health").await;

    response.assert_status_ok();
}

/// Test that /health?deep=true checks database connectivity.
#[tokio::test]
async fn test_health_endpoint_deep_check() {
    let upstream = MockUpstream::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();
    let response = client.get("/health?deep=true").await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "healthy");
    assert!(body["version"].is_string());
    assert!(body["checks"]["database"].is_string());
}
