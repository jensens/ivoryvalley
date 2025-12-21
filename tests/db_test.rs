//! Integration tests for the database module.
//!
//! These tests verify the seen-URI storage functionality.

mod common;

use common::create_temp_dir;
use ivoryvalley::db::SeenUriStore;

/// Test that we can create and open a SeenUriStore.
#[test]
fn test_create_seen_uri_store() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    let store = SeenUriStore::open(&db_path).expect("Failed to open store");
    drop(store);

    // The database file should now exist
    assert!(db_path.exists());
}

/// Test storing and checking a single URI.
#[test]
fn test_store_and_check_uri() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    let store = SeenUriStore::open(&db_path).expect("Failed to open store");

    let uri = "https://mastodon.social/users/testuser/statuses/123456";

    // URI should not be seen initially
    assert!(!store.is_seen(uri).expect("Failed to check URI"));

    // Store the URI
    store.mark_seen(uri).expect("Failed to mark URI as seen");

    // URI should now be seen
    assert!(store.is_seen(uri).expect("Failed to check URI"));
}

/// Test that storing the same URI twice doesn't cause an error.
#[test]
fn test_store_duplicate_uri() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    let store = SeenUriStore::open(&db_path).expect("Failed to open store");

    let uri = "https://mastodon.social/users/testuser/statuses/123456";

    // Store the URI twice - should not error
    store.mark_seen(uri).expect("Failed to mark URI as seen");
    store
        .mark_seen(uri)
        .expect("Failed to mark URI as seen again");

    // URI should still be seen
    assert!(store.is_seen(uri).expect("Failed to check URI"));
}

/// Test checking multiple URIs.
#[test]
fn test_multiple_uris() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    let store = SeenUriStore::open(&db_path).expect("Failed to open store");

    let uri1 = "https://mastodon.social/users/user1/statuses/111";
    let uri2 = "https://mastodon.social/users/user2/statuses/222";
    let uri3 = "https://fosstodon.org/users/user3/statuses/333";

    // Store uri1 and uri2
    store.mark_seen(uri1).expect("Failed to mark URI 1");
    store.mark_seen(uri2).expect("Failed to mark URI 2");

    // uri1 and uri2 should be seen, uri3 should not
    assert!(store.is_seen(uri1).expect("Failed to check URI 1"));
    assert!(store.is_seen(uri2).expect("Failed to check URI 2"));
    assert!(!store.is_seen(uri3).expect("Failed to check URI 3"));
}

/// Test that the store persists data across reopens.
#[test]
fn test_persistence() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    let uri = "https://mastodon.social/users/testuser/statuses/789";

    // Open store, mark URI, close
    {
        let store = SeenUriStore::open(&db_path).expect("Failed to open store");
        store.mark_seen(uri).expect("Failed to mark URI");
    }

    // Reopen and check
    {
        let store = SeenUriStore::open(&db_path).expect("Failed to reopen store");
        assert!(store.is_seen(uri).expect("Failed to check URI"));
    }
}

/// Test cleanup of old URIs.
#[test]
fn test_cleanup_old_uris() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    let store = SeenUriStore::open(&db_path).expect("Failed to open store");

    let uri = "https://mastodon.social/users/testuser/statuses/999";
    store.mark_seen(uri).expect("Failed to mark URI");

    // Cleanup with max_age_secs = 0 should remove all entries
    let removed = store.cleanup(0).expect("Failed to cleanup");
    assert_eq!(removed, 1);

    // URI should no longer be seen
    assert!(!store.is_seen(uri).expect("Failed to check URI"));
}

/// Test cleanup doesn't remove recent URIs.
#[test]
fn test_cleanup_keeps_recent() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.path().join("test.db");

    let store = SeenUriStore::open(&db_path).expect("Failed to open store");

    let uri = "https://mastodon.social/users/testuser/statuses/999";
    store.mark_seen(uri).expect("Failed to mark URI");

    // Cleanup with max_age_secs = 1 week (604800) should keep the entry
    let removed = store.cleanup(604800).expect("Failed to cleanup");
    assert_eq!(removed, 0);

    // URI should still be seen
    assert!(store.is_seen(uri).expect("Failed to check URI"));
}
