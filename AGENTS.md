# Poster agent manual

Poster is a local desktop HTTP API client written in Rust (egui/eframe). It rewrites the
Python/PyQt6 prototype at `~/dev/pystman`, which stays the reference implementation; its
`AGENTS.md` is the behavioral spec. Port behaviors, not code structure.

## About this file

- **This file does not describe the codebase. The code describes itself.** Names, types, crate
  docs (`//!`), each crate's `description` in `Cargo.toml`, and tests (the executable spec) say
  what exists and how it works. Do not list modules, files, functions or data flows here.
  Why: such lists go stale on the next refactor, and a stale manual misleads more than none.
- **This file holds only architecture principles, rules and decisions, each with its reason.**
  Why: a rule without a reason gets misapplied at the edges or "fixed" by the next contributor.
- **A rule that a tool can check is checked by a tool**, and the rule names that check. Prose is
  the fallback for what tools cannot express. Rules not yet checked are listed in
  [Enforcement gaps](#enforcement-gaps). Why: agents and people forget prose; CI does not.
- **`PLAN.md` is the roadmap; this file is the durable record.** When a stage settles an open
  question from `PLAN.md`, record the outcome and its reason under [Decisions](#decisions) in
  the same commit.
- **Changing a rule or decision updates this file in the same commit.** Replace a superseded
  decision rather than keeping both; mention the rejected option in the reason when that stops
  someone from proposing it again. Git keeps the history.

## Verification

- **Every feature, decision, bug fix and change is covered by tests or lints.**
  - Feature: behavior tests at the boundary it affects.
  - Bug: a regression test that fails before the fix and passes after it.
  - Decision: a test or lint that fails when the decision is violated (for example, a loopback
    test that checks header order in the raw bytes received).
  - Refactor: keeps existing behavioral coverage; no test is weakened to make it pass.

  Why: the tests and lints are the only record of past decisions and bugs that is checked on
  every change.
- **Never weaken a check to get green.** No lowered coverage floors, no `#[ignore]` on failing
  tests, no deleted assertions, no `#[allow]` without a reason next to it.
  Why: a gate that bends under pressure stops being a gate.
- **Test at the real boundary.** HTTP: loopback servers on ephemeral ports, asserting the bytes
  received. SQLite: a real database in a temp dir. UI: real actions through `egui_kittest`. Mock
  only the boundary a fault-injection test targets.
  Why: mocks confirm what we assumed; real boundaries catch what we did not.
- **Tests are deterministic and independent.** No fixed sleeps: wait on channels or events with a
  deadline. No external network, no personal data directory (`POSTER_DATA_DIR` points at a temp
  dir). Every test owns and cleans up its resources.
  Why: flaky tests train people to ignore red CI.
- **Report only what was run.** A change is done when `scripts/check.sh` passes locally; say
  which checks ran and which did not (for example, CI on other OSes). Commit only after it
  passed: the script stops at the first failure, so never chain a commit after it with `;`.

```sh
scripts/check.sh     # fmt, clippy (-D warnings), all tests under the coverage floor; as CI
cargo test -p <crate>
cargo run -p poster
```

Coverage must stay at or above 85% of lines and 70% of regions (stable Rust has no branch
coverage; regions are the closest measure). `main.rs` only opens the window; the
installed-binary smoke test covers it. `poster-testkit` is test code and would inflate the
numbers, so both are excluded. Each stage in `PLAN.md` is its own commit.

## Architecture principles

- **Dependency direction: `core` <- `http`, `store`, `transfer` <- `app` <- `ui` <- `poster`.**
  Never add an edge against it.
  Why: each layer stays testable without the ones above it, and `ui` can change without touching
  the domain.
  Enforced by: `crates/poster/tests/architecture.rs` (every manifest against the allowed
  graph). `poster-ui` reaches the store and transfer crates only through `poster-app`.
- **`poster-core` does no IO** (no files, network, database, clock or environment).
  Why: the domain rules (variables, settings resolution, validation) can then be tested
  exhaustively, quickly and deterministically.
  Enforced by: `crates/poster-core/clippy.toml` (`disallowed-types` / `disallowed-methods` for
  files, sockets, clocks, environment and sleeping); tests that read fixtures allow it locally.
- **The UI thread never blocks or awaits.** Background work starts through the app handle and
  its results come back over a channel drained at the start of each frame.
  Why: egui redraws every frame on that thread; any block freezes the whole window.
- **One thread owns the SQLite connection**; every other part sends it commands, and one
  command is one transaction.
  Why: writes are serialized without locks, and a failed command rolls back as a unit.
- **Dependencies are declared once** in the root `[workspace.dependencies]`; crates use
  `name.workspace = true`.
  Why: one version per dependency across the workspace, upgraded in one place.
  Enforced by: the architecture test (no crate manifest sets its own version).
- **Dependencies must build on macOS, Windows and Linux without system libraries** (pure Rust, or
  C sources vendored by the crate, as with `rusqlite`'s `bundled` feature). No C/C++ code of our
  own.
  Why: the product is one self-contained binary per OS.
  Enforced by: the CI matrix builds on all three OSes.

## Rules

- **No `unsafe`.** Why: nothing in this app needs it, and it cancels the compiler's guarantees.
  Enforced by: `unsafe_code = "deny"`.
- **No `unwrap()` outside tests; `expect()` only for real invariants, and its message states
  the invariant.** Errors go up with `?`.
  Why: a desktop app must report a failure, not crash on it.
  Enforced by: `clippy::unwrap_used`, allowed in test code via `clippy.toml`. Clippy only counts
  `#[test]` functions and `#[cfg(test)]` modules as test code, so every integration-test file
  (`tests/*.rs`, `tests/support/`) starts with `#![cfg(test)]`.
- **No `dbg!`, `todo!` or `unimplemented!` in committed code.** Do the work, or put it in `PLAN.md`.
  Enforced by: `clippy::dbg_macro`, `clippy::todo`, `clippy::unimplemented`.
- **Test support lives in `poster-testkit`, used only as a dev-dependency.** Enforced by the
  architecture test.
- **Errors: `thiserror` enums in library crates; `anyhow` only in the `poster` binary.**
  Enforced by the architecture test.
  Why: callers match on error variants (the UI shows a validation error differently from a
  network error); only the binary just reports them.
- **Make invalid states unrepresentable** (enums, newtypes, private fields with checked
  constructors) instead of documenting invariants in comments.
  Why: the compiler then checks them, which is how the code describes itself.
- **Comments explain why, never what.** No commented-out code; git keeps it.
- **Lint levels are set only in the root `Cargo.toml`** (`[workspace.lints]`: clippy pedantic
  plus the restriction lints above), and every crate sets `[lints] workspace = true`. Lint
  settings go in `clippy.toml` (the root one, or a crate's own for crate-specific lists).
  Workspace-wide allows go in the root with a reason. A local `#[allow]` is only for a reason
  specific to that item, stated in a comment on the same line.
  Why: one visible policy instead of exceptions spread through the code.
- **Never log credentials, tokens, or request or response bodies.**
  Why: logs end up in bug reports and screenshots.
  Enforced by: `poster-http/tests/logging.rs` (trace-level capture, dependencies included).
  Transport error messages drop the URL, which can carry `user:password@`.
- **The app's data is `~/.poster/poster.db`, overridable with `POSTER_DATA_DIR`. Never read or
  write the Python app's database files.**
  Why: the Python app must keep working; users migrate through `.poster.json` export and import.

## Decisions

- **Rust only, one native binary per OS; no webview, JVM or own C/C++.** Why: the Python app was
  hard to distribute (a specific interpreter plus a resolved environment). Speed and memory must
  not get worse, but are secondary.
- **GUI: egui + eframe (glow backend); Iced not needed.** Why: the most mature tables, trees and
  panels, custom text layout for `{{var}}` highlighting, and a UI test harness (`egui_kittest`,
  headless, no GPU). Slint was rejected for limited text-editing control and license
  conditions; gpui for an unstable API. Accepted trade-off: a custom dark theme (accent
  `#ff6c37`), not a native look. Confirmed in stage 0: release binary 5.6 MiB (LTO, stripped,
  macOS arm64); first frame about 225 ms after process start (24 ms after app creation).
- **Response viewer: virtualized rows, never one big text widget.** Only visible rows are laid
  out (`ScrollArea::show_rows`, monospace, wrap by column count). Why: a 5 MiB read-only
  `TextEdit` costs about 730 ms per frame; virtualized rows cost about 0.1 ms, highlighted or
  not. Measured by `cargo run --release -p poster-ui --example text_perf` (M-series Mac).
- **Highlighting: small hand-written tokenizers, not syntect.** Why: JSON highlighting of
  256 KiB takes about 2 ms by hand and about 100 ms with syntect, which is also a large
  dependency.
- **Body editor: egui `TextEdit`; `{{var}}`/JSON highlighting only up to 64 KiB.** Why: per
  interactive frame the editor costs about 4 ms at 64 KiB, 24 ms at 256 KiB and 130 ms at 1 MiB
  (plain), and highlighting adds about 30%. Larger bodies stay editable but slow; a custom
  virtualized editor would be needed to fix that and is out of scope.
- **TLS: rustls with the `ring` provider, configured by us** (`poster_http::tls`) and passed to
  every reqwest client with `use_preconfigured_tls`, including plain-HTTP clients (reqwest
  panics without a provider). Why: no OpenSSL, cmake or system library on any OS; full control
  of roots, verification and client keys. A custom CA bundle replaces the platform roots.
- **Client keys: PKCS#8 (plain or encrypted, PBES2), PKCS#1 and SEC1. Legacy encrypted PEM
  (`Proc-Type: 4,ENCRYPTED`) is rejected** with advice to run `openssl pkcs8 -topk8`. Why:
  rustls and RustCrypto do not implement the legacy OpenSSL scheme. PBKDF2-HMAC-SHA1 is enabled
  because LibreSSL and older OpenSSL use it by default (fixtures in `poster-http/tests/fixtures`).
- **HTTP/1.1 only.** Why: matches the Python app (httpx default) and keeps the wire format the
  tests pin; HTTP/2 settings are out of scope.
- **Header order: values of one name keep their order; names go out in first-occurrence order,
  lowercased.** So `A, B, A` is sent as `a, a, b`. Why: hyper stores headers in a `HeaderMap`
  and its order-preserving extensions are private; writing our own HTTP/1.1 client is not
  worth the risk. Pinned by `poster-http/tests/header_order.rs`.
- **Migrations: our own runner over `user_version`, not `rusqlite_migration`.** Each migration
  runs in its own transaction and must pass `foreign_key_check`; an existing database is copied
  to `poster.db.bak-<version>` with SQLite's backup API first; a database from a newer app is
  refused. Why: about 40 lines, no second rusqlite version to keep in step, and the backup
  happens exactly before the first change. Released migrations are frozen
  (`poster-store/tests/schema.rs` pins their fingerprints); schema changes are new migrations.
- **A request row stores its template as JSON, the single source of truth.** The tree reads
  `name`, `method` and `position` through generated columns, so it never decodes bodies;
  rename and reorder use `json_set`. Why: one copy of every field, and new template fields
  need no migration (serde defaults).
- **Variables are ordered JSON objects** (collections, environments, the single `globals` row).
  No environment may be named `Global`, which names the globals scope. Why: order is kept for
  display and export, and saving a scope replaces it whole, as in the Python app.
- **History rows carry summary columns** (name, method, URL, status, outcome, elapsed) next to
  the JSON payload. Why: the list must not decode multi-megabyte payloads; details load one.
  Retention deletes oldest first by count and total payload size; a record larger than the
  whole budget is refused.
- **Redirects are followed by `poster-http`, not reqwest, with httpx's rules:** at most 20;
  303 becomes GET (except HEAD), 301/302 turn POST into a bodiless GET; 307/308 keep the method
  and replay the body; `Authorization` is dropped across origins (except an http→https upgrade
  of the same host); explicit `Cookie` headers are never carried to the next location. Why:
  reqwest cannot replay streamed uploads, and users expect the Python app's behavior.
- **Digest is answered once per hop** (MD5, SHA, SHA-256, SHA-512, `-sess`, qop `auth`; not
  `auth-int`), rebuilding the body for the retry. Verified against the RFC 2617/7616 vectors.
- **Uploads stream with chunked transfer encoding** (no `Content-Length`), as httpx does, and
  are reopened for every attempt. Multipart parts match the Python app byte for byte
  (`parity/multipart.json`).
- **Timeouts apply to connecting and to each read**, not to the whole exchange, as in httpx: a
  slow but steady download is not cut off.
- **Response headers arrive grouped by name and lowercased, and the reason phrase is the
  standard one for the status.** Why: hyper exposes neither the received order across names
  nor custom reason phrases. Values of one name keep their order.
- **`User-Agent: Poster/<version>`** unless the request sets one; environment and system proxies
  are never used, only the configured one.
- **An execution reads its settings, variables and collection in one database command.**
  The database thread runs commands in order, so this is the snapshot taken before the first
  await: an environment switch queued before Send applies, a later one cannot change a running
  request. The variable inspector uses the same function (`executor::inputs`).
- **Requests that cannot be sent are not history.** Unresolved variables, a bad URL, auth or
  cookie conflicts are reported and nothing is recorded (the Python app recorded URL errors as
  transport errors). Cancelled and failed sends are recorded with their outcome. A failed
  history write keeps the response and says so (`history_error`).
- **The saver is plain UI-side state** (`poster_app::saver`): one write in flight at a time,
  the newest snapshot per request, failures kept for retry or explicit discard. The UI runs
  the writes through the bridge.
- **UI architecture: widgets emit actions, background work returns messages.** Widgets edit the
  editor's form state in place and push `Action`s; the app applies them after the frame and runs
  database and network work through the bridge; results come back as `Message`s drained at the
  start of the next frame. Every navigation bumps a generation and results carry the generation
  they started under, so a late result never lands on another request. Why: immediate-mode
  drawing stays free of IO, and the workflow (guard, saves, close) is testable as state.
- **Every navigation that replaces the editor goes through one guard**: clean → proceed; saved
  request with autosave on → save first (failure asks Retry/Discard/Cancel); otherwise ask
  Save/Discard/Cancel. Closing uses the same guard after background work drained, then writes
  UI state (layout, last selection, window) and closes; any failure keeps the window open.
- **The tree is refreshed after a save that changed a request's method or name** (the Python app
  left it stale).
- **UI tests drive the real app through AccessKit** (`egui_kittest`): every widget a test or a
  screen reader needs has an accessible name, set with `widgets::name` when no visible label
  exists. Only characters present in egui's bundled fonts are used; kittest fails on a missing
  glyph, which would otherwise render as a box. `cargo run -p poster-ui --example screenshot --
  <dir>` renders the app offscreen (wgpu) for looking at layouts.
- **Variables in the editor:** fields mark `{{name}}` tokens with the Python app's highlight
  pattern (`\{\{([^{}\n]+)\}\}`, stricter than the resolution pattern): defined names in the
  accent color, unknown ones in the error color. Typing `{{` suggests names with their scope and
  source, never values. Values appear only in the inspector, opened explicitly (click on a token
  or Alt+Enter); from a password field they stay masked until "Reveal". The inspector and
  execution share `executor::inputs`, so it explains exactly what a send would use.
- **The Variables screen edits one scope at a time** (Global, an environment, a collection),
  independently of the active execution environment, with its own unsaved-changes guard for
  scope changes, reloads, environment management, "Open definition" and closing. Closing
  resolves the request guard first, then the variables guard, then writes UI state.
- **Release profile keeps `panic = "unwind"`.** Why: a panic in one background task is caught by
  tokio and must not abort the app.
- **Compatibility contract: native `.poster.json` version 1.** Why: it is how users move their
  collections from the Python app. Python exports must round-trip byte for byte
  (`poster-transfer/tests/native_golden.rs`). Bytes are base64 with the URL-safe alphabet and
  padding, as Pydantic writes them; both alphabets are read.
- **Parity with the Python app is tested against fixtures produced by running the Python code**
  (`tests/fixtures/python/generate.py`), never against expectations written by hand. Why: a
  port written from reading code copies the reader's misreadings. Change a fixture only by
  regenerating it; a deliberate divergence is asserted in its own test with the reason.
  Current divergences: no empty `Cookie:` header when every cookie is disabled.
- **Toolchain pinned in `rust-toolchain.toml`, `Cargo.lock` committed, CI uses `--locked`.**
  Why: every machine and CI run builds the same thing.

Open questions are in `PLAN.md` ("Open questions"); their outcomes are recorded here.

## Enforcement gaps

Rules above that no tool checks yet. Close a gap together with the first code it applies to.

- No blocking calls in `poster-ui`: `disallowed-methods` in its own `clippy.toml`.
