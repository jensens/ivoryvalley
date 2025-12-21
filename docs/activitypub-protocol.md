# ActivityPub Protocol Study

This document summarizes the key findings from studying the ActivityPub protocol specification,
with a focus on message format, IDs, boosts/reposts (Announce activities), and deduplication strategies.

## 1. Message Format and Structure

ActivityPub uses **JSON-LD** format with the **Activity Streams 2.0** vocabulary.

### Content Type
All communications require:
```
Content-Type: application/ld+json; profile="https://www.w3.org/ns/activitystreams"
```

### Core Concept
Messages follow the pattern: **"some activity by some actor being taken on some object"**

### Basic Structure
```json
{
  "@context": "https://www.w3.org/ns/activitystreams",
  "type": "Create",
  "id": "https://example.com/activities/123",
  "actor": "https://example.com/users/alice",
  "object": {
    "type": "Note",
    "id": "https://example.com/notes/456",
    "content": "Hello World!"
  }
}
```

### Inbox/Outbox Model
- **Outbox**: Where actors publish messages (POST for publishing, GET for retrieving)
- **Inbox**: Where actors receive messages (POST for federation, GET for reading)

Clients POST to outboxes; servers POST to remote inboxes for federation.

## 2. How IDs Work (Local vs Federated)

### ID Requirements
- Must be **unique global identifiers**
- Must be **publicly dereferenceable URIs** (preferably HTTPS)
- Must belong to the **originating server**
- Can be `null` for anonymous/transient objects

### ID Formats (Mastodon Examples)
| Object Type | URI Pattern |
|-------------|-------------|
| Actor/User | `https://mastodon.example/users/alice` |
| Status | `https://mastodon.example/users/alice/statuses/1009947848598745` |
| Activity | `https://mastodon.example/bd06bb61-01e0-447a-9dc8-95915db9aec8` |
| Public Key | `https://mastodon.example/users/alice#main-key` |
| Collection | `https://mastodon.example/@alice/collections/featured` |

### Local vs Federated
- **Local IDs**: URIs on the same server (e.g., `https://myserver.example/...`)
- **Federated IDs**: URIs from remote servers (e.g., `https://mastodon.social/...`)
- IDs enable **origin validation** to prevent impersonation
- Fragment identifiers (`#create`, `#main-key`) are valid and dereferenceable

### WebFinger Discovery
Username mentions (`@alice@example.com`) must be translated to HTTPS URIs via WebFinger before federation:
```
@alice@example.com -> https://example.com/users/alice
```

## 3. Boosts/Reposts (Announce Activities)

### Purpose
The `Announce` activity shares or reposts objects across the network.

### Key Characteristics
- **Not idempotent**: Same object can be announced multiple times by same actor
- Can announce: Notes, Articles, Images, Videos, Audio, and even other activities

### Announce Structure

**By Reference** (recommended for remote objects):
```json
{
  "@context": "https://www.w3.org/ns/activitystreams",
  "type": "Announce",
  "id": "https://example.com/announces/789",
  "actor": "https://example.com/users/bob",
  "object": "https://remote.example/notes/123",
  "published": "2024-01-15T10:30:00Z"
}
```

**By Embedding** (recommended for same-server objects):
```json
{
  "@context": "https://www.w3.org/ns/activitystreams",
  "type": "Announce",
  "id": "https://example.com/announces/789",
  "actor": "https://example.com/users/bob",
  "object": {
    "type": "Note",
    "id": "https://example.com/notes/123",
    "attributedTo": "https://example.com/users/alice",
    "content": "Original post content",
    "published": "2024-01-15T09:00:00Z"
  }
}
```

### Tracking Announcements
- Objects have a `shares` collection containing all Announce activities
- Managed by the server responsible for the original object
- Updated when receiving Announce activities

## 4. Fields Available for Deduplication

### Primary Identifier
| Field | Description | Use Case |
|-------|-------------|----------|
| `id` | Unique global identifier (HTTPS URI) | **Primary deduplication key** |

### Secondary Fields
| Field | Description | Use Case |
|-------|-------------|----------|
| `url` | Link to external representation | Alternative access point |
| `published` | ISO 8601 creation timestamp | Ordering, freshness |
| `updated` | ISO 8601 modification timestamp | Version tracking, edits |
| `attributedTo` | Content author identifier | Author-based grouping |

### Activity-Specific Fields
| Field | Description | Use Case |
|-------|-------------|----------|
| `actor` | Who performed the activity | Tracking who boosted/liked |
| `object` | Target of the activity | Original content reference |

### Deduplication Strategies

1. **Activity Deduplication**: Use `id` of the Activity
   - Each Announce/Like/etc. has a unique `id`
   - Prevents processing same federation event twice

2. **Object Deduplication**: Use `id` of the Object
   - Same status has same `id` regardless of how it arrives
   - Boosts reference same object `id`

3. **Boost Deduplication**: Combine `actor` + `object`
   - Track which actor announced which object
   - Note: ActivityPub allows multiple announces by same actor

4. **Inbox Batching**: Group activities by target
   - WordPress ActivityPub plugin batches and deduplicates at inbox level
   - Reduces redundant processing during high activity

### Example: Deduplicating Boosts of Same Post

When multiple servers send you the same boosted post:
```
Announce from server A: { id: "A/1", object: "https://original/post/123" }
Announce from server B: { id: "B/2", object: "https://original/post/123" }
```

- Both Announces are **different activities** (different `id`)
- Both reference the **same object** (`https://original/post/123`)
- Store the object once, but track both Announce activities

## References

- [ActivityPub W3C Specification](https://www.w3.org/TR/activitypub/)
- [Activity Streams 2.0 Core](https://www.w3.org/TR/activitystreams-core/)
- [ActivityPub Announce Activity Primer](https://www.w3.org/wiki/ActivityPub/Primer/Announce_activity)
- [Mastodon ActivityPub Documentation](https://docs.joinmastodon.org/spec/activitypub/)
