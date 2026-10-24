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
- **Report only what was run.** A change is done when the commands below pass locally; say which
  checks ran and which did not (for example, CI on other OSes).

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

From stage 2 on, coverage must also stay at or above 85% lines (`cargo-llvm-cov`). Each stage in
`PLAN.md` is its own commit.

## Architecture principles

- **Dependency direction: `core` <- `http`, `store`, `transfer` <- `app` <- `ui` <- `poster`.**
  Never add an edge against it.
  Why: each layer stays testable without the ones above it, and `ui` can change without touching
  the domain.
  Enforced by: Cargo rejects dependency cycles. The full allowed graph is a gap.
- **`poster-core` does no IO** (no files, network, database, clock or environment).
  Why: the domain rules (variables, settings resolution, validation) can then be tested
  exhaustively, quickly and deterministically.
  Enforced by: `poster-core` has no IO dependencies. Use of `std::fs`/`std::net` is a gap.
- **The UI thread never blocks or awaits.** Background work starts through the app handle and
  its results come back over a channel drained at the start of each frame.
  Why: egui redraws every frame on that thread; any block freezes the whole window.
- **One thread owns the SQLite connection**; every other part sends it commands, and one
  command is one transaction.
  Why: writes are serialized without locks, and a failed command rolls back as a unit.
- **Dependencies are declared once** in the root `[workspace.dependencies]`; crates use
  `name.workspace = true`.
  Why: one version per dependency across the workspace, upgraded in one place.
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
- **Errors: `thiserror` enums in library crates; `anyhow` only in the `poster` binary.**
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
  Enforced by: tests that capture `tracing` output (from stage 3 on).
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
- **Release profile keeps `panic = "unwind"`.** Why: a panic in one background task is caught by
  tokio and must not abort the app.
- **Compatibility contract: native `.poster.json` version 1.** Why: it is how users move their
  collections from the Python app; golden tests with Python exports guard it.
- **Toolchain pinned in `rust-toolchain.toml`, `Cargo.lock` committed, CI uses `--locked`.**
  Why: every machine and CI run builds the same thing.

Open questions are in `PLAN.md` ("Open questions"); their outcomes are recorded here.

## Enforcement gaps

Rules above that no tool checks yet. Close a gap together with the first code it applies to.

- Allowed crate dependency graph: a workspace test over `cargo metadata`.
- No IO in `poster-core` and no blocking calls in `poster-ui`: `disallowed_methods` /
  `disallowed_types` in a per-crate `clippy.toml`.
- `thiserror`/`anyhow` placement: the same dependency-graph test.
