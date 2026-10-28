#!/usr/bin/env python3
"""Smoke-test a packaged Imposter binary: it must start, send one request to a loopback
server, record it in history and exit cleanly, using a throwaway data directory.

Usage: scripts/smoke.py <path to the imposter executable>

Opens the real window for a moment, so it needs a display (Xvfb on Linux CI).
"""

import http.server
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

TIMEOUT = 120


class Handler(http.server.BaseHTTPRequestHandler):
    seen: list[str] = []
    agents: list[str] = []

    def do_GET(self) -> None:  # noqa: N802 - http.server's naming
        Handler.seen.append(self.path)
        Handler.agents.append(self.headers.get("User-Agent", ""))
        body = b"smoke ok"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: object) -> None:
        pass


def main() -> int:
    binary = Path(sys.argv[1]).resolve()
    manifest = (Path(__file__).resolve().parent.parent / "Cargo.toml").read_text(encoding="utf-8")
    version = re.search(r'^version = "([^"]+)"$', manifest, re.MULTILINE)
    if version is None:
        raise ValueError("workspace version is missing from Cargo.toml")
    expected_agent = f"Imposter/{version.group(1)}"
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{server.server_port}/smoke"
    with tempfile.TemporaryDirectory(prefix="imposter-smoke-") as work:
        data = Path(work) / "data"
        legacy = Path(work) / "ignored-legacy-override"
        env = {
            **os.environ,
            "IMPOSTER_DATA_DIR": str(data),
            "POSTER_DATA_DIR": str(legacy),
            "IMPOSTER_LOG": "off",
            "POSTER_LOG": "trace",
        }
        result = subprocess.run(  # noqa: S603 - the binary under test
            [str(binary), "--smoke-test", url],
            cwd=work,
            env=env,
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )
        server.shutdown()
        print(result.stdout, end="")
        if result.returncode != 0:
            print(result.stderr, end="", file=sys.stderr)
            print(f"FAIL: exit code {result.returncode}", file=sys.stderr)
            return 1
        failures = []
        if legacy.exists():
            failures.append("the removed POSTER_DATA_DIR variable was used")
        if "TRACE" in result.stderr or " INFO " in result.stderr:
            failures.append("IMPOSTER_LOG=off did not suppress app logs")
        if Handler.agents != [expected_agent]:
            failures.append(f"unexpected User-Agent: {Handler.agents}")
        if Handler.seen != ["/smoke"]:
            failures.append(f"server saw {Handler.seen}, expected one GET /smoke")
        if "smoke test: HTTP 200" not in result.stdout:
            failures.append("no 'smoke test: HTTP 200' line")
        database = data / "imposter.db"
        if not database.exists():
            failures.append(f"no database at {database}")
        else:
            connection = sqlite3.connect(database)  # WAL needs write access even to read
            try:
                (count,) = connection.execute("SELECT count(*) FROM executions").fetchone()
            finally:
                connection.close()
            if count != 1:
                failures.append(f"{count} executions recorded, expected 1")
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        if failures:
            return 1
    print("smoke test passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
