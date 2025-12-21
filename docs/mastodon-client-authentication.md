# Mastodon Client Authentication

This document describes how Mastodon clients authenticate and communicate with servers using OAuth 2.0.

## Overview

Mastodon uses OAuth 2.0 for client authentication. The API operates as a REST interface using HTTP requests and JSON responses. All authenticated requests use Bearer tokens passed in the `Authorization` header.

## OAuth 2.0 Flow

### 1. Application Registration

Before authenticating users, applications must register with the Mastodon server.

**Endpoint:** `POST /api/v1/apps`

**Required Parameters:**
- `client_name` - A name for your application
- `redirect_uris` - Where the user should be redirected after authorization (can be a single string or array)

**Optional Parameters:**
- `scopes` - Space-separated list of permissions (defaults to `read`)
- `website` - Homepage URL for your application

**Response:**
```json
{
  "id": "12345",
  "client_id": "abc123...",
  "client_secret": "secret456...",
  "client_secret_expires_at": 0,
  "redirect_uris": ["https://myapp.example/callback"],
  "scopes": ["read", "write"]
}
```

**Security Note:** Treat `client_id` and `client_secret` as passwords. Encrypt when storing.

### 2. Authorization Request

Direct the user to the authorization endpoint to grant access.

**Endpoint:** `GET /oauth/authorize`

**Parameters:**
| Parameter | Required | Description |
|-----------|----------|-------------|
| `response_type` | Yes | Must be `code` |
| `client_id` | Yes | Application identifier from registration |
| `redirect_uri` | Yes | Must match registered URI, or `urn:ietf:wg:oauth:2.0:oob` for display |
| `scope` | No | Space-separated permissions (must be subset of registered scopes) |
| `state` | Recommended | Arbitrary value for CSRF protection |
| `code_challenge` | Recommended | PKCE challenge (S256 method only) |
| `code_challenge_method` | With PKCE | Must be `S256` |
| `force_login` | No | Enables multi-account authorization |
| `lang` | No | ISO 639-1 language code for authorization form |

**Response:** User is redirected to `redirect_uri` with `code` query parameter.

### 3. Token Exchange

Exchange the authorization code for an access token.

**Endpoint:** `POST /oauth/token`

**Parameters:**
| Parameter | Required | Description |
|-----------|----------|-------------|
| `grant_type` | Yes | `authorization_code` or `client_credentials` |
| `code` | For auth code | The authorization code received |
| `client_id` | Yes | Application identifier |
| `client_secret` | Yes | Application secret |
| `redirect_uri` | For auth code | Must match authorization request |
| `code_verifier` | With PKCE | The original PKCE verifier |
| `scope` | For client_credentials | Requested scopes |

**Response:**
```json
{
  "access_token": "token123...",
  "token_type": "Bearer",
  "scope": "read write",
  "created_at": 1703123456
}
```

## Bearer Token Usage

All authenticated API requests must include the access token in the `Authorization` header:

```http
GET /api/v1/accounts/verify_credentials HTTP/1.1
Host: mastodon.social
Authorization: Bearer <access_token>
```

Example with curl:
```bash
curl -H "Authorization: Bearer <access_token>" \
  https://mastodon.social/api/v1/accounts/verify_credentials
```

## Grant Types

### Authorization Code Flow (for end-users)
The standard OAuth 2.0 flow for applications acting on behalf of users:
1. Register application
2. Redirect user to `/oauth/authorize`
3. User approves access
4. Receive authorization code via redirect
5. Exchange code for access token at `/oauth/token`

### Client Credentials Flow (for applications)
For applications that do not act on behalf of users:
1. Register application
2. Request token directly at `/oauth/token` with `grant_type=client_credentials`

**Note:** Password Grant was previously supported but has been removed for security reasons.

## PKCE Support

Since Mastodon v4.3.0, PKCE (Proof Key for Code Exchange) is supported and recommended for both confidential and public clients.

**Implementation:**
1. Generate a random `code_verifier` (43-128 characters)
2. Create `code_challenge` by base64url encoding the SHA-256 hash of the verifier
3. Include `code_challenge` and `code_challenge_method=S256` in authorization request
4. Include `code_verifier` in token exchange request

## OAuth Scopes

### High-Level Scopes

| Scope | Description |
|-------|-------------|
| `profile` | Minimal access to authenticated user info only |
| `read` | Read-only access to all data |
| `write` | Create and modify data |
| `push` | Web Push API subscription management (v2.4.0+) |
| `follow` | Deprecated since v3.5.0, use granular scopes |

### Granular Read Scopes
- `read:accounts` - Account information
- `read:blocks` - Blocked accounts
- `read:bookmarks` - Bookmarked statuses
- `read:favourites` - Favourited statuses
- `read:filters` - Content filters
- `read:follows` - Following relationships
- `read:lists` - User lists
- `read:mutes` - Muted accounts
- `read:notifications` - Notifications
- `read:search` - Search results
- `read:statuses` - Statuses/toots

### Granular Write Scopes
- `write:accounts` - Update profile
- `write:blocks` - Block/unblock accounts
- `write:bookmarks` - Add/remove bookmarks
- `write:conversations` - Manage conversations
- `write:favourites` - Favourite/unfavourite
- `write:filters` - Create/update filters
- `write:follows` - Follow/unfollow
- `write:lists` - Manage lists
- `write:media` - Upload media
- `write:mutes` - Mute/unmute accounts
- `write:notifications` - Dismiss notifications
- `write:reports` - File reports
- `write:statuses` - Create/delete statuses

### Admin Scopes
- `admin:read` / `admin:write` - Full admin access (v2.9.1+)
- Granular admin scopes: `admin:read:accounts`, `admin:write:reports`, etc.

**Best Practice:** Request the most limited scopes possible for your application.

## Key API Endpoints

### OAuth Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/v1/apps` | POST | Register application |
| `/.well-known/oauth-authorization-server` | GET | Server metadata (RFC 8414) |
| `/oauth/authorize` | GET | User authorization form |
| `/oauth/token` | POST | Obtain access token |
| `/oauth/revoke` | POST | Revoke access token |
| `/oauth/userinfo` | GET/POST | OpenID Connect user info |

### Token Revocation

**Endpoint:** `POST /oauth/revoke`

**Parameters:**
- `client_id` - Application identifier
- `client_secret` - Application secret
- `token` - Token to revoke

**Response:** Empty 200 response (operation is idempotent)

### Verify Credentials

**Endpoint:** `GET /api/v1/accounts/verify_credentials`

Used to verify the access token is valid and retrieve the authenticated user's account.

**Required Scope:** `read:accounts` or `read`

## HTTP Methods

The Mastodon API uses standard REST methods:
- **GET** - Read/view resources
- **POST** - Create resources or send data
- **PUT/PATCH** - Update resources
- **DELETE** - Remove resources

## Request Parameter Formats

Parameters can be submitted via:
1. **Query strings** - For GET requests (`?parameter=value`)
2. **Form data** - For POST/PUT/PATCH/DELETE requests
3. **JSON body** - With `Content-Type: application/json` header

## Response Status Codes

| Code | Meaning |
|------|---------|
| 200 | Success |
| 401 | Unauthorized - Invalid or missing token |
| 403 | Forbidden - Token lacks required scope |
| 404 | Not Found |
| 422 | Unprocessable Entity - Invalid parameters |
| 429 | Rate Limited |
| 5xx | Server Error |

## Security Recommendations

1. **Always use HTTPS** for all API communication
2. **Implement PKCE** even for confidential clients
3. **Use the `state` parameter** to prevent CSRF attacks
4. **Request minimal scopes** needed for your application
5. **Store tokens securely** - encrypt at rest
6. **Implement token refresh** and revocation
7. **Validate the `state` parameter** on redirect

## References

- [Mastodon OAuth Documentation](https://docs.joinmastodon.org/spec/oauth/)
- [Mastodon API Methods: OAuth](https://docs.joinmastodon.org/methods/oauth/)
- [Mastodon OAuth Scopes](https://docs.joinmastodon.org/api/oauth-scopes/)
- [RFC 6749 - OAuth 2.0](https://datatracker.ietf.org/doc/html/rfc6749)
- [RFC 7636 - PKCE](https://datatracker.ietf.org/doc/html/rfc7636)
