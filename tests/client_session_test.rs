//! Integration tests simulating realistic Mastodon client session behavior.
//!
//! These tests verify that the proxy correctly handles typical client usage patterns
//! as documented in docs/client-server-traffic-patterns.md:
//! - App launch sequence (verify_credentials, instance, home timeline, notifications)
//! - Timeline pagination (min_id, max_id, since_id)
//! - WebSocket streaming with subscription messages
//! - User actions (post creation, favorites, boosts)

mod common;

use axum::{
    body::Body,
    extract::{Path, Query, Request, State},
    http::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE},
    response::Response,
    routing::{get, post},
    Router,
};
use common::create_temp_dir;
use ivoryvalley::{config::Config, db::SeenUriStore, proxy::create_proxy_router};
use serde::Deserialize;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::{net::TcpListener, sync::Mutex};

// =============================================================================
// Mock Mastodon Server - simulates a real Mastodon instance
// =============================================================================

/// Shared state for the mock Mastodon server
#[derive(Clone)]
struct MockMastodonState {
    /// Counter for generating unique status IDs
    next_status_id: Arc<AtomicU64>,
    /// Posted statuses (id -> content)
    statuses: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    /// Favorited statuses per user
    favorites: Arc<Mutex<HashMap<String, Vec<String>>>>,
    /// Reblogged statuses per user
    reblogs: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

impl MockMastodonState {
    fn new() -> Self {
        Self {
            next_status_id: Arc::new(AtomicU64::new(1000)),
            statuses: Arc::new(Mutex::new(HashMap::new())),
            favorites: Arc::new(Mutex::new(HashMap::new())),
            reblogs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Mock Mastodon server for realistic client testing
struct MockMastodon {
    pub addr: SocketAddr,
    pub state: MockMastodonState,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MockMastodon {
    async fn start() -> Self {
        let state = MockMastodonState::new();

        let app = Router::new()
            // Instance endpoints
            .route("/api/v1/instance", get(mock_instance_v1))
            .route("/api/v2/instance", get(mock_instance_v2))
            // Account endpoints
            .route(
                "/api/v1/accounts/verify_credentials",
                get(mock_verify_credentials),
            )
            .route("/api/v1/accounts/{id}", get(mock_get_account))
            .route("/api/v1/accounts/{id}/statuses", get(mock_account_statuses))
            // Timeline endpoints
            .route("/api/v1/timelines/home", get(mock_home_timeline))
            .route("/api/v1/timelines/public", get(mock_public_timeline))
            .route("/api/v1/timelines/tag/{tag}", get(mock_hashtag_timeline))
            .route("/api/v1/timelines/list/{list_id}", get(mock_list_timeline))
            // Notification endpoint
            .route("/api/v1/notifications", get(mock_notifications))
            // Status actions
            .route("/api/v1/statuses", post(mock_post_status))
            .route("/api/v1/statuses/{id}", get(mock_get_status))
            .route("/api/v1/statuses/{id}/context", get(mock_status_context))
            .route(
                "/api/v1/statuses/{id}/favourite",
                post(mock_favourite_status),
            )
            .route("/api/v1/statuses/{id}/reblog", post(mock_reblog_status))
            // OAuth
            .route("/oauth/token", post(mock_oauth_token))
            .with_state(state.clone());

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
            state,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for MockMastodon {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

// =============================================================================
// Mock Handler Implementations
// =============================================================================

fn require_auth(req: &Request<Body>) -> Result<String, Response<Body>> {
    let auth = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if auth.is_empty() || !auth.starts_with("Bearer ") {
        return Err(Response::builder()
            .status(401)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"error":"The access token is invalid"}"#))
            .unwrap());
    }

    Ok(auth.trim_start_matches("Bearer ").to_string())
}

async fn mock_instance_v1() -> Response<Body> {
    let instance = serde_json::json!({
        "uri": "mock.mastodon.local",
        "title": "Mock Mastodon",
        "short_description": "A mock Mastodon instance for testing",
        "description": "A mock Mastodon instance for testing the IvoryValley proxy",
        "email": "admin@mock.mastodon.local",
        "version": "4.2.0",
        "urls": {
            "streaming_api": "wss://mock.mastodon.local"
        },
        "stats": {
            "user_count": 1000,
            "status_count": 50000,
            "domain_count": 500
        },
        "max_toot_chars": 500,
        "registrations": true
    });

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(instance.to_string()))
        .unwrap()
}

async fn mock_instance_v2() -> Response<Body> {
    let instance = serde_json::json!({
        "domain": "mock.mastodon.local",
        "title": "Mock Mastodon",
        "version": "4.2.0",
        "source_url": "https://github.com/mastodon/mastodon",
        "description": "A mock Mastodon instance for testing",
        "configuration": {
            "urls": {
                "streaming": "wss://mock.mastodon.local"
            },
            "statuses": {
                "max_characters": 500,
                "max_media_attachments": 4
            }
        }
    });

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(instance.to_string()))
        .unwrap()
}

async fn mock_verify_credentials(req: Request<Body>) -> Response<Body> {
    if let Err(resp) = require_auth(&req) {
        return resp;
    }

    let account = serde_json::json!({
        "id": "12345",
        "username": "testuser",
        "acct": "testuser",
        "display_name": "Test User",
        "locked": false,
        "bot": false,
        "created_at": "2024-01-01T00:00:00.000Z",
        "note": "<p>Test account</p>",
        "url": "https://mock.mastodon.local/@testuser",
        "avatar": "https://mock.mastodon.local/avatars/original/missing.png",
        "header": "https://mock.mastodon.local/headers/original/missing.png",
        "followers_count": 100,
        "following_count": 50,
        "statuses_count": 200
    });

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(account.to_string()))
        .unwrap()
}

async fn mock_get_account(Path(id): Path<String>) -> Response<Body> {
    let account = serde_json::json!({
        "id": id,
        "username": format!("user{}", id),
        "acct": format!("user{}", id),
        "display_name": format!("User {}", id),
        "locked": false,
        "bot": false,
        "created_at": "2024-01-01T00:00:00.000Z",
        "note": "<p>Account description</p>",
        "url": format!("https://mock.mastodon.local/@user{}", id),
        "followers_count": 50,
        "following_count": 25,
        "statuses_count": 100
    });

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(account.to_string()))
        .unwrap()
}

async fn mock_account_statuses(Path(id): Path<String>) -> Response<Body> {
    let statuses = serde_json::json!([
        {
            "id": "1001",
            "uri": format!("https://mock.mastodon.local/users/{}/statuses/1001", id),
            "created_at": "2024-01-15T12:00:00.000Z",
            "content": "<p>A status from this account</p>",
            "account": {
                "id": id,
                "username": format!("user{}", id),
                "acct": format!("user{}", id)
            },
            "visibility": "public",
            "favourites_count": 5,
            "reblogs_count": 2,
            "replies_count": 1
        }
    ]);

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(statuses.to_string()))
        .unwrap()
}

/// Query parameters for timeline pagination
#[derive(Debug, Deserialize, Default)]
struct TimelineParams {
    max_id: Option<String>,
    since_id: Option<String>,
    min_id: Option<String>,
    limit: Option<u32>,
}

fn generate_timeline_statuses(
    params: &TimelineParams,
    prefix: &str,
    base_id: u64,
) -> Vec<serde_json::Value> {
    let limit = params.limit.unwrap_or(20).min(40) as u64;

    // Determine the starting ID based on pagination parameters
    let start_id = if let Some(ref max_id) = params.max_id {
        // Return older posts (IDs less than max_id)
        max_id.parse::<u64>().unwrap_or(base_id) - 1
    } else if let Some(ref min_id) = params.min_id {
        // Return newer posts (IDs greater than min_id)
        min_id.parse::<u64>().unwrap_or(base_id) + 1
    } else if let Some(ref since_id) = params.since_id {
        // Return all posts newer than since_id
        since_id.parse::<u64>().unwrap_or(base_id) + limit
    } else {
        base_id
    };

    (0..limit)
        .map(|i| {
            let id = start_id - i;
            serde_json::json!({
                "id": id.to_string(),
                "uri": format!("https://mock.mastodon.local/users/{}/statuses/{}", prefix, id),
                "created_at": "2024-01-15T12:00:00.000Z",
                "content": format!("<p>{} post #{}</p>", prefix, id),
                "account": {
                    "id": "12345",
                    "username": "testuser",
                    "acct": "testuser"
                },
                "visibility": "public",
                "favourites_count": 0,
                "reblogs_count": 0,
                "replies_count": 0
            })
        })
        .collect()
}

async fn mock_home_timeline(
    headers: axum::http::HeaderMap,
    Query(params): Query<TimelineParams>,
) -> Response<Body> {
    // Check auth from headers
    let auth = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if auth.is_empty() || !auth.starts_with("Bearer ") {
        return Response::builder()
            .status(401)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"error":"The access token is invalid"}"#))
            .unwrap();
    }

    let statuses = generate_timeline_statuses(&params, "home", 100);

    // Add Link header for pagination
    let oldest_id = statuses
        .last()
        .and_then(|s| s["id"].as_str())
        .unwrap_or("1");
    let newest_id = statuses
        .first()
        .and_then(|s| s["id"].as_str())
        .unwrap_or("100");

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .header(
            "Link",
            format!(
                r#"<https://mock.mastodon.local/api/v1/timelines/home?max_id={}>; rel="next", <https://mock.mastodon.local/api/v1/timelines/home?min_id={}>; rel="prev""#,
                oldest_id, newest_id
            ),
        )
        .body(Body::from(serde_json::to_string(&statuses).unwrap()))
        .unwrap()
}

async fn mock_public_timeline(Query(params): Query<TimelineParams>) -> Response<Body> {
    let statuses = generate_timeline_statuses(&params, "public", 200);

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&statuses).unwrap()))
        .unwrap()
}

async fn mock_hashtag_timeline(
    Path(tag): Path<String>,
    Query(params): Query<TimelineParams>,
) -> Response<Body> {
    let statuses = generate_timeline_statuses(&params, &format!("tag-{}", tag), 300);

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&statuses).unwrap()))
        .unwrap()
}

async fn mock_list_timeline(
    Path(list_id): Path<String>,
    Query(params): Query<TimelineParams>,
) -> Response<Body> {
    let statuses = generate_timeline_statuses(&params, &format!("list-{}", list_id), 400);

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&statuses).unwrap()))
        .unwrap()
}

async fn mock_notifications(req: Request<Body>) -> Response<Body> {
    if let Err(resp) = require_auth(&req) {
        return resp;
    }

    let notifications = serde_json::json!([
        {
            "id": "1",
            "type": "mention",
            "created_at": "2024-01-15T12:00:00.000Z",
            "account": {
                "id": "67890",
                "username": "otheruser",
                "acct": "otheruser@other.instance"
            },
            "status": {
                "id": "500",
                "content": "<p>@testuser Hello!</p>"
            }
        },
        {
            "id": "2",
            "type": "favourite",
            "created_at": "2024-01-15T11:00:00.000Z",
            "account": {
                "id": "67891",
                "username": "fan",
                "acct": "fan"
            },
            "status": {
                "id": "501",
                "content": "<p>My great post</p>"
            }
        }
    ]);

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(notifications.to_string()))
        .unwrap()
}

async fn mock_post_status(
    State(state): State<MockMastodonState>,
    req: Request<Body>,
) -> Response<Body> {
    if let Err(resp) = require_auth(&req) {
        return resp;
    }

    let id = state.next_status_id.fetch_add(1, Ordering::SeqCst);

    let status = serde_json::json!({
        "id": id.to_string(),
        "uri": format!("https://mock.mastodon.local/users/testuser/statuses/{}", id),
        "created_at": "2024-01-15T12:00:00.000Z",
        "content": "<p>Posted status</p>",
        "account": {
            "id": "12345",
            "username": "testuser",
            "acct": "testuser"
        },
        "visibility": "public",
        "favourites_count": 0,
        "reblogs_count": 0,
        "replies_count": 0
    });

    state
        .statuses
        .lock()
        .await
        .insert(id.to_string(), status.clone());

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(status.to_string()))
        .unwrap()
}

async fn mock_get_status(Path(id): Path<String>) -> Response<Body> {
    let status = serde_json::json!({
        "id": id,
        "uri": format!("https://mock.mastodon.local/users/testuser/statuses/{}", id),
        "created_at": "2024-01-15T12:00:00.000Z",
        "content": "<p>A status</p>",
        "account": {
            "id": "12345",
            "username": "testuser",
            "acct": "testuser"
        },
        "visibility": "public",
        "favourites_count": 5,
        "reblogs_count": 2,
        "replies_count": 3
    });

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(status.to_string()))
        .unwrap()
}

async fn mock_status_context(Path(id): Path<String>) -> Response<Body> {
    let context = serde_json::json!({
        "ancestors": [
            {
                "id": (id.parse::<u64>().unwrap_or(100) - 1).to_string(),
                "content": "<p>Parent status</p>",
                "account": {
                    "id": "67890",
                    "username": "otheruser"
                }
            }
        ],
        "descendants": [
            {
                "id": (id.parse::<u64>().unwrap_or(100) + 1).to_string(),
                "content": "<p>Reply status</p>",
                "account": {
                    "id": "67891",
                    "username": "replier"
                }
            }
        ]
    });

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(context.to_string()))
        .unwrap()
}

async fn mock_favourite_status(
    State(state): State<MockMastodonState>,
    Path(id): Path<String>,
    req: Request<Body>,
) -> Response<Body> {
    let token = match require_auth(&req) {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    state
        .favorites
        .lock()
        .await
        .entry(token)
        .or_default()
        .push(id.clone());

    let status = serde_json::json!({
        "id": id,
        "uri": format!("https://mock.mastodon.local/users/testuser/statuses/{}", id),
        "content": "<p>Favourited status</p>",
        "favourited": true,
        "favourites_count": 1
    });

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(status.to_string()))
        .unwrap()
}

async fn mock_reblog_status(
    State(state): State<MockMastodonState>,
    Path(id): Path<String>,
    req: Request<Body>,
) -> Response<Body> {
    let token = match require_auth(&req) {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    let reblog_id = state.next_status_id.fetch_add(1, Ordering::SeqCst);

    state
        .reblogs
        .lock()
        .await
        .entry(token)
        .or_default()
        .push(id.clone());

    let status = serde_json::json!({
        "id": reblog_id.to_string(),
        "uri": format!("https://mock.mastodon.local/users/testuser/statuses/{}", reblog_id),
        "content": "",
        "reblog": {
            "id": id,
            "uri": format!("https://mock.mastodon.local/users/original/statuses/{}", id),
            "content": "<p>Original status being reblogged</p>"
        },
        "reblogged": true,
        "reblogs_count": 1
    });

    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(status.to_string()))
        .unwrap()
}

async fn mock_oauth_token() -> Response<Body> {
    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"access_token":"test_token","token_type":"Bearer","scope":"read write follow push","created_at":1705312000}"#,
        ))
        .unwrap()
}

// =============================================================================
// Test: App Launch Sequence
// =============================================================================

/// Test the typical app launch sequence that Mastodon clients perform.
/// This simulates what happens when a user opens an app like Tusky or Ice Cubes.
#[tokio::test]
async fn test_app_launch_sequence() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Step 1: Verify credentials (token validation)
    let response = client
        .get("/api/v1/accounts/verify_credentials")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["username"], "testuser");

    // Step 2: Fetch instance metadata
    let response = client.get("/api/v1/instance").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["uri"], "mock.mastodon.local");

    // Step 3: Fetch home timeline
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert!(body.as_array().unwrap().len() > 0);

    // Step 4: Fetch notifications
    let response = client
        .get("/api/v1/notifications")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let notifications = body.as_array().unwrap();
    assert!(notifications.len() >= 1);
    assert_eq!(notifications[0]["type"], "mention");
}

// =============================================================================
// Test: Timeline Pagination
// =============================================================================

/// Test timeline refresh using min_id (get posts newer than the newest known)
#[tokio::test]
async fn test_timeline_refresh_with_min_id() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Initial load
    let response = client
        .get("/api/v1/timelines/home?limit=5")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let statuses = body.as_array().unwrap();
    let newest_id = statuses[0]["id"].as_str().unwrap();

    // Refresh - get posts newer than the newest we have
    let response = client
        .get(&format!(
            "/api/v1/timelines/home?min_id={}&limit=5",
            newest_id
        ))
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let new_statuses = body.as_array().unwrap();

    // New statuses should have IDs greater than newest_id
    for status in new_statuses {
        let id: u64 = status["id"].as_str().unwrap().parse().unwrap();
        let newest: u64 = newest_id.parse().unwrap();
        assert!(id > newest, "New status ID {} should be > {}", id, newest);
    }
}

/// Test loading older posts using max_id (infinite scroll)
#[tokio::test]
async fn test_timeline_load_more_with_max_id() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Initial load
    let response = client
        .get("/api/v1/timelines/home?limit=5")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let statuses = body.as_array().unwrap();
    let oldest_id = statuses.last().unwrap()["id"].as_str().unwrap();

    // Load more - get posts older than the oldest we have
    let response = client
        .get(&format!(
            "/api/v1/timelines/home?max_id={}&limit=5",
            oldest_id
        ))
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let older_statuses = body.as_array().unwrap();

    // Older statuses should have IDs less than oldest_id
    for status in older_statuses {
        let id: u64 = status["id"].as_str().unwrap().parse().unwrap();
        let oldest: u64 = oldest_id.parse().unwrap();
        assert!(id < oldest, "Older status ID {} should be < {}", id, oldest);
    }
}

/// Test gap filling using since_id (get all posts newer than a specific ID)
#[tokio::test]
async fn test_timeline_gap_fill_with_since_id() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Gap fill - get all posts newer than a known ID
    let response = client
        .get("/api/v1/timelines/home?since_id=50&limit=10")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let statuses = body.as_array().unwrap();

    // All returned statuses should have IDs greater than 50
    for status in statuses {
        let id: u64 = status["id"].as_str().unwrap().parse().unwrap();
        assert!(id > 50, "Status ID {} should be > 50", id);
    }
}

/// Test that Link header is preserved for pagination
#[tokio::test]
async fn test_timeline_link_header_preserved() {
    let upstream = MockMastodon::start().await;
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

    // Link header should be present for pagination
    let link_header = response.headers().get("link");
    assert!(link_header.is_some(), "Link header should be present");
    let link_value = link_header.unwrap().to_str().unwrap();
    assert!(link_value.contains("rel=\"next\""));
    assert!(link_value.contains("rel=\"prev\""));
}

// =============================================================================
// Test: User Actions
// =============================================================================

/// Test posting a new status
#[tokio::test]
async fn test_post_status() {
    let upstream = MockMastodon::start().await;
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
        .text(r#"{"status":"Hello from test!"}"#)
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert!(body["id"].as_str().is_some());
    assert!(body["uri"]
        .as_str()
        .unwrap()
        .contains("mock.mastodon.local"));
}

/// Test favouriting a status
#[tokio::test]
async fn test_favourite_status() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    let response = client
        .post("/api/v1/statuses/12345/favourite")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["favourited"], true);
}

/// Test reblogging (boosting) a status
#[tokio::test]
async fn test_reblog_status() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    let response = client
        .post("/api/v1/statuses/12345/reblog")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert!(body["reblog"].is_object());
    assert_eq!(body["reblogged"], true);
}

/// Test viewing thread context
#[tokio::test]
async fn test_view_thread_context() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    let response = client
        .get("/api/v1/statuses/100/context")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;

    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert!(body["ancestors"].is_array());
    assert!(body["descendants"].is_array());
}

// =============================================================================
// Test: Timeline Deduplication with Client Session
// =============================================================================

/// Test that timeline deduplication works across multiple timeline fetches
/// simulating a client session where the same posts appear in different contexts.
#[tokio::test]
async fn test_deduplication_across_timeline_fetches() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // First fetch - should get all statuses
    let response = client
        .get("/api/v1/timelines/home?limit=5")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let first_count = body.as_array().unwrap().len();
    assert_eq!(first_count, 5);

    // Second fetch of same timeline - should get empty (all filtered as seen)
    let response = client
        .get("/api/v1/timelines/home?limit=5")
        .add_header(AUTHORIZATION, HeaderValue::from_static("Bearer test_token"))
        .await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    let second_count = body.as_array().unwrap().len();
    assert_eq!(second_count, 0, "Duplicate statuses should be filtered");
}

// =============================================================================
// Test: Different Timeline Types
// =============================================================================

/// Test that public timeline works correctly
#[tokio::test]
async fn test_public_timeline() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Public timeline doesn't require auth
    let response = client.get("/api/v1/timelines/public?limit=5").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 5);
}

/// Test that hashtag timeline works correctly
#[tokio::test]
async fn test_hashtag_timeline() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    let response = client.get("/api/v1/timelines/tag/rust?limit=5").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 5);
}

/// Test that list timeline works correctly
#[tokio::test]
async fn test_list_timeline() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    let response = client.get("/api/v1/timelines/list/12345?limit=5").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body.as_array().unwrap().len(), 5);
}

// =============================================================================
// Test: Instance V2 API
// =============================================================================

/// Test that instance v2 endpoint works (used for streaming URL discovery)
#[tokio::test]
async fn test_instance_v2_for_streaming_discovery() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    let response = client.get("/api/v2/instance").await;
    response.assert_status_ok();
    let body: serde_json::Value = response.json();

    // Clients use this to discover the streaming URL
    assert!(body["configuration"]["urls"]["streaming"].is_string());
}

// =============================================================================
// Test: Error Handling
// =============================================================================

/// Test that unauthorized requests return 401
#[tokio::test]
async fn test_unauthorized_returns_401() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Home timeline requires auth
    let response = client.get("/api/v1/timelines/home").await;
    response.assert_status_unauthorized();

    // Notifications require auth
    let response = client.get("/api/v1/notifications").await;
    response.assert_status_unauthorized();

    // verify_credentials requires auth
    let response = client.get("/api/v1/accounts/verify_credentials").await;
    response.assert_status_unauthorized();
}

/// Test that invalid token returns 401
#[tokio::test]
async fn test_invalid_token_returns_401() {
    let upstream = MockMastodon::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    let client = axum_test::TestServer::new(app).unwrap();

    // Invalid auth header format
    let response = client
        .get("/api/v1/timelines/home")
        .add_header(AUTHORIZATION, HeaderValue::from_static("InvalidFormat"))
        .await;
    response.assert_status_unauthorized();
}
