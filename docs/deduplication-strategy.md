# Deduplication Strategy

This document defines the complete deduplication strategy for IvoryValley, building on the foundations established in [message-uniqueness.md](./message-uniqueness.md).

## Goals

1. **Remove duplicate content** - User sees each unique post only once
2. **Preserve timeline semantics** - Boosts are shown as boosts, not filtered
3. **Handle edits correctly** - Updated content is shown
4. **Minimize storage** - Only store what's necessary for dedup decisions
5. **Low latency** - Dedup check must be fast (< 1ms)

## Deduplication Scope

### What Gets Deduplicated

| Content Type | Dedup? | Reason |
|--------------|--------|--------|
| Original posts | Yes | Core use case |
| Boosts of same post | Configurable | User may want to see boost activity |
| Replies | Yes | Same as original posts |
| Notifications | No | Different semantics (mentions, follows, etc.) |
| Direct messages | No | Private, always show |

### Where Deduplication Happens

Per [proxy-interception-strategy.md](./proxy-interception-strategy.md):

1. **REST API responses** - Timeline endpoints (`/api/v1/timelines/*`)
2. **Streaming events** - `update` events on WebSocket

## Filtering Decision Logic

### Core Algorithm

```
function should_filter(status, seen_store, config):
    // Extract the content URI (original post, not boost wrapper)
    content_uri = get_content_uri(status)

    // Check if we've seen this content before
    seen_record = seen_store.get(content_uri)

    if seen_record is null:
        // First time seeing this content
        seen_store.set(content_uri, {
            first_seen: now(),
            last_seen: now(),
            edited_at: status.edited_at,
            boost_count: is_boost(status) ? 1 : 0
        })
        return PASS_THROUGH

    // We've seen this before - check for updates
    if status.edited_at > seen_record.edited_at:
        // Content was edited, show the update
        seen_store.update(content_uri, {
            last_seen: now(),
            edited_at: status.edited_at
        })
        return PASS_THROUGH

    // Check boost handling policy
    if is_boost(status):
        if config.show_boost_activity:
            seen_store.increment_boost_count(content_uri)
            return PASS_THROUGH
        else:
            return FILTER_OUT

    // Duplicate original post
    return FILTER_OUT
```

### URI Extraction

From [message-uniqueness.md](./message-uniqueness.md):

```
function get_content_uri(status):
    if status.reblog exists and status.reblog is not null:
        // This is a boost - use the original content's URI
        return status.reblog.uri
    else:
        // Original post or reply
        return status.uri
```

### Boost Detection

```
function is_boost(status):
    return status.reblog exists and status.reblog is not null
```

## Storage Interface

### Required Operations

The seen-message store must support these operations:

| Operation | Description | Performance Target |
|-----------|-------------|-------------------|
| `get(uri)` | Retrieve record by URI | < 1ms |
| `set(uri, record)` | Store new record | < 5ms |
| `update(uri, fields)` | Update existing record | < 5ms |
| `exists(uri)` | Check if URI exists | < 0.5ms |
| `delete(uri)` | Remove record | < 5ms |
| `cleanup(older_than)` | Remove old records | Background |

### Record Schema

```
SeenRecord {
    uri: String           // Primary key - the content URI
    first_seen: Timestamp // When we first saw this content
    last_seen: Timestamp  // Most recent encounter
    edited_at: Timestamp? // Last known edit timestamp (nullable)
    boost_count: Integer  // Number of boosts seen
}
```

### Storage Size Estimation

| Metric | Estimate |
|--------|----------|
| Average URI length | ~80 bytes |
| Record overhead | ~40 bytes |
| Total per record | ~120 bytes |
| Active posts (30 days) | ~10,000 |
| **Storage needed** | **~1.2 MB** |

This is small enough for in-memory storage with persistence.

## Edge Cases

### 1. Edited Posts

Mastodon supports editing posts. The `edited_at` field indicates the last edit time.

**Strategy:** If `edited_at` is newer than stored, pass through and update record.

```
Timeline:
1. User sees post (edited_at: null)     → stored
2. Author edits post (edited_at: T1)    → pass through, update stored
3. Post appears again (edited_at: T1)   → filter (same edit)
4. Author edits again (edited_at: T2)   → pass through, update stored
```

### 2. Deleted Posts

Mastodon sends `delete` events for removed posts.

**Strategy:** Remove from seen store so re-posted content (if un-deleted) can appear again.

```
function handle_delete_event(status_id):
    // Note: delete events only have the status ID, not full URI
    // May need to maintain id → uri mapping or ignore
    seen_store.delete_by_id(status_id)
```

### 3. Boosts of Boosts

A user can boost a boosted post. The API unwraps this:

```json
{
  "id": "boost-of-boost-id",
  "reblog": {
    "id": "original-id",
    "uri": "https://original.server/statuses/123",
    "reblog": null  // Always null - fully unwrapped
  }
}
```

**Strategy:** No special handling needed. `reblog` always points to original content.

### 4. Self-Boosts

Users can boost their own posts.

**Strategy:** Treat like any other boost. If `show_boost_activity` is enabled, show it.

### 5. Cross-Timeline Duplicates

Same post may appear in home timeline and hashtag timeline.

**Strategy:** Dedup is per-user, not per-timeline. Once seen, filtered everywhere.

### 6. Pagination Edge Cases

When loading older posts (`max_id` pagination), user may encounter previously-seen content.

**Strategy:** Still filter. User already saw it when it was new.

## Configuration Options

```yaml
deduplication:
  enabled: true

  # How to handle boosts of already-seen content
  # "show_first" - Only show first boost
  # "show_all" - Show all boosts (just dedupe content)
  # "filter_all" - Filter all duplicate boosts
  boost_policy: "show_first"

  # How long to remember seen URIs
  retention_days: 30

  # Cleanup interval
  cleanup_interval_hours: 24
```

## Metrics to Track

| Metric | Description |
|--------|-------------|
| `posts_seen_total` | Total posts processed |
| `posts_filtered_total` | Posts filtered as duplicates |
| `posts_passed_total` | Posts passed through |
| `edits_detected_total` | Edited posts shown |
| `boosts_seen_total` | Boost events processed |
| `storage_records_count` | Current records in store |
| `dedup_latency_ms` | Time for dedup decision |

## Integration Points

### REST API Response Processing

```
function process_timeline_response(response):
    statuses = parse_json(response.body)
    filtered_statuses = []

    for status in statuses:
        decision = should_filter(status, seen_store, config)
        if decision == PASS_THROUGH:
            filtered_statuses.append(status)
        else:
            metrics.increment('posts_filtered_total')

    metrics.add('posts_passed_total', len(filtered_statuses))

    // Return modified response with filtered statuses
    return create_response(filtered_statuses, response.headers)
```

### Streaming Event Processing

```
function process_streaming_event(event):
    if event.type == 'update':
        status = parse_json(event.payload)
        decision = should_filter(status, seen_store, config)
        if decision == FILTER_OUT:
            return null  // Don't forward to client

    if event.type == 'delete':
        handle_delete_event(event.payload)

    return event  // Forward unchanged
```

## Related Documents

- [Message Uniqueness](./message-uniqueness.md) - URI-based identification
- [ActivityPub Protocol](./activitypub-protocol.md) - Underlying protocol
- [Proxy Interception Strategy](./proxy-interception-strategy.md) - Where dedup runs

## References

- [Mastodon Status Entity](https://docs.joinmastodon.org/entities/Status/)
- [Mastodon Streaming API](https://docs.joinmastodon.org/methods/streaming/)
