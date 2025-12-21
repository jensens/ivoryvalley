# Client-Server Traffic Patterns

This document analyzes client-server traffic patterns in Mastodon to inform the design of an ActivityPub-focused bridge.

## Overview

Mastodon clients communicate with servers using two primary mechanisms:

1. **REST API** - Synchronous HTTP requests for CRUD operations and data retrieval
2. **Streaming API** - WebSocket connections for real-time event delivery

## Most Used API Endpoints

Based on typical client behavior, these endpoints see the highest traffic:

### High-Frequency Polling Endpoints

| Endpoint | Purpose | Typical Frequency |
|----------|---------|-------------------|
| `GET /api/v1/timelines/home` | Home timeline fetch | Every 30-60 seconds |
| `GET /api/v1/notifications` | Notification polling | Every 15-30 seconds |
| `GET /api/v1/accounts/verify_credentials` | Token validation | On app launch |
| `GET /api/v1/instance` | Server metadata | On app launch |

### User-Triggered Endpoints

| Endpoint | Purpose | Trigger |
|----------|---------|---------|
| `POST /api/v1/statuses` | Create post | User action |
| `POST /api/v1/statuses/:id/favourite` | Like a post | User action |
| `POST /api/v1/statuses/:id/reblog` | Boost a post | User action |
| `GET /api/v1/statuses/:id/context` | Load thread | Viewing conversation |
| `GET /api/v1/accounts/:id` | View profile | User navigation |
| `GET /api/v1/accounts/:id/statuses` | Profile posts | User navigation |

## Timeline Fetch Patterns

### Pagination Model

All timeline endpoints use cursor-based pagination with three parameters:

```
┌──────────────────────────────────────────────────────────────────┐
│                     Timeline ID Space                             │
│                                                                   │
│  ◄───── older                                        newer ─────► │
│                                                                   │
│     │                    │                    │                   │
│     ▼                    ▼                    ▼                   │
│  since_id             min_id               max_id                 │
│  (exclusive)          (exclusive)          (exclusive)           │
│                                                                   │
│  Returns all newer    Returns page          Returns all older    │
│  than this ID         closest to            than this ID         │
│                       this ID                                     │
└──────────────────────────────────────────────────────────────────┘
```

**Parameter Usage:**
- `max_id` - Get posts older than this ID (scroll down)
- `since_id` - Get all posts newer than this ID (gap-filling)
- `min_id` - Get a page of posts immediately after this ID (refresh)
- `limit` - Results per page (default: 20, max: 40)

### Common Fetch Patterns

**Initial Load:**
```http
GET /api/v1/timelines/home?limit=20
Authorization: Bearer <token>
```

**Refresh (load newer):**
```http
GET /api/v1/timelines/home?min_id=<newest_id>&limit=20
```

**Load More (infinite scroll):**
```http
GET /api/v1/timelines/home?max_id=<oldest_id>&limit=20
```

**Gap Fill (missed posts):**
```http
GET /api/v1/timelines/home?since_id=<gap_start>&max_id=<gap_end>
```

### Timeline Types

| Timeline | Endpoint | Auth Required |
|----------|----------|---------------|
| Home | `/api/v1/timelines/home` | Yes |
| Local | `/api/v1/timelines/public?local=true` | Depends on server |
| Federated | `/api/v1/timelines/public` | Depends on server |
| Hashtag | `/api/v1/timelines/tag/:tag` | Depends on server |
| List | `/api/v1/timelines/list/:id` | Yes |

## Streaming API Usage

### Connection Architecture

```
┌─────────────────┐         WebSocket          ┌──────────────────┐
│                 │ ◄─────────────────────────► │                  │
│   Mastodon      │   wss://host/api/v1/       │   Streaming      │
│   Client        │       streaming             │   Server         │
│                 │                             │                  │
└─────────────────┘                             └──────────────────┘
         │                                               │
         │  subscribe: {stream: "user"}                  │
         │  subscribe: {stream: "public:local"}          │
         │  subscribe: {stream: "hashtag", tag: "rust"}  │
         │ ─────────────────────────────────────────────►│
         │                                               │
         │◄──────── update events ───────────────────────│
         │◄──────── notification events ─────────────────│
         │◄──────── delete events ───────────────────────│
```

### Available Streams

| Stream | Description | Use Case |
|--------|-------------|----------|
| `user` | Home timeline + notifications | Primary stream |
| `user:notification` | Notifications only | Focused notification handling |
| `public` | Federated timeline | Discover content |
| `public:local` | Local server timeline | Community focus |
| `public:remote` | Remote server posts | Federation monitoring |
| `hashtag` | Posts with specific tag | Topic tracking |
| `list` | Specific list updates | Curated feeds |
| `direct` | Direct messages | Private conversations |

### Event Types

```json
// Update event (new status)
{
  "event": "update",
  "payload": "{\"id\":\"123\",\"content\":\"<p>Hello</p>\",...}"
}

// Delete event
{
  "event": "delete",
  "payload": "123"
}

// Notification event
{
  "event": "notification",
  "payload": "{\"id\":\"456\",\"type\":\"favourite\",...}"
}

// Status edit event
{
  "event": "status.update",
  "payload": "{\"id\":\"123\",\"content\":\"<p>Updated</p>\",...}"
}
```

### Connection Management

**Discovery:**
Clients must first query the instance endpoint to find the streaming URL:
```http
GET /api/v2/instance
```
Response includes `configuration.urls.streaming` which may differ from the main API host.

**Authentication:**
Three methods supported (in order of preference):
1. `Authorization: Bearer <token>` header (recommended)
2. `Sec-Websocket-Protocol` header with token
3. `access_token` query parameter (not recommended - appears in logs)

**Health Check:**
```http
GET /api/v1/streaming/health
```
Returns `OK` when the streaming service is available.

**Heartbeats:**
Server sends periodic `:` comment lines to maintain connection.

### Subscription Protocol

**Subscribe to stream:**
```json
{"type": "subscribe", "stream": "user"}
{"type": "subscribe", "stream": "hashtag", "tag": "rust"}
{"type": "subscribe", "stream": "list", "list": "12345"}
```

**Unsubscribe from stream:**
```json
{"type": "unsubscribe", "stream": "user"}
```

## Request/Response Formats

### Request Format

**Headers (all requests):**
```http
Authorization: Bearer <access_token>
Content-Type: application/json (for POST/PUT)
Accept: application/json
```

**Parameter Submission:**
- GET requests: Query string parameters
- POST/PUT/PATCH/DELETE: Form data or JSON body
- Array parameters: Use bracket notation (`types[]=mention&types[]=follow`)

### Response Format

**Success Response:**
```json
{
  "id": "123456789",
  "created_at": "2024-01-15T12:00:00.000Z",
  "content": "<p>Post content here</p>",
  "account": {...},
  "media_attachments": [...],
  "mentions": [...],
  "tags": [...]
}
```

**Pagination Headers:**
```http
Link: <https://mastodon.social/api/v1/timelines/home?max_id=123>; rel="next",
      <https://mastodon.social/api/v1/timelines/home?min_id=456>; rel="prev"
```

**Error Response:**
```json
{
  "error": "The access token is invalid"
}
```

### Common Status Codes

| Code | Meaning | Typical Cause |
|------|---------|---------------|
| 200 | Success | Normal operation |
| 401 | Unauthorized | Invalid/expired token |
| 403 | Forbidden | Insufficient scope |
| 404 | Not Found | Invalid ID or deleted resource |
| 422 | Unprocessable | Invalid parameters |
| 429 | Rate Limited | Too many requests |

## Traffic Flow Diagrams

### Typical App Session

```
┌─────────┐     ┌──────────┐     ┌───────────┐
│  App    │     │  REST    │     │ Streaming │
│ Launch  │     │  API     │     │ Server    │
└────┬────┘     └────┬─────┘     └─────┬─────┘
     │               │                 │
     │──verify_creds─►                 │
     │◄──────────────│                 │
     │               │                 │
     │──get instance─►                 │
     │◄──────────────│                 │
     │               │                 │
     │──home timeline─►                │
     │◄──────────────│                 │
     │               │                 │
     │──notifications─►                │
     │◄──────────────│                 │
     │               │                 │
     │──────── WebSocket connect ─────►│
     │──────── subscribe: user ───────►│
     │                 │               │
     │◄────────── update events ───────│
     │◄────────── notification events ─│
     │               │                 │
```

### Post Creation Flow

```
Client                    Server
   │                         │
   │  POST /api/v1/statuses  │
   │  {                      │
   │    "status": "Hello!",  │
   │    "visibility": "public"│
   │  }                      │
   │ ───────────────────────►│
   │                         │
   │◄────────────────────────│
   │  {                      │
   │    "id": "123",         │
   │    "content": "<p>Hello!</p>",
   │    "created_at": "...", │
   │    ...                  │
   │  }                      │
```

## Implications for Proxy Design

### Caching Opportunities

1. **Instance metadata** - Cache aggressively (hours)
2. **Account data** - Cache with short TTL (minutes)
3. **Public timelines** - Shared cache possible
4. **Home timelines** - User-specific, no shared caching

### Connection Pooling

- Streaming connections should be multiplexed per user
- REST API calls can use connection pooling
- Consider WebSocket reconnection with exponential backoff

### Rate Limiting Considerations

- Respect upstream rate limits
- Implement client-side request coalescing
- Cache timeline responses to reduce upstream calls

## References

- [Mastodon Timelines API](https://docs.joinmastodon.org/methods/timelines/)
- [Mastodon Streaming API](https://docs.joinmastodon.org/methods/streaming/)
- [Mastodon Statuses API](https://docs.joinmastodon.org/methods/statuses/)
- [Mastodon Notifications API](https://docs.joinmastodon.org/methods/notifications/)
- [Mastodon Accounts API](https://docs.joinmastodon.org/methods/accounts/)
