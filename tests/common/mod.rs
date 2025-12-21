//! Shared test utilities and helpers for IvoryValley tests.
//!
//! This module provides common functionality used across integration tests.

use std::path::PathBuf;
use tempfile::TempDir;

/// Creates a temporary directory for test data.
///
/// The directory is automatically cleaned up when the `TempDir` is dropped.
pub fn create_temp_dir() -> TempDir {
    tempfile::tempdir().expect("Failed to create temporary directory")
}

/// Returns the path to the test fixtures directory.
pub fn fixtures_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Test configuration builder for creating test instances of the proxy.
pub struct TestConfig {
    pub db_path: Option<PathBuf>,
    pub upstream_url: String,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            db_path: None,
            upstream_url: "http://localhost:3000".to_string(),
        }
    }
}

impl TestConfig {
    /// Creates a new test configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the database path for tests.
    pub fn with_db_path(mut self, path: PathBuf) -> Self {
        self.db_path = Some(path);
        self
    }

    /// Sets the upstream URL for tests.
    pub fn with_upstream_url(mut self, url: impl Into<String>) -> Self {
        self.upstream_url = url.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_temp_dir() {
        let dir = create_temp_dir();
        assert!(dir.path().exists());
    }

    #[test]
    fn test_config_builder() {
        let config = TestConfig::new().with_upstream_url("http://example.com");
        assert_eq!(config.upstream_url, "http://example.com");
    }
}
