#!/usr/bin/env python3
"""
Anonymize recorded traffic for use as test fixtures.

This script reads JSONL traffic recordings and replaces sensitive data:
- Account names/usernames -> anonymous placeholders
- Instance URLs -> example.com variants
- Access tokens -> placeholder tokens
- Status IDs -> sequential anonymous IDs
- URIs -> anonymized URIs

Usage:
    python scripts/anonymize_traffic.py recordings/traffic.jsonl > tests/fixtures/timeline_traffic.jsonl
"""

import json
import re
import sys
import hashlib
from typing import Any

# Counters for generating sequential anonymous IDs
_id_counter = 0
_id_map: dict[str, str] = {}

_username_counter = 0
_username_map: dict[str, str] = {}

_instance_counter = 0
_instance_map: dict[str, str] = {}


def get_anonymous_id(original_id: str) -> str:
    """Generate a consistent anonymous ID for a given original ID."""
    global _id_counter
    if original_id not in _id_map:
        _id_counter += 1
        _id_map[original_id] = str(100000 + _id_counter)
    return _id_map[original_id]


def get_anonymous_username(original: str) -> str:
    """Generate a consistent anonymous username."""
    global _username_counter
    if original not in _username_map:
        _username_counter += 1
        _username_map[original] = f"user{_username_counter}"
    return _username_map[original]


def get_anonymous_instance(original: str) -> str:
    """Generate a consistent anonymous instance name."""
    global _instance_counter
    if original not in _instance_map:
        _instance_counter += 1
        _instance_map[original] = f"instance{_instance_counter}.example"
    return _instance_map[original]


def anonymize_instance_url(url: str) -> str:
    """Replace instance URLs with anonymous versions."""
    # Match common Mastodon instance URL patterns
    pattern = r'https?://([a-zA-Z0-9][-a-zA-Z0-9]*\.)+[a-zA-Z]{2,}'

    def replace_match(m: re.Match) -> str:
        instance = m.group(0)
        # Extract just the domain
        domain = re.sub(r'^https?://', '', instance)
        anon_domain = get_anonymous_instance(domain)
        return f"https://{anon_domain}"

    return re.sub(pattern, replace_match, url)


def anonymize_headers(headers: dict[str, str]) -> dict[str, str]:
    """Anonymize sensitive headers."""
    result = {}
    for key, value in headers.items():
        key_lower = key.lower()
        if key_lower == "authorization":
            # Replace Bearer token
            if value.startswith("Bearer "):
                result[key] = "Bearer anonymous_token_xxx"
            else:
                result[key] = "anonymous_auth"
        elif key_lower == "cookie":
            result[key] = "_mastodon_session=anonymous_session"
        elif key_lower == "set-cookie":
            # Preserve cookie structure but anonymize values
            result[key] = re.sub(r'=([^;]+)', '=anonymous', value)
        elif key_lower in ("host", "origin", "referer"):
            result[key] = anonymize_instance_url(value)
        else:
            result[key] = value
    return result


def anonymize_json_value(value: Any, depth: int = 0) -> Any:
    """Recursively anonymize JSON values."""
    if depth > 50:  # Prevent infinite recursion
        return value

    if isinstance(value, dict):
        return anonymize_json_object(value, depth + 1)
    elif isinstance(value, list):
        return [anonymize_json_value(item, depth + 1) for item in value]
    elif isinstance(value, str):
        return anonymize_string_value(value)
    else:
        return value


def anonymize_string_value(value: str) -> str:
    """Anonymize string values that might contain sensitive data."""
    # Don't anonymize short strings that are likely enum values
    if len(value) < 5:
        return value

    # Anonymize URLs
    if value.startswith("http://") or value.startswith("https://"):
        return anonymize_instance_url(value)

    # Anonymize email-like patterns
    if "@" in value and "." in value:
        parts = value.split("@")
        if len(parts) == 2:
            return f"{get_anonymous_username(parts[0])}@{get_anonymous_instance(parts[1])}"

    return value


def anonymize_json_object(obj: dict, depth: int = 0) -> dict:
    """Anonymize a JSON object representing a Mastodon API response."""
    result = {}

    for key, value in obj.items():
        # Handle specific Mastodon API fields
        if key == "id" and isinstance(value, str):
            result[key] = get_anonymous_id(value)
        elif key == "uri" and isinstance(value, str):
            result[key] = anonymize_instance_url(value)
        elif key == "url" and isinstance(value, str):
            result[key] = anonymize_instance_url(value)
        elif key == "username" and isinstance(value, str):
            result[key] = get_anonymous_username(value)
        elif key == "acct" and isinstance(value, str):
            # Handle both local (username) and remote (username@instance) formats
            if "@" in value:
                parts = value.split("@")
                result[key] = f"{get_anonymous_username(parts[0])}@{get_anonymous_instance(parts[1])}"
            else:
                result[key] = get_anonymous_username(value)
        elif key == "display_name" and isinstance(value, str):
            result[key] = f"Anonymous User {get_anonymous_username(value)}"
        elif key == "email" and isinstance(value, str):
            result[key] = f"{get_anonymous_username(value.split('@')[0])}@example.com"
        elif key == "note" and isinstance(value, str):
            # User bio - replace with placeholder
            result[key] = "<p>This is an anonymized user bio.</p>"
        elif key == "content" and isinstance(value, str):
            # Post content - keep HTML structure but anonymize mentions/links
            result[key] = anonymize_html_content(value)
        elif key == "avatar" or key == "avatar_static":
            result[key] = "https://example.com/avatars/original/missing.png"
        elif key == "header" or key == "header_static":
            result[key] = "https://example.com/headers/original/missing.png"
        elif key == "access_token" and isinstance(value, str):
            result[key] = "anonymous_access_token_xxx"
        elif key == "token" and isinstance(value, str):
            result[key] = "anonymous_token_xxx"
        elif key in ("created_at", "updated_at", "edited_at", "last_status_at"):
            # Keep timestamps but normalize them
            result[key] = value  # Keep as-is for now
        elif key == "account":
            result[key] = anonymize_json_value(value, depth)
        elif key == "reblog":
            result[key] = anonymize_json_value(value, depth)
        elif key == "media_attachments" and isinstance(value, list):
            result[key] = [anonymize_media_attachment(m) for m in value]
        else:
            result[key] = anonymize_json_value(value, depth)

    return result


def anonymize_media_attachment(attachment: dict) -> dict:
    """Anonymize a media attachment object."""
    result = dict(attachment)
    if "url" in result:
        result["url"] = "https://example.com/media/original/anonymized.jpg"
    if "preview_url" in result:
        result["preview_url"] = "https://example.com/media/small/anonymized.jpg"
    if "remote_url" in result:
        result["remote_url"] = "https://example.com/media/original/anonymized.jpg"
    if "id" in result:
        result["id"] = get_anonymous_id(str(result["id"]))
    return result


def anonymize_html_content(html: str) -> str:
    """Anonymize HTML content while preserving structure."""
    # Replace @mentions
    html = re.sub(
        r'@<span>([^<]+)</span>',
        lambda m: f'@<span>{get_anonymous_username(m.group(1))}</span>',
        html
    )
    # Replace href links
    html = re.sub(
        r'href="([^"]+)"',
        lambda m: f'href="{anonymize_instance_url(m.group(1))}"',
        html
    )
    return html


def anonymize_exchange(exchange: dict) -> dict:
    """Anonymize a complete request/response exchange."""
    result = {
        "timestamp": exchange.get("timestamp", "2025-01-01T00:00:00Z"),
    }

    # Anonymize request
    if "request" in exchange:
        req = exchange["request"]
        result["request"] = {
            "method": req.get("method", "GET"),
            "path": req.get("path", "/"),
            "headers": anonymize_headers(req.get("headers", {})),
        }
        if "body" in req and req["body"]:
            try:
                body_json = json.loads(req["body"])
                result["request"]["body"] = json.dumps(anonymize_json_value(body_json))
            except json.JSONDecodeError:
                result["request"]["body"] = req["body"]

    # Anonymize response
    if "response" in exchange:
        resp = exchange["response"]
        result["response"] = {
            "status": resp.get("status", 200),
            "headers": anonymize_headers(resp.get("headers", {})),
        }
        if "body" in resp:
            try:
                body_json = json.loads(resp["body"])
                result["response"]["body"] = json.dumps(anonymize_json_value(body_json))
            except json.JSONDecodeError:
                result["response"]["body"] = resp["body"]

    return result


def main():
    if len(sys.argv) < 2:
        print("Usage: python anonymize_traffic.py <input.jsonl> [output.jsonl]", file=sys.stderr)
        print("\nReads recorded traffic and outputs anonymized version.", file=sys.stderr)
        sys.exit(1)

    input_file = sys.argv[1]
    output_file = sys.argv[2] if len(sys.argv) > 2 else None

    output = open(output_file, 'w') if output_file else sys.stdout

    try:
        with open(input_file, 'r') as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    exchange = json.loads(line)
                    anonymized = anonymize_exchange(exchange)
                    print(json.dumps(anonymized), file=output)
                except json.JSONDecodeError as e:
                    print(f"Warning: Skipping invalid JSON line: {e}", file=sys.stderr)
    finally:
        if output_file:
            output.close()

    print(f"\nAnonymization complete!", file=sys.stderr)
    print(f"  - Anonymized {_id_counter} IDs", file=sys.stderr)
    print(f"  - Anonymized {_username_counter} usernames", file=sys.stderr)
    print(f"  - Anonymized {_instance_counter} instances", file=sys.stderr)


if __name__ == "__main__":
    main()
