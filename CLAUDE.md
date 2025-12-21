# Claude Code Guidelines for IvoryValley

## Mandatory: Test-Driven Development

**You MUST follow TDD for all implementation work.**

### TDD Workflow

1. **Write tests first** - Before any implementation, write failing tests
2. **Verify tests fail** - Run `cargo test` to confirm tests fail as expected
3. **Implement minimally** - Write only enough code to make tests pass
4. **Verify tests pass** - Run `cargo test` to confirm all tests pass
5. **Refactor** - Clean up while keeping tests green

### Commands

```bash
# Run all tests
cargo test

# Run with verbose output
cargo test -- --nocapture

# Check code quality
cargo clippy --all-features -- -D warnings

# Format code
cargo fmt
```

## Project Structure

- `src/` - Main source code
  - `main.rs` - Entry point
  - `config.rs` - Configuration handling
  - `db.rs` - Database operations (SQLite for seen-message storage)
  - `proxy.rs` - HTTP proxy logic
  - `websocket.rs` - WebSocket handling
- `tests/` - Integration tests
  - `common/mod.rs` - Shared test utilities
  - `fixtures/` - Test data files
- `docs/` - Design documentation

## Technology Stack

- **Runtime**: Tokio (async)
- **Web Framework**: Axum
- **HTTP Client**: Reqwest
- **WebSocket**: tokio-tungstenite
- **Database**: SQLite (rusqlite)
- **Serialization**: Serde

## Before Committing

Always run:
```bash
cargo test && cargo clippy --all-features -- -D warnings && cargo fmt --check
```
