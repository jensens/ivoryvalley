#!/usr/bin/env python3
"""
WebSocket streaming test client for IvoryValley.

Tests WebSocket streaming and shows deduplication in action.
Connects to the proxy and displays incoming statuses with their URIs.

Usage:
    ./scripts/ws-test-client.py [--token TOKEN] [--stream public|user]

You can get a token by logging in via web browser and copying from dev tools.
"""

import asyncio
import argparse
import json
import ssl
import sys

try:
    import websockets
except ImportError:
    print("Error: websockets library required. Install with: pip install websockets")
    sys.exit(1)


async def stream_timeline(host: str, port: int, token: str | None, stream: str, use_ssl: bool):
    """Connect to WebSocket streaming API and display events."""

    protocol = "wss" if use_ssl else "ws"

    # Build URL with query params
    url = f"{protocol}://{host}:{port}/api/v1/streaming?stream={stream}"
    if token:
        url += f"&access_token={token}"

    print(f"Connecting to: {url}")
    print("-" * 60)

    # SSL context for self-signed certs
    ssl_ctx = None
    if use_ssl:
        ssl_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        ssl_ctx.check_hostname = False
        ssl_ctx.verify_mode = ssl.CERT_NONE

    seen_uris = set()
    total_received = 0
    duplicates_filtered = 0

    try:
        async with websockets.connect(url, ssl=ssl_ctx) as ws:
            print(f"Connected! Listening for {stream} stream events...")
            print("(Press Ctrl+C to stop)\n")

            async for message in ws:
                try:
                    data = json.loads(message)
                    event_type = data.get("event", "unknown")

                    if event_type == "update":
                        total_received += 1
                        payload = json.loads(data.get("payload", "{}"))

                        uri = payload.get("uri", "no-uri")
                        account = payload.get("account", {}).get("acct", "unknown")
                        content = payload.get("content", "")[:80]
                        reblog = payload.get("reblog")

                        # Check if this is a duplicate (for our tracking)
                        is_dup = uri in seen_uris
                        seen_uris.add(uri)

                        if is_dup:
                            duplicates_filtered += 1
                            print(f"[DUP #{duplicates_filtered}] Would have been duplicate: {uri[:50]}...")
                        else:
                            if reblog:
                                orig_uri = reblog.get("uri", "?")
                                orig_account = reblog.get("account", {}).get("acct", "?")
                                print(f"[BOOST] @{account} boosted @{orig_account}")
                                print(f"        URI: {orig_uri[:60]}")
                            else:
                                print(f"[POST]  @{account}")
                                print(f"        URI: {uri[:60]}")
                            print()

                    elif event_type == "notification":
                        print(f"[NOTIF] {data.get('payload', '')[:60]}...")

                    elif event_type == "delete":
                        print(f"[DEL]   Status deleted: {data.get('payload', '')}")

                    else:
                        print(f"[{event_type.upper()}] {str(data)[:60]}...")

                except json.JSONDecodeError:
                    print(f"[RAW] {message[:100]}...")

    except websockets.exceptions.ConnectionClosed as e:
        print(f"\nConnection closed: {e}")
    except ConnectionRefusedError:
        print(f"\nError: Connection refused. Is the proxy running?")
    except Exception as e:
        print(f"\nError: {e}")
    finally:
        print("\n" + "=" * 60)
        print(f"Session stats:")
        print(f"  Total events received: {total_received}")
        print(f"  Unique URIs seen: {len(seen_uris)}")
        print(f"  Duplicates (client-side check): {duplicates_filtered}")
        print("=" * 60)


def main():
    parser = argparse.ArgumentParser(description="WebSocket streaming test client")
    parser.add_argument("--host", default="localhost", help="Proxy host (default: localhost)")
    parser.add_argument("--port", type=int, default=8080, help="Proxy port (default: 8080)")
    parser.add_argument("--token", help="Access token (required for user stream)")
    parser.add_argument("--stream", default="public", choices=["public", "user", "public:local"],
                        help="Stream type (default: public)")
    parser.add_argument("--ssl", action="store_true", help="Use SSL/TLS (wss://)")

    args = parser.parse_args()

    if args.stream == "user" and not args.token:
        print("Error: --token is required for user stream")
        print("Get a token by logging in via browser and checking dev tools.")
        sys.exit(1)

    try:
        asyncio.run(stream_timeline(
            host=args.host,
            port=args.port,
            token=args.token,
            stream=args.stream,
            use_ssl=args.ssl
        ))
    except KeyboardInterrupt:
        print("\nStopped.")


if __name__ == "__main__":
    main()
