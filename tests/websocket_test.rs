//! Integration tests for the WebSocket streaming functionality.
//!
//! These tests verify the end-to-end behavior of the WebSocket proxy.

mod common;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
    routing::get,
    Router,
};
use common::create_temp_dir;
use futures_util::{SinkExt, StreamExt};
use ivoryvalley::{config::Config, db::SeenUriStore, proxy::create_proxy_router};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite};

/// Mock upstream WebSocket server state
#[derive(Clone)]
struct MockWsState {
    messages_to_send: std::sync::Arc<tokio::sync::Mutex<Vec<String>>>,
}

/// Mock upstream WebSocket server for testing
struct MockUpstreamWs {
    pub addr: SocketAddr,
    pub state: MockWsState,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MockUpstreamWs {
    async fn start() -> Self {
        let state = MockWsState {
            messages_to_send: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
        };

        let app = Router::new()
            .route("/api/v1/streaming", get(mock_ws_handler))
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

    /// Queue a message to be sent to clients
    async fn queue_message(&self, msg: String) {
        self.state.messages_to_send.lock().await.push(msg);
    }
}

impl Drop for MockUpstreamWs {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Mock WebSocket handler that echoes messages and sends queued messages
async fn mock_ws_handler(ws: WebSocketUpgrade, State(state): State<MockWsState>) -> Response {
    ws.on_upgrade(move |socket| handle_mock_ws(socket, state))
}

async fn handle_mock_ws(socket: WebSocket, state: MockWsState) {
    let (mut sender, mut receiver) = socket.split();

    // Send any queued messages, draining to avoid cloning
    let messages = {
        let mut locked = state.messages_to_send.lock().await;
        std::mem::take(&mut *locked)
    };
    for msg in messages {
        if sender.send(Message::Text(msg.into())).await.is_err() {
            return;
        }
    }

    // Echo received messages back
    while let Some(msg) = receiver.next().await {
        if let Ok(msg) = msg {
            match msg {
                Message::Text(text) => {
                    if sender
                        .send(Message::Text(format!("echo: {}", text).into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        } else {
            break;
        }
    }
}

/// Helper to create a WebSocket client connection to the proxy
async fn connect_to_proxy(
    proxy_url: &str,
) -> (
    futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tungstenite::Message,
    >,
    futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) {
    let ws_url = format!(
        "{}/api/v1/streaming?access_token=test_token",
        proxy_url.replace("http://", "ws://")
    );
    let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
    ws_stream.split()
}

/// Test that WebSocket upgrade succeeds
#[tokio::test]
async fn test_websocket_upgrade_succeeds() {
    let upstream = MockUpstreamWs::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    // Start the proxy server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let proxy_url = format!("http://{}", proxy_addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect to the proxy
    let (mut _sink, mut stream) = connect_to_proxy(&proxy_url).await;

    // Connection should be established - try to receive a message or close gracefully
    tokio::select! {
        _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
            // Connection stayed open for 1 second - success
        }
        msg = stream.next() => {
            // Received a message or close - also success
            assert!(msg.is_some());
        }
    }
}

/// Test bidirectional message relay
#[tokio::test]
async fn test_bidirectional_message_relay() {
    let upstream = MockUpstreamWs::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    // Start the proxy server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let proxy_url = format!("http://{}", proxy_addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect to the proxy
    let (mut sink, mut stream) = connect_to_proxy(&proxy_url).await;

    // Send a message to upstream (through proxy)
    sink.send(tungstenite::Message::Text("hello".into()))
        .await
        .expect("Failed to send message");

    // Receive echo response
    let response = tokio::time::timeout(tokio::time::Duration::from_secs(2), stream.next())
        .await
        .expect("Timeout waiting for response")
        .expect("Stream ended")
        .expect("Error receiving message");

    if let tungstenite::Message::Text(text) = response {
        assert_eq!(text, "echo: hello");
    } else {
        panic!("Expected text message, got {:?}", response);
    }
}

/// Test that upstream messages are relayed to client
#[tokio::test]
async fn test_upstream_to_client_relay() {
    let upstream = MockUpstreamWs::start().await;

    // Queue a message to be sent from upstream
    upstream
        .queue_message(r#"{"event":"notification","payload":"test"}"#.to_string())
        .await;

    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    // Start the proxy server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let proxy_url = format!("http://{}", proxy_addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect to the proxy
    let (_sink, mut stream) = connect_to_proxy(&proxy_url).await;

    // Receive the queued message
    let response = tokio::time::timeout(tokio::time::Duration::from_secs(2), stream.next())
        .await
        .expect("Timeout waiting for response")
        .expect("Stream ended")
        .expect("Error receiving message");

    if let tungstenite::Message::Text(text) = response {
        assert!(text.contains("notification"));
        assert!(text.contains("test"));
    } else {
        panic!("Expected text message, got {:?}", response);
    }
}

/// Test that deduplication works through WebSocket connection
#[tokio::test]
async fn test_websocket_deduplication() {
    let upstream = MockUpstreamWs::start().await;

    // Queue two identical update events
    // Using a helper to create the event JSON for better readability
    let create_status_event = || {
        let payload = serde_json::json!({
            "id": "123",
            "uri": "https://example.com/status/123"
        })
        .to_string();
        serde_json::json!({
            "event": "update",
            "payload": payload
        })
        .to_string()
    };

    upstream.queue_message(create_status_event()).await;
    upstream.queue_message(create_status_event()).await;

    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    // Start the proxy server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let proxy_url = format!("http://{}", proxy_addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect to the proxy
    let (_sink, mut stream) = connect_to_proxy(&proxy_url).await;

    // Receive the first message (should pass through)
    let first_msg = tokio::time::timeout(tokio::time::Duration::from_secs(2), stream.next())
        .await
        .expect("Timeout waiting for first message")
        .expect("Stream ended")
        .expect("Error receiving first message");

    assert!(
        matches!(first_msg, tungstenite::Message::Text(_)),
        "Expected text message"
    );

    // Try to receive second message - should timeout because it was filtered
    let second_msg =
        tokio::time::timeout(tokio::time::Duration::from_millis(500), stream.next()).await;

    // The second message should have been filtered, so we expect a timeout
    assert!(
        second_msg.is_err(),
        "Second duplicate message should have been filtered out"
    );
}

/// Test connection close handling
#[tokio::test]
async fn test_websocket_close_handling() {
    let upstream = MockUpstreamWs::start().await;
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    // Start the proxy server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let proxy_url = format!("http://{}", proxy_addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect to the proxy
    let (mut sink, mut stream) = connect_to_proxy(&proxy_url).await;

    // Send close message
    sink.send(tungstenite::Message::Close(None))
        .await
        .expect("Failed to send close");

    // Should receive close confirmation or stream should end
    let response = tokio::time::timeout(tokio::time::Duration::from_secs(2), stream.next()).await;

    match response {
        Ok(Some(Ok(tungstenite::Message::Close(_)))) => {
            // Received close frame - success
        }
        Ok(None) => {
            // Stream ended - also success
        }
        _ => panic!("Expected close frame or stream end, got {:?}", response),
    }
}

// =============================================================================
// Legitimate message tests (Issue #20)
// These tests verify that deduplication doesn't drop valid content.
// =============================================================================

/// Test that different statuses via WebSocket are NOT deduplicated.
/// Each status has a unique URI, so both should pass through.
#[tokio::test]
async fn test_websocket_different_statuses_not_deduplicated() {
    let upstream = MockUpstreamWs::start().await;

    // Queue two different update events with unique URIs
    let status1 = serde_json::json!({
        "id": "1",
        "uri": "https://example.com/status/1",
        "content": "<p>First post</p>"
    })
    .to_string();
    let event1 = serde_json::json!({
        "event": "update",
        "payload": status1
    })
    .to_string();

    let status2 = serde_json::json!({
        "id": "2",
        "uri": "https://example.com/status/2",
        "content": "<p>Second post</p>"
    })
    .to_string();
    let event2 = serde_json::json!({
        "event": "update",
        "payload": status2
    })
    .to_string();

    upstream.queue_message(event1).await;
    upstream.queue_message(event2).await;

    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");
    let config = Config::new(&upstream.url(), "0.0.0.0", 0, db_path);
    let seen_store = SeenUriStore::open(":memory:").unwrap();
    let app = create_proxy_router(config, seen_store);

    // Start the proxy server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    let proxy_url = format!("http://{}", proxy_addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect to the proxy
    let (_sink, mut stream) = connect_to_proxy(&proxy_url).await;

    // Receive the first message (should pass through)
    let first_msg = tokio::time::timeout(tokio::time::Duration::from_secs(2), stream.next())
        .await
        .expect("Timeout waiting for first message")
        .expect("Stream ended")
        .expect("Error receiving first message");

    assert!(
        matches!(first_msg, tungstenite::Message::Text(_)),
        "Expected text message for first status"
    );

    // Receive the second message (should also pass through - different URI)
    let second_msg = tokio::time::timeout(tokio::time::Duration::from_secs(2), stream.next())
        .await
        .expect("Timeout waiting for second message")
        .expect("Stream ended")
        .expect("Error receiving second message");

    assert!(
        matches!(second_msg, tungstenite::Message::Text(_)),
        "Expected text message for second status - both unique statuses should pass through"
    );
}
