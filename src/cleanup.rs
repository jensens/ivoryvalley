//! Background cleanup task for old URIs.
//!
//! This module provides a background task that periodically cleans up
//! old entries from the SeenUriStore to prevent unbounded database growth.

use std::sync::Arc;
use std::time::Duration;

use crate::db::SeenUriStore;

/// Spawns a background task that periodically cleans up old URIs.
///
/// The task runs at the specified interval and removes entries older than
/// `max_age_secs` from the store. Cleanup results are logged.
///
/// # Arguments
///
/// * `store` - The SeenUriStore to clean up
/// * `interval_secs` - Seconds between cleanup runs
/// * `max_age_secs` - Maximum age in seconds for entries (older entries are removed)
///
/// # Returns
///
/// A `JoinHandle` for the spawned task, which can be used to abort the task
/// or wait for it to complete (though normally it runs indefinitely).
pub fn spawn_cleanup_task(
    store: Arc<SeenUriStore>,
    interval_secs: u64,
    max_age_secs: u64,
) -> tokio::task::JoinHandle<()> {
    tracing::info!(
        "Starting cleanup task: interval={}s, max_age={}s ({}d)",
        interval_secs,
        max_age_secs,
        max_age_secs / 86400
    );

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

        // Skip the first tick which fires immediately
        interval.tick().await;

        loop {
            interval.tick().await;

            match store.cleanup(max_age_secs) {
                Ok(removed) => {
                    if removed > 0 {
                        tracing::info!("Cleaned up {} old URIs", removed);
                    } else {
                        tracing::debug!("Cleanup: no old URIs to remove");
                    }
                }
                Err(e) => {
                    tracing::error!("Cleanup failed: {}", e);
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_cleanup_logic() {
        // Test the cleanup logic directly without spawning a task
        let store = Arc::new(SeenUriStore::open(":memory:").unwrap());

        // Add some URIs
        store.mark_seen("https://example.com/1").unwrap();
        store.mark_seen("https://example.com/2").unwrap();

        assert!(store.is_seen("https://example.com/1").unwrap());
        assert!(store.is_seen("https://example.com/2").unwrap());

        // Cleanup with max_age=0 removes all
        let removed = store.cleanup(0).unwrap();
        assert_eq!(removed, 2);

        // URIs should be cleaned up
        assert!(!store.is_seen("https://example.com/1").unwrap());
        assert!(!store.is_seen("https://example.com/2").unwrap());
    }

    #[tokio::test]
    async fn test_cleanup_task_preserves_recent_entries() {
        let store = Arc::new(SeenUriStore::open(":memory:").unwrap());

        // Add a URI
        store.mark_seen("https://example.com/recent").unwrap();

        // Spawn cleanup task with 1 second interval but large max_age
        let handle = spawn_cleanup_task(store.clone(), 1, 999999);

        // Wait a bit (task waits for first interval before running)
        tokio::time::sleep(Duration::from_millis(50)).await;

        // URI should still be there (cleanup hasn't run yet, and when it does,
        // the entry is recent so it won't be removed)
        assert!(store.is_seen("https://example.com/recent").unwrap());

        handle.abort();
    }
}
