# Technology Stack Recommendation

This document analyzes and recommends the technology stack for implementing the IvoryValley proxy.

## Executive Summary

**Recommendation: Rust with Axum**

Rust best fulfills the core requirements:
- Single binary deployment (cross-platform)
- Minimal memory footprint (critical for resource-constrained environments)
- Native WebSocket support for streaming
- Excellent async I/O performance

## Requirements Analysis

Based on existing documentation and issue discussions:

| Requirement | Source | Priority |
|-------------|--------|----------|
| HTTP Proxy with header rewriting | [proxy-interception-strategy.md](./proxy-interception-strategy.md) | Must have |
| WebSocket bidirectional relay | [client-server-traffic-patterns.md](./client-server-traffic-patterns.md) | Must have |
| JSON parsing/modification | [message-uniqueness.md](./message-uniqueness.md) | Must have |
| SQLite integration | Issue #11 | Must have |
| Single binary, cross-platform | Issue #23 | Must have |
| Low memory footprint | User requirement | Must have |
| Multi-user support (post-MVP) | User requirement | Should have |

## Candidates Evaluated

### Languages Considered

| Language | Single Binary | Memory | WebSocket | Verdict |
|----------|---------------|--------|-----------|---------|
| **Rust** | Native | ~2-10 MB | Excellent | **Recommended** |
| Go | Native | ~10-30 MB | Good | Excluded (user preference) |
| Python | Problematic | ~50-300+ MB | Good | Not suitable |
| TypeScript/Node | Experimental | ~50-150 MB | Good | Not suitable |

### Why Not Python?

- **PyOxidizer** requires Rust knowledge for configuration and has known issues with `__file__` attributes
- **PyInstaller** cannot cross-compile - requires building on each target platform
- Memory usage can explode to 3GB+ under high concurrency without careful optimization
- No true static linking possible

### Why Not TypeScript/Node?

- Node.js Single Executable Applications (SEA) is still experimental (Stage 1.1)
- `pkg` tool has been archived (2024)
- Requires CommonJS bundling, `fs.readFile` dependencies break
- Cross-platform testing only stable on Linux per documentation

### Why Rust?

1. **Memory**: Lowest footprint of all candidates, Axum specifically optimized for minimal memory per connection
2. **Single Binary**: `cargo build --release` produces a static binary
3. **Cross-Platform**: Built-in cross-compilation support
4. **WebSocket**: Native async support with tokio-tungstenite
5. **Future-Proof**: Multi-user scaling is naturally efficient

## Recommended Stack

### Core Dependencies

| Component | Crate | Version | Purpose |
|-----------|-------|---------|---------|
| Runtime | `tokio` | 1.x | Async runtime, de-facto standard |
| HTTP Server | `axum` | 0.8.x | Web framework, Tower-compatible |
| HTTP Client | `reqwest` | 0.12.x | Upstream requests, connection pooling |
| WebSocket | `tokio-tungstenite` | 0.26.x | Bidirectional streaming |
| JSON | `serde` + `serde_json` | 1.x | Serialization/deserialization |
| Database | `rusqlite` | 0.32.x | SQLite for seen-message storage |
| Config | `config` | 0.14.x | YAML/TOML configuration |
| Logging | `tracing` | 0.1.x | Structured logging |

### Cargo.toml

```toml
[package]
name = "ivoryvalley"
version = "0.1.0"
edition = "2024"

[dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# Web framework
axum = { version = "0.8", features = ["ws"] }

# HTTP client for upstream requests
reqwest = { version = "0.12", features = ["json"] }

# WebSocket client for upstream streaming
tokio-tungstenite = "0.26"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Database
rusqlite = { version = "0.32", features = ["bundled"] }

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Configuration
config = "0.14"
```

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      IvoryValley Proxy                          │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    axum Router                           │   │
│  │  ├── GET/POST /api/* ──► HTTP Proxy Handler              │   │
│  │  │                       └── reqwest ──► Upstream        │   │
│  │  │                                                       │   │
│  │  └── GET /api/v1/streaming ──► WebSocket Upgrade         │   │
│  │                                └── tokio-tungstenite     │   │
│  │                                    └── Upstream WS       │   │
│  └─────────────────────────────────────────────────────────┘   │
│                              │                                  │
│                              ▼                                  │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │              Deduplication Layer                         │   │
│  │  ├── Parse JSON responses                                │   │
│  │  ├── Extract URI (or reblog.uri for boosts)              │   │
│  │  └── Check/store in SQLite                               │   │
│  └─────────────────────────────────────────────────────────┘   │
│                              │                                  │
│                              ▼                                  │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │              rusqlite Database                           │   │
│  │  seen_uris (uri TEXT PRIMARY KEY, first_seen DATETIME)   │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Implementation Notes

### SQLite with Async

`rusqlite` is synchronous. Two approaches:

**Option A: spawn_blocking (simpler)**
```rust
let uri = status.uri.clone();
let is_new = tokio::task::spawn_blocking(move || {
    db.execute("INSERT OR IGNORE INTO seen_uris (uri) VALUES (?)", [&uri])
}).await??;
```

**Option B: sqlx with sqlite feature (fully async)**
```rust
sqlx::query("INSERT OR IGNORE INTO seen_uris (uri) VALUES (?)")
    .bind(&status.uri)
    .execute(&pool)
    .await?;
```

Recommendation: Start with `rusqlite` + `spawn_blocking` for simplicity.

### WebSocket Proxy Pattern

```rust
async fn handle_streaming(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| streaming_relay(socket, state))
}

async fn streaming_relay(client_ws: WebSocket, state: AppState) {
    let upstream_ws = connect_upstream(&state.upstream_url).await?;

    let (client_tx, client_rx) = client_ws.split();
    let (upstream_tx, upstream_rx) = upstream_ws.split();

    // Bidirectional relay with event inspection
    tokio::select! {
        _ = relay_to_upstream(client_rx, upstream_tx) => {},
        _ = relay_to_client(upstream_rx, client_tx, &state.db) => {},
    }
}
```

### Cross-Platform Builds

```bash
# Linux (native)
cargo build --release

# Windows (cross-compile from Linux)
cargo build --release --target x86_64-pc-windows-gnu

# macOS (requires macOS or cross-compilation setup)
cargo build --release --target x86_64-apple-darwin
```

For CI/CD, consider using `cross` for simplified cross-compilation:
```bash
cargo install cross
cross build --release --target x86_64-pc-windows-gnu
```

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Async Rust learning curve | Medium | Medium | Tokio has excellent tutorials; axum examples are pragmatic |
| WebSocket proxy complexity | Medium | High | Reference implementations exist (e.g., Lynx Proxy) |
| SQLite contention (multi-user) | Low (MVP) | Medium | Use connection pooling or switch to sqlx for async |
| Dependency security | Low | High | Use `cargo audit`, dependabot |

## Alternative Considered: Hybrid Approach

A two-phase approach was considered:
1. Prototype in Python/TypeScript for rapid iteration
2. Rewrite in Rust for production

**Rejected because:**
- Double implementation effort
- User has 30+ years programming experience, confident in learning Rust quickly
- Requirements strongly favor Rust from the start

## References

- [Rust Web Frameworks Benchmark 2025](https://markaicode.com/rust-web-frameworks-performance-benchmark-2025/)
- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)
- [Axum WebSocket Examples](https://github.com/tokio-rs/axum/tree/main/examples)
- [Building Real-time WebSockets with Rust and Axum](https://medium.com/rustaceans/beyond-rest-building-real-time-websockets-with-rust-and-axum-in-2025-91af7c45b5df)
- [Lynx Proxy - Rust HTTP/WebSocket Proxy](https://users.rust-lang.org/t/lynx-proxy-a-modern-high-performance-proxy-tool-in-rust/129897)

## Related Issues

- #32 - Research technology stack (this document)
- #23 - Build standalone binary
- #10 - Proxy interception strategy
- #11 - Database selection (SQLite)
- #12 - Deduplication strategy
