#!/usr/bin/env python3
"""
Simple HTTPS reverse proxy for local development.
Forwards https://localhost:8443 -> http://localhost:8080
"""

import http.server
import http.client
import ssl
import sys
import os

LISTEN_PORT = 443
UPSTREAM_HOST = "localhost"
UPSTREAM_PORT = 8080


class ProxyHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_proxy(self):
        # Read request body if present
        content_length = self.headers.get('Content-Length')
        body = None
        if content_length:
            body = self.rfile.read(int(content_length))

        # Connect to upstream
        conn = http.client.HTTPConnection(UPSTREAM_HOST, UPSTREAM_PORT)

        # Build headers (pass through all except hop-by-hop)
        headers = {}
        hop_by_hop = ('connection', 'keep-alive', 'transfer-encoding',
                      'te', 'trailer', 'upgrade', 'proxy-authorization',
                      'proxy-authenticate')
        for key, value in self.headers.items():
            if key.lower() not in hop_by_hop:
                headers[key] = value

        try:
            conn.request(self.command, self.path, body=body, headers=headers)
            resp = conn.getresponse()

            # Send response status
            self.send_response_only(resp.status, resp.reason)

            # Forward all response headers
            # Note: getheaders() combines Set-Cookie headers, we need them separate
            for key, value in resp.getheaders():
                if key.lower() == 'set-cookie':
                    # Handle multiple cookies (they come combined with ', ' but that breaks parsing)
                    # Actually resp.msg.get_all() gives us the raw headers
                    continue  # Handle below
                elif key.lower() not in ('transfer-encoding', 'connection'):
                    self.send_header(key, value)

            # Handle Set-Cookie headers separately (each must be its own header)
            if hasattr(resp, 'msg') and resp.msg:
                cookies = resp.msg.get_all('Set-Cookie') or []
                for cookie in cookies:
                    self.send_header('Set-Cookie', cookie)

            self.send_header('Connection', 'close')
            self.end_headers()

            # Forward response body
            self.wfile.write(resp.read())

        except Exception as e:
            self.send_error(502, f"Upstream error: {e}")
        finally:
            conn.close()

    def do_GET(self): self.do_proxy()
    def do_POST(self): self.do_proxy()
    def do_PUT(self): self.do_proxy()
    def do_DELETE(self): self.do_proxy()
    def do_PATCH(self): self.do_proxy()
    def do_OPTIONS(self): self.do_proxy()
    def do_HEAD(self): self.do_proxy()

    def log_message(self, format, *args):
        print(f"[HTTPS] {self.address_string()} - {format % args}")


def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    cert_dir = os.path.join(script_dir, ".certs")
    cert_file = os.path.join(cert_dir, "localhost.crt")
    key_file = os.path.join(cert_dir, "localhost.key")

    if not os.path.exists(cert_file):
        print(f"Error: Certificate not found at {cert_file}")
        print("Run the dev-https.sh script first to generate certificates.")
        sys.exit(1)

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert_file, key_file)

    server = http.server.HTTPServer(("0.0.0.0", LISTEN_PORT), ProxyHandler)
    server.socket = context.wrap_socket(server.socket, server_side=True)

    print(f"HTTPS proxy running on https://localhost")
    print(f"Forwarding to http://{UPSTREAM_HOST}:{UPSTREAM_PORT}")
    print("Press Ctrl+C to stop")

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down...")


if __name__ == "__main__":
    main()
