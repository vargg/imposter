# Imposter agent manual

Imposter is a local desktop HTTP API client written in Rust (egui/eframe), with a command
line in the same binary.

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
- **This file holds only repository-wide principles and rules; subsystem decisions live in
  `docs/decisions/`.** Keep this file focused on universal invariants that every task needs.
  Feature-specific decisions and rationale belong in `docs/decisions/<subsystem>.md`, not
  appended here.
  Why: agent harnesses inject this file into the system prompt on every turn; keeping it
  lean prevents prompt context bloat and avoids silent truncation by the LLM runner.
- **This file records engineering rules and core decisions.** Planned guarantees are not shipped
  behavior. Foundational decisions live under [Foundational decisions](#foundational-decisions)
  below, while subsystem-specific decisions are recorded in [docs/decisions/](docs/decisions/).
  When work settles an open question, record the outcome and its reason in the relevant file
  in the same commit. User-facing documentation is `README.md`.
- **Changing a rule or decision updates this file or its decision file in the same commit.**
  Replace a superseded decision rather than keeping both; mention the rejected option in the
  reason when that stops someone from proposing it again. Git keeps the history.

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
- **Tests assert our own wording, never the operating system's.** System error text
  differs by OS ("Connection refused" vs Windows' "actively refused it"); check the part
  every OS shares ("os error") or the error kind.
  Why: a test that passed on macOS and Linux failed only in the Windows CI job.
- **Tests are deterministic and independent.** No fixed sleeps: wait on channels or events with a
  deadline. No external network, no personal data directory (`IMPOSTER_DATA_DIR` points at a temp
  dir). Every test owns and cleans up its resources.
  Why: flaky tests train people to ignore red CI.
- **Report only what was run.** A change is done when `scripts/check.sh` passes locally; say
  which checks ran and which did not (for example, CI on other OSes). Commit only after it
  passed: the script stops at the first failure, so never chain a commit after it with `;`.

```sh
scripts/check.sh     # Python tooling tests, fmt, clippy, Rust tests under coverage floors; as CI
cargo test -p <crate>
cargo run -p imposter
```

Coverage must stay at or above 85% of lines and 70% of regions (stable Rust has no branch
coverage; regions are the closest measure). `main.rs` only opens the window; the
installed-binary smoke test covers it. `imposter-testkit` is test code and would inflate the
numbers, so both are excluded. Each logical change is its own commit.

## Architecture principles

- **Dependency direction: `core` <- `http`, `store`, `transfer`, `script` <- `app` <- `ui`, `cli` <- `imposter`.**
  Never add an edge against it. `cli` (the command line) is a sibling of `ui`: neither
  depends on the other, and anything both need (the result schema) lives below them.
  Why: each layer stays testable without the ones above it, and `ui` can change without touching
  the domain.
  Enforced by: `crates/imposter/tests/architecture.rs` (every manifest against the allowed
  graph). `imposter-ui` and `imposter-cli` reach the store and transfer crates only through
  `imposter-app`.
- **`imposter-core` does no IO** (no files, network, database, clock or environment).
  Why: the domain rules (variables, settings resolution, validation) can then be tested
  exhaustively, quickly and deterministically.
  Enforced by: `crates/imposter-core/clippy.toml` (`disallowed-types` / `disallowed-methods` for
  files, sockets, clocks, environment and sleeping); tests that read fixtures allow it locally.
- **The UI thread never blocks or awaits.** Background work starts through the app handle and
  its results come back over a channel drained at the start of each frame.
  Why: egui redraws every frame on that thread; any block freezes the whole window.
- **One thread owns the SQLite connection and the collections folder**; every other part
  sends it commands, and one command is one transaction over both (`store::Tx`): the
  folder's changed files are written just before SQLite commits, and a file that cannot be
  written rolls the command back.
  Why: writes are serialized without locks, a failed command rolls back as a unit, and the
  database never gets ahead of the files. The files are small, so writing them on the
  database thread is accepted.
  Another process (the command line beside the app) has its own connection; every
  connection waits up to `schema::BUSY_TIMEOUT` (5 s) for the other's write instead of
  failing at once with "database is locked".
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
- **No `dbg!`, `todo!` or `unimplemented!` in committed code.** Do the work, or open an issue.
  Enforced by: `clippy::dbg_macro`, `clippy::todo`, `clippy::unimplemented`.
- **Test support lives in `imposter-testkit`, used only as a dev-dependency.** Enforced by the
  architecture test.
- **Errors: `thiserror` enums in library crates; `anyhow` only in the `imposter` binary.**
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
  Enforced by: `imposter-http/tests/logging.rs` (trace-level capture, dependencies included).
  Transport error messages drop the URL, which can carry `user:password@`.
- **The app's data is `~/.imposter/imposter.db` (personal data) and a collections folder,
  by default `collections/` beside it; `IMPOSTER_DATA_DIR` moves both.**
  Why: one place to back up, and tests and second profiles point elsewhere without touching
  it. The folder can be set elsewhere (a repository); the profile remembers it
  (`files::workspace::FOLDER_KEY`).

## Decisions by subsystem

Detailed decisions and their reasons are organized by subsystem in `docs/decisions/`.
**When modifying a subsystem, consult its decision file before making changes:**

- **[UI Architecture](docs/decisions/ui.md)** (`imposter-ui`):
  egui/eframe (glow), virtualized response rows, 256 KiB preview limits, table headers,
  JSON navigation & folding, dark/light themes, Inter & JetBrains Mono fonts, Phosphor icons,
  Compact/Comfortable density, accent color conventions, request bar & body editor
  controls, forms, modal dialogs, left navigation rail, and navigation guards.
- **[HTTP & Transport](docs/decisions/http.md)** (`imposter-http`):
  rustls (`ring` provider), PKCS#8/PKCS#1/SEC1 client keys, HTTP/1.1-only, server-sent
  events (SSE), wire header ordering, in-crate redirect handling, RFC 6265 cookie jar,
  API-key/OAuth/Digest auth schemes, chunked streaming uploads, and timeouts.
- **[Storage & State](docs/decisions/store.md)** (`imposter-store`, `imposter-core`):
  SQLite migrations over `user_version`, file-backed collections and environments (no
  database copy), ordered variable maps, global variables, local vs shared values,
  environment colors, history schema, secret references (`{{secret:name}}`), and
  `~/.imposter` data directory conventions.
- **[Scripts & Mocks](docs/decisions/scripts-and-mocks.md)** (`imposter-script`, `imposter-http`):
  Rhai script engine, request pre/post scripts around send, data-driven checks vs scripts,
  captures, `$function` generated values, loopback mock server lifecycle, and scripted
  mock routes.
- **[Transfer & Formats](docs/decisions/transfer.md)** (`imposter-transfer`):
  Native format (`.imposter.json` v2/v3/v4), collections folder format, offline
  schema-validated Postman v2.1 import, OpenAPI 3.0/3.1/3.2 import, atomic opt-in exports,
  and curl command translation.
- **[CLI & MCP](docs/decisions/cli-and-mcp.md)** (`imposter-cli`, `imposter`):
  `imposter send`, `list`, `show`, `envs`, and `run` subcommands, pure `executor::run`
  engine, headless result schemas, per-run literal `--var`, coding agent `imposter skill`,
  and stdio MCP server experiment.
- **[Platform & Release](docs/decisions/platform.md)** (`imposter`):
  Rust only (one self-contained binary per OS, no webview/JVM/C++), `cargo-packager`,
  automated binary smoke testing, code-drawn app icons, `panic = "unwind"`, golden test
  fixtures, toolchain pinning, and branding (`I’mposter`/`Imposter`).

## Foundational decisions

- **Rust only, one native binary per OS; no webview, JVM or own C/C++.** Why: one file per OS
  to distribute, with no interpreter or runtime to install. Speed and memory matter, but come
  second.
- **GUI: egui + eframe (glow backend); Iced not needed.** Why: the most mature tables, trees and
  panels, custom text layout for `{{var}}` highlighting, and a UI test harness (`egui_kittest`,
  headless, no GPU). Slint was rejected for limited text-editing control and license
  conditions; gpui for an unstable API. Accepted trade-off: custom dark and light themes (accent
  `#ff6c37`), not a native look. Confirmed in stage 0: release binary 5.6 MiB (LTO, stripped,
  macOS arm64); first frame about 225 ms after process start (24 ms after app creation).
- **HTTP/1.1 only.** Why: HTTP/2 adds complex multiplexing, framing, and dependencies without
  tangible benefit for a desktop debug client where requests are inspected individually.
- **One engine runs every request: `executor::run`, which reads and writes nothing.** Why:
  pure execution decoupled from persistence and presentation keeps the core testable and
  allows headless and GUI runners to share the exact same execution semantics.
- **Collections and environments are files; there is no database copy.** Files in the
  collections folder are the single source of truth for definitions. SQLite stores only
  local state, history, and cached indexes. Why: easy git tracking, team collaboration,
  and zero sync drift.

## Enforcement gaps

Rules above that no tool checks yet. Close a gap together with the first code it applies to.

None at the moment.
