<picture>
  <source media="(prefers-color-scheme: dark)" srcset="artwork/wordmark-claim-light.svg">
  <source media="(prefers-color-scheme: light)" srcset="artwork/wordmark-claim-dark.svg">
  <img alt="IvoryValley" src="artwork/wordmark-claim-dark.svg" width="400">
</picture>

# IvoryValley

> **Status: Under Heavy Development** - This project is in early development. APIs and features may change without notice.

A transparent deduplication proxy for Mastodon and the Fediverse.

## The Problem

Following users across multiple Fediverse accounts often results in seeing the same posts repeatedly due to boosts/reposts. Your timeline can show the same content 10+ times.

## The Solution

IvoryValley sits between your Mastodon client and the upstream server, filtering out duplicate posts before they reach you. It tracks seen post URIs and removes duplicates from timeline responses.

```
┌─────────────┐     ┌─────────────────┐     ┌──────────────┐
│   Client    │────▶│   IvoryValley   │────▶│   Mastodon   │
│  (Tusky,    │◀────│                 │◀────│   Instance   │
│   etc.)     │     │  - Filter dupes │     │              │
└─────────────┘     │  - Store URIs   │     └──────────────┘
                    │  - Pass auth    │
                    └─────────────────┘
```

## Features

- **Transparent proxying** - Works with any Mastodon-compatible client
- **Deduplication** - Filters duplicate posts and boosts from timelines
- **WebSocket streaming** - Real-time filtering for streaming connections
- **OAuth passthrough** - No credential handling, tokens are forwarded directly
- **SQLite storage** - Lightweight local database for tracking seen URIs
- **Configurable** - CLI args, environment variables, or config file
- **Health endpoint** - `/health` endpoint for load balancers and Kubernetes probes

## Installation

### From Docker (Recommended)

Pull and run the latest image:

```bash
docker pull ghcr.io/jensens/ivoryvalley:latest
docker run -d \
  --name ivoryvalley \
  -p 8080:8080 \
  -v ivoryvalley-data:/data \
  -e IVORYVALLEY_UPSTREAM_URL=https://mastodon.social \
  ghcr.io/jensens/ivoryvalley:latest
```

Or use docker-compose (see [docker-compose.yml](docker-compose.yml)):

```bash
# Edit docker-compose.yml with your upstream URL
docker compose up -d
```

### From crates.io

```bash
cargo install ivoryvalley
```

### From GitHub Releases

Download the appropriate binary for your platform from the [Releases](https://github.com/jensens/ivoryvalley/releases) page.

### From Source

```bash
git clone https://github.com/jensens/ivoryvalley.git
cd ivoryvalley
cargo build --release
```

The binary will be at `target/release/ivoryvalley`.

## Quick Start

### 1. Start IvoryValley

```bash
# Using Docker (recommended)
docker run -d -p 8080:8080 \
  -v ivoryvalley-data:/data \
  -e IVORYVALLEY_UPSTREAM_URL=https://mastodon.social \
  ghcr.io/jensens/ivoryvalley:latest

# Or using the binary directly
ivoryvalley --upstream-url https://mastodon.social
```

### 2. Configure Your Client

Point your Mastodon client to your IvoryValley URL instead of your instance URL. Most clients require HTTPS - see [Client Setup](#client-setup) for detailed instructions and [Local HTTPS Setup](#local-https-setup) for development options.

### 3. Log In Normally

Use your regular credentials. IvoryValley passes authentication through to your instance transparently.

## Usage

```bash
# Basic usage
ivoryvalley --upstream-url https://mastodon.social

# With custom host and port
ivoryvalley --upstream-url https://mastodon.social --host 127.0.0.1 --port 3000

# With environment variables
IVORYVALLEY_UPSTREAM_URL=https://mastodon.social ivoryvalley

# With a config file
ivoryvalley --config /path/to/ivoryvalley.toml
```

## Configuration

IvoryValley supports configuration via:

1. **CLI arguments** (highest priority)
2. **Environment variables** (prefixed with `IVORYVALLEY_`)
3. **Config file** (`config.toml`, `config.yaml`, `ivoryvalley.toml`, or `ivoryvalley.yaml`)
4. **Default values**

### Configuration Options

| Option | Env Variable | Default | Description |
|--------|--------------|---------|-------------|
| `--upstream-url` | `IVORYVALLEY_UPSTREAM_URL` | `https://mastodon.social` | Upstream Mastodon instance URL |
| `--host` | `IVORYVALLEY_HOST` | `0.0.0.0` | Address to bind to |
| `-p, --port` | `IVORYVALLEY_PORT` | `8080` | Port to listen on |
| `--database-path` | `IVORYVALLEY_DATABASE_PATH` | `ivoryvalley.db` | SQLite database path |
| `--max-body-size` | `IVORYVALLEY_MAX_BODY_SIZE` | `52428800` (50MB) | Maximum request body size in bytes |
| `--connect-timeout-secs` | `IVORYVALLEY_CONNECT_TIMEOUT_SECS` | `10` | HTTP connection timeout in seconds |
| `--request-timeout-secs` | `IVORYVALLEY_REQUEST_TIMEOUT_SECS` | `30` | HTTP request timeout in seconds |
| `--record-traffic-path` | `IVORYVALLEY_RECORD_TRAFFIC_PATH` | - | Path to record traffic (JSONL format) |
| `-c, --config` | `IVORYVALLEY_CONFIG` | - | Path to configuration file |

### Config File Example

Create an `ivoryvalley.toml` file:

```toml
upstream_url = "https://mastodon.social"
host = "127.0.0.1"
port = 8080
database_path = "/var/lib/ivoryvalley/seen.db"
```

Or `ivoryvalley.yaml`:

```yaml
upstream_url: "https://mastodon.social"
host: "127.0.0.1"
port: 8080
database_path: "/var/lib/ivoryvalley/seen.db"
```

## Health Check Endpoint

The proxy exposes a `/health` endpoint for monitoring and orchestration systems.

### Basic Health Check

```bash
curl http://localhost:8080/health
```

Response:
```json
{"status": "healthy", "version": "0.1.0"}
```

### Deep Health Check

Use the `?deep=true` query parameter to include database connectivity verification:

```bash
curl http://localhost:8080/health?deep=true
```

Response:
```json
{"status": "healthy", "version": "0.1.0", "checks": {"database": "ok"}}
```

This endpoint:
- Returns HTTP 200 when the service is healthy
- Does not require authentication
- Suitable for load balancer health checks and Kubernetes liveness/readiness probes

## How It Works

1. Client sends request to IvoryValley
2. IvoryValley forwards request to upstream Mastodon instance
3. For timeline endpoints, IvoryValley:
   - Extracts post URIs (globally unique across federation)
   - Checks each URI against the seen-URI database
   - Filters out duplicates
   - Records new URIs as seen
4. Returns filtered response to client

## Client Setup

IvoryValley works with any Mastodon-compatible client. You simply change the server URL from your instance (e.g., `https://mastodon.social`) to IvoryValley (e.g., `http://localhost:8080`).

### Important Notes

- **HTTP vs HTTPS**: Most Mastodon clients (both desktop and mobile) require HTTPS for OAuth authentication. See [Local HTTPS Setup](#local-https-setup) for development options.
- **Authentication**: Your login credentials and OAuth tokens are passed through to the upstream server. IvoryValley does not store or access them.
- **Same Instance**: Make sure to configure IvoryValley with the same upstream URL as your Mastodon account.

### Desktop Clients

> **Note**: Despite being desktop applications, many Mastodon clients require HTTPS even for localhost connections due to OAuth security requirements and embedded browser security policies. See [Local HTTPS Setup](#local-https-setup) for workarounds.

#### Tuba (Linux GTK)

1. Install: `flatpak install flathub dev.geopjr.Tuba`
2. Open Tuba and click "Add Account"
3. Enter your IvoryValley HTTPS URL (e.g., `https://localhost:8443` or your mkcert domain)
4. Log in with your normal Mastodon credentials

#### Tokodon (Linux KDE)

1. Install: `apt install tokodon` or via your package manager
2. Open Tokodon and add a new account
3. Enter your IvoryValley HTTPS URL as the server
4. Complete the OAuth flow normally

#### Whalebird (Cross-platform)

1. Download from [Whalebird releases](https://github.com/h3poteto/whalebird-desktop/releases)
2. Add new account and enter your IvoryValley HTTPS URL as the instance
3. Log in with your credentials

### Mobile Clients

Mobile clients require HTTPS. You need to deploy IvoryValley behind a reverse proxy with a valid SSL certificate.

**Recommended: Deploy with a Reverse Proxy**

Use Caddy, nginx, or Traefik with a valid SSL certificate. This enables full functionality including WebSocket streaming.

Example with Caddy:
```
ivoryvalley.example.com {
    reverse_proxy localhost:8080
}
```

#### Tusky (Android)

1. Deploy IvoryValley with a valid HTTPS certificate
2. Open Tusky and tap "Log in"
3. Enter your IvoryValley URL as the instance
4. Complete the OAuth login

#### Ivory / Ice Cubes / Mona (iOS)

1. Deploy IvoryValley with a valid HTTPS certificate (iOS requires valid certificates)
2. Enter your IvoryValley URL as the instance
3. Log in normally

### Web Clients

You can also access Mastodon's web interface through IvoryValley:

```bash
# Open in browser
xdg-open http://localhost:8080
```

Note that the web interface works best with HTTPS for all features.

### Local HTTPS Setup

For local development and testing with desktop/mobile clients, you'll need HTTPS with trusted certificates. Here are recommended approaches:

#### Option 1: mkcert (Recommended for Development)

[mkcert](https://github.com/FiloSottile/mkcert) creates locally-trusted development certificates.

```bash
# Install mkcert
# macOS
brew install mkcert

# Linux (check your package manager, or use the binary release)
# See https://github.com/FiloSottile/mkcert#installation

# Create and install the local CA
mkcert -install

# Generate certificates for localhost
mkcert localhost 127.0.0.1 ::1

# This creates localhost+2.pem and localhost+2-key.pem
```

Then use a reverse proxy like Caddy to serve IvoryValley over HTTPS:

```bash
# Caddyfile
localhost:8443 {
    tls localhost+2.pem localhost+2-key.pem
    reverse_proxy localhost:8080
}
```

#### Option 2: Caddy with Automatic HTTPS

[Caddy](https://caddyserver.com/) can automatically provision certificates. For local development with a custom domain:

1. Add an entry to `/etc/hosts`: `127.0.0.1 ivoryvalley.local`
2. Use Caddy with mkcert certificates:

```bash
# Caddyfile
ivoryvalley.local {
    tls internal
    reverse_proxy localhost:8080
}
```

#### Option 3: Deploy with a Real Domain

For the most seamless experience, deploy IvoryValley on a server with a real domain name. Caddy will automatically obtain Let's Encrypt certificates:

```bash
# Caddyfile
ivoryvalley.example.com {
    reverse_proxy localhost:8080
}
```

## Troubleshooting

### Connection Issues

**"Connection refused" error**

- Ensure IvoryValley is running: check with `curl http://localhost:8080/api/v1/instance`
- Verify the port isn't blocked by a firewall
- Check if another service is using port 8080

**"Timeout" errors**

- Check if the upstream server is reachable: `curl https://mastodon.social/api/v1/instance`
- Increase timeout settings if on a slow connection:
  ```bash
  ivoryvalley --upstream-url https://mastodon.social \
    --connect-timeout-secs 30 \
    --request-timeout-secs 120
  ```

### Authentication Issues

**OAuth login fails or redirects to wrong URL**

- Make sure your client is configured to use the IvoryValley URL (not the upstream instance)
- For HTTPS, ensure the certificate is accepted/trusted

**"Unauthorized" errors after login**

- Your token may have expired; try logging in again
- Make sure the upstream URL exactly matches your Mastodon instance

### Deduplication Issues

**Duplicates still appear**

- IvoryValley only filters posts it has seen before; the first occurrence always appears
- Check if the database file exists and is writable
- Duplicates in notifications are expected (only timeline endpoints are filtered)

**Want to reset seen posts?**

Delete the database file to start fresh:

```bash
# Find the database location
ls -la ivoryvalley.db

# Stop IvoryValley, delete database, restart
rm ivoryvalley.db
```

### Docker Issues

**Container won't start**

- Check logs: `docker logs ivoryvalley`
- Ensure the `IVORYVALLEY_UPSTREAM_URL` environment variable is set

**Database not persisting**

- Make sure you're using a volume: `-v ivoryvalley-data:/data`
- Check volume permissions: the container runs as user 1000

### Logging and Debugging

Enable debug logging to see detailed information:

```bash
# With the binary
RUST_LOG=ivoryvalley=debug ivoryvalley --upstream-url https://mastodon.social

# With Docker
docker run -e RUST_LOG=ivoryvalley=debug \
  -e IVORYVALLEY_UPSTREAM_URL=https://mastodon.social \
  ghcr.io/jensens/ivoryvalley:latest
```

Log levels: `error`, `warn`, `info`, `debug`, `trace`

### Recording Traffic for Debugging

If you need to debug API issues, you can record all traffic:

```bash
ivoryvalley --upstream-url https://mastodon.social \
  --record-traffic-path /tmp/traffic.jsonl
```

This creates a JSONL file with all request/response pairs.

## Development

This project uses [just](https://github.com/casey/just) as a command runner. Install with `cargo install just`.

```bash
# List all available commands
just

# Run the proxy in development mode
just dev

# Run all tests
just test

# Run all quality checks before committing
just check
```

### Testing with Clients

See [Client Setup](#client-setup) for detailed client configuration.

For quick HTTPS testing with the **web interface only**:

```bash
just dev-https
```

Note: The development HTTPS proxy does not support WebSocket connections, so native clients (desktop/mobile apps) won't have working streaming. For full client testing, deploy with a proper reverse proxy like Caddy.

### Manual Commands

```bash
# Run with logging
RUST_LOG=ivoryvalley=debug cargo run -- --upstream-url https://mastodon.social

# Check code quality
cargo clippy --all-features -- -D warnings

# Format code
cargo fmt
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

MIT
