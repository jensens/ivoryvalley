//! Traffic recording module for capturing request/response pairs.
//!
//! This module provides functionality to record HTTP traffic passing through the proxy,
//! which can later be anonymized and used as test fixtures for replay testing.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Mutex;

/// A recorded HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordedRequest {
    /// HTTP method (GET, POST, etc.)
    pub method: String,
    /// Request path with query string
    pub path: String,
    /// Request headers (filtered to relevant ones)
    pub headers: HashMap<String, String>,
    /// Request body (if any), base64-encoded for binary safety
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// A recorded HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordedResponse {
    /// HTTP status code
    pub status: u16,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Response body (JSON string for API responses)
    pub body: String,
}

/// A complete request/response exchange.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordedExchange {
    /// ISO 8601 timestamp when the exchange occurred
    pub timestamp: String,
    /// The recorded request
    pub request: RecordedRequest,
    /// The recorded response
    pub response: RecordedResponse,
}

/// Traffic recorder that writes exchanges to a JSONL file.
pub struct TrafficRecorder {
    writer: Mutex<BufWriter<File>>,
    path: PathBuf,
}

impl TrafficRecorder {
    /// Create a new traffic recorder that writes to the specified file.
    ///
    /// Creates the parent directory if it doesn't exist.
    /// Appends to the file if it already exists.
    pub fn new(path: PathBuf) -> std::io::Result<Self> {
        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
            path,
        })
    }

    /// Record an exchange to the file.
    ///
    /// Each exchange is written as a single JSON line (JSONL format).
    pub fn record(&self, exchange: &RecordedExchange) -> std::io::Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| std::io::Error::other("Failed to acquire lock"))?;

        let json = serde_json::to_string(exchange)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        writeln!(writer, "{}", json)?;
        writer.flush()?;

        Ok(())
    }

    /// Get the path to the recording file.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

/// Helper to create a timestamp string in ISO 8601 format.
pub fn now_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    // Format as ISO 8601 (simplified, no chrono dependency)
    let secs = duration.as_secs();
    let days = secs / 86400;
    let remaining = secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    // Calculate year, month, day from days since epoch (simplified)
    // This is a basic implementation - for production use chrono
    let (year, month, day) = days_to_ymd(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since Unix epoch to year/month/day.
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Simplified calculation - accurate enough for timestamps
    let mut remaining = days as i64;
    let mut year = 1970;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    let days_in_months: [i64; 12] = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1;
    for days in days_in_months.iter() {
        if remaining < *days {
            break;
        }
        remaining -= *days;
        month += 1;
    }

    (year as u64, month, (remaining + 1) as u64)
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use tempfile::tempdir;

    #[test]
    fn test_recorded_exchange_serialization() {
        let exchange = RecordedExchange {
            timestamp: "2025-12-25T10:00:00Z".to_string(),
            request: RecordedRequest {
                method: "GET".to_string(),
                path: "/api/v1/timelines/home".to_string(),
                headers: HashMap::from([("authorization".to_string(), "Bearer xxx".to_string())]),
                body: None,
            },
            response: RecordedResponse {
                status: 200,
                headers: HashMap::from([(
                    "content-type".to_string(),
                    "application/json".to_string(),
                )]),
                body: r#"[{"id":"1","content":"test"}]"#.to_string(),
            },
        };

        let json = serde_json::to_string(&exchange).unwrap();
        let deserialized: RecordedExchange = serde_json::from_str(&json).unwrap();

        assert_eq!(exchange, deserialized);
    }

    #[test]
    fn test_traffic_recorder_creates_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recordings/traffic.jsonl");

        let recorder = TrafficRecorder::new(path.clone()).unwrap();

        assert!(path.exists());
        assert_eq!(recorder.path(), &path);
    }

    #[test]
    fn test_traffic_recorder_writes_jsonl() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("traffic.jsonl");

        let recorder = TrafficRecorder::new(path.clone()).unwrap();

        let exchange1 = RecordedExchange {
            timestamp: "2025-12-25T10:00:00Z".to_string(),
            request: RecordedRequest {
                method: "GET".to_string(),
                path: "/api/v1/timelines/home".to_string(),
                headers: HashMap::new(),
                body: None,
            },
            response: RecordedResponse {
                status: 200,
                headers: HashMap::new(),
                body: "[]".to_string(),
            },
        };

        let exchange2 = RecordedExchange {
            timestamp: "2025-12-25T10:00:01Z".to_string(),
            request: RecordedRequest {
                method: "GET".to_string(),
                path: "/api/v1/timelines/public".to_string(),
                headers: HashMap::new(),
                body: None,
            },
            response: RecordedResponse {
                status: 200,
                headers: HashMap::new(),
                body: "[]".to_string(),
            },
        };

        recorder.record(&exchange1).unwrap();
        recorder.record(&exchange2).unwrap();

        // Read back and verify
        let file = File::open(&path).unwrap();
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().map(|l| l.unwrap()).collect();

        assert_eq!(lines.len(), 2);

        let parsed1: RecordedExchange = serde_json::from_str(&lines[0]).unwrap();
        let parsed2: RecordedExchange = serde_json::from_str(&lines[1]).unwrap();

        assert_eq!(parsed1, exchange1);
        assert_eq!(parsed2, exchange2);
    }

    #[test]
    fn test_traffic_recorder_appends_to_existing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("traffic.jsonl");

        // First recorder writes one exchange
        {
            let recorder = TrafficRecorder::new(path.clone()).unwrap();
            let exchange = RecordedExchange {
                timestamp: "2025-12-25T10:00:00Z".to_string(),
                request: RecordedRequest {
                    method: "GET".to_string(),
                    path: "/first".to_string(),
                    headers: HashMap::new(),
                    body: None,
                },
                response: RecordedResponse {
                    status: 200,
                    headers: HashMap::new(),
                    body: "{}".to_string(),
                },
            };
            recorder.record(&exchange).unwrap();
        }

        // Second recorder appends another
        {
            let recorder = TrafficRecorder::new(path.clone()).unwrap();
            let exchange = RecordedExchange {
                timestamp: "2025-12-25T10:00:01Z".to_string(),
                request: RecordedRequest {
                    method: "GET".to_string(),
                    path: "/second".to_string(),
                    headers: HashMap::new(),
                    body: None,
                },
                response: RecordedResponse {
                    status: 200,
                    headers: HashMap::new(),
                    body: "{}".to_string(),
                },
            };
            recorder.record(&exchange).unwrap();
        }

        // Verify both lines exist
        let file = File::open(&path).unwrap();
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().map(|l| l.unwrap()).collect();

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("/first"));
        assert!(lines[1].contains("/second"));
    }

    #[test]
    fn test_now_timestamp_format() {
        let ts = now_timestamp();

        // Should match ISO 8601 format: YYYY-MM-DDTHH:MM:SSZ
        assert!(ts.len() == 20);
        assert!(ts.ends_with('Z'));
        assert!(ts.contains('T'));
    }

    #[test]
    fn test_request_with_body() {
        let request = RecordedRequest {
            method: "POST".to_string(),
            path: "/api/v1/statuses".to_string(),
            headers: HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: Some(r#"{"status":"Hello world"}"#.to_string()),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("body"));

        let request_no_body = RecordedRequest {
            method: "GET".to_string(),
            path: "/api/v1/instance".to_string(),
            headers: HashMap::new(),
            body: None,
        };

        let json_no_body = serde_json::to_string(&request_no_body).unwrap();
        assert!(!json_no_body.contains("body")); // skip_serializing_if works
    }
}
