# IvoryValley development commands
#
# Usage: just <command>
#
# Run `just --list` to see all available commands.

# Default upstream for development
default_upstream := "https://nerdculture.de"

# List available commands
default:
    @just --list

# Build debug binary
build:
    cargo build

# Build release binary
release:
    cargo build --release

# Run the proxy in development mode
dev upstream=default_upstream:
    cargo run -- --upstream-url {{upstream}} --host 0.0.0.0 --port 8080

# Run the proxy with HTTPS (uses Python reverse proxy)
dev-https upstream=default_upstream:
    ./scripts/dev-https.sh {{upstream}}

# Run all tests
test:
    cargo test

# Run tests with output
test-verbose:
    cargo test -- --nocapture

# Run clippy linter
lint:
    cargo clippy --all-features -- -D warnings

# Format code
fmt:
    cargo fmt

# Check formatting without modifying
fmt-check:
    cargo fmt --check

# Run all quality checks (use before committing)
check: test lint fmt-check

# Test WebSocket streaming (run in separate terminal while proxy is running)
# Usage: just ws-test [stream] [token]
ws-test stream="public" token="":
    uv run --with websockets ./scripts/ws-test-client.py --stream {{stream}} {{ if token != "" { "--token " + token } else { "" } }}

# Clean build artifacts
clean:
    cargo clean
