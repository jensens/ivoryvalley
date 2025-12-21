# Message Uniqueness Criteria

This document defines how to identify unique messages and handle deduplication
in the context of ActivityPub/Mastodon.

## Primary Identifier

| API | Field | Description |
|-----|-------|-------------|
| ActivityPub | `id` | Unique HTTPS URI identifying the object |
| Mastodon Client API | `uri` | Same as ActivityPub `id` - federation URI |

### URI Format Examples

```
https://mastodon.social/users/alice/statuses/123456789
https://fosstodon.org/users/bob/statuses/987654321
```

The URI is:
- Globally unique across the Fediverse
- Stable and permanent for the lifetime of the status
- Owned by the originating server

## Boost/Reblog Detection

### ActivityPub: Announce Activity

In ActivityPub, a boost is an `Announce` activity:

```json
{
  "type": "Announce",
  "id": "https://example.com/announces/789",
  "actor": "https://example.com/users/bob",
  "object": "https://remote.example/notes/123"
}
```

- The `Announce` has its own unique `id`
- The `object` field contains/references the original content
- **Not idempotent**: Same actor can boost the same object multiple times

### Mastodon Client API: reblog Field

In the Mastodon Client API, boosts are represented differently:

```json
{
  "id": "110123456789",
  "uri": "https://example.com/users/bob/statuses/110123456789",
  "reblog": {
    "id": "109876543210",
    "uri": "https://remote.example/users/alice/statuses/109876543210",
    "content": "Original post content..."
  }
}
```

- The outer status is the boost activity
- `reblog` field contains the complete original status
- If `reblog` is `null`, this is an original post

## Deduplication Strategy

### Algorithm

```
function getOriginalUri(status):
    if status.reblog exists:
        return status.reblog.uri
    else:
        return status.uri
```

### Implementation Notes

1. **Store original content once**: Use `uri` as the primary key for content storage
2. **Track boost activities separately**: Each boost has its own `id`/`uri`
3. **Timeline display**: Show boosts as distinct events, but reference shared content

### Example Scenario

User's timeline receives:
1. Original post from Alice: `uri = "https://a.example/statuses/100"`
2. Bob boosts Alice's post: `uri = "https://b.example/statuses/200"`, `reblog.uri = "https://a.example/statuses/100"`
3. Carol boosts Alice's post: `uri = "https://c.example/statuses/300"`, `reblog.uri = "https://a.example/statuses/100"`

Deduplication result:
- Content storage: 1 entry with key `"https://a.example/statuses/100"`
- Timeline: 3 events (original + 2 boosts), all referencing the same content

## Field Reference

### Mastodon Status Entity (Key Fields)

| Field | Type | Description |
|-------|------|-------------|
| `id` | String | Database ID (local to server) |
| `uri` | String | Federation URI (globally unique) |
| `url` | String? | Human-readable HTML URL |
| `reblog` | Status? | Original status if this is a boost |
| `created_at` | Datetime | Creation timestamp |
| `edited_at` | Datetime? | Last edit timestamp |
| `content` | String | HTML-encoded message content |

### Which ID to Use?

| Use Case | Field |
|----------|-------|
| Deduplication | `uri` (globally unique) |
| API calls to same server | `id` (database ID) |
| Display link | `url` (human-readable) |
| Checking for boost | `reblog` (null or Status) |

## References

- [Mastodon Status Entity](https://docs.joinmastodon.org/entities/Status/)
- [ActivityPub Specification](https://www.w3.org/TR/activitypub/)
- [Project: ActivityPub Protocol Study](./activitypub-protocol.md)
