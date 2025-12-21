//! HTTP proxy handlers
//!
//! This module implements the core proxy functionality that forwards requests
//! from Mastodon clients to the upstream Mastodon server.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    Router,
};

use crate::config::{AppState, Config};

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

/// Create the proxy router with all routes
pub fn create_proxy_router(config: Config) -> Router {
    let state = AppState::new(config);

    Router::new().fallback(proxy_handler).with_state(state)
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
        let body_bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
            .await
            .map_err(|e| ProxyError::BodyRead(e.to_string()))?;
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

    // Build the response
    let mut response = Response::builder().status(status);

    // Forward response headers
    for (name, value) in response_headers.iter() {
        if !STRIP_HEADERS.contains(&name.as_str().to_lowercase().as_str()) {
            response = response.header(name, value);
        }
    }

    response
        .body(Body::from(body))
        .map_err(|e| ProxyError::ResponseBuild(e.to_string()))
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
            .unwrap()
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
}
