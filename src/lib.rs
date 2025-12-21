//! IvoryValley - Mastodon proxy for content deduplication

pub mod config;
pub mod db;
pub mod proxy;
pub mod websocket;

// Re-export main deduplication API
pub use db::{extract_dedup_uri, SeenUriStore};
