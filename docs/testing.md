# Testing Imposter

The rules (what must be tested, real boundaries, no sleeps, never weakening a gate) are in
[AGENTS.md](../AGENTS.md#verification). This page is how to apply them.

## Running

```sh
scripts/check.sh                      # Python tooling tests, fmt, clippy, Rust tests + coverage
python3 -B -m unittest discover -s scripts -p 'test_*.py'  # smoke-tool tests only
cargo test -p imposter-http             # one crate
cargo test -p imposter-ui --test settings   # one test binary
cargo run -p imposter-ui --example screenshot -- /tmp/shots   # offscreen renders to look at
```

`scripts/check.sh` passing is the definition of done for a change. CI runs the same gates
(`.github/workflows/ci.yml`): Python tooling tests, fmt, clippy and Rust tests on macOS,
Windows and Linux, coverage on Linux. A local run covers only the local OS. Python 3.9 or
newer runs the tooling tests; `PYTHON=python scripts/check.sh` selects another executable.

## Where a test goes

Tests sit at the boundary the behavior crosses, in the crate that owns it:

| Behavior | Boundary | Harness |
|---|---|---|
| Pure rules (variables, URLs, settings resolution, codecs) | function input/output | unit tests in `imposter-core`, `imposter-transfer` |
| Wire format, redirects, auth, TLS, uploads, timeouts | bytes on a loopback socket | `imposter-testkit` servers and PKI |
| Persistence, migrations, retention, transactions | a real SQLite file in a temp dir | `imposter-store` integration tests |
| Services across store, HTTP and files | the service API | `imposter-app` integration tests |
| UI workflows | AccessKit nodes of the real app | `egui_kittest` over a real database (`imposter-ui/tests/support`) |
| Exact behavior (preparation, resolution, multipart bytes, conversions, the native format) | golden cases | `tests/fixtures/golden`, `tests/fixtures/native` |
| The command line (`imposter send`) | exit status, stdout and stderr | `imposter-cli/tests/send.rs` in process; `imposter/tests/cli.rs` runs the binary with an empty environment |
| The MCP spike (`imposter mcp`) | JSON-RPC messages on its stdio | `imposter-cli/tests/mcp.rs` over channels; a process test in `imposter/tests/cli.rs` |
| Headless results | the result JSON and its redaction | `imposter-core/tests/run_result.rs` and `tests/fixtures/results` |
| The packaged binary | a process with a temp `IMPOSTER_DATA_DIR` | the installed-binary smoke test; `--headless` runs `send` with no display variables |
| Smoke measurements | sample parsing and aggregation, loopback and subprocess lifetime | `scripts/test_smoke.py`; packaged `--measure` runs are separate evidence |

UI tests find widgets by accessible name and role, never by position or by reading private
state. They may set up state through the store before the app opens, and may read app state to
assert results. File dialogs are answered with `ImposterApp::script_file_dialogs`; everything
else goes through clicks, typing and key presses.

## Conventions

- **Waiting:** `support::settle` runs frames until background work is idle; servers and
  workers signal with channels and deadlines. No fixed sleeps, no wall-clock polling.
- **Isolation:** every test owns a temp directory and an ephemeral port. Nothing touches
  `~/.imposter` or the network.
- **Golden fixtures:** a change to one is a change of behavior, made in the same commit as
  the code with the reason in its message; never to make a failing test pass.
- **Regressions:** a bug fix starts with a test that fails on the old code. Check it fails by
  reverting the fix once (or by a quick mutation) before committing.
- **Blocking in tests:** test and example crates allow `clippy::disallowed_methods` at their
  top, because they drive the app from outside its event loop. Production UI code may not.

## Coverage

`cargo llvm-cov` over the workspace, floors 85% lines and 70% regions (stable Rust has no branch
coverage). `main.rs` and `imposter-testkit` are excluded; the smoke test covers `main.rs`.
Coverage shows what ran, not what was asserted: a line covered by a test that asserts nothing
is not tested. Never lower a floor to get green.

## What automated tests do not cover

Native look and feel, OS file dialogs, platform keyboard conventions, window-manager behavior,
HiDPI rendering and real GPU backends. These are checked by hand with
[validation.md](validation.md) before each release.
