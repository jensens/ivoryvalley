#!/usr/bin/env bash
#
# Start IvoryValley with HTTPS via Python reverse proxy
#
# This script starts both the IvoryValley proxy and a Python HTTPS proxy.
# Requires: python3, openssl
#
# Usage: ./scripts/dev-https.sh [upstream-url]
#
# The proxy will be available at:
#   - http://localhost:8080  (direct, no HTTPS)
#   - https://localhost       (via Python HTTPS proxy, requires sudo)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
UPSTREAM_URL="${1:-https://nerdculture.de}"
CERT_DIR="$SCRIPT_DIR/.certs"

# Generate self-signed cert if needed (no system trust store involvement)
if [[ ! -f "$CERT_DIR/localhost.crt" ]]; then
    echo "Generating self-signed certificate..."
    mkdir -p "$CERT_DIR"
    openssl req -x509 -newkey rsa:4096 -sha256 -days 365 -nodes \
        -keyout "$CERT_DIR/localhost.key" \
        -out "$CERT_DIR/localhost.crt" \
        -subj "/CN=localhost" \
        -addext "subjectAltName=DNS:localhost,DNS:ivoryvalley.test,IP:127.0.0.1" \
        2>/dev/null
    echo "Certificate generated at $CERT_DIR/"
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
    sudo kill $HTTPS_PID 2>/dev/null || true
    exit 0
}

trap cleanup SIGINT SIGTERM

echo "Starting IvoryValley proxy..."
echo "  Upstream: $UPSTREAM_URL"
echo "  HTTP:     http://localhost:8080"
echo "  HTTPS:    https://localhost or https://127.0.0.1.sslip.io (port 443)"
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

# Start Python HTTPS proxy (needs sudo for port 443)
sudo python3 "$SCRIPT_DIR/https-proxy.py" &
HTTPS_PID=$!

# Wait for either process to exit
wait $PROXY_PID $HTTPS_PID
