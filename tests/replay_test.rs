//! Replay tests using recorded traffic fixtures.
//!
//! These tests load pre-recorded (and anonymized) request/response pairs
//! and replay them through the proxy to verify deduplication behavior
//! with real-world data patterns.

mod common;

use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use ivoryvalley::{config::Config, proxy::create_proxy_router, SeenUriStore};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// A recorded HTTP request from a fixture file.
#[derive(Debug, Deserialize)]
struct RecordedRequest {
    method: String,
    path: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: Option<String>,
}

/// A recorded HTTP response from a fixture file.
#[derive(Debug, Deserialize)]
struct RecordedResponse {
    status: u16,
    #[serde(default)]
    headers: HashMap<String, String>,
    body: String,
}

/// A complete request/response exchange from a fixture file.
#[derive(Debug, Deserialize)]
struct RecordedExchange {
    #[allow(dead_code)]
    timestamp: String,
    request: RecordedRequest,
    response: RecordedResponse,
}

/// Load exchanges from a JSONL fixture file.
fn load_fixtures(fixture_name: &str) -> Vec<RecordedExchange> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture_name);

    let file = File::open(&path).unwrap_or_else(|e| {
        panic!("Failed to open fixture file {}: {}", path.display(), e);
    });

    let reader = BufReader::new(file);
    let mut exchanges = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line.expect("Failed to read line");
        if line.trim().is_empty() {
            continue;
        }
        let exchange: RecordedExchange = serde_json::from_str(&line).unwrap_or_else(|e| {
            panic!(
                "Failed to parse fixture line {} in {}: {}",
                line_num + 1,
                fixture_name,
                e
            );
        });
        exchanges.push(exchange);
    }

    exchanges
}

/// Shared state for the replay mock server.
#[derive(Clone)]
struct ReplayServerState {
    /// Queue of responses to return, indexed by request path.
    responses: Arc<Mutex<HashMap<String, Vec<RecordedResponse>>>>,
}

impl ReplayServerState {
    fn new(exchanges: &[RecordedExchange]) -> Self {
        let mut responses: HashMap<String, Vec<RecordedResponse>> = HashMap::new();

        for exchange in exchanges {
            // Extract just the path without query for matching
            let path = exchange.request.path.split('?').next().unwrap_or("/");
            responses
                .entry(path.to_string())
                .or_default()
                .push(RecordedResponse {
                    status: exchange.response.status,
                    headers: exchange.response.headers.clone(),
                    body: exchange.response.body.clone(),
                });
        }

        Self {
            responses: Arc::new(Mutex::new(responses)),
        }
    }

    fn get_next_response(&self, path: &str) -> Option<RecordedResponse> {
        let path_only = path.split('?').next().unwrap_or("/");
        let mut responses = self.responses.lock().unwrap();
        if let Some(queue) = responses.get_mut(path_only) {
            if !queue.is_empty() {
                return Some(queue.remove(0));
            }
        }
        None
    }
}

/// Handler for the replay mock server.
async fn replay_handler(
    axum::extract::State(state): axum::extract::State<ReplayServerState>,
    request: Request<Body>,
) -> Response {
    let path = request
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    if let Some(recorded) = state.get_next_response(path) {
        let mut response =
            Response::builder().status(StatusCode::from_u16(recorded.status).unwrap());

        for (name, value) in &recorded.headers {
            response = response.header(name.as_str(), value.as_str());
        }

        response.body(Body::from(recorded.body)).unwrap()
    } else {
        // No more recorded responses for this path
        (StatusCode::NOT_FOUND, "No recorded response available").into_response()
    }
}

/// Start a replay mock server that returns pre-recorded responses.
async fn start_replay_server(
    exchanges: &[RecordedExchange],
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let state = ReplayServerState::new(exchanges);

    let app = Router::new()
        .route("/{*path}", any(replay_handler))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (addr, handle)
}

#[tokio::test]
async fn test_replay_timeline_deduplication() {
    // Load the sample fixture
    let exchanges = load_fixtures("sample_timeline.jsonl");
    assert!(!exchanges.is_empty(), "Fixture should contain exchanges");

    // Start the replay server
    let (mock_addr, _handle) = start_replay_server(&exchanges).await;

    // Create temporary database
    let temp_dir = common::create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    // Create the proxy
    let config = Config::new(
        &format!("http://{}", mock_addr),
        "127.0.0.1",
        0,
        db_path.clone(),
    );
    let seen_store = SeenUriStore::open(&db_path).unwrap();
    let proxy_router = create_proxy_router(config, std::sync::Arc::new(seen_store));

    // Create a test server
    let test_server = axum_test::TestServer::new(proxy_router).unwrap();

    // First request - should return all 3 statuses
    let response1 = test_server
        .get("/api/v1/timelines/home")
        .add_query_param("limit", "20")
        .add_header(axum::http::header::AUTHORIZATION, "Bearer test_token")
        .await;

    assert_eq!(response1.status_code(), StatusCode::OK);

    let body1: Vec<serde_json::Value> = response1.json();

    // The response should have 2 statuses:
    // - 100001: Original post (passes, URI marked as seen)
    // - 100002: Second post (passes, URI marked as seen)
    // - 100003: Boost of 100001 - uses reblog.uri for dedup, which is same as 100001, so FILTERED!
    assert_eq!(
        body1.len(),
        2,
        "First request should return 2 statuses (boost of first post filtered because original was just seen)"
    );

    // Verify the statuses we got
    let ids1: Vec<&str> = body1.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert!(ids1.contains(&"100001"), "Should contain first post");
    assert!(ids1.contains(&"100002"), "Should contain second post");
    assert!(
        !ids1.contains(&"100003"),
        "Should NOT contain boost - original was just seen"
    );

    // Second request
    let response2 = test_server
        .get("/api/v1/timelines/home")
        .add_query_param("limit", "20")
        .add_query_param("min_id", "100003")
        .add_header(axum::http::header::AUTHORIZATION, "Bearer test_token")
        .await;

    assert_eq!(response2.status_code(), StatusCode::OK);

    let body2: Vec<serde_json::Value> = response2.json();

    // The second response in fixtures has 2 statuses:
    // - 100004: New post (passes, URI marked as seen)
    // - 100005: Another boost of 100001 - uses reblog.uri which was already seen, so FILTERED!
    assert_eq!(
        body2.len(),
        1,
        "Second request should return only 1 status (boost of first post filtered)"
    );

    let ids2: Vec<&str> = body2.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert!(ids2.contains(&"100004"), "Should contain fourth post");
    assert!(
        !ids2.contains(&"100005"),
        "Should NOT contain another boost of first post"
    );
}

#[tokio::test]
async fn test_fixture_loading() {
    let exchanges = load_fixtures("sample_timeline.jsonl");

    assert_eq!(exchanges.len(), 2, "Should have 2 exchanges");

    // Verify first exchange
    assert_eq!(exchanges[0].request.method, "GET");
    assert!(exchanges[0].request.path.contains("/api/v1/timelines/home"));
    assert_eq!(exchanges[0].response.status, 200);

    // Parse the response body to verify it's valid JSON
    let body: Vec<serde_json::Value> = serde_json::from_str(&exchanges[0].response.body).unwrap();
    assert_eq!(body.len(), 3, "First response should have 3 statuses");
}

/// Test deduplication with real anonymized traffic from a live Mastodon session.
///
/// This test uses traffic recorded from an actual browsing session, anonymized
/// to remove PII. It verifies that deduplication works correctly with real-world
/// data patterns including:
/// - Multiple timeline fetches with pagination
/// - Reblogs (boosts) of posts from various instances
/// - Duplicate boosts of the same content across requests
#[tokio::test]
async fn test_real_traffic_deduplication() {
    // Load real anonymized traffic
    let exchanges = load_fixtures("real_timeline.jsonl");
    assert_eq!(exchanges.len(), 5, "Should have 5 timeline requests");

    // Start the replay server
    let (mock_addr, _handle) = start_replay_server(&exchanges).await;

    // Create temporary database
    let temp_dir = common::create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    // Create the proxy
    let config = Config::new(
        &format!("http://{}", mock_addr),
        "127.0.0.1",
        0,
        db_path.clone(),
    );
    let seen_store = SeenUriStore::open(&db_path).unwrap();
    let proxy_router = create_proxy_router(config, std::sync::Arc::new(seen_store));

    let test_server = axum_test::TestServer::new(proxy_router).unwrap();

    // Track total statuses received vs upstream
    let mut total_upstream = 0;
    let mut total_received = 0;

    // Replay all 5 requests
    for (i, exchange) in exchanges.iter().enumerate() {
        let upstream_body: Vec<serde_json::Value> =
            serde_json::from_str(&exchange.response.body).unwrap();
        total_upstream += upstream_body.len();

        let response = test_server
            .get("/api/v1/timelines/home")
            .add_query_param("limit", "20")
            .add_header(axum::http::header::AUTHORIZATION, "Bearer test_token")
            .await;

        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "Request {} should succeed",
            i + 1
        );

        let body: Vec<serde_json::Value> = response.json();
        total_received += body.len();

        // Each response should have same or fewer statuses than upstream
        // (deduplication removes duplicates)
        assert!(
            body.len() <= upstream_body.len(),
            "Request {} should not have more statuses than upstream",
            i + 1
        );
    }

    // With deduplication, we should receive fewer total statuses
    // The fixture has 1 duplicate boost (status 100241 in request 5 is a boost
    // of the same post as status 100042 in request 1)
    assert!(
        total_received < total_upstream,
        "Deduplication should reduce total statuses: received {} vs upstream {}",
        total_received,
        total_upstream
    );

    // Verify at least 1 status was filtered (the duplicate boost)
    let filtered = total_upstream - total_received;
    assert!(
        filtered >= 1,
        "At least 1 duplicate should be filtered, got {}",
        filtered
    );
}
