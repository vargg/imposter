"""Deterministic checks for smoke measurement reporting and resource boundaries."""

import contextlib
import http.server
import io
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch
from urllib.request import ProxyHandler, Request, build_opener

import smoke


class MeasurementTests(unittest.TestCase):
    def test_macos_rss_is_bytes_and_ignores_other_counters(self):
        text = "  0.10 real 0.05 user 0.02 sys\n  104857600  maximum resident set size\n  14 page faults\n"
        self.assertEqual(smoke.peak_rss(text), 100 * 1024 * 1024)

    def test_rejects_missing_duplicate_or_invalid_rss(self):
        for text in (
            "", "1024 Maximum resident set size (kbytes)",
            "0 maximum resident set size", "-5 maximum resident set size",
            "NaN maximum resident set size", "1.5 maximum resident set size",
            "10 maximum resident set size\n20 maximum resident set size",
        ):
            with self.subTest(text=text), self.assertRaises(ValueError):
                smoke.peak_rss(text)

    def test_rejects_invalid_samples(self):
        for elapsed in (0, -1, float("nan"), float("inf"), -float("inf")):
            with self.subTest(elapsed=elapsed), self.assertRaises(ValueError):
                smoke.Sample(elapsed, 1024)
        for rss in (0, -1, 1.5, float("inf"), True):
            with self.subTest(rss=rss), self.assertRaises(ValueError):
                smoke.Sample(1, rss)

    def test_summary_keeps_samples_and_units(self):
        samples = [smoke.Sample(n, n * 1024) for n in range(1, 11)]
        report = smoke.summarize(samples)
        self.assertEqual(report["summary"], {
            "launch_to_request_ms": {"median": 5.5, "minimum": 1, "maximum": 10},
            "peak_rss_bytes": {"median": 5632, "minimum": 1024, "maximum": 10240},
        })
        self.assertEqual(report["samples"][0], {"launch_to_request_ms": 1, "peak_rss_bytes": 1024})
        self.assertEqual(len(report["samples"]), 10)
        self.assertEqual(report["warmup_runs_excluded"], 1)

    def test_partial_and_oversized_series_are_not_reports(self):
        for count in (0, 1, 9, 11):
            with self.subTest(count=count), self.assertRaises(ValueError):
                smoke.summarize([smoke.Sample(1, 1024)] * count)

    def test_warmup_is_validated_but_excluded(self):
        # Inject samples only at the subprocess workflow boundary to pin series selection.
        samples = [smoke.Sample(n, n * 1024) for n in range(1, 12)]
        with patch.object(smoke, "run_once", side_effect=samples) as run, contextlib.redirect_stderr(io.StringIO()):
            report = smoke.measure_series(Path("unused"), "Imposter/test")
        self.assertEqual(run.call_count, 11)
        self.assertTrue(all(call.kwargs == {"measure": True} for call in run.call_args_list))
        self.assertEqual(report["summary"]["launch_to_request_ms"], {
            "median": 6.5, "minimum": 2, "maximum": 11,
        })

    def test_failed_warmup_or_sample_invalidates_series(self):
        for failed_index in (0, 5, 10):
            samples = [smoke.Sample(1, 1024)] * failed_index + [ValueError("smoke failed")]
            with self.subTest(failed_index=failed_index):
                # Fault injection: stop at the failing run, never summarize partial successes.
                with patch.object(smoke, "run_once", side_effect=samples) as run, contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaisesRegex(ValueError, "smoke failed"):
                        smoke.measure_series(Path("unused"), "Imposter/test")
                self.assertEqual(run.call_count, failed_index + 1)

    def test_missing_sample_is_not_success(self):
        with patch.object(smoke, "run_once", return_value=None), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaisesRegex(ValueError, "did not produce a sample"):
                smoke.measure_series(Path("unused"), "Imposter/test")

    def test_failed_series_has_no_json_output_and_exits_unsuccessfully(self):
        output, errors = io.StringIO(), io.StringIO()
        with patch.object(sys, "argv", ["smoke.py", "--measure", "unused"]), patch.object(sys, "platform", "darwin"):
            # Fault injection at process execution, including the public CLI failure path.
            with patch.object(smoke, "run_once", side_effect=ValueError("smoke failed")):
                with contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
                    self.assertEqual(smoke.main(), 1)
        self.assertEqual(output.getvalue(), "")
        self.assertIn("FAIL: smoke failed", errors.getvalue())


class BoundaryTests(unittest.TestCase):
    def test_every_smoke_assertion_still_rejects_a_broken_app(self):
        for fault, message in (
            ("exit", "exit code 7"), ("legacy", "removed POSTER_DATA_DIR"),
            ("trace", "did not suppress app logs"), ("info", "did not suppress app logs"),
            ("agent", "unexpected User-Agent"), ("path", "expected one GET /smoke"),
            ("missing_request", "expected one GET /smoke"),
            ("duplicate_request", "expected one GET /smoke"),
            ("stdout", "no 'smoke test: HTTP 200' line"),
            ("database", "no database"), ("history", "0 executions recorded"),
        ):
            with self.subTest(fault=fault):
                def broken_app(command, work, env, measure):
                    # Fault injection only at the app process; HTTP and SQLite stay real.
                    data = Path(env["IMPOSTER_DATA_DIR"])
                    data.mkdir()
                    if fault != "database":
                        with sqlite3.connect(data / "imposter.db") as connection:
                            connection.execute("CREATE TABLE executions (id INTEGER)")
                            if fault != "history":
                                connection.execute("INSERT INTO executions VALUES (1)")
                        connection.close()
                    if fault == "legacy":
                        Path(env["POSTER_DATA_DIR"]).mkdir()
                    if fault != "missing_request":
                        url = command[-1] + ("wrong" if fault == "path" else "")
                        agent = "wrong" if fault == "agent" else "Imposter/test"
                        request = Request(url, headers={"User-Agent": agent})
                        for _ in range(2 if fault == "duplicate_request" else 1):
                            with build_opener(ProxyHandler({})).open(request, timeout=5) as response:
                                response.read()
                    stderr = {"trace": "TRACE log", "info": " INFO log"}.get(fault, "")
                    return subprocess.CompletedProcess(
                        command, 7 if fault == "exit" else 0,
                        "" if fault == "stdout" else "smoke test: HTTP 200\n", stderr,
                    )

                with patch.object(smoke, "run_process", side_effect=broken_app):
                    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                        with self.assertRaisesRegex(ValueError, message):
                            smoke.run_once(Path("unused"), "Imposter/test")

    def test_every_headless_assertion_still_rejects_a_broken_app(self):
        for fault, message in (
            ("exit", "exit code 3"), ("outcome", "outcome 'failed'"), ("json", "outcome None"),
            ("agent", "unexpected User-Agent"), ("path", "expected one GET /smoke"),
            ("missing_request", "expected one GET /smoke"), ("profile", "created a profile"),
            ("display", "unexpected User-Agent"),
        ):
            with self.subTest(fault=fault):
                def broken_app(command, work, env, measure):
                    # Fault injection only at the app process; HTTP stays real.
                    self.assertEqual(command[1:3], ["send", str(Path(work) / "smoke.imposter.json")])
                    document = json.loads(Path(command[2]).read_text(encoding="utf-8"))
                    self.assertEqual(document["collection"]["folders"][0]["requests"][0]["name"], "Smoke")
                    if fault == "profile":
                        Path(env["IMPOSTER_DATA_DIR"]).mkdir()
                    shown = any(name in env for name in smoke.DISPLAY_VARIABLES)
                    if fault != "missing_request":
                        url = env["SMOKE_BASE"] + "/smoke" + ("wrong" if fault == "path" else "")
                        agent = "wrong" if fault == "agent" or shown else "Imposter/test"
                        request = Request(url, headers={"User-Agent": agent})
                        with build_opener(ProxyHandler({})).open(request, timeout=5) as response:
                            response.read()
                    stdout = {"json": "not json", "outcome": '{"outcome": "failed"}'}.get(fault, '{"outcome": "passed"}')
                    return subprocess.CompletedProcess(command, 3 if fault == "exit" else 0, stdout, "")

                display = {"DISPLAY": ":0"} if fault == "display" else {}
                with patch.dict(os.environ, display), patch.object(smoke, "run_process", side_effect=broken_app):
                    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                        if fault == "display":
                            smoke.run_headless(Path("unused"), "Imposter/test")
                        else:
                            with self.assertRaisesRegex(ValueError, message):
                                smoke.run_headless(Path("unused"), "Imposter/test")

    def test_loopback_observations_are_isolated_per_run(self):
        first = []
        second = []
        for records in (first, second):
            with http.server.ThreadingHTTPServer(("127.0.0.1", 0), smoke.handler_for(records)) as server:
                thread = threading.Thread(target=server.serve_forever)
                thread.start()
                try:
                    with build_opener(ProxyHandler({})).open(f"http://127.0.0.1:{server.server_port}/smoke", timeout=5) as response:
                        self.assertEqual(response.status, 200)
                        self.assertEqual(response.read(), b"smoke ok")
                finally:
                    server.shutdown()
                    thread.join(timeout=5)
                self.assertFalse(thread.is_alive())
        self.assertEqual(len(first), 1)
        self.assertEqual(len(second), 1)
        self.assertEqual(first[0][0], "/smoke")
        self.assertTrue(first[0][1].startswith("Python-urllib/"))
        self.assertTrue(isinstance(first[0][2], float))

    def test_subprocess_output_and_exit_status_are_preserved(self):
        with tempfile.TemporaryDirectory() as work:
            result = smoke.run_process(
                [sys.executable, "-c", "import sys; print('out'); print('err', file=sys.stderr); sys.exit(7)"],
                work, dict(os.environ), False,
            )
        self.assertEqual((result.returncode, result.stdout, result.stderr), (7, "out\n", "err\n"))

    def test_hung_process_is_stopped_at_deadline(self):
        for grouped in ((False, True) if os.name == "posix" else (False,)):
            with self.subTest(grouped=grouped), tempfile.TemporaryDirectory() as work:
                with patch.object(smoke, "TIMEOUT", 0.1), self.assertRaises(subprocess.TimeoutExpired):
                    smoke.run_process(
                        [sys.executable, "-c", "import threading; threading.Event().wait()"],
                        work, dict(os.environ), grouped,
                    )


if __name__ == "__main__":
    unittest.main()
