//! Database module for seen-URI storage and deduplication.
//!
//! Provides deduplication storage using SQLite with WAL mode,
//! and utilities for extracting URIs from Mastodon status entities.

use rusqlite::{Connection, Result};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Extracts the URI to use for deduplication from a Mastodon status.
///
/// For regular statuses, returns the status's own `uri`.
/// For boosts (reblogs), returns the `reblog.uri` to deduplicate on the original content.
///
/// Returns `None` if the status doesn't have a valid URI.
pub fn extract_dedup_uri(status: &Value) -> Option<&str> {
    // If this is a reblog, use the reblog's URI
    if let Some(reblog) = status.get("reblog") {
        if !reblog.is_null() {
            return reblog.get("uri")?.as_str();
        }
    }

    // Otherwise use the status's own URI
    status.get("uri")?.as_str()
}

/// Storage for tracking seen message URIs.
///
/// Uses SQLite with WAL mode for concurrent read access and durability.
/// Thread-safe via internal Mutex.
pub struct SeenUriStore {
    conn: Mutex<Connection>,
}

impl SeenUriStore {
    /// Opens or creates a SeenUriStore at the given path.
    ///
    /// Initializes the database schema if it doesn't exist.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;

        // Enable WAL mode for better concurrent access
        conn.pragma_update(None, "journal_mode", "WAL")?;

        // Create schema
        conn.execute(
            "CREATE TABLE IF NOT EXISTS seen_uris (
                uri TEXT PRIMARY KEY,
                first_seen INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_first_seen ON seen_uris(first_seen)",
            [],
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Checks if a URI has been seen before.
    pub fn is_seen(&self, uri: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached("SELECT 1 FROM seen_uris WHERE uri = ?")?;
        let exists = stmt.exists([uri])?;
        Ok(exists)
    }

    /// Marks a URI as seen.
    ///
    /// If the URI was already seen, this is a no-op.
    pub fn mark_seen(&self, uri: &str) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs() as i64;

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO seen_uris (uri, first_seen) VALUES (?, ?)",
            (uri, now),
        )?;

        Ok(())
    }

    /// Atomically checks if a URI has been seen and marks it as seen if not.
    ///
    /// Returns `true` if the URI was already seen, `false` if it was newly marked.
    /// This avoids the race condition between separate is_seen() and mark_seen() calls.
    pub fn check_and_mark(&self, uri: &str) -> Result<bool> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs() as i64;

        let conn = self.conn.lock().unwrap();

        // Try to insert; if it already exists, the INSERT OR IGNORE does nothing
        let rows_changed = conn.execute(
            "INSERT OR IGNORE INTO seen_uris (uri, first_seen) VALUES (?, ?)",
            (uri, now),
        )?;

        // If rows_changed is 0, the URI already existed (was seen before)
        // If rows_changed is 1, we just inserted it (first time seeing it)
        Ok(rows_changed == 0)
    }

    /// Removes URIs older than max_age_secs.
    ///
    /// If max_age_secs is 0, removes all entries.
    /// Returns the number of removed entries.
    pub fn cleanup(&self, max_age_secs: u64) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let removed = if max_age_secs == 0 {
            // Special case: remove all entries
            conn.execute("DELETE FROM seen_uris", [])?
        } else {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs() as i64;

            let cutoff = now - (max_age_secs as i64);

            conn.execute("DELETE FROM seen_uris WHERE first_seen < ?", [cutoff])?
        };

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_in_memory_store() {
        let store = SeenUriStore::open(":memory:").unwrap();

        let uri = "https://example.com/status/123";

        assert!(!store.is_seen(uri).unwrap());
        store.mark_seen(uri).unwrap();
        assert!(store.is_seen(uri).unwrap());
    }

    #[test]
    fn test_check_and_mark_atomic() {
        let store = SeenUriStore::open(":memory:").unwrap();

        let uri = "https://example.com/status/456";

        // First call: URI not seen, should return false (newly marked)
        assert!(!store.check_and_mark(uri).unwrap());

        // Second call: URI was seen, should return true
        assert!(store.check_and_mark(uri).unwrap());

        // Verify it's actually in the store
        assert!(store.is_seen(uri).unwrap());
    }

    #[test]
    fn test_extract_uri_from_regular_status() {
        let status = json!({
            "id": "123456",
            "uri": "https://mastodon.social/users/testuser/statuses/123456",
            "content": "<p>Hello, world!</p>"
        });

        let uri = extract_dedup_uri(&status);
        assert_eq!(
            uri,
            Some("https://mastodon.social/users/testuser/statuses/123456")
        );
    }

    #[test]
    fn test_extract_uri_from_reblog() {
        let status = json!({
            "id": "789012",
            "uri": "https://mastodon.social/users/booster/statuses/789012",
            "reblog": {
                "id": "123456",
                "uri": "https://fosstodon.org/users/original/statuses/123456",
                "content": "<p>Original post</p>"
            }
        });

        let uri = extract_dedup_uri(&status);
        // Should return the reblog's URI, not the boost's URI
        assert_eq!(
            uri,
            Some("https://fosstodon.org/users/original/statuses/123456")
        );
    }

    #[test]
    fn test_extract_uri_with_null_reblog() {
        let status = json!({
            "id": "123456",
            "uri": "https://mastodon.social/users/testuser/statuses/123456",
            "reblog": null
        });

        let uri = extract_dedup_uri(&status);
        // With null reblog, should return the status's own URI
        assert_eq!(
            uri,
            Some("https://mastodon.social/users/testuser/statuses/123456")
        );
    }

    #[test]
    fn test_extract_uri_missing() {
        let status = json!({
            "id": "123456",
            "content": "<p>No URI field</p>"
        });

        let uri = extract_dedup_uri(&status);
        assert_eq!(uri, None);
    }
}
