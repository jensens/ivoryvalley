# Manual Testing Guide for Real Mastodon Clients

This guide documents how to test the IvoryValley proxy with real Mastodon clients to verify correct proxy behavior and deduplication functionality.

## Prerequisites

1. **A Mastodon account** - You'll need an account on a Mastodon instance (e.g., mastodon.social)
2. **IvoryValley proxy running** - The proxy must be accessible from your test devices
3. **Test client apps** - Install the clients you want to test

## Setting Up the Proxy

### Local Development

```bash
# Build and run the proxy
cargo build --release
./target/release/ivoryvalley \
    --upstream-url https://mastodon.social \
    --host 0.0.0.0 \
    --port 8080

# Or using environment variables
IVORYVALLEY_UPSTREAM_URL=https://mastodon.social \
IVORYVALLEY_PORT=8080 \
cargo run --release
```

### For Mobile Testing

To test with mobile clients, the proxy must be accessible from your device:

1. **Same network**: Use your computer's local IP (e.g., `http://192.168.1.100:8080`)
2. **Remote access**: Use a tunneling service like ngrok or expose via a reverse proxy with HTTPS

**Note**: Some clients require HTTPS. For testing, you may need to set up a local certificate or use a tunneling service that provides HTTPS.

## Test Cases

### 1. App Launch Sequence

**What to verify**: Client can successfully authenticate and load initial data.

**Steps**:
1. Configure the client to use the proxy URL as the server
2. Log in with your Mastodon credentials
3. Verify the home timeline loads
4. Check that notifications appear
5. Verify profile information is correct

**Expected behavior**:
- Authentication succeeds
- Home timeline displays posts
- Notifications are visible
- User avatar and profile name appear correctly

### 2. Timeline Deduplication

**What to verify**: Duplicate posts are filtered from timelines.

**Steps**:
1. Open the home timeline
2. Scroll through and note the visible posts
3. Force-refresh the timeline (pull-to-refresh)
4. Observe the returned posts

**Expected behavior**:
- Previously seen posts should not reappear on refresh
- Only genuinely new posts appear
- The timeline doesn't show duplicates

**Verification via logs**:
```bash
RUST_LOG=ivoryvalley=debug cargo run
```
Look for log entries like:
```
INFO Timeline filtering: 20 total, 15 filtered, 5 passed, 0 errors
```

### 3. Timeline Pagination

**What to verify**: Infinite scroll and refresh work correctly.

**Steps**:
1. Load the home timeline
2. Scroll down to trigger "load more" (pagination)
3. Continue scrolling to load additional pages
4. Pull-to-refresh to load new posts

**Expected behavior**:
- Older posts load when scrolling down
- No gaps in the timeline
- New posts appear at the top on refresh
- Pagination Link headers are preserved

### 4. Streaming (Real-time Updates)

**What to verify**: WebSocket streaming works and deduplicates events.

**Steps**:
1. Open the app and keep it in the foreground
2. Have another account (or use the web interface) post to your timeline
3. Observe if the new post appears in real-time

**Expected behavior**:
- New posts appear without manual refresh
- Duplicate streaming events are filtered
- Notifications arrive in real-time

### 5. User Actions

**What to verify**: Post creation, favorites, and boosts work correctly.

**Steps**:
1. Create a new post
2. Favorite a post
3. Boost (reblog) a post
4. View a thread (tap on a post to see context)

**Expected behavior**:
- Actions complete successfully
- UI reflects the action (heart fills in, boost icon changes)
- New posts appear in your profile

### 6. Boost Deduplication

**What to verify**: Seeing the original prevents boost duplicates.

**Steps**:
1. Note a specific post in your timeline
2. Have someone boost that same post
3. Refresh the timeline

**Expected behavior**:
- The boost should not appear (you've already seen the original content)
- Check logs for deduplication of reblog content

## Client-Specific Instructions

### Tusky (Android)

1. Install Tusky from F-Droid or Google Play
2. Tap "Log in" on the welcome screen
3. Enter your proxy URL (e.g., `http://192.168.1.100:8080`)
4. Follow OAuth flow (you'll be redirected to the real Mastodon server)
5. Grant permissions and return to Tusky

**Known considerations**:
- Tusky may require HTTPS for some features
- Check that push notifications still work (may need additional configuration)

### Ice Cubes (iOS)

1. Install Ice Cubes from the App Store
2. Tap "Add Account"
3. Enter your proxy URL as the instance
4. Complete the OAuth authentication
5. Return to the app

**Known considerations**:
- iOS enforces App Transport Security - may need HTTPS
- Test on a real device if simulator has networking issues

### Mastodon Web Interface

1. Configure your browser to use the proxy (or modify `/etc/hosts`)
2. Navigate to the proxy URL
3. The login page should appear
4. Log in with your credentials

**Alternative approach**:
Use browser developer tools to intercept and redirect API calls to the proxy:
```javascript
// Example for testing specific endpoints
fetch('http://localhost:8080/api/v1/timelines/home', {
  headers: { 'Authorization': 'Bearer YOUR_TOKEN' }
}).then(r => r.json()).then(console.log);
```

### Other Clients

The proxy should work with any Mastodon-compatible client:
- **Megalodon** (Android)
- **Mast** (iOS)
- **Whalebird** (Desktop)
- **Sengi** (Desktop)
- **Toot!** (iOS)

Follow similar steps: configure the instance URL to point to your proxy.

## Troubleshooting

### Connection Refused

- Verify the proxy is running: `curl http://localhost:8080/api/v1/instance`
- Check firewall rules allow the port
- For mobile: ensure device and computer are on the same network

### SSL/TLS Errors

- Use a tunneling service for HTTPS (ngrok, Cloudflare Tunnel)
- Or configure a local SSL certificate

### Authentication Fails

- Check that OAuth redirects work correctly
- Verify the upstream URL is correct
- Check proxy logs for error messages

### Posts Not Appearing

- Check if deduplication is overly aggressive
- Clear the seen URI database: `rm ivoryvalley.db`
- Run with debug logging to see filtering decisions

### WebSocket Issues

- Verify streaming URL is discovered correctly
- Check that WebSocket upgrade succeeds (look for 101 response)
- Test with: `websocat ws://localhost:8080/api/v1/streaming?access_token=YOUR_TOKEN`

## Monitoring and Logs

### Enable Debug Logging

```bash
RUST_LOG=ivoryvalley=debug cargo run
```

### Key Log Messages to Watch

```
# Successful proxy startup
INFO Starting IvoryValley proxy
INFO Upstream: https://mastodon.social
INFO Listening on: 0.0.0.0:8080

# Timeline filtering
INFO Timeline filtering: 20 total, 15 filtered, 5 passed, 0 errors

# WebSocket connections
INFO WebSocket upgrade request received
INFO Connected to upstream WebSocket
DEBUG Filtering duplicate status: https://...

# Errors to investigate
WARN Failed to connect to upstream
ERROR Failed to parse timeline response
```

## Test Verification Checklist

Use this checklist when testing with each client:

- [ ] Can log in via OAuth
- [ ] Home timeline loads
- [ ] Notifications appear
- [ ] Timeline refresh works
- [ ] Infinite scroll (load more) works
- [ ] Real-time streaming updates work
- [ ] Can post new statuses
- [ ] Can favorite posts
- [ ] Can boost posts
- [ ] Can view thread context
- [ ] Duplicate posts are filtered
- [ ] Boosted content is deduplicated
- [ ] No errors in proxy logs
- [ ] App doesn't crash or hang

## Reporting Issues

When reporting issues, include:

1. Client name and version
2. Proxy version and configuration
3. Upstream Mastodon instance
4. Steps to reproduce
5. Expected vs actual behavior
6. Relevant proxy logs (with `RUST_LOG=debug`)
