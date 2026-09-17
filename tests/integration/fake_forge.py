#!/usr/bin/env python3
"""A fake GitHub REST API for the bats integration tests.

The forge connectors speak HTTP themselves since JOY-0298-E4 (design
D2.8), so the marked stub at the forge boundary is no longer a `curl`
or a `gh` on the PATH: it is this, one small server on the loopback
interface that the connector reaches through `forges.yaml`'s
`api_base`. No test ever touches the network.

Usage: fake_forge.py <state-dir>

It writes the port it bound to into <state-dir>/port and appends one
line per request to <state-dir>/calls. Two files carry the state a test
wants to set or read:

  <state-dir>/email          the verified addresses `GET /user/emails`
                             answers, separated by commas
  <state-dir>/login          the login `GET /user` answers
  <state-dir>/release_body   the notes of the release for the tag; an
                             empty file means there is no release yet
"""

import json
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

STATE = sys.argv[1] if len(sys.argv) > 1 else "."


def read(name, default=""):
    try:
        with open(os.path.join(STATE, name), "r", encoding="utf-8") as handle:
            return handle.read()
    except OSError:
        return default


def write(name, value):
    with open(os.path.join(STATE, name), "w", encoding="utf-8") as handle:
        handle.write(value)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):
        pass

    def record(self, body):
        with open(os.path.join(STATE, "calls"), "a", encoding="utf-8") as handle:
            handle.write(f"{self.command} {self.path} {body}\n")

    def reply(self, status, payload):
        raw = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(raw)

    def body(self):
        length = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(length).decode("utf-8") if length else ""

    def handle_one(self, body):
        path = self.path.split("?")[0]
        if path in ("/user/emails", "/user") and not self.headers.get("Authorization"):
            # the account endpoints need a credential, exactly as a real
            # forge does, so a test cannot pass by accident
            return self.reply(401, {"message": "Requires authentication"})
        if path == "/user/emails":
            addresses = read("email", "alice@example.com").strip().split(",")
            return self.reply(
                200,
                [{"email": one.strip(), "verified": True} for one in addresses if one.strip()],
            )
        if path == "/user":
            return self.reply(200, {"login": read("login", "alice-login").strip(), "email": None})
        if "/releases/tags/" in path:
            notes = read("release_body")
            if not notes:
                return self.reply(404, {"message": "Not Found"})
            return self.reply(
                200,
                {"id": 1, "html_url": "https://forge.example/r/releases/tag/x", "body": notes},
            )
        if path.endswith("/releases") and self.command == "POST":
            write("release_body", json.loads(body or "{}").get("body", ""))
            return self.reply(
                201, {"id": 1, "html_url": "https://forge.example/r/releases/tag/x"}
            )
        if "/releases/" in path and self.command == "PATCH":
            write("release_body", json.loads(body or "{}").get("body", ""))
            return self.reply(200, {"id": 1, "html_url": "https://forge.example/r/releases/tag/x"})
        return self.reply(404, {"message": "Not Found"})

    def do_GET(self):  # noqa: N802 - the base class names it
        self.record("")
        self.handle_one("")

    def do_POST(self):  # noqa: N802
        body = self.body()
        self.record(body)
        self.handle_one(body)

    def do_PATCH(self):  # noqa: N802
        body = self.body()
        self.record(body)
        self.handle_one(body)


def main():
    server = HTTPServer(("127.0.0.1", 0), Handler)
    write("port", str(server.server_address[1]))
    server.serve_forever()


if __name__ == "__main__":
    main()
