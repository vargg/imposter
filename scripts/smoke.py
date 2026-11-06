#!/usr/bin/env python3
"""Smoke-test a packaged Imposter binary with an isolated profile and loopback server.

Usage: scripts/smoke.py [--measure] <path to the imposter executable>
Opens a real window; needs a display (Xvfb on Linux CI). Measurement mode is macOS-only.
"""

from __future__ import annotations

import argparse
import http.server
import json
import math
import os
import re
import signal
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import asdict, dataclass
from pathlib import Path

TIMEOUT = 120
MEASURED_RUNS = 10


@dataclass(frozen=True)
class Sample:
    launch_to_request_ms: float
    peak_rss_bytes: int

    def __post_init__(self) -> None:
        if not math.isfinite(self.launch_to_request_ms) or self.launch_to_request_ms <= 0:
            raise ValueError("launch-to-request time must be finite and positive")
        if type(self.peak_rss_bytes) is not int or self.peak_rss_bytes <= 0:
            raise ValueError("peak resident memory must be a positive integer in bytes")


def peak_rss(text: str) -> int:
    # macOS time -l reports ru_maxrss in bytes, unlike Linux time's KiB output.
    values = re.findall(r"^\s*(\d+)\s+maximum resident set size\s*$", text, re.MULTILINE)
    if len(values) != 1 or int(values[0]) <= 0:
        raise ValueError("missing, duplicate or invalid maximum resident set size")
    return int(values[0])


def summarize(samples: list[Sample]) -> dict:
    if len(samples) != MEASURED_RUNS:
        raise ValueError(f"measurement requires {MEASURED_RUNS} successful samples")
    summary = {}
    for name in ("launch_to_request_ms", "peak_rss_bytes"):
        values = [getattr(sample, name) for sample in samples]
        summary[name] = {
            "median": statistics.median(values),
            "minimum": min(values),
            "maximum": max(values),
        }
    return {
        "warmup_runs_excluded": 1,
        "samples": [asdict(sample) for sample in samples],
        "summary": summary,
    }


def handler_for(requests: list[tuple[str, str, float]]) -> type[http.server.BaseHTTPRequestHandler]:
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802 - http.server's naming
            received = time.perf_counter()
            requests.append((self.path, self.headers.get("User-Agent", ""), received))
            body = b"smoke ok"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_args: object) -> None:
            pass

    return Handler


def run_process(command: list[str], work: str, env: dict[str, str], measure: bool) -> subprocess.CompletedProcess:
    with subprocess.Popen(
        command, cwd=work, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, start_new_session=measure,
    ) as process:
        try:
            stdout, stderr = process.communicate(timeout=TIMEOUT)
        except subprocess.TimeoutExpired:
            # time is a wrapper: killing only it would leave the app and its profile open.
            if measure:
                os.killpg(process.pid, signal.SIGKILL)
            else:
                process.kill()
            process.communicate()
            raise
        return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def run_once(binary: Path, expected_agent: str, measure: bool = False) -> Sample | None:
    requests: list[tuple[str, str, float]] = []
    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler_for(requests)) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory(prefix="imposter-smoke-") as work:
                data = Path(work) / "data"
                legacy = Path(work) / "ignored-legacy-override"
                stats = Path(work) / "time.txt"
                env = {
                    **os.environ,
                    "IMPOSTER_DATA_DIR": str(data),
                    "POSTER_DATA_DIR": str(legacy),
                    "IMPOSTER_LOG": "off",
                    "POSTER_LOG": "trace",
                    "LC_ALL": "C",
                }
                command = [str(binary), "--smoke-test", f"http://127.0.0.1:{server.server_port}/smoke"]
                if measure:
                    command = ["/usr/bin/time", "-l", "-o", str(stats), *command]
                started = time.perf_counter()
                result = run_process(command, work, env, measure)
                print(result.stdout, end="", file=sys.stderr if measure else sys.stdout)
                if result.returncode != 0:
                    print(result.stderr, end="", file=sys.stderr)
                    raise ValueError(f"exit code {result.returncode}")
                failures = []
                if legacy.exists():
                    failures.append("the removed POSTER_DATA_DIR variable was used")
                if "TRACE" in result.stderr or " INFO " in result.stderr:
                    failures.append("IMPOSTER_LOG=off did not suppress app logs")
                agents = [agent for _, agent, _ in requests]
                if agents != [expected_agent]:
                    failures.append(f"unexpected User-Agent: {agents}")
                seen = [path for path, _, _ in requests]
                if seen != ["/smoke"]:
                    failures.append(f"server saw {seen}, expected one GET /smoke")
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
                if failures:
                    raise ValueError("; ".join(failures))
                if measure:
                    return Sample(
                        (requests[0][2] - started) * 1000,
                        peak_rss(stats.read_text(encoding="utf-8")),
                    )
                return None
        finally:
            server.shutdown()
            thread.join(timeout=TIMEOUT)


def measure_series(binary: Path, expected_agent: str) -> dict:
    samples = []
    for index in range(MEASURED_RUNS + 1):
        print("Warm-up" if index == 0 else f"Sample {index}/{MEASURED_RUNS}", file=sys.stderr)
        sample = run_once(binary, expected_agent, measure=True)
        if sample is None:
            raise ValueError("measurement did not produce a sample")
        if index:
            samples.append(sample)
    return summarize(samples)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--measure", action="store_true", help="macOS: one warm-up, ten samples, JSON report")
    args = parser.parse_args()
    if args.measure and sys.platform != "darwin":
        parser.error("--measure requires macOS /usr/bin/time -l; ordinary smoke tests work on every OS")
    try:
        manifest = (Path(__file__).resolve().parent.parent / "Cargo.toml").read_text(encoding="utf-8")
        version = re.search(r'^version = "([^"]+)"$', manifest, re.MULTILINE)
        if version is None:
            raise ValueError("workspace version is missing from Cargo.toml")
        expected_agent = f"Imposter/{version.group(1)}"
        if args.measure:
            report = measure_series(args.binary.resolve(), expected_agent)
            print(json.dumps(report, indent=2, allow_nan=False))
        else:
            run_once(args.binary.resolve(), expected_agent)
            print("smoke test passed")
        return 0
    except (ValueError, OSError, sqlite3.Error, subprocess.TimeoutExpired) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
