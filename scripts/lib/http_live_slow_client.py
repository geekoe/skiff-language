#!/usr/bin/env python3
"""Zero-window slow HTTP client for the `router-live:http` backpressure case.

This helper sends the request, reads only the response head, then holds the
connection open without reading. On hosts with small socket windows (Linux
CI, ~200 KiB) the Router's HTTP writer stalls, the bounded stream channel
fills and the 10s drain deadline produces the `backpressure` terminal; on
hosts whose kernel autotunes the window past the Router's per-session
inbound byte budget (macOS absorbs ~800 KiB of a 1 MiB budget) the burst
completes into the socket and the harness records that OS-absorption
boundary instead.
"""

import socket
import sys
import time


def main() -> int:
    if len(sys.argv) != 6:
        print(
            "usage: http_live_slow_client.py <port> <path> <service> <version> <hold-seconds>",
            file=sys.stderr,
        )
        return 2
    port = int(sys.argv[1])
    path = sys.argv[2]
    service = sys.argv[3]
    version = sys.argv[4]
    hold_seconds = float(sys.argv[5])

    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        sock.settimeout(60)
        sock.connect(("127.0.0.1", port))
        request = (
            f"POST {path} HTTP/1.1\r\n"
            f"host: 127.0.0.1:{port}\r\n"
            f"x-skiff-service: {service}\r\n"
            f"x-skiff-version: {version}\r\n"
            "content-length: 0\r\n"
            "connection: close\r\n"
            "\r\n"
        )
        sock.sendall(request.encode("ascii"))
        data = b""
        while b"\r\n\r\n" not in data:
            chunk = sock.recv(4096)
            if not chunk:
                break
            data += chunk
        head, _, _ = data.partition(b"\r\n\r\n")
        status_line = head.split(b"\r\n", 1)[0]
        print(status_line.decode("ascii", "replace"))
        sys.stdout.flush()
        # Do not read the body; keep the connection open so the peer never
        # observes a disconnect (only a stalled window).
        time.sleep(hold_seconds)
        return 0
    finally:
        try:
            sock.close()
        except OSError:
            pass


if __name__ == "__main__":
    raise SystemExit(main())
