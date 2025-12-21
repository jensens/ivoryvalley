# Authentication Passthrough Design

This document describes how the proxy handles authentication between Mastodon clients and upstream servers.

## Design Principle

**Transparent passthrough**: The proxy does not participate in authentication. It simply forwards credentials between client and server without inspection or modification.

## Why Passthrough?

1. **Simplicity**: No token storage, no credential management, no security liability
2. **Privacy**: Proxy never sees or stores user credentials
3. **Compatibility**: Works with any OAuth flow the client/server support
4. **Security**: No attack surface for credential theft at the proxy layer

## Architecture

```
┌────────┐     ┌───────┐     ┌────────────────┐
│ Client │────▶│ Proxy │────▶│ Mastodon Server│
└────────┘     └───────┘     └────────────────┘
     │              │                 │
     │  OAuth Flow  │                 │
     └──────────────┼─────────────────┘
                    │
            (passthrough only,
             no interception)
```

## What Gets Passed Through

### Authorization Header

All API requests include the Bearer token:

```http
GET /api/v1/timelines/home HTTP/1.1
Host: mastodon.social
Authorization: Bearer <access_token>
```

The proxy forwards this header unchanged:

```python
# Pseudocode
def proxy_request(request):
    # Copy Authorization header as-is
    upstream_headers = {
        'Authorization': request.headers.get('Authorization'),
        # ... other headers
    }
    return forward_to_upstream(request, upstream_headers)
```

### OAuth Endpoints

These endpoints are passed through without modification:

| Endpoint | Purpose | Proxy Action |
|----------|---------|--------------|
| `POST /api/v1/apps` | App registration | Passthrough |
| `GET /oauth/authorize` | User authorization | Passthrough |
| `POST /oauth/token` | Token exchange | Passthrough |
| `POST /oauth/revoke` | Token revocation | Passthrough |

## What the Proxy Does NOT Do

1. **No token inspection**: Never reads or validates token contents
2. **No token storage**: Never persists credentials
3. **No OAuth participation**: Never acts as OAuth client or server
4. **No scope checking**: Scope enforcement is upstream's responsibility
5. **No token refresh**: Client handles token lifecycle

## Request Flow

### Initial Authentication (Client → Server)

```
1. Client registers app with server (POST /api/v1/apps)
   Proxy: passthrough

2. Client redirects user to authorization
   Proxy: passthrough (or not involved if direct browser redirect)

3. Client exchanges code for token (POST /oauth/token)
   Proxy: passthrough

4. Client receives access token
   Proxy: never sees or stores it
```

### Authenticated API Calls

```
1. Client sends request with Authorization: Bearer <token>
2. Proxy forwards request unchanged
3. Server validates token and responds
4. Proxy forwards response to client (may apply deduplication)
```

## Error Handling

Authentication errors from upstream are passed through unchanged:

| Status | Meaning | Proxy Action |
|--------|---------|--------------|
| 401 | Invalid/expired token | Passthrough |
| 403 | Insufficient scope | Passthrough |

The client handles re-authentication when needed.

## Security Considerations

### Transport Security

- All connections MUST use HTTPS
- Proxy terminates TLS with client
- Proxy initiates new TLS connection to upstream
- Authorization header is encrypted in transit on both legs

### Header Handling

```python
# Headers to always forward
PASSTHROUGH_HEADERS = [
    'Authorization',
    'Content-Type',
    'Accept',
    'Accept-Language',
]

# Headers to never forward (security)
STRIP_HEADERS = [
    'Cookie',  # Proxy session, not server session
    'X-Forwarded-For',  # Will be set by proxy
]
```

### Logging

**Never log Authorization headers or tokens.** Example safe logging:

```python
def log_request(request):
    logger.info(f"{request.method} {request.path}")
    # DO NOT log: request.headers['Authorization']
```

## Implementation Notes

### Proxy Transparency

From the client's perspective, the proxy should be invisible for auth:

```python
class ProxyHandler:
    def handle_request(self, request):
        # Auth-related paths: pure passthrough
        if self.is_oauth_path(request.path):
            return self.passthrough(request)

        # API paths: passthrough + potential response processing
        response = self.passthrough(request)
        return self.process_response(response)

    def is_oauth_path(self, path):
        return path.startswith('/oauth/') or path == '/api/v1/apps'
```

### Multi-Server Support

When the proxy supports multiple upstream servers:

```python
def get_upstream(request):
    # Server determined by request, not by proxy session
    # Token is only valid for its issuing server
    return determine_server_from_request(request)
```

## Relation to Other Components

- **Deduplication**: Happens after authentication succeeds, on response data
- **Message Storage**: Stores content, never credentials
- **Client API**: Proxy doesn't add auth, client provides it

## References

- [Mastodon Client Authentication](./mastodon-client-authentication.md)
- [OAuth 2.0 Bearer Token Usage (RFC 6750)](https://datatracker.ietf.org/doc/html/rfc6750)
