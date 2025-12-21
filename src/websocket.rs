//! WebSocket relay handlers for Mastodon streaming API.
//!
//! This module implements the WebSocket proxy that:
//! - Accepts WebSocket connections from Mastodon clients
//! - Connects to the upstream Mastodon streaming server
//! - Relays messages bidirectionally between client and upstream
//! - Filters `update` events for deduplication

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::{connect_async, tungstenite};
use tracing::{debug, error, info, warn};

use crate::config::AppState;
use crate::db::{extract_dedup_uri, SeenUriStore};

/// Query parameters for WebSocket streaming endpoint
#[derive(Debug, Deserialize)]
pub struct StreamingParams {
    /// Access token for authentication (not recommended but supported)
    pub access_token: Option<String>,
    /// Stream type (user, public, etc.)
    pub stream: Option<String>,
    /// Tag for hashtag streams
    pub tag: Option<String>,
    /// List ID for list streams
    pub list: Option<String>,
}

/// State for WebSocket connections with deduplication support
#[derive(Clone)]
pub struct WebSocketState {
    pub app_state: AppState,
    pub seen_store: Arc<Mutex<SeenUriStore>>,
}

impl WebSocketState {
    pub fn new(app_state: AppState, seen_store: SeenUriStore) -> Self {
        Self {
            app_state,
            seen_store: Arc::new(Mutex::new(seen_store)),
        }
    }
}

/// Handle WebSocket upgrade requests for streaming API
pub async fn streaming_handler(
    ws: WebSocketUpgrade,
    State(state): State<WebSocketState>,
    Query(params): Query<StreamingParams>,
) -> Response {
    info!("WebSocket upgrade request received");

    // Extract what we need before the upgrade to avoid Send issues
    let upstream_url = state.app_state.config.upstream_url.clone();
    let seen_store = state.seen_store.clone();

    ws.on_upgrade(move |socket| handle_streaming(socket, upstream_url, seen_store, params))
}

/// Handle the streaming WebSocket connection
async fn handle_streaming(
    client_ws: WebSocket,
    upstream_url: String,
    seen_store: Arc<Mutex<SeenUriStore>>,
    params: StreamingParams,
) {
    // Build upstream WebSocket URL
    let upstream_ws_url = build_upstream_ws_url(&upstream_url, &params);

    info!("Connecting to upstream: {}", upstream_ws_url);

    // Connect to upstream
    let upstream_result = connect_async(&upstream_ws_url).await;

    let (upstream_ws, _response) = match upstream_result {
        Ok(conn) => conn,
        Err(e) => {
            error!("Failed to connect to upstream WebSocket: {}", e);
            return;
        }
    };

    info!("Connected to upstream WebSocket");

    // Split both connections
    let (mut client_sink, mut client_stream) = client_ws.split();
    let (mut upstream_sink, mut upstream_stream) = upstream_ws.split();

    // Create channels for message passing
    let (client_tx, mut client_rx) = mpsc::channel::<Message>(32);
    let (upstream_tx, mut upstream_rx) = mpsc::channel::<tungstenite::Message>(32);

    // Clone store for the filtering task
    let filter_store = seen_store.clone();

    // Task: Forward filtered messages from upstream to client
    let upstream_to_client = tokio::spawn(async move {
        while let Some(msg_result) = upstream_stream.next().await {
            match msg_result {
                Ok(msg) => {
                    // Convert and potentially filter the message
                    if let Some(client_msg) =
                        filter_upstream_message(msg, filter_store.clone()).await
                    {
                        if client_tx.send(client_msg).await.is_err() {
                            debug!("Client channel closed");
                            break;
                        }
                    }
                }
                Err(e) => {
                    warn!("Upstream WebSocket error: {}", e);
                    break;
                }
            }
        }
    });

    // Task: Forward messages from client to upstream
    let client_to_upstream = tokio::spawn(async move {
        while let Some(msg_result) = client_stream.next().await {
            match msg_result {
                Ok(msg) => {
                    // Convert axum Message to tungstenite Message
                    if let Some(upstream_msg) = convert_client_to_upstream(msg) {
                        if upstream_tx.send(upstream_msg).await.is_err() {
                            debug!("Upstream channel closed");
                            break;
                        }
                    }
                }
                Err(e) => {
                    warn!("Client WebSocket error: {}", e);
                    break;
                }
            }
        }
    });

    // Task: Send messages to client
    let send_to_client = tokio::spawn(async move {
        while let Some(msg) = client_rx.recv().await {
            if client_sink.send(msg).await.is_err() {
                debug!("Failed to send to client");
                break;
            }
        }
    });

    // Task: Send messages to upstream
    let send_to_upstream = tokio::spawn(async move {
        while let Some(msg) = upstream_rx.recv().await {
            if upstream_sink.send(msg).await.is_err() {
                debug!("Failed to send to upstream");
                break;
            }
        }
    });

    // Wait for any task to complete (connection closed)
    tokio::select! {
        _ = upstream_to_client => info!("Upstream to client task ended"),
        _ = client_to_upstream => info!("Client to upstream task ended"),
        _ = send_to_client => info!("Send to client task ended"),
        _ = send_to_upstream => info!("Send to upstream task ended"),
    }

    info!("WebSocket connection closed");
}

/// Build the upstream WebSocket URL with query parameters
fn build_upstream_ws_url(upstream_base: &str, params: &StreamingParams) -> String {
    // Convert HTTP(S) URL to WS(S)
    let ws_base = upstream_base
        .replace("https://", "wss://")
        .replace("http://", "ws://");

    let mut url = format!("{}/api/v1/streaming", ws_base);

    // Build query string
    let mut query_parts = Vec::new();

    if let Some(ref token) = params.access_token {
        query_parts.push(format!("access_token={}", token));
    }
    if let Some(ref stream) = params.stream {
        query_parts.push(format!("stream={}", stream));
    }
    if let Some(ref tag) = params.tag {
        query_parts.push(format!("tag={}", tag));
    }
    if let Some(ref list) = params.list {
        query_parts.push(format!("list={}", list));
    }

    if !query_parts.is_empty() {
        url.push('?');
        url.push_str(&query_parts.join("&"));
    }

    url
}

/// Filter messages from upstream, applying deduplication to update events
async fn filter_upstream_message(
    msg: tungstenite::Message,
    seen_store: Arc<Mutex<SeenUriStore>>,
) -> Option<Message> {
    match msg {
        tungstenite::Message::Text(text) => {
            // Try to parse as streaming event, filter out duplicates
            filter_streaming_event(&text, seen_store)
                .await
                .map(|filtered| Message::Text(filtered.into()))
        }
        tungstenite::Message::Binary(data) => Some(Message::Binary(data)),
        tungstenite::Message::Ping(data) => Some(Message::Ping(data)),
        tungstenite::Message::Pong(data) => Some(Message::Pong(data)),
        tungstenite::Message::Close(frame) => {
            let axum_frame = frame.map(|f| axum::extract::ws::CloseFrame {
                code: f.code.into(),
                reason: f.reason.to_string().into(),
            });
            Some(Message::Close(axum_frame))
        }
        tungstenite::Message::Frame(_) => None, // Raw frames not supported
    }
}

/// Filter a streaming event, returning None if it should be deduplicated
async fn filter_streaming_event(
    text: &str,
    seen_store: Arc<Mutex<SeenUriStore>>,
) -> Option<String> {
    // Parse the event JSON
    let event: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            // Not valid JSON, pass through (could be heartbeat comment line)
            return Some(text.to_string());
        }
    };

    // Check if this is an update event
    let event_type = event.get("event").and_then(|e| e.as_str());

    if event_type != Some("update") {
        // Not an update event, pass through
        return Some(text.to_string());
    }

    // Parse the payload (it's a JSON string inside the event)
    let payload_str = event.get("payload").and_then(|p| p.as_str())?;
    let payload: serde_json::Value = serde_json::from_str(payload_str).ok()?;

    // Extract the deduplication URI
    let dedup_uri = extract_dedup_uri(&payload)?;

    // Check if we've seen this URI (lock the store)
    let store = seen_store.lock().await;

    match store.is_seen(dedup_uri) {
        Ok(true) => {
            debug!("Filtering duplicate status: {}", dedup_uri);
            None // Filter out duplicate
        }
        Ok(false) => {
            // Mark as seen and pass through
            if let Err(e) = store.mark_seen(dedup_uri) {
                warn!("Failed to mark URI as seen: {}", e);
            }
            Some(text.to_string())
        }
        Err(e) => {
            warn!("Failed to check if URI is seen: {}", e);
            // On error, pass through to avoid dropping messages
            Some(text.to_string())
        }
    }
}

/// Convert client message to upstream tungstenite message
fn convert_client_to_upstream(msg: Message) -> Option<tungstenite::Message> {
    match msg {
        Message::Text(text) => Some(tungstenite::Message::Text(text.to_string().into())),
        Message::Binary(data) => Some(tungstenite::Message::Binary(data)),
        Message::Ping(data) => Some(tungstenite::Message::Ping(data)),
        Message::Pong(data) => Some(tungstenite::Message::Pong(data)),
        Message::Close(frame) => {
            let tung_frame = frame.map(|f| tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::from(f.code),
                reason: f.reason.to_string().into(),
            });
            Some(tungstenite::Message::Close(tung_frame))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_upstream_ws_url_basic() {
        let params = StreamingParams {
            access_token: None,
            stream: None,
            tag: None,
            list: None,
        };

        let url = build_upstream_ws_url("https://mastodon.social", &params);
        assert_eq!(url, "wss://mastodon.social/api/v1/streaming");
    }

    #[test]
    fn test_build_upstream_ws_url_with_token() {
        let params = StreamingParams {
            access_token: Some("test_token".to_string()),
            stream: Some("user".to_string()),
            tag: None,
            list: None,
        };

        let url = build_upstream_ws_url("https://mastodon.social", &params);
        assert_eq!(
            url,
            "wss://mastodon.social/api/v1/streaming?access_token=test_token&stream=user"
        );
    }

    #[test]
    fn test_build_upstream_ws_url_with_hashtag() {
        let params = StreamingParams {
            access_token: Some("token".to_string()),
            stream: Some("hashtag".to_string()),
            tag: Some("rust".to_string()),
            list: None,
        };

        let url = build_upstream_ws_url("https://mastodon.social", &params);
        assert!(url.contains("stream=hashtag"));
        assert!(url.contains("tag=rust"));
    }

    #[test]
    fn test_build_upstream_ws_url_http_to_ws() {
        let params = StreamingParams {
            access_token: None,
            stream: None,
            tag: None,
            list: None,
        };

        let url = build_upstream_ws_url("http://localhost:3000", &params);
        assert_eq!(url, "ws://localhost:3000/api/v1/streaming");
    }

    #[tokio::test]
    async fn test_filter_streaming_event_passes_non_update() {
        let store = Arc::new(Mutex::new(SeenUriStore::open(":memory:").unwrap()));

        // Notification event should pass through
        let event = r#"{"event":"notification","payload":"{\"id\":\"123\"}"}"#;
        let result = filter_streaming_event(event, store.clone()).await;
        assert_eq!(result, Some(event.to_string()));

        // Delete event should pass through
        let delete_event = r#"{"event":"delete","payload":"123456"}"#;
        let result = filter_streaming_event(delete_event, store).await;
        assert_eq!(result, Some(delete_event.to_string()));
    }

    #[tokio::test]
    async fn test_filter_streaming_event_deduplicates_updates() {
        let store = Arc::new(Mutex::new(SeenUriStore::open(":memory:").unwrap()));

        let event = r#"{"event":"update","payload":"{\"id\":\"123\",\"uri\":\"https://mastodon.social/users/test/statuses/123\"}"}"#;

        // First time should pass through
        let result = filter_streaming_event(event, store.clone()).await;
        assert!(result.is_some());

        // Second time should be filtered
        let result = filter_streaming_event(event, store).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_filter_streaming_event_deduplicates_reblogs() {
        let store = Arc::new(Mutex::new(SeenUriStore::open(":memory:").unwrap()));

        // Original status
        let original = r#"{"event":"update","payload":"{\"id\":\"123\",\"uri\":\"https://mastodon.social/users/original/statuses/123\"}"}"#;

        // Reblog of the same status
        let reblog = r#"{"event":"update","payload":"{\"id\":\"456\",\"uri\":\"https://mastodon.social/users/booster/statuses/456\",\"reblog\":{\"id\":\"123\",\"uri\":\"https://mastodon.social/users/original/statuses/123\"}}"}"#;

        // Original passes through
        let result = filter_streaming_event(original, store.clone()).await;
        assert!(result.is_some());

        // Reblog is filtered (same underlying content)
        let result = filter_streaming_event(reblog, store).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_filter_streaming_event_passes_invalid_json() {
        let store = Arc::new(Mutex::new(SeenUriStore::open(":memory:").unwrap()));

        // Heartbeat comment line (not JSON)
        let heartbeat = ":";
        let result = filter_streaming_event(heartbeat, store.clone()).await;
        assert_eq!(result, Some(heartbeat.to_string()));

        // Invalid JSON passes through
        let invalid = "not json at all";
        let result = filter_streaming_event(invalid, store).await;
        assert_eq!(result, Some(invalid.to_string()));
    }

    #[tokio::test]
    async fn test_filter_streaming_event_different_statuses_pass() {
        let store = Arc::new(Mutex::new(SeenUriStore::open(":memory:").unwrap()));

        let event1 =
            r#"{"event":"update","payload":"{\"id\":\"1\",\"uri\":\"https://example.com/1\"}"}"#;
        let event2 =
            r#"{"event":"update","payload":"{\"id\":\"2\",\"uri\":\"https://example.com/2\"}"}"#;

        // Both different statuses should pass
        assert!(filter_streaming_event(event1, store.clone())
            .await
            .is_some());
        assert!(filter_streaming_event(event2, store).await.is_some());
    }
}
