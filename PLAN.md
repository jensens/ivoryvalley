# ActivityPub Deduplication Proxy - Project Plan

## Problem Statement
Following users across Fediverse clients results in duplicate posts due to boosts/reposts, sometimes 10x the same content.

## Solution Concept
A transparent proxy between your Mastodon client and the upstream server that filters duplicates before delivery.

---

## High-Level Phases

### 1. Research & Discovery
- Study ActivityPub protocol specification (message format, IDs, how boosts work)
- Analyze how Mastodon clients authenticate and communicate (OAuth flow, API endpoints)
- Identify what makes a message "unique" vs a duplicate (content ID? object ID? content hash?)
- Survey existing client-server traffic to understand what needs proxying

### 2. Architecture Design
- Define how proxy intercepts requests (MITM-style vs explicit proxy config)
- Choose database for seen-message storage (SQLite? Redis? LevelDB?)
- Decide on deduplication strategy (ID-based, content-hash, or hybrid)
- Plan authentication passthrough mechanism

### 3. Core Implementation
- Build proxy server that speaks Mastodon API
- Implement message ID extraction and storage
- Implement filtering logic on timeline/feed endpoints
- Handle passthrough for all non-feed requests (posting, auth, etc.)

### 4. Testing & Validation
- Test with real clients (Tusky, Ice Cubes, web interface, etc.)
- Verify deduplication actually works
- Ensure no legitimate messages are dropped
- Performance testing with realistic feed sizes

### 5. Deployment & Usability
- Packaging (Docker, standalone binary, etc.)
- Configuration (upstream server, credentials, filter rules)
- Documentation for end users

---

## Key Technical Risks to Investigate Early
- OAuth token handling - can we pass through auth transparently?
- WebSocket/streaming endpoints - do clients use these?
- What exactly identifies a "duplicate" in ActivityPub semantics?

---

## Investigation Notes

### Finding 1: Deduplication Strategy (SOLVED)

The Mastodon Status entity has clear fields for deduplication:

| Field | Type | Purpose |
|-------|------|---------|
| `id` | string | Database ID (local to instance) |
| `uri` | string | **Federated URI - globally unique across all instances** |
| `reblog` | Status? | If present, contains the original boosted status |

**Deduplication approach:**
- Use `uri` as the unique identifier (it's globally unique across federation)
- For boosts: extract `reblog.uri` to find the original post
- If we've seen a URI before → filter it out
- Simple hash set or key-value store is sufficient

### Finding 2: OAuth Passthrough (LOW RISK)

Mastodon uses standard OAuth 2.0 with Bearer tokens:
- Token passed via `Authorization: Bearer <token>` header
- Proxy can pass this header through transparently to upstream
- No need to intercept or modify the auth flow
- Client authenticates directly with upstream, proxy just forwards

### Finding 3: API Endpoints to Proxy

**Timeline endpoints (need filtering):**
- `GET /api/v1/timelines/home` - home timeline
- `GET /api/v1/timelines/public` - federated/local timeline
- `GET /api/v1/timelines/list/:list_id` - list timelines
- `GET /api/v1/timelines/tag/:hashtag` - hashtag timelines

**Passthrough (no filtering needed):**
- All POST/PUT/DELETE requests (actions)
- `/api/v1/accounts/*` - account info
- `/oauth/*` - authentication
- Everything else

### Finding 4: WebSocket Streaming (MEDIUM RISK)

Streaming uses WebSocket at `/api/v1/streaming`:
- Token passed as query param: `?access_token=<token>&stream=user`
- Some instances use separate streaming domain (check `/api/v2/instance`)
- Events include `update`, `delete`, `notification`, etc.

**Challenge:** Need to proxy WebSocket connections and filter events in real-time.
**Mitigation:** Can start with REST-only support, add streaming later.

### Finding 5: Architecture Decision

**Recommended: Explicit Proxy Configuration**
- Client points to proxy URL instead of real server
- Proxy rewrites requests to upstream
- Simpler than MITM, no certificate issues
- User configures: `proxy_url` + `upstream_server`

---

## Refined Architecture

```
┌─────────────┐     ┌─────────────────┐     ┌──────────────┐
│   Client    │────▶│  Dedup Proxy    │────▶│   Mastodon   │
│  (Tusky,    │◀────│                 │◀────│   Instance   │
│   etc.)     │     │  - Filter dupes │     │              │
└─────────────┘     │  - Store URIs   │     └──────────────┘
                    │  - Pass auth    │
                    └─────────────────┘
                           │
                    ┌──────▼──────┐
                    │   SQLite    │
                    │  (seen URIs)│
                    └─────────────┘
```

---

## Sources

- [Mastodon Status Entity](https://docs.joinmastodon.org/entities/Status/)
- [Mastodon Timelines API](https://docs.joinmastodon.org/methods/timelines/)
- [Mastodon OAuth Documentation](https://docs.joinmastodon.org/spec/oauth/)
- [Mastodon Streaming API](https://docs.joinmastodon.org/methods/streaming/)
- [ActivityPub W3C Spec](https://www.w3.org/TR/activitypub/)
- [ActivityPub Announce Activity](https://www.w3.org/wiki/ActivityPub/Primer/Announce_activity)
