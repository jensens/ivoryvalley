//! Graceful shutdown handling for the IvoryValley proxy server.
//!
//! This module provides signal handling for graceful shutdown on SIGTERM and SIGINT.
//! When a shutdown signal is received, in-flight requests are allowed to complete
//! before the server terminates.

use tokio::signal;

/// Creates a future that completes when a shutdown signal is received.
///
/// This function listens for:
/// - SIGINT (Ctrl+C)
/// - SIGTERM (common in containerized environments)
///
/// When either signal is received, the future completes, allowing the server
/// to initiate graceful shutdown.
///
/// # Example
///
/// ```ignore
/// use ivoryvalley::shutdown::shutdown_signal;
///
/// axum::serve(listener, app)
///     .with_graceful_shutdown(shutdown_signal())
///     .await
///     .expect("Server error");
/// ```
pub async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {
            tracing::info!("Received SIGINT, initiating graceful shutdown");
        }
        () = terminate => {
            tracing::info!("Received SIGTERM, initiating graceful shutdown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    /// Test that shutdown_signal returns a valid future that can be polled.
    /// We can't easily trigger actual signals in tests, but we can verify
    /// the future doesn't immediately complete (it waits for a signal).
    #[tokio::test]
    async fn test_shutdown_signal_waits_for_signal() {
        // The shutdown signal should not complete within a short timeout
        // since no signal has been sent
        let result = timeout(Duration::from_millis(10), shutdown_signal()).await;

        // Should timeout (Err) because no signal was sent
        assert!(result.is_err(), "shutdown_signal should wait for a signal");
    }
}
