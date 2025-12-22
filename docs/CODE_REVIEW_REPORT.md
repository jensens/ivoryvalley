# IvoryValley Code Review Report

**Date:** 2025-12-22
**Reviewer:** Claude Code (Rust Review Skill)
**Codebase:** IvoryValley v0.1.0
**LOC:** ~1300 lines Rust

## Executive Summary

The IvoryValley codebase is **well-structured** with good separation of concerns and comprehensive test coverage (63 tests). However, there are **2 critical** and **4 medium** priority issues that should be addressed.

**Recommendation:** Approve with mandatory fixes for critical issues.

---

## Ownership Analysis

### Clone Patterns

| Location | Pattern | Assessment |
|----------|---------|------------|
| `main.rs:31` | `config.clone()` | Necessary for Arc sharing |
| `proxy.rs:56-57` | `seen_store.clone()` | Required for multi-handler sharing |
| `proxy.rs:74` | `request.method().clone()` | Necessary - Method is borrowed |
| `websocket.rs:82-83` | State clones before upgrade | Correct pattern for WebSocket upgrade |

**Finding:** All clone usage is justified. No unnecessary allocations detected.

### Arc Usage

```rust
pub struct SeenUriStore {
    conn: Mutex<Connection>,  // Not Arc<Mutex<...>>
}
```

The `SeenUriStore` uses internal `Mutex<Connection>` and is wrapped in `Arc` at the call site (`proxy.rs:54`). This is the correct pattern - the struct itself doesn't dictate its sharing strategy.

---

## Error Handling

### Critical Issues

#### [E1] Panic in Error Handler (CRITICAL)

**File:** `proxy.rs:265-269`

```rust
impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        // ...
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(...))
            .unwrap()  // PANIC RISK
    }
}
```

**Risk:** If `Response::builder()` fails (e.g., invalid header value), the application panics during error handling, potentially causing cascading failures.

**Recommendation:** Use `.expect()` with context or return a fallback response:

```rust
.unwrap_or_else(|_| {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(Body::empty())
        .unwrap() // Safe: minimal response
})
```

### Acceptable expect() Usage

| Location | Code | Assessment |
|----------|------|------------|
| `main.rs:19` | `Config::load().expect(...)` | OK - fail-fast at startup |
| `main.rs:28` | `SeenUriStore::open(...).expect(...)` | OK - fail-fast at startup |
| `main.rs:36` | `TcpListener::bind(...).expect(...)` | OK - fail-fast at startup |
| `db.rs:89,107,129,150` | Mutex lock expects | OK - documented panic recovery policy |
| `db.rs:104,126,160` | `SystemTime` expects | OK - system clock going backwards is unrecoverable |
| `config.rs:192` | HTTP client creation | OK - fail-fast at startup |

### Pattern Issue

#### [E2] Option Unwrap in Spawn (MEDIUM)

**File:** `websocket.rs:162`

```rust
let mut stream = client_stream.take().unwrap();
```

**Assessment:** This is *technically* safe because `client_stream` is always `Some` at this point (set at line 134), but the pattern is fragile. If refactoring moves code, this could panic.

**Recommendation:** Use `expect()` with invariant documentation:

```rust
let mut stream = client_stream.take()
    .expect("client_stream must be Some; set at line 134");
```

---

## Concurrency Review

### Async Patterns

#### Task Management (websocket.rs:140-214)

```rust
tokio::select! {
    _ = &mut upstream_to_client => info!("Upstream to client task ended"),
    _ = &mut client_to_upstream => info!("Client to upstream task ended"),
    _ = &mut send_to_client => info!("Send to client task ended"),
    _ = &mut send_to_upstream => info!("Send to upstream task ended"),
}

// Cleanup
upstream_to_client.abort();
client_to_upstream.abort();
send_to_client.abort();
send_to_upstream.abort();
```

**Assessment:** Correct pattern. Tasks are properly aborted on completion of any one.

### Sync Primitives

#### [C1] Mutex Contention (MEDIUM)

**File:** `db.rs:54-56`

```rust
pub struct SeenUriStore {
    conn: Mutex<Connection>,
}
```

**Assessment:** The Mutex serializes all database access. Under high load with many concurrent WebSocket connections, this could become a bottleneck.

**Current Mitigations (documented in db.rs:35-53):**
- DB operations are fast (indexed lookups)
- Critical section is small
- SQLite WAL mode enabled

**Future Optimization Path:**
- Connection pool (r2d2 or deadpool)
- `tokio-rusqlite` for async DB access
- Separate read connection

### Missing: Graceful Shutdown

**Finding:** No signal handling or graceful shutdown mechanism.

```rust
// main.rs - Server runs until process kill
axum::serve(listener, app).await.expect("Server error");
```

**Recommendation:** Add `tokio::signal` for SIGTERM/SIGINT handling.

---

## Unsafe Audit

**Result:** No `unsafe` blocks in the codebase.

The codebase relies on safe abstractions from:
- `rusqlite` (FFI to SQLite handled internally)
- `tokio-tungstenite` (WebSocket protocol)
- `reqwest` (HTTP client)

---

## Security Review

### Critical Issues

#### [S1] Unbounded Body Size (CRITICAL)

**File:** `proxy.rs:100`

```rust
let body_bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
    .await
    .map_err(|e| ProxyError::BodyRead(e.to_string()))?;
```

**Risk:** Memory exhaustion DoS. A malicious client can send an arbitrarily large body, consuming all server memory.

**Recommendation:**

```rust
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024; // 10MB

let body_bytes = axum::body::to_bytes(request.into_body(), MAX_BODY_SIZE)
    .await
    .map_err(|e| ProxyError::BodyRead(e.to_string()))?;
```

### Medium Issues

#### [S2] No HTTP Client Timeouts (MEDIUM)

**File:** `config.rs:189-192`

```rust
let http_client = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .expect("Failed to create HTTP client");
```

**Risk:** Slow upstream responses can tie up connections indefinitely.

**Recommendation:**

```rust
let http_client = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .connect_timeout(Duration::from_secs(10))
    .timeout(Duration::from_secs(30))
    .build()
    .expect("Failed to create HTTP client");
```

### Acceptable Patterns

| Pattern | Location | Assessment |
|---------|----------|------------|
| Access token in query params | `websocket.rs:41-47` | Mastodon API limitation, well-documented |
| Header whitelist | `proxy.rs:21-28` | Correct - only passes known headers |
| SQL prepared statements | `db.rs` | Correct - no SQL injection risk |
| URL encoding | `websocket.rs:232-242` | Correct - uses `urlencoding::encode()` |

---

## Dependencies Review

### Cargo Audit

```
$ cargo audit
Scanning Cargo.lock for vulnerabilities (286 crate dependencies)
No vulnerabilities found!
```

### Clippy

```
$ cargo clippy --all-features -- -D warnings
Finished dev profile [0 warnings]
```

### Dependency Notes

| Crate | Version | Notes |
|-------|---------|-------|
| `tokio` | 1.x | Current, "full" features appropriate |
| `axum` | 0.8 | Latest stable |
| `reqwest` | 0.12 | Latest stable |
| `rusqlite` | 0.32 | Latest, bundled SQLite |
| `tokio-tungstenite` | 0.26 | Compatible with tungstenite 0.26 |

---

## Test Coverage Analysis

### Test Count by Module

| Module | Unit Tests | Integration Tests | Total |
|--------|------------|-------------------|-------|
| config | 11 | - | 11 |
| db | 5 | 9 | 14 |
| proxy | 8 | 8 | 16 |
| websocket | 14 | 7 | 21 |
| **Total** | **38** | **24** | **62** |

### Coverage Gaps

1. **Negative path testing:**
   - Network timeout scenarios
   - Malformed upstream responses
   - Database corruption recovery

2. **Edge cases:**
   - Very large timelines (1000+ items)
   - Unicode edge cases in URIs
   - Concurrent access stress tests

3. **Missing integration tests:**
   - Full HTTP proxy flow with deduplication
   - Reconnection scenarios
   - Rate limiting behavior (not implemented)

---

## Architecture Review

### Current State

```
┌─────────────┐     ┌──────────────────┐     ┌──────────────┐
│   Client    │────▶│   IvoryValley    │────▶│   Mastodon   │
│ (Mastodon)  │◀────│     Proxy        │◀────│   Server     │
└─────────────┘     └──────────────────┘     └──────────────┘
                           │
                           ▼
                    ┌──────────────┐
                    │   SQLite DB  │
                    │ (seen URIs)  │
                    └──────────────┘
```

### Feature Gaps

| Feature | Status | Priority |
|---------|--------|----------|
| REST API Deduplication | Implemented | - |
| WebSocket Deduplication | Implemented | - |
| Background Cleanup | Not implemented | Medium |
| Health Check Endpoint | Not implemented | Medium |
| Metrics Endpoint | Not implemented | Low |
| Graceful Shutdown | Not implemented | Medium |
| Rate Limiting | Not implemented | Low |

---

## Recommendations

### Critical (Must Fix)

1. **[S1] Add body size limit** - DoS vulnerability
2. **[E1] Fix panic in error handler** - Reliability issue

### High Priority

3. **[S2] Add HTTP client timeouts** - Resource exhaustion risk
4. **[E2] Document Option::take() invariant** - Maintainability

### Medium Priority

5. Add graceful shutdown handling
6. Implement health check endpoint
7. Add background cleanup task for old URIs
8. Consider connection pooling for high-load scenarios

### Low Priority

9. Add metrics/observability
10. Implement rate limiting
11. Add fuzzing for parsers

---

## Action Items for Sub-Issues

Create the following GitHub issues:

1. **Critical: Fix unbounded body size in proxy handler**
2. **Critical: Fix unwrap() in ProxyError::into_response**
3. **Add HTTP client timeouts**
4. **Add graceful shutdown handling**
5. **Implement health check endpoint**
6. **Implement background cleanup task**
