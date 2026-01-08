//! HTTP proxy handlers
//!
//! This module implements the core proxy functionality that forwards requests
//! from Mastodon clients to the upstream Mastodon server.

use axum::{
    body::Body,
    extract::{Query, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::config::{AppState, Config};
use crate::db::{extract_dedup_uri, SeenUriStore};
use crate::recording::{now_timestamp, RecordedExchange, RecordedRequest, RecordedResponse};
use crate::websocket::{streaming_handler, WebSocketState};
use flate2::read::{DeflateDecoder, GzDecoder};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

/// Headers that should NOT be forwarded to upstream
const STRIP_HEADERS: &[&str] = &["host", "connection", "transfer-encoding"];

/// Timeline endpoint prefixes that should have deduplication applied
const TIMELINE_ENDPOINTS: &[&str] = &[
    "/api/v1/timelines/home",
    "/api/v1/timelines/public",
    "/api/v1/timelines/list/",
    "/api/v1/timelines/tag/",
    "/api/v1/timelines/link",
    "/api/v1/trends/statuses",
];

/// Query parameters for the health endpoint
#[derive(Debug, Deserialize)]
pub struct HealthQuery {
    /// If true, perform deep health checks (database, etc.)
    #[serde(default)]
    pub deep: bool,
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    /// Overall health status
    pub status: &'static str,
    /// Application version
    pub version: &'static str,
    /// Optional detailed checks (only present for deep health checks)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<HealthChecks>,
}

/// Detailed health check results
#[derive(Debug, Serialize)]
pub struct HealthChecks {
    /// Database connectivity status
    pub database: &'static str,
}

/// Health check endpoint handler
///
/// Returns 200 OK with health status and version information.
/// Use `?deep=true` to include database connectivity check.
async fn health_handler(
    State(state): State<AppState>,
    Query(query): Query<HealthQuery>,
) -> Json<HealthResponse> {
    let checks = if query.deep {
        // Perform deep health check by verifying database connectivity
        let db_status = match state.seen_uri_store.is_seen("__health_check__") {
            Ok(_) => "ok",
            Err(_) => "error",
        };
        Some(HealthChecks {
            database: db_status,
        })
    } else {
        None
    };

    Json(HealthResponse {
        status: "healthy",
        version: env!("CARGO_PKG_VERSION"),
        checks,
    })
}

/// Extract the proxy's base URL from the incoming request for redirect rewriting.
///
/// This determines how clients are reaching the proxy so we can rewrite
/// upstream redirect URLs to point back to the proxy.
fn get_proxy_base_url(headers: &HeaderMap, uri: &axum::http::Uri) -> Option<String> {
    // Try to get the Host header
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .or_else(|| uri.host())?;

    // Determine scheme - check X-Forwarded-Proto first (for reverse proxies),
    // then fall back to the URI scheme, then default to http
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .or_else(|| uri.scheme_str())
        .unwrap_or("http");

    Some(format!("{}://{}", scheme, host))
}

/// Rewrite a Set-Cookie header for proxy compatibility.
///
/// Removes Domain and Secure attributes so cookies work on the proxy origin.
fn rewrite_set_cookie_header(cookie: &str) -> String {
    let mut parts: Vec<&str> = cookie.split(';').map(|s| s.trim()).collect();

    // Filter out Domain and Secure attributes
    parts.retain(|part| {
        let lower = part.to_lowercase();
        !lower.starts_with("domain=") && lower != "secure"
    });

    parts.join("; ")
}

/// Rewrite a Location header value, replacing the upstream URL with the proxy URL.
fn rewrite_location_header(
    location: &str,
    upstream_url: &str,
    proxy_base_url: &Option<String>,
) -> String {
    let Some(ref proxy_url) = proxy_base_url else {
        return location.to_string();
    };

    // Parse both URLs to compare their origins properly
    let Ok(location_parsed) = url::Url::parse(location) else {
        return location.to_string();
    };

    let Ok(upstream_parsed) = url::Url::parse(upstream_url) else {
        return location.to_string();
    };

    // Compare scheme, host, and port (origins must match exactly)
    let location_origin = location_parsed.origin();
    let upstream_origin = upstream_parsed.origin();

    if location_origin == upstream_origin {
        // Parse proxy URL to extract its components
        if let Ok(proxy_parsed) = url::Url::parse(proxy_url) {
            // Build new URL with proxy origin but location's path/query/fragment
            let mut new_url = proxy_parsed.clone();
            new_url.set_path(location_parsed.path());
            new_url.set_query(location_parsed.query());
            new_url.set_fragment(location_parsed.fragment());
            return new_url.to_string();
        }
    }

    location.to_string()
}

/// Check if the given path is a timeline endpoint that should be filtered
fn is_timeline_endpoint(path: &str) -> bool {
    // Extract just the path without query parameters
    let path_only = path.split('?').next().unwrap_or(path);

    TIMELINE_ENDPOINTS
        .iter()
        .any(|prefix| path_only.starts_with(prefix))
}

/// Create the proxy router with all routes
pub fn create_proxy_router(config: Config, seen_store: Arc<SeenUriStore>) -> Router {
    // The store is already wrapped in Arc for sharing between HTTP proxy,
    // WebSocket handlers, and the background cleanup task

    let app_state = AppState::new(config, seen_store.clone());
    let ws_state = WebSocketState::new(app_state.clone(), seen_store);

    // The streaming route uses WebSocketState (with SeenUriStore for deduplication).
    // The health route uses AppState for optional deep health checks.
    // The fallback HTTP proxy uses AppState. Axum's .with_state() applies to
    // routes added before that call, so the order here is intentional.
    Router::new()
        .route("/api/v1/streaming", get(streaming_handler))
        .with_state(ws_state)
        .route("/health", get(health_handler))
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
        .unwrap_or("/")
        .to_string();

    // Capture the proxy's base URL from the Host header for rewriting redirects
    let proxy_base_url = get_proxy_base_url(request.headers(), request.uri());

    // Determine if this is a timeline endpoint that should be filtered
    let should_filter = method == Method::GET && is_timeline_endpoint(&path);

    // Check if we should record this request (only API requests)
    let should_record = state.traffic_recorder.is_some() && path.starts_with("/api/");

    // Capture request headers for recording
    let request_headers_for_recording: HashMap<String, String> = if should_record {
        request
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.as_str().to_string(), v.to_string()))
            })
            .collect()
    } else {
        HashMap::new()
    };

    // Log request path for debugging
    // All API requests at trace level (useful for discovering which endpoints clients use)
    if path.starts_with("/api/") {
        tracing::trace!(
            "API request: {} {} (dedup: {})",
            method,
            path,
            should_filter
        );
    }
    // Timeline requests with filtering at debug level
    if should_filter {
        tracing::debug!("Timeline request (filtering enabled): {} {}", method, path);
    }

    // Build the upstream URL
    let upstream_url = format!("{}{}", state.config.upstream_url, path);

    // Build the upstream request
    let mut upstream_request = state.http_client.request(method.clone(), &upstream_url);

    // Forward headers (rewriting Origin/Referer for CSRF)
    let headers = build_upstream_headers(request.headers(), &state.config.upstream_url);
    for (name, value) in headers.iter() {
        if let Ok(value_str) = value.to_str() {
            upstream_request = upstream_request.header(name.as_str(), value_str);
        }
    }

    // Forward body for methods that have one, capturing for recording if needed
    let request_body_for_recording: Option<String> =
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

            // Capture body for recording (as UTF-8 string if possible)
            let body_for_recording = if should_record {
                String::from_utf8(body_bytes.to_vec()).ok()
            } else {
                None
            };

            upstream_request = upstream_request.body(body_bytes);
            body_for_recording
        } else {
            None
        };

    // Send request to upstream
    let upstream_response = upstream_request.send().await.map_err(|e| {
        if e.is_timeout() {
            ProxyError::Timeout(e.to_string())
        } else {
            ProxyError::Upstream(e.to_string())
        }
    })?;

    // Convert the response
    let status = upstream_response.status();
    let response_headers = upstream_response.headers().clone();
    let raw_body = upstream_response
        .bytes()
        .await
        .map_err(|e| ProxyError::ResponseRead(e.to_string()))?;

    // Decompress the body if needed (gzip or deflate)
    let body = decompress_response_body(&raw_body, &response_headers)?;

    // Record the exchange if traffic recording is enabled
    if should_record {
        if let Some(ref recorder) = state.traffic_recorder {
            let response_headers_map: HashMap<String, String> = response_headers
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|v| (name.as_str().to_string(), v.to_string()))
                })
                .collect();

            let exchange = RecordedExchange {
                timestamp: now_timestamp(),
                request: RecordedRequest {
                    method: method.to_string(),
                    path: path.clone(),
                    headers: request_headers_for_recording,
                    body: request_body_for_recording,
                },
                response: RecordedResponse {
                    status: status.as_u16(),
                    headers: response_headers_map,
                    body: String::from_utf8_lossy(&body).to_string(),
                },
            };

            if let Err(e) = recorder.record(&exchange) {
                tracing::warn!("Failed to record traffic: {}", e);
            }
        }
    }

    // Filter timeline responses if applicable
    let final_body = if should_filter && status.is_success() {
        filter_timeline_response(&body, &state)
    } else {
        body
    };

    // Build the response
    let mut response = Response::builder().status(status);

    // Forward response headers (except Content-Length which may have changed and
    // Content-Encoding since we decompress before forwarding)
    for (name, value) in response_headers.iter() {
        let name_lower = name.as_str().to_lowercase();
        if STRIP_HEADERS.contains(&name_lower.as_str())
            || name_lower == "content-length"
            || name_lower == "content-encoding"
        {
            continue;
        }

        // Rewrite Location headers to point back to the proxy
        if name_lower == "location" {
            if let Ok(location_str) = value.to_str() {
                let rewritten = rewrite_location_header(
                    location_str,
                    &state.config.upstream_url,
                    &proxy_base_url,
                );
                if let Ok(header_value) = rewritten.parse::<header::HeaderValue>() {
                    tracing::debug!("Rewrote Location header: {} -> {}", location_str, rewritten);
                    response = response.header(name, header_value);
                    continue;
                }
            }
        }

        // Rewrite Set-Cookie headers to work with the proxy origin
        if name_lower == "set-cookie" {
            if let Ok(cookie_str) = value.to_str() {
                let rewritten = rewrite_set_cookie_header(cookie_str);
                if let Ok(header_value) = rewritten.parse::<header::HeaderValue>() {
                    tracing::debug!("Rewrote Set-Cookie header: {} -> {}", cookie_str, rewritten);
                    response = response.header(name, header_value);
                    continue;
                }
            }
        }

        response = response.header(name, value);
    }

    response
        .body(Body::from(final_body))
        .map_err(|e| ProxyError::ResponseBuild(e.to_string()))
}

/// Decompress response body if Content-Encoding indicates compression.
///
/// Returns the decompressed body bytes, or the original body if not compressed.
/// Returns an error if decompression fails for a compressed response.
fn decompress_response_body(
    body: &[u8],
    headers: &reqwest::header::HeaderMap,
) -> Result<Vec<u8>, ProxyError> {
    let content_encoding = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_lowercase());

    match content_encoding.as_deref() {
        Some("gzip") => {
            let mut decoder = GzDecoder::new(body);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed).map_err(|e| {
                tracing::error!("Failed to decompress gzip response: {}", e);
                ProxyError::Decompression(format!("gzip decompression failed: {}", e))
            })?;
            tracing::debug!(
                "Decompressed gzip response: {} -> {} bytes",
                body.len(),
                decompressed.len()
            );
            Ok(decompressed)
        }
        Some("deflate") => {
            let mut decoder = DeflateDecoder::new(body);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed).map_err(|e| {
                tracing::error!("Failed to decompress deflate response: {}", e);
                ProxyError::Decompression(format!("deflate decompression failed: {}", e))
            })?;
            tracing::debug!(
                "Decompressed deflate response: {} -> {} bytes",
                body.len(),
                decompressed.len()
            );
            Ok(decompressed)
        }
        Some(encoding) => {
            // Unknown encoding - pass through unchanged
            tracing::warn!(
                "Unknown Content-Encoding '{}', passing through unchanged",
                encoding
            );
            Ok(body.to_vec())
        }
        None => {
            // No compression
            Ok(body.to_vec())
        }
    }
}

/// Filter a timeline response, removing statuses that have already been seen.
///
/// Returns the filtered JSON as bytes. If parsing fails, returns the original body unchanged.
/// Handles edge cases like empty bodies (304 Not Modified) and non-JSON responses (HTML errors).
fn filter_timeline_response(body: &[u8], state: &AppState) -> Vec<u8> {
    // Handle empty body (e.g., 304 Not Modified responses)
    if body.is_empty() {
        tracing::debug!("Empty timeline response body, passing through");
        return body.to_vec();
    }

    // Quick check if response looks like a JSON array (starts with '[')
    // This avoids trying to parse HTML error pages or other non-JSON content
    let first_byte = body.first().copied().unwrap_or(0);
    if first_byte != b'[' {
        tracing::debug!(
            "Timeline response is not a JSON array (starts with '{}'{}), passing through",
            first_byte as char,
            if body.len() > 1 {
                format!("{}...", body.get(1).map(|b| *b as char).unwrap_or(' '))
            } else {
                String::new()
            }
        );
        return body.to_vec();
    }

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

/// Build headers to send to upstream, filtering and transforming as needed.
///
/// Rewrites Origin and Referer headers to point to the upstream server
/// so CSRF protection works correctly. Sets the Host header to the upstream
/// hostname (similar to nginx's `proxy_set_header Host $proxy_host`).
fn build_upstream_headers(client_headers: &HeaderMap, upstream_url: &str) -> HeaderMap {
    let mut upstream_headers = HeaderMap::new();

    // Parse upstream URL to get its origin and host
    let upstream_parsed = url::Url::parse(upstream_url).ok();
    let upstream_origin = upstream_parsed
        .as_ref()
        .map(|u| format!("{}://{}", u.scheme(), u.host_str().unwrap_or("")));

    // Set the Host header to the upstream hostname
    // This is critical for upstream servers that check the Host header
    if let Some(ref parsed) = upstream_parsed {
        let host_value = build_host_header_value(parsed);
        if let Ok(header_value) = host_value.parse::<header::HeaderValue>() {
            upstream_headers.insert(header::HOST, header_value);
            tracing::debug!("Set Host header to upstream: {}", host_value);
        }
    }

    for (name, value) in client_headers.iter() {
        let name_lower = name.as_str().to_lowercase();

        // Skip headers we shouldn't forward
        if STRIP_HEADERS.contains(&name_lower.as_str()) {
            continue;
        }

        // Rewrite Origin header to upstream (for CSRF protection)
        if name_lower == "origin" {
            if let Some(ref origin) = upstream_origin {
                if let Ok(header_value) = origin.parse::<header::HeaderValue>() {
                    tracing::debug!("Rewrote Origin header to upstream: {}", origin);
                    upstream_headers.insert(name.clone(), header_value);
                    continue;
                }
            }
        }

        // Rewrite Referer header to upstream (for CSRF protection)
        if name_lower == "referer" {
            if let Ok(referer_str) = value.to_str() {
                // Replace the proxy origin with upstream origin in the referer
                if let (Ok(referer_url), Ok(upstream_parsed)) =
                    (url::Url::parse(referer_str), url::Url::parse(upstream_url))
                {
                    let mut new_referer = upstream_parsed.clone();
                    new_referer.set_path(referer_url.path());
                    new_referer.set_query(referer_url.query());
                    new_referer.set_fragment(referer_url.fragment());
                    if let Ok(header_value) = new_referer.as_str().parse::<header::HeaderValue>() {
                        tracing::debug!(
                            "Rewrote Referer header: {} -> {}",
                            referer_str,
                            new_referer
                        );
                        upstream_headers.insert(name.clone(), header_value);
                        continue;
                    }
                }
            }
        }

        // Forward all other headers
        upstream_headers.insert(name.clone(), value.clone());
    }

    // Add Accept-Encoding to request gzip/deflate from upstream
    if let Ok(header_value) = "gzip, deflate".parse::<header::HeaderValue>() {
        upstream_headers.insert(header::ACCEPT_ENCODING, header_value);
    }

    upstream_headers
}

/// Build the Host header value from a parsed URL.
///
/// Follows HTTP conventions: omit default ports (80 for http, 443 for https),
/// include non-default ports.
fn build_host_header_value(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or("");
    let port = url.port();

    // Determine if we should include the port
    let include_port = match (url.scheme(), port) {
        ("http", Some(80)) => false,   // Default HTTP port
        ("https", Some(443)) => false, // Default HTTPS port
        (_, Some(_)) => true,          // Non-default port
        (_, None) => false,            // No port specified
    };

    if include_port {
        format!("{}:{}", host, port.unwrap())
    } else {
        host.to_string()
    }
}

/// Errors that can occur during proxying
#[derive(Debug)]
pub enum ProxyError {
    /// Failed to read request body
    BodyRead(String),
    /// Request body exceeds the configured size limit
    PayloadTooLarge,
    /// Request to upstream server timed out
    Timeout(String),
    /// Failed to reach upstream server
    Upstream(String),
    /// Failed to read response from upstream
    ResponseRead(String),
    /// Failed to build response
    ResponseBuild(String),
    /// Failed to decompress response body
    Decompression(String),
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ProxyError::BodyRead(e) => (StatusCode::BAD_REQUEST, format!("Body read error: {}", e)),
            ProxyError::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Request body exceeds maximum allowed size".to_string(),
            ),
            ProxyError::Timeout(e) => (
                StatusCode::GATEWAY_TIMEOUT,
                format!("Gateway timeout: {}", e),
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
            ProxyError::Decompression(e) => (
                StatusCode::BAD_GATEWAY,
                format!("Decompression error: {}", e),
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
    fn test_build_upstream_headers_rewrites_host_to_upstream() {
        let mut client_headers = HeaderMap::new();
        client_headers.insert("host", "proxy.local".parse().unwrap());
        client_headers.insert("authorization", "Bearer token".parse().unwrap());

        let upstream = build_upstream_headers(&client_headers, "https://mastodon.social");

        // Host header should be rewritten to upstream hostname, not stripped
        assert_eq!(upstream.get("host").unwrap(), "mastodon.social");
        assert!(upstream.get("authorization").is_some());
    }

    #[test]
    fn test_build_upstream_headers_sets_host_with_port() {
        let mut client_headers = HeaderMap::new();
        client_headers.insert("host", "localhost:8080".parse().unwrap());

        let upstream = build_upstream_headers(&client_headers, "https://example.com:8443");

        // Host header should include port when non-default
        assert_eq!(upstream.get("host").unwrap(), "example.com:8443");
    }

    #[test]
    fn test_build_upstream_headers_sets_host_default_https_port() {
        let mut client_headers = HeaderMap::new();
        client_headers.insert("host", "localhost:8080".parse().unwrap());

        let upstream = build_upstream_headers(&client_headers, "https://example.com:443");

        // Default HTTPS port (443) should be omitted
        assert_eq!(upstream.get("host").unwrap(), "example.com");
    }

    #[test]
    fn test_build_upstream_headers_sets_host_default_http_port() {
        let mut client_headers = HeaderMap::new();
        client_headers.insert("host", "localhost:8080".parse().unwrap());

        let upstream = build_upstream_headers(&client_headers, "http://example.com:80");

        // Default HTTP port (80) should be omitted
        assert_eq!(upstream.get("host").unwrap(), "example.com");
    }

    #[test]
    fn test_build_upstream_headers_passes_auth() {
        let mut client_headers = HeaderMap::new();
        client_headers.insert("authorization", "Bearer test_token".parse().unwrap());
        client_headers.insert("content-type", "application/json".parse().unwrap());

        let upstream = build_upstream_headers(&client_headers, "https://mastodon.social");

        assert_eq!(upstream.get("authorization").unwrap(), "Bearer test_token");
        assert_eq!(upstream.get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn test_build_upstream_headers_rewrites_origin() {
        let mut client_headers = HeaderMap::new();
        client_headers.insert("origin", "http://localhost:8080".parse().unwrap());

        let upstream = build_upstream_headers(&client_headers, "https://nerdculture.de");

        assert_eq!(upstream.get("origin").unwrap(), "https://nerdculture.de");
    }

    #[test]
    fn test_build_upstream_headers_rewrites_referer() {
        let mut client_headers = HeaderMap::new();
        client_headers.insert(
            "referer",
            "http://localhost:8080/auth/sign_in".parse().unwrap(),
        );

        let upstream = build_upstream_headers(&client_headers, "https://nerdculture.de");

        assert_eq!(
            upstream.get("referer").unwrap(),
            "https://nerdculture.de/auth/sign_in"
        );
    }

    #[test]
    fn test_build_upstream_headers_sets_accept_encoding() {
        // Proxy now sends Accept-Encoding to request compressed responses
        // and decompresses them before parsing for deduplication
        let mut client_headers = HeaderMap::new();
        client_headers.insert("accept-encoding", "br".parse().unwrap()); // Client sends br
        client_headers.insert("authorization", "Bearer token".parse().unwrap());

        let upstream = build_upstream_headers(&client_headers, "https://mastodon.social");

        // Proxy should override with its own Accept-Encoding
        let accept_encoding = upstream
            .get("accept-encoding")
            .expect("Accept-Encoding should be set");
        let value = accept_encoding.to_str().unwrap();
        assert!(
            value.contains("gzip") && value.contains("deflate"),
            "Accept-Encoding should include gzip and deflate"
        );
        // Other headers should still pass through
        assert!(upstream.get("authorization").is_some());
    }

    #[test]
    fn test_decompress_gzip_body() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let original = b"Hello, World!";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());

        let result = decompress_response_body(&compressed, &headers).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_decompress_deflate_body() {
        use flate2::write::DeflateEncoder;
        use flate2::Compression;
        use std::io::Write;

        let original = b"Hello, World!";
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, "deflate".parse().unwrap());

        let result = decompress_response_body(&compressed, &headers).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_decompress_no_encoding() {
        let original = b"Hello, World!";
        let headers = reqwest::header::HeaderMap::new();

        let result = decompress_response_body(original, &headers).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_decompress_unknown_encoding_passes_through() {
        let original = b"Hello, World!";
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, "br".parse().unwrap()); // Brotli not supported

        let result = decompress_response_body(original, &headers).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_decompress_invalid_gzip_returns_error() {
        let invalid = vec![0x1f, 0x8b, 0x08, 0x00, 0xff, 0xff];
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());

        let result = decompress_response_body(&invalid, &headers);
        assert!(result.is_err());
        match result.unwrap_err() {
            ProxyError::Decompression(msg) => {
                assert!(msg.contains("gzip"));
            }
            _ => panic!("Expected ProxyError::Decompression"),
        }
    }

    #[test]
    fn test_decompress_empty_body() {
        let empty: &[u8] = &[];
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());

        // Empty gzip is invalid, but we should handle it gracefully
        let result = decompress_response_body(empty, &headers);
        // This will fail because empty is not valid gzip
        assert!(result.is_err());
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
    fn test_is_timeline_endpoint_link() {
        // Trending article timeline (added in newer Mastodon versions)
        assert!(is_timeline_endpoint("/api/v1/timelines/link"));
        assert!(is_timeline_endpoint("/api/v1/timelines/link?limit=20"));
        assert!(is_timeline_endpoint(
            "/api/v1/timelines/link?url=https://example.com/article"
        ));
    }

    #[test]
    fn test_is_timeline_endpoint_trends_statuses() {
        // Trending statuses endpoint
        assert!(is_timeline_endpoint("/api/v1/trends/statuses"));
        assert!(is_timeline_endpoint("/api/v1/trends/statuses?limit=20"));
        assert!(is_timeline_endpoint("/api/v1/trends/statuses?offset=10"));
    }

    #[test]
    fn test_is_timeline_endpoint_trends_tags_not_filtered() {
        // Trends/tags returns Tag objects, not statuses - should NOT be filtered
        assert!(!is_timeline_endpoint("/api/v1/trends/tags"));
        assert!(!is_timeline_endpoint("/api/v1/trends/links"));
    }

    #[test]
    fn test_is_timeline_endpoint_bookmarks_not_filtered() {
        // User wants to see all bookmarks, no filtering
        assert!(!is_timeline_endpoint("/api/v1/bookmarks"));
        assert!(!is_timeline_endpoint("/api/v1/bookmarks?limit=40"));
    }

    #[test]
    fn test_is_timeline_endpoint_favourites_not_filtered() {
        // User wants to see all favourites, no filtering
        assert!(!is_timeline_endpoint("/api/v1/favourites"));
        assert!(!is_timeline_endpoint("/api/v1/favourites?limit=40"));
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

    #[test]
    fn test_rewrite_set_cookie_removes_domain_and_secure() {
        let cookie = "_mastodon_session=abc123; Domain=nerdculture.de; Path=/; Secure; HttpOnly; SameSite=Lax";
        let result = rewrite_set_cookie_header(cookie);

        assert_eq!(
            result,
            "_mastodon_session=abc123; Path=/; HttpOnly; SameSite=Lax"
        );
    }

    #[test]
    fn test_rewrite_set_cookie_preserves_simple_cookie() {
        let cookie = "session=xyz; Path=/; HttpOnly";
        let result = rewrite_set_cookie_header(cookie);

        assert_eq!(result, "session=xyz; Path=/; HttpOnly");
    }

    #[test]
    fn test_get_proxy_base_url_from_host_header() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "localhost:8080".parse().unwrap());

        let uri: axum::http::Uri = "/api/v1/instance".parse().unwrap();
        let result = get_proxy_base_url(&headers, &uri);

        assert_eq!(result, Some("http://localhost:8080".to_string()));
    }

    #[test]
    fn test_get_proxy_base_url_with_forwarded_proto() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "proxy.example.com".parse().unwrap());
        headers.insert("x-forwarded-proto", "https".parse().unwrap());

        let uri: axum::http::Uri = "/api/v1/instance".parse().unwrap();
        let result = get_proxy_base_url(&headers, &uri);

        assert_eq!(result, Some("https://proxy.example.com".to_string()));
    }

    #[test]
    fn test_get_proxy_base_url_no_host() {
        let headers = HeaderMap::new();
        let uri: axum::http::Uri = "/api/v1/instance".parse().unwrap();
        let result = get_proxy_base_url(&headers, &uri);

        assert_eq!(result, None);
    }

    #[test]
    fn test_rewrite_location_header_rewrites_upstream() {
        let location = "https://mastodon.social/oauth/authorize?client_id=abc";
        let upstream_url = "https://mastodon.social";
        let proxy_base_url = Some("http://localhost:8080".to_string());

        let result = rewrite_location_header(location, upstream_url, &proxy_base_url);

        assert_eq!(
            result,
            "http://localhost:8080/oauth/authorize?client_id=abc"
        );
    }

    #[test]
    fn test_rewrite_location_header_preserves_non_upstream() {
        let location = "https://other.example.com/callback";
        let upstream_url = "https://mastodon.social";
        let proxy_base_url = Some("http://localhost:8080".to_string());

        let result = rewrite_location_header(location, upstream_url, &proxy_base_url);

        // Should not rewrite URLs that don't match the upstream
        assert_eq!(result, "https://other.example.com/callback");
    }

    #[test]
    fn test_rewrite_location_header_no_proxy_url() {
        let location = "https://mastodon.social/oauth/authorize";
        let upstream_url = "https://mastodon.social";
        let proxy_base_url = None;

        let result = rewrite_location_header(location, upstream_url, &proxy_base_url);

        // Should pass through unchanged when no proxy URL
        assert_eq!(result, "https://mastodon.social/oauth/authorize");
    }

    #[test]
    fn test_rewrite_location_header_with_default_port() {
        // Port 443 is the default for https, so these are the same origin
        let location = "https://nerdculture.de:443/oauth/authorize?response_type=code";
        let upstream_url = "https://nerdculture.de";
        let proxy_base_url = Some("http://localhost:8080".to_string());

        let result = rewrite_location_header(location, upstream_url, &proxy_base_url);

        // Default port 443 is equivalent to no port for https - should rewrite
        assert_eq!(
            result,
            "http://localhost:8080/oauth/authorize?response_type=code"
        );
    }

    #[test]
    fn test_rewrite_location_header_with_non_default_port() {
        // Port 8443 is NOT the default for https, so these are different origins
        let location = "https://nerdculture.de:8443/oauth/authorize?response_type=code";
        let upstream_url = "https://nerdculture.de";
        let proxy_base_url = Some("http://localhost:8080".to_string());

        let result = rewrite_location_header(location, upstream_url, &proxy_base_url);

        // Different port means different origin - should NOT rewrite
        assert_eq!(
            result,
            "https://nerdculture.de:8443/oauth/authorize?response_type=code"
        );
    }
}
