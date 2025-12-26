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

## Usage

```bash
# Basic usage
ivoryvalley --upstream https://mastodon.social

# With custom port
ivoryvalley --upstream https://mastodon.social --port 8080

# With environment variables
IVORYVALLEY_UPSTREAM=https://mastodon.social ivoryvalley
```

Then configure your Mastodon client to use `http://localhost:8080` (or your chosen port) as the server URL instead of your actual instance.

## Configuration

IvoryValley supports configuration via:

1. **CLI arguments** (highest priority)
2. **Environment variables** (prefixed with `IVORYVALLEY_`)
3. **Config file** (`ivoryvalley.toml`)

### Options

| Option | Env Variable | Default | Description |
|--------|--------------|---------|-------------|
| `--upstream` | `IVORYVALLEY_UPSTREAM` | - | Upstream Mastodon instance URL (required) |
| `--host` | `IVORYVALLEY_HOST` | `127.0.0.1` | Address to bind to |
| `--port` | `IVORYVALLEY_PORT` | `3000` | Port to listen on |
| `--database` | `IVORYVALLEY_DATABASE` | `ivoryvalley.db` | SQLite database path |

## How It Works

1. Client sends request to IvoryValley
2. IvoryValley forwards request to upstream Mastodon instance
3. For timeline endpoints, IvoryValley:
   - Extracts post URIs (globally unique across federation)
   - Checks each URI against the seen-URI database
   - Filters out duplicates
   - Records new URIs as seen
4. Returns filtered response to client

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

### Testing with Mastodon Clients

Most Mastodon clients require HTTPS for OAuth. Use the built-in HTTPS proxy:

```bash
# Start proxy with HTTPS (available at https://localhost:8443)
just dev-https
```

This uses a Python HTTPS reverse proxy with a self-signed certificate. You'll need to accept the certificate warning in your client.

Then configure your Mastodon client to use `https://localhost:8443` as the server.

**Compatible clients:**

| Client | Platform | Notes |
|--------|----------|-------|
| Tuba | Linux (GTK) | `flatpak install flathub dev.geopjr.Tuba` |
| Tokodon | Linux (KDE) | `apt install tokodon` |
| Whalebird | Linux | Electron-based, AppImage available |
| Tusky | Android | Works with HTTPS proxy |

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
