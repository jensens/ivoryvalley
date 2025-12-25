#!/usr/bin/env bash
#
# Start IvoryValley with HTTPS via Caddy reverse proxy
#
# This script starts both the IvoryValley proxy and Caddy for local HTTPS testing.
# Requires: caddy (apt install caddy)
#
# Usage: ./scripts/dev-https.sh [upstream-url]
#
# The proxy will be available at:
#   - http://localhost:8080  (direct, no HTTPS)
#   - https://localhost:8443 (via Caddy, with HTTPS)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
UPSTREAM_URL="${1:-https://nerdculture.de}"

# Check dependencies
if ! command -v caddy &> /dev/null; then
    echo "Error: caddy is not installed. Install with: sudo apt install caddy"
    exit 1
fi

# Build if needed
if [[ ! -f "$PROJECT_DIR/target/debug/ivoryvalley" ]]; then
    echo "Building IvoryValley..."
    cargo build --manifest-path "$PROJECT_DIR/Cargo.toml"
fi

# Cleanup function
cleanup() {
    echo ""
    echo "Shutting down..."
    kill $PROXY_PID 2>/dev/null || true
    kill $CADDY_PID 2>/dev/null || true
    exit 0
}

trap cleanup SIGINT SIGTERM

echo "Starting IvoryValley proxy..."
echo "  Upstream: $UPSTREAM_URL"
echo "  HTTP:     http://localhost:8080"
echo "  HTTPS:    https://localhost:8443"
echo ""
echo "Press Ctrl+C to stop"
echo ""

# Start IvoryValley
"$PROJECT_DIR/target/debug/ivoryvalley" \
    --upstream-url "$UPSTREAM_URL" \
    --host 0.0.0.0 \
    --port 8080 &
PROXY_PID=$!

# Give the proxy a moment to start
sleep 1

# Start Caddy
caddy run --config "$SCRIPT_DIR/Caddyfile" &
CADDY_PID=$!

# Wait for either process to exit
wait $PROXY_PID $CADDY_PID
