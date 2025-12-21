# Database Choice for Seen-Message Storage

This document evaluates database options for tracking seen message URIs and makes a recommendation.

## Requirements

Based on [Proxy Interception Strategy](./proxy-interception-strategy.md) and [Message Uniqueness](./message-uniqueness.md):

| Requirement | Description |
|-------------|-------------|
| **Primary Function** | Store seen URIs for deduplication |
| **Data Model** | Key-value: URI string → seen timestamp |
| **Operations** | Fast lookups, inserts |
| **Concurrency** | Support for async I/O, up to 100 concurrent connections |
| **Persistence** | Required (survive restarts) |
| **Optional** | TTL-based expiry for caching |

### Data Characteristics

- **Key**: URI string (~50-100 characters)
- **Value**: Timestamp or boolean
- **Volume**: Thousands to tens of thousands of entries per user
- **Access Pattern**: Write-once, read-many lookups

## Options Evaluated

### SQLite

**Embedded relational database**

| Aspect | Assessment |
|--------|------------|
| Persistence | Built-in, ACID compliant |
| Performance | Good for reads, WAL mode enables concurrent reads during writes |
| Concurrency | Single writer, but sufficient for personal proxy |
| Simplicity | No external dependencies, single file |
| Tooling | Excellent (CLI, GUI tools, bindings for all languages) |
| TTL Support | Manual (via timestamp column + cleanup query) |

```
// Schema example
CREATE TABLE seen_uris (
    uri TEXT PRIMARY KEY,
    first_seen INTEGER NOT NULL  // Unix timestamp
);

CREATE INDEX idx_seen_uris_timestamp ON seen_uris(first_seen);
```

### Redis

**In-memory data store**

| Aspect | Assessment |
|--------|------------|
| Persistence | Optional (RDB/AOF), risk of data loss |
| Performance | Excellent (in-memory) |
| Concurrency | Excellent (single-threaded but non-blocking) |
| Simplicity | Requires external server process |
| Tooling | Good CLI, but separate deployment |
| TTL Support | Built-in per-key expiry |

```
// Usage example
SET "seen:https://mastodon.social/..." 1 EX 604800  // 7 days TTL
EXISTS "seen:https://mastodon.social/..."
```

### LevelDB

**Embedded key-value store**

| Aspect | Assessment |
|--------|------------|
| Persistence | Built-in, log-structured |
| Performance | Excellent for sequential writes, good reads |
| Concurrency | Single process only |
| Simplicity | Embedded, but less common |
| Tooling | Limited compared to SQLite |
| TTL Support | Manual implementation required |

```
// Usage example
db.put("https://mastodon.social/...", timestamp)
db.get("https://mastodon.social/...")
```

## Comparison Matrix

| Criterion | SQLite | Redis | LevelDB |
|-----------|--------|-------|---------|
| **No external server** | Yes | No | Yes |
| **Persistence default** | Yes | No | Yes |
| **Concurrent reads** | Yes (WAL) | Yes | Limited |
| **Built-in TTL** | No | Yes | No |
| **Language support** | Excellent | Good | Moderate |
| **Debugging tools** | Excellent | Good | Limited |
| **Query flexibility** | SQL | Commands | Key-value only |

## Decision: SQLite

**Recommended:** SQLite with WAL mode

### Rationale

1. **Simplicity**: No external services to deploy or manage
2. **Persistence**: ACID-compliant, data survives restarts
3. **Sufficient Performance**: WAL mode handles concurrent reads while writing
4. **Excellent Tooling**: Easy debugging with standard SQL tools
5. **Future Flexibility**: SQL allows complex queries if needed later
6. **Single-User Proxy**: Write contention is minimal for personal use

### Configuration

```yaml
# Recommended SQLite configuration
database:
  type: sqlite
  path: ./data/ivoryvalley.db
  wal_mode: true
  busy_timeout: 5000  # ms
```

### Schema

```sql
-- Seen URIs for deduplication
CREATE TABLE IF NOT EXISTS seen_uris (
    uri TEXT PRIMARY KEY,
    first_seen INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_first_seen ON seen_uris(first_seen);

-- Optional: cleanup old entries
-- DELETE FROM seen_uris WHERE first_seen < (strftime('%s', 'now') - 604800);
```

### Trade-offs Accepted

- **Manual TTL**: Periodic cleanup via scheduled task instead of automatic expiry
- **Write serialization**: Single writer, but acceptable for personal proxy workload

### When to Reconsider

Consider Redis if:
- Multiple proxy instances need shared state
- Very high write throughput required
- Built-in TTL becomes critical

## Related Documents

- [Proxy Interception Strategy](./proxy-interception-strategy.md)
- [Message Uniqueness](./message-uniqueness.md)

## References

- [SQLite WAL Mode](https://www.sqlite.org/wal.html)
- [SQLite in Multi-Threaded Applications](https://www.sqlite.org/threadsafe.html)
