//! HTTP proxy handlers
//!
//! This module implements the core proxy functionality that forwards requests
//! from Mastodon clients to the upstream Mastodon server.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use crate::config::{AppState, Config};
use crate::db::{extract_dedup_uri, SeenUriStore};
use crate::websocket::{streaming_handler, WebSocketState};
use std::sync::Arc;

/// Headers that should be passed through from client to upstream
const PASSTHROUGH_HEADERS: &[&str] = &[
    "authorization",
    "content-type",
    "accept",
    "accept-language",
    "user-agent",
    "content-length",
];

/// Headers that should NOT be forwarded
const STRIP_HEADERS: &[&str] = &["host", "connection", "transfer-encoding"];

/// Timeline endpoint prefixes that should have deduplication applied
const TIMELINE_ENDPOINTS: &[&str] = &[
    "/api/v1/timelines/home",
    "/api/v1/timelines/public",
    "/api/v1/timelines/list/",
    "/api/v1/timelines/tag/",
];

/// Check if the given path is a timeline endpoint that should be filtered
fn is_timeline_endpoint(path: &str) -> bool {
    // Extract just the path without query parameters
    let path_only = path.split('?').next().unwrap_or(path);

    TIMELINE_ENDPOINTS
        .iter()
        .any(|prefix| path_only.starts_with(prefix))
}

/// Create the proxy router with all routes
pub fn create_proxy_router(config: Config, seen_store: SeenUriStore) -> Router {
    // Wrap the store in Arc to share between HTTP proxy and WebSocket handlers
    let seen_store = Arc::new(seen_store);

    let app_state = AppState::new(config, seen_store.clone());
    let ws_state = WebSocketState::new(app_state.clone(), seen_store);

    // The streaming route uses WebSocketState (with SeenUriStore for deduplication).
    // The fallback HTTP proxy uses AppState. Axum's .with_state() applies to
    // routes added before that call, so the order here is intentional.
    Router::new()
        .route("/api/v1/streaming", get(streaming_handler))
        .with_state(ws_state)
        .fallback(proxy_handler)
        .with_state(app_state)
}

/// Main proxy handler that forwards all requests to the upstream server
async fn proxy_handler(
    State(state): State<AppState>,
    request: Request<Body>,
) -> Result<Response, ProxyError> {
    let method = request.method().clone();
    let path = request
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    // Determine if this is a timeline endpoint that should be filtered
    let should_filter = method == Method::GET && is_timeline_endpoint(path);

    // Build the upstream URL
    let upstream_url = format!("{}{}", state.config.upstream_url, path);

    // Build the upstream request
    let mut upstream_request = state.http_client.request(method.clone(), &upstream_url);

    // Forward headers
    let headers = build_upstream_headers(request.headers());
    for (name, value) in headers.iter() {
        if let Ok(value_str) = value.to_str() {
            upstream_request = upstream_request.header(name.as_str(), value_str);
        }
    }

    // Forward body for methods that have one
    if method == Method::POST || method == Method::PUT || method == Method::PATCH {
        let max_body_size = state.config.max_body_size;
        let body_bytes = axum::body::to_bytes(request.into_body(), max_body_size)
            .await
            .map_err(|e| {
                // Check if this is a length limit error
                let error_msg = e.to_string();
                if error_msg.contains("length limit exceeded") {
                    ProxyError::PayloadTooLarge
                } else {
                    ProxyError::BodyRead(error_msg)
                }
            })?;
        upstream_request = upstream_request.body(body_bytes);
    }

    // Send request to upstream
    let upstream_response = upstream_request
        .send()
        .await
        .map_err(|e| ProxyError::Upstream(e.to_string()))?;

    // Convert the response
    let status = upstream_response.status();
    let response_headers = upstream_response.headers().clone();
    let body = upstream_response
        .bytes()
        .await
        .map_err(|e| ProxyError::ResponseRead(e.to_string()))?;

    // Filter timeline responses if applicable
    let final_body = if should_filter && status.is_success() {
        filter_timeline_response(&body, &state)
    } else {
        body.to_vec()
    };

    // Build the response
    let mut response = Response::builder().status(status);

    // Forward response headers (except Content-Length which may have changed)
    for (name, value) in response_headers.iter() {
        let name_lower = name.as_str().to_lowercase();
        if !STRIP_HEADERS.contains(&name_lower.as_str()) && name_lower != "content-length" {
            response = response.header(name, value);
        }
    }

    response
        .body(Body::from(final_body))
        .map_err(|e| ProxyError::ResponseBuild(e.to_string()))
}

/// Filter a timeline response, removing statuses that have already been seen.
///
/// Returns the filtered JSON as bytes. If parsing fails, returns the original body unchanged.
fn filter_timeline_response(body: &[u8], state: &AppState) -> Vec<u8> {
    // Try to parse the body as a JSON array of statuses
    let statuses: Vec<serde_json::Value> = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("Failed to parse timeline response as JSON array: {}", e);
            // If we can't parse it, just pass through unchanged
            return body.to_vec();
        }
    };

    let original_count = statuses.len();
    tracing::debug!("Processing {} statuses for deduplication", original_count);

    // Filter out statuses we've already seen
    let mut filtered = Vec::new();
    let mut filtered_count = 0;
    let mut error_count = 0;

    for status in statuses {
        // Extract the deduplication URI
        let should_include = if let Some(uri) = extract_dedup_uri(&status) {
            // Atomically check if seen and mark as seen
            match state.seen_uri_store.check_and_mark(uri) {
                Ok(was_seen) => {
                    if was_seen {
                        tracing::debug!("Filtered duplicate status with URI: {}", uri);
                        filtered_count += 1;
                        false
                    } else {
                        tracing::trace!("Allowing new status with URI: {}", uri);
                        true
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to check/mark URI {}: {}", uri, e);
                    error_count += 1;
                    // On error, pass through the status
                    true
                }
            }
        } else {
            // No URI to deduplicate on, pass through
            tracing::trace!("Allowing status without URI field");
            true
        };

        if should_include {
            filtered.push(status);
        }
    }

    let final_count = filtered.len();
    if filtered_count > 0 || error_count > 0 {
        tracing::info!(
            "Timeline filtering: {} total, {} filtered, {} passed, {} errors",
            original_count,
            filtered_count,
            final_count,
            error_count
        );
    }

    // Serialize the filtered list back to JSON
    serde_json::to_vec(&filtered).unwrap_or_else(|e| {
        tracing::error!("Failed to serialize filtered timeline: {}", e);
        body.to_vec()
    })
}

/// Build headers to send to upstream, filtering and transforming as needed
fn build_upstream_headers(client_headers: &HeaderMap) -> HeaderMap {
    let mut upstream_headers = HeaderMap::new();

    for (name, value) in client_headers.iter() {
        let name_lower = name.as_str().to_lowercase();

        // Skip headers we shouldn't forward
        if STRIP_HEADERS.contains(&name_lower.as_str()) {
            continue;
        }

        // Only forward known passthrough headers
        if PASSTHROUGH_HEADERS.contains(&name_lower.as_str()) {
            upstream_headers.insert(name.clone(), value.clone());
        }
    }

    upstream_headers
}

/// Errors that can occur during proxying
#[derive(Debug)]
pub enum ProxyError {
    /// Failed to read request body
    BodyRead(String),
    /// Request body exceeds the configured size limit
    PayloadTooLarge,
    /// Failed to reach upstream server
    Upstream(String),
    /// Failed to read response from upstream
    ResponseRead(String),
    /// Failed to build response
    ResponseBuild(String),
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ProxyError::BodyRead(e) => (StatusCode::BAD_REQUEST, format!("Body read error: {}", e)),
            ProxyError::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Request body exceeds maximum allowed size".to_string(),
            ),
            ProxyError::Upstream(e) => (StatusCode::BAD_GATEWAY, format!("Upstream error: {}", e)),
            ProxyError::ResponseRead(e) => (
                StatusCode::BAD_GATEWAY,
                format!("Response read error: {}", e),
            ),
            ProxyError::ResponseBuild(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Response build error: {}", e),
            ),
        };

        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(r#"{{"error":"{}"}}"#, message)))
            .unwrap_or_else(|_| {
                // Fallback: minimal response that always succeeds
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .expect("minimal response build should never fail")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_upstream_headers_filters_host() {
        let mut client_headers = HeaderMap::new();
        client_headers.insert("host", "proxy.local".parse().unwrap());
        client_headers.insert("authorization", "Bearer token".parse().unwrap());

        let upstream = build_upstream_headers(&client_headers);

        assert!(upstream.get("host").is_none());
        assert!(upstream.get("authorization").is_some());
    }

    #[test]
    fn test_build_upstream_headers_passes_auth() {
        let mut client_headers = HeaderMap::new();
        client_headers.insert("authorization", "Bearer test_token".parse().unwrap());
        client_headers.insert("content-type", "application/json".parse().unwrap());

        let upstream = build_upstream_headers(&client_headers);

        assert_eq!(upstream.get("authorization").unwrap(), "Bearer test_token");
        assert_eq!(upstream.get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn test_is_timeline_endpoint_home() {
        assert!(is_timeline_endpoint("/api/v1/timelines/home"));
        assert!(is_timeline_endpoint("/api/v1/timelines/home?limit=20"));
        assert!(is_timeline_endpoint(
            "/api/v1/timelines/home?max_id=123&limit=20"
        ));
    }

    #[test]
    fn test_is_timeline_endpoint_public() {
        assert!(is_timeline_endpoint("/api/v1/timelines/public"));
        assert!(is_timeline_endpoint("/api/v1/timelines/public?local=true"));
    }

    #[test]
    fn test_is_timeline_endpoint_list() {
        assert!(is_timeline_endpoint("/api/v1/timelines/list/12345"));
        assert!(is_timeline_endpoint(
            "/api/v1/timelines/list/12345?limit=20"
        ));
    }

    #[test]
    fn test_is_timeline_endpoint_tag() {
        assert!(is_timeline_endpoint("/api/v1/timelines/tag/rust"));
        assert!(is_timeline_endpoint(
            "/api/v1/timelines/tag/mastodon?limit=40"
        ));
    }

    #[test]
    fn test_is_timeline_endpoint_non_timeline() {
        assert!(!is_timeline_endpoint("/api/v1/accounts/verify_credentials"));
        assert!(!is_timeline_endpoint("/api/v1/statuses"));
        assert!(!is_timeline_endpoint("/api/v1/notifications"));
        assert!(!is_timeline_endpoint("/oauth/token"));
    }

    #[tokio::test]
    async fn test_proxy_error_into_response_body_read() {
        let error = ProxyError::BodyRead("test error".to_string());
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Body read error"));
        assert!(body_str.contains("test error"));
    }

    #[tokio::test]
    async fn test_proxy_error_into_response_upstream() {
        let error = ProxyError::Upstream("connection refused".to_string());
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Upstream error"));
        assert!(body_str.contains("connection refused"));
    }

    #[tokio::test]
    async fn test_proxy_error_into_response_response_read() {
        let error = ProxyError::ResponseRead("timeout".to_string());
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Response read error"));
    }

    #[tokio::test]
    async fn test_proxy_error_into_response_response_build() {
        let error = ProxyError::ResponseBuild("invalid header".to_string());
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Response build error"));
    }
}
