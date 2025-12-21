# Proxy Interception Strategy

This document defines how the IvoryValley proxy intercepts and handles client-server communication.

## Decision: Explicit Proxy Configuration

**Chosen approach:** Explicit proxy configuration where the client points directly to the proxy URL.

### Why Explicit Proxy?

| Aspect | Explicit Proxy | MITM Proxy |
|--------|----------------|------------|
| **Setup** | Client configures proxy URL | Requires certificate installation |
| **Certificates** | No custom CA needed | Must install and trust custom CA |
| **Complexity** | Simple HTTP/WebSocket forwarding | TLS interception, cert generation |
| **Privacy** | Transparent to user | Hidden interception |
| **Compatibility** | Works with any client | May break certificate pinning |
| **Debugging** | Easy to trace requests | Complex certificate chain |

### Client Configuration

Instead of connecting to their Mastodon server directly, clients configure the proxy as their server:

```
Original:     Client → mastodon.social
With Proxy:   Client → proxy.local → mastodon.social
```

The client treats the proxy as if it were the Mastodon server. The proxy then forwards requests to the actual upstream server.

## Architecture Overview

```
┌─────────────┐         ┌─────────────┐         ┌──────────────────┐
│   Mastodon  │  HTTP   │   Ivory     │  HTTPS  │    Upstream      │
│   Client    │ ──────► │   Valley    │ ───────►│    Mastodon      │
│             │         │   Proxy     │         │    Server        │
└─────────────┘         └─────────────┘         └──────────────────┘
       │                       │                         │
       │    REST API           │    Rewritten            │
       │    WebSocket          │    Requests             │
       │                       │                         │
       └───────────────────────┴─────────────────────────┘
```

## Request Rewriting

### URL Transformation

The proxy rewrites incoming requests to target the upstream server:

```
Incoming:   GET /api/v1/timelines/home HTTP/1.1
            Host: proxy.local:8080

Outgoing:   GET /api/v1/timelines/home HTTP/1.1
            Host: mastodon.social
```

### Header Transformation

```
function transform_request(request, upstream_host):
    headers = copy(request.headers)
    headers['Host'] = upstream_host

    // Forward authentication unchanged
    // See: authentication-passthrough.md

    // Add proxy identification (optional)
    headers['X-Forwarded-For'] = request.client_ip
    headers['X-Forwarded-Proto'] = 'https'

    return headers
```

### Headers Preserved

| Header | Action | Reason |
|--------|--------|--------|
| `Authorization` | Passthrough | Client authentication |
| `Content-Type` | Passthrough | Request format |
| `Accept` | Passthrough | Response negotiation |
| `Accept-Language` | Passthrough | Localization |
| `User-Agent` | Passthrough | Client identification |

### Headers Modified

| Header | Action | Reason |
|--------|--------|--------|
| `Host` | Replace | Target upstream server |
| `X-Forwarded-For` | Add | Client IP tracking |
| `X-Forwarded-Proto` | Add | Original protocol |

## REST API Handling

### Request Flow

```
1. Client sends request to proxy
2. Proxy determines upstream server (from configuration)
3. Proxy rewrites Host header
4. Proxy forwards request to upstream
5. Upstream responds
6. Proxy processes response (deduplication, caching)
7. Proxy returns response to client
```

### Upstream Server Determination

The proxy needs to know which upstream server to forward to. Options:

**Option A: Single upstream (simplest)**
```yaml
upstream: mastodon.social
```

**Option B: Per-client configuration**
```yaml
clients:
  - token_hash: abc123...
    upstream: mastodon.social
  - token_hash: def456...
    upstream: fosstodon.org
```

**Option C: Custom header**
```http
X-Upstream-Server: mastodon.social
```

**Recommendation:** Start with Option A (single upstream), extend later if needed.

### Response Processing

After receiving the upstream response, the proxy can:

1. **Deduplicate content** - Store unique messages by URI (see [message-uniqueness.md](./message-uniqueness.md))
2. **Cache responses** - Reduce upstream calls for repeated data
3. **Transform responses** - Add metadata, filter content

```
function process_response(response, request_path):
    if is_timeline_endpoint(request_path):
        // Apply deduplication
        statuses = parse_json(response.body)
        for status in statuses:
            store_unique_content(status)
        return response

    // Other endpoints: passthrough
    return response
```

## WebSocket/Streaming Handling

### Streaming API Architecture

The Mastodon Streaming API uses WebSocket connections for real-time updates.

```
┌──────────┐    WebSocket    ┌───────┐    WebSocket    ┌──────────┐
│  Client  │ ◄─────────────► │ Proxy │ ◄─────────────► │ Upstream │
└──────────┘                 └───────┘                 └──────────┘
                                 │
                         Event Processing
                         - Deduplication
                         - Filtering
```

### WebSocket Proxy Strategy

**Approach: Bidirectional relay with event inspection**

```
class StreamingProxy:
    function handle_connection(client_ws, upstream_url):
        upstream_ws = websocket_connect(upstream_url)

        // Relay messages bidirectionally (concurrent)
        parallel:
            relay_client_to_upstream(client_ws, upstream_ws)
            relay_upstream_to_client(upstream_ws, client_ws)

    function relay_upstream_to_client(upstream_ws, client_ws):
        for each message from upstream_ws:
            event = parse_streaming_event(message)
            if event.type == 'update':
                store_unique_content(event.payload)
            client_ws.send(message)

    function relay_client_to_upstream(client_ws, upstream_ws):
        for each message from client_ws:
            // Forward subscription commands unchanged
            upstream_ws.send(message)
```

### Streaming Discovery

Clients discover the streaming URL from the instance endpoint:

```http
GET /api/v2/instance
```

Response includes:
```json
{
  "configuration": {
    "urls": {
      "streaming": "wss://streaming.mastodon.social"
    }
  }
}
```

**Proxy must intercept this response** and rewrite the streaming URL to point back to the proxy:

```
function rewrite_instance_response(response, proxy_streaming_url):
    data = parse_json(response.body)
    if 'configuration' in data and 'urls' in data['configuration']:
        data['configuration']['urls']['streaming'] = proxy_streaming_url
    return to_json(data)
```

### Authentication for Streaming

WebSocket connections require authentication via one of:

1. `Authorization: Bearer <token>` header (preferred)
2. `Sec-Websocket-Protocol` header with token
3. `access_token` query parameter (not recommended)

The proxy forwards authentication headers unchanged (see [authentication-passthrough.md](./authentication-passthrough.md)).

## Connection Management

### Connection Pooling

For REST API calls, maintain a connection pool to the upstream server:

```
class UpstreamPool:
    function init(upstream_host, max_connections=100):
        this.pool = create_connection_pool(
            limit=max_connections,
            keepalive_timeout=30
        )
```

### WebSocket Lifecycle

- One upstream WebSocket per client WebSocket
- Handle reconnection with exponential backoff
- Clean up when client disconnects

```
function handle_disconnect(client_ws, upstream_ws):
    upstream_ws.close()
    // Clean up any client-specific state
```

## Error Handling

### Upstream Errors

Pass through error responses unchanged:

| Upstream Status | Proxy Action |
|-----------------|--------------|
| 401 Unauthorized | Passthrough |
| 403 Forbidden | Passthrough |
| 404 Not Found | Passthrough |
| 429 Rate Limited | Passthrough (consider backoff) |
| 5xx Server Error | Passthrough |

### Proxy-Specific Errors

| Scenario | Response |
|----------|----------|
| Upstream unreachable | 502 Bad Gateway |
| Upstream timeout | 504 Gateway Timeout |
| Invalid request | 400 Bad Request |

## Security Considerations

### TLS Configuration

```
Client ──── HTTP/HTTPS ──── Proxy ──── HTTPS ──── Upstream
```

- Client to proxy: Can be HTTP for local development, HTTPS for production
- Proxy to upstream: Always HTTPS

### Credential Safety

- Never log Authorization headers or tokens
- Never store credentials
- See [authentication-passthrough.md](./authentication-passthrough.md) for details

## Configuration Example

```yaml
# proxy-config.yaml
server:
  host: 0.0.0.0
  port: 8080

upstream:
  host: mastodon.social
  port: 443
  tls: true

streaming:
  proxy_url: ws://localhost:8080/api/v1/streaming

connection_pool:
  max_connections: 100
  keepalive_timeout: 30

features:
  deduplication: true
  caching: true
  cache_ttl: 300
```

## Implementation Roadmap

1. **Phase 1: Basic REST proxy**
   - Request forwarding
   - Header transformation
   - Response passthrough

2. **Phase 2: Response processing**
   - Timeline deduplication
   - Content storage
   - Response caching

3. **Phase 3: WebSocket support**
   - Streaming proxy
   - Instance URL rewriting
   - Event processing

4. **Phase 4: Multi-upstream support**
   - Per-client configuration
   - Dynamic routing

## Related Documents

- [Client-Server Traffic Patterns](./client-server-traffic-patterns.md) - API endpoints and traffic flow
- [Authentication Passthrough](./authentication-passthrough.md) - How auth is handled
- [Message Uniqueness](./message-uniqueness.md) - Deduplication criteria

## References

- [HTTP/1.1 RFC 7230](https://datatracker.ietf.org/doc/html/rfc7230)
- [WebSocket RFC 6455](https://datatracker.ietf.org/doc/html/rfc6455)
- [Mastodon Streaming API](https://docs.joinmastodon.org/methods/streaming/)
