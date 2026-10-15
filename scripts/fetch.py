#!/usr/bin/env python3
"""
Fetch a URL the way a browser would.

Uses curl_cffi to impersonate a browser's TLS/HTTP2 fingerprint, which gets past
Cloudflare's fingerprint check (plain `requests`/`curl` get a 403 "Just a moment...").

The status line goes to stderr, the body to stdout (or `-o`). Exit status: 0 on
success, 1 on an HTTP error, 2 when Cloudflare served a challenge.

For multi-step flows (tokens, cookies), import `session()` from a script instead:

    from fetch import session
    s = session()
    token = s.post(...).json()["token"]
"""

from __future__ import annotations

import argparse
import json
import sys

from curl_cffi import requests

DEFAULT_IMPERSONATE = "chrome"


def session(impersonate: str = DEFAULT_IMPERSONATE) -> requests.Session:
    return requests.Session(impersonate=impersonate)


def is_challenge(response: requests.Response) -> bool:
    if response.headers.get("cf-mitigated") == "challenge":
        return True
    return response.status_code in (403, 503) and "Just a moment..." in response.text[:4096]


def parse_header(value: str) -> tuple[str, str]:
    name, sep, content = value.partition(":")
    if not sep:
        raise argparse.ArgumentTypeError(f"expected 'Name: value', got {value!r}")
    return name.strip(), content.strip()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("url")
    parser.add_argument("-X", "--method", default=None, help="HTTP method (default: GET, or POST with -d)")
    parser.add_argument("-H", "--header", action="append", type=parse_header, default=[], help="'Name: value'")
    parser.add_argument("-d", "--data", help="request body; '@file' reads it from a file")
    parser.add_argument("-r", "--referer", help="set Origin and Referer from this base URL, as most sources do")
    parser.add_argument("--impersonate", default=DEFAULT_IMPERSONATE, help="browser target (chrome, safari, firefox, ...)")
    parser.add_argument("-i", "--include", action="store_true", help="print response headers to stderr")
    parser.add_argument("--json", action="store_true", help="pretty-print a JSON body")
    parser.add_argument("-o", "--output", help="write the body to this file instead of stdout")
    args = parser.parse_args()

    headers = dict(args.header)
    if args.referer:
        base = args.referer.rstrip("/")
        headers.setdefault("Origin", base)
        headers.setdefault("Referer", f"{base}/")
    data = args.data
    if data is not None and data.startswith("@"):
        with open(data[1:], encoding="utf-8") as file:
            data = file.read()
    if data is not None and data.lstrip().startswith(("{", "[")):
        headers.setdefault("Content-Type", "application/json")
    method = args.method or ("POST" if data is not None else "GET")

    response = session(args.impersonate).request(method, args.url, headers=headers, data=data, timeout=30)

    content_type = response.headers.get("content-type", "")
    print(f"{response.status_code} {method} {response.url} [{content_type}] {len(response.content)} bytes", file=sys.stderr)
    if args.include:
        for name, value in response.headers.items():
            print(f"{name}: {value}", file=sys.stderr)
    if is_challenge(response):
        print("Cloudflare challenge: this site needs a real browser, curl_cffi cannot pass it", file=sys.stderr)
        return 2

    if args.output:
        with open(args.output, "wb") as file:
            file.write(response.content)
    elif args.json:
        print(json.dumps(response.json(), indent=2, ensure_ascii=False))
    elif content_type.startswith(("text/", "application/json", "application/javascript", "application/xml")):
        sys.stdout.write(response.text)
    else:
        print("(binary body not printed; use -o to save it)", file=sys.stderr)

    return 1 if response.status_code >= 400 else 0


if __name__ == "__main__":
    sys.exit(main())
