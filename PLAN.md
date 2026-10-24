# Poster (Rust): project plan

A local desktop HTTP API client, rewritten in a single language (Rust) from the Python/PyQt6
prototype at `~/dev/pystman`. This plan is standalone: it should be enough to start the project
in a fresh session.

## Why

The Python version works, but it is hard to distribute: users need a specific interpreter (3.13)
and a resolved dependency environment. Goal: **one self-contained native binary per OS, with no
runtime to install and nothing to resolve.** Speed and memory are secondary (the Python version is
already fine), but should not get worse.

Choices already made:
- **Rust only.** No HTML/TS frontend, no webview, no JVM, no C/C++ code of our own.
- New project, not an in-place migration. The Python app stays as the reference implementation.
- `~/dev/pystman/AGENTS.md` is the behavioral spec. Port behaviors, not code structure
  (no DI framework, no qasync patterns).

## Migration path for users

The Rust app reads and writes the existing **native `.poster.json` format, version 1**, so users can
export collections from the Python app and import them here. No direct access to the old SQLite
database (`~/.poster/app-v1.db`) is needed. The Rust app uses its own database path
(`~/.poster/poster.db`, see Persistence) and never touches the Python files.

Reusable assets from `~/dev/pystman`:
- `src/poster/adapters/schemas/postman-v2.1.json` (pinned official schema) and its README
- `tests/fixtures/postman-v2.1.json`, plus `.poster.json` files exported from the Python app (golden compatibility tests)
- `docs/validation.md` (manual walkthrough checklist) and `docs/testing.md` (test philosophy)

---

## Tech stack

| Concern | Choice | Notes |
|---|---|---|
| GUI | **egui + eframe** (glow/wgpu backend) | immediate mode; good for dev tools; custom text layouters for `{{var}}` highlighting |
| Panels / splitters | egui `SidePanel`/`TopBottomPanel` + **egui_tiles** (or hand-rolled split) | must support switching between horizontal and vertical layout |
| Tables | `egui_extras::TableBuilder` | key/value editors (headers, queries, form, cookies) |
| Syntax highlighting | **syntect** (via `egui_extras` syntect feature or own layouter) | JSON/XML/HTML response preview, capped at 256 KiB |
| Async runtime | **tokio** (multi-thread, in background threads) | UI thread never awaits; see "Concurrency model" |
| HTTP | **reqwest** (rustls, stream) on hyper | exact header-order requirement must be validated in stage 0 |
| TLS | **rustls** + `rustls-platform-verifier` (default roots) | custom CA bundle replaces default roots; `pkcs8` (encryption feature) for encrypted keys |
| Digest auth | in-house module (`md-5`, `sha2`) | replay must rebuild streamed upload bodies |
| SQLite | **rusqlite** (`bundled` feature) on a dedicated DB thread | no ORM; plain SQL + typed row mappers |
| Migrations | **rusqlite_migration** (embedded SQL) | SQLite backup API before upgrading |
| Serialization | **serde** + serde_json, `#[serde(deny_unknown_fields)]`, base64 for bytes | replaces Pydantic DTOs |
| JSON Schema | **jsonschema** crate, schema embedded with `include_str!` | offline Postman validation |
| Errors | `thiserror` (libraries), `anyhow` only at the app edge | |
| Logging | `tracing` + `tracing-subscriber` | must never log credentials or bodies |
| File dialogs | **rfd** | native open/save dialogs |
| Tests | `cargo test`, **egui_kittest** (UI, AccessKit queries), tokio loopback servers, **rcgen** (test certs) | |
| Coverage | **cargo-llvm-cov** | same floors as Python: 85% lines / 70% branches (branches need nightly; else enforce regions) |
| Lint/format | `cargo fmt --check`, `cargo clippy -- -D warnings` (pedantic, selective allows) | |
| Packaging | **cargo-packager** (or cargo-bundle) | .app/.dmg, .msi/.exe, AppImage/.deb |

Minimum Rust: current stable, pinned in `rust-toolchain.toml`. `Cargo.lock` is committed.

### Why egui over the alternatives
- **Slint**: declarative and polished, but limited control over text editing/highlighting, and the license has conditions (GPL, or royalty-free with attribution).
- **Iced**: good `text_editor` widget, but tables, trees and splitters are weaker, and the API changes often.
- **gpui** (Zed): very fast, but its API is unstable and its documentation thin.
- **egui**: the most mature widgets for tables, trees and panels; custom text layout; a UI test harness (kittest).
  Trade-off: it doesn't look native. A dark theme with the `#ff6c37` accent works well with it.
  Stage 0 confirms the fit. **Fallback: Iced.**

---

## Workspace layout

```
poster/
  Cargo.toml                # workspace, shared lints/profiles
  rust-toolchain.toml
  AGENTS.md                 # agent manual for this repo (ported conventions)
  PLAN.md
  crates/
    poster-core/            # pure domain: DTOs, settings resolution, variables, URL/query/header logic. No IO.
    poster-http/            # transport: request preparation, reqwest client, TLS, auth (Basic/Bearer/Digest),
                            # replayable uploads, capped streaming, cancellation
    poster-store/           # SQLite: schema, migrations, backup, repositories, DB actor thread
    poster-transfer/        # native .poster.json v1 + Postman v2.1 conversion/validation, import preview
    poster-app/             # services + runtime glue: executor, saver, history retention, task registry
    poster-ui/              # egui widgets and screens (library, testable with kittest)
    poster/                 # binary: main(), eframe setup, logging, paths
  tests/fixtures/           # shared golden documents (native, Postman)
  docs/                     # testing.md, validation.md, release.md
```

Dependency direction: `core` ← `http`, `store`, `transfer` ← `app` ← `ui` ← `poster` (bin).
`core` depends on nothing IO-related, so it is easy to test exhaustively.

---

## Concurrency model (replaces qasync + TaskManager)

- The eframe UI thread owns all UI state. It never blocks and never awaits.
- A tokio runtime runs on background threads. The UI starts work through an `AppHandle`
  (`spawn(op) -> TaskId`); results come back through an `mpsc` channel, which is drained at the
  start of each frame. `ctx.request_repaint()` is called when a result arrives.
- **TaskRegistry** (the equivalent of TaskManager): owns `JoinHandle`s plus a `CancellationToken`
  per task. On shutdown it cancels and drains everything.
- **Generation guards**: each selection or execution carries a generation number; stale results are dropped.
- One HTTP execution at a time; Send toggles to Cancel (cancel = token + dropping the request future).
- **DB actor**: one thread owns the `rusqlite::Connection` and processes commands in order.
  This serializes writes the same way `RequestSaver` does today. Transactions are the actor's job:
  a command = a unit of work.
- File IO (upload chunks, exports) runs in `spawn_blocking` or on the tokio fs APIs, never on the UI thread.

---

## Behavioral spec to preserve (ported from pystman AGENTS.md)

### Request model
- `RequestTemplate` (portable) has: name, description, method (custom tokens keep their case), url,
  `auth` {none|basic|digest|bearer}, ordered `queries`/`headers`/`cookies` (name, value, enabled; duplicates and
  empty values allowed), body bytes + `body_type`
  {none, json, plain, text, form-data, urlencoded, multipart, binary}, and `options` (nullable overrides).
- `Request` adds identity: id, collection_id, folder_id?, collection_variables_snapshot.
- Upload DTOs store **file paths, never contents**. Multipart items: text or file, content type, filename.
- Options: `None` inherits the application default; an explicit empty string disables an inherited path or proxy.
- Defaults: 30 s timeout, follow redirects, verify TLS, attach cookies, no proxy, no env proxy
  (`trust_env=false`), 5 MiB capture limit, autosave on, history 1,000 entries / 250 MiB.

### Execution
- Validate before sending: absolute http(s) URL; auth and an `Authorization` header conflict; cookies and a `Cookie` header conflict;
  a key without a certificate is an error; no unresolved `{{variables}}`.
- Queries: the editor rows win; never merge them with a parsed URL query.
- Preserve ordered duplicate and empty headers, queries and form fields **on the wire**.
- Streaming capture with a limit (default 5 MiB), with a truncation flag. Keep duplicate response headers in order.
  Format the preview only up to 256 KiB.
- Replayable uploads read 64 KiB chunks off the UI thread, including on Digest retries.
- Snapshot environment identity and variable values **before** the first await of an execution.

### Variables
- Precedence: global → collection → active environment; empty overrides count as values.
- Resolve in text and configuration fields and in file paths; never inside uploaded file contents.
- Inspection uses the same snapshot code as execution (with provenance). Values are shown only on explicit
  inspection; masked values have Reveal; never in autocomplete rows or passive tooltips.
- Highlighting never changes the text, undo history, selection or dirty state.

### Collections
- Collections → optional nested folders → requests (requests can sit at a collection's root).
- Moves stay inside one collection and reject cycles. Load the tree with bounded queries.
- IDs are never reused (`AUTOINCREMENT`).

### History
- Executions are independent records with no foreign key to the source request. They capture the template, resolved request,
  effective settings, UTC timestamp, monotonic duration, environment identity, collection variable context,
  response or outcome, and truncation. Bytes are stored as base64 in JSON. Upload contents are never archived.
- Restoring creates an unsaved draft and never sends. Retention removes oldest first. A failed history write must leave
  the response visible. Logs omit credentials and bodies.

### Saving and closing
- Per-request dirty tracking against a persisted baseline; reverting an edit clears dirty state.
- Autosave debounce lives in the UI layer; the saver serializes writes and keeps failures.
  With autosave off, Save is explicit.
- Replacing the request (navigation) goes through a guard: Save/Discard/Cancel (manual mode) or
  Retry/Discard/Cancel (failed autosave). Ordinary tab switching keeps edits.
- Close: stop work → drain tasks → flush drafts and UI state → close the DB. Failed saves keep the window
  open for retry or explicit discard.

### Transfer
- Native `.poster.json` v1 is the full-fidelity contract: strict (`deny_unknown_fields` at every level),
  ASCII method tokens validated, max 20 MiB, unknown versions rejected.
- Import: validate → preview → commit atomically with fresh IDs and numbered names; global conflicts keep
  existing values and are reported. Environment and global imports never overwrite.
- Export: flush autosave first (or overlay the current draft when autosave is off); environments and globals are opt-in;
  write to a temp file in the same directory, then rename atomically, off the UI thread.
- Postman v2.1 is a subset: flatten inherited Basic/Bearer/Digest; preview scripts, unsupported auth,
  dynamic variables and examples; execution-affecting losses need explicit consent. Companion exports
  are published together into a new directory.

### UI
- Single dark theme, accent `#ff6c37`, defined in one theme module (the equivalent of `GLOBAL_STYLE`).
  Semantic states (error, warning, ok) come from one palette; test contrast.
- Adaptive workspace: sidebar plus a request/response split that changes orientation with the window width.
  Editors keep their state through reflow. Layout proportions are stored (versioned) in SQLite;
  Reset affects layout only.
- Response viewer: Copy/Find/Wrap on the bounded displayed text; search shortcuts scoped to the viewer;
  distinguish an empty body from no response; keep the previous response while a new run is pending.
- Must stay usable at 1000×650 (multipart editor included).
- Variables tab and Settings have independent dirty scopes and close guards.

---

## Persistence

- Path: `~/.poster/poster.db` (separate from the Python app's `app.db`/`app-v1.db`). Override with
  `POSTER_DATA_DIR` for tests and smoke runs.
- `PRAGMA foreign_keys=ON`, WAL journal, `AUTOINCREMENT` ids.
- Tables: collections, folders, requests, environments (namespaces), variables, settings (key/JSON),
  ui_state (key/JSON), executions.
- Back up (SQLite backup API) to `poster.db.bak-<version>` before running pending migrations.
  Released migrations are frozen. Batch migrations validate `foreign_key_check` before commit.

---

## Stages

Each stage ends green on: `cargo fmt --check`, `cargo clippy --all-targets -D warnings`,
`cargo test --workspace`, the coverage floor (from stage 2 on), and is committed separately.

**Stage 0: foundation and risk spike (go/no-go for egui)**
- Workspace skeleton, toolchain pin, CI matrix (macOS, Windows, Linux), `AGENTS.md`.
- eframe window with the theme, plus the tokio bridge (spawn, result channel, repaint, cancel).
- Spike tests settling the open questions below: exact header order on the wire,
  5 MiB response in the viewer with highlighting (256 KiB) and search, encrypted PKCS#8 client key, release binary size and cold start.
- Exit: decisions recorded in AGENTS.md; egui confirmed or switched to Iced.

**Stage 1: core domain** (`poster-core`)
- DTOs with serde, settings resolution (`None` inherits, empty disables), variable parsing and resolution with
  provenance, URL/query/header preparation, validation errors.
- Golden test: deserialize and re-serialize native v1 documents exported from the Python app, byte for byte at the value level.

**Stage 2: persistence** (`poster-store`)
- Schema, migrations with backup, DB actor, repositories (collections, folders with cycle checks, requests,
  environments and variables, settings, ui_state, executions with retention).
- Tests: a temp DB per test, rollback on mid-transaction failure, IDs never reused, FK checks.

**Stage 3: HTTP transport** (`poster-http`)
- Client per execution policy (timeout, redirects, verify, proxy, `no_proxy`/no env), custom CA, client cert and key
  (encrypted PKCS#8), Basic/Bearer/Digest (MD5, SHA-256, qop auth, with replay), bodies of every type, replayable multipart and binary uploads,
  capped streaming, cancellation.
- Loopback tests check the raw received bytes (header order and duplicates), the Digest exchange, TLS with a
  CA and client certs (rcgen), disconnects, truncation, and cancellation mid-stream.

**Stage 4: services** (`poster-app`)
- Executor (snapshot → resolve → validate → send → history write without losing the response),
  saver with baselines and pending snapshots, task registry, shutdown sequence.

**Stage 5: UI shell and request workflow** (`poster-ui`)
- Workspace layout (adaptive split, persisted proportions), collections tree (CRUD, rename, move, drag&drop),
  request editor (method combo, URL, tabs: params/headers/cookies/auth/body/options), response viewer,
  Send/Cancel, save feedback, navigation guard, close flow.
- kittest UI tests for the full workflow with real persistence in a temp dir.

**Stage 6: variables and environments**
- Variables tab (global, collection, environments) with its own dirty scope, environment selector,
  `{{var}}` highlighting and autocomplete, inspector with Reveal.

**Stage 7: history**
- History list, details, restore-as-draft, clear, retention settings.

**Stage 8: transfer**
- Native import/export, Postman import/export with schema validation, preview dialog, loss consent,
  companion exports, atomic file writes.

**Stage 9: settings, polish, accessibility**
- Settings screen with a dirty baseline, keyboard shortcuts (Send, Save, Find, New), contrast tests,
  manual walkthrough (`docs/validation.md`) on all three OSes.

**Stage 10: distribution**
- Release profile (`lto`, `codegen-units=1`, `strip`), cargo-packager configs, macOS signing and
  notarization, Windows signing (optional), Linux AppImage/.deb, GitHub Releases from CI tags.
- Smoke test: launch the packaged binary with a temp `POSTER_DATA_DIR`, send a request to a local server, exit cleanly.

---

## Open questions (settled in stage 0)

1. **Exact cross-name header order.** reqwest/hyper store headers in `HeaderMap`, which groups values
   by name (A, B, A becomes A, A, B). The Python app sends exactly in editor order.
   Options: (a) check hyper's original-order/case extensions; (b) accept order *within the same name* only,
   and record that divergence in the spec; (c) a thin HTTP/1.1 writer for the request head over hyper's connection.
   Prefer (a), then (b).
2. **Legacy encrypted PEM keys** (`Proc-Type: 4,ENCRYPTED`): rustls and `pkcs8` don't support them. Either
   support PKCS#8 only and show a clear error suggesting `openssl pkcs8 -topk8`, or add a converter. Default: PKCS#8 only.
3. **Large text editing in egui**: the JSON body editor with highlighting needs to stay responsive at around 1 MiB;
   otherwise, highlight only the visible range or cap highlighting.
4. **Native look vs a custom theme**: accepted as custom (dark, `#ff6c37`).

## Out of scope (for now)
Scripts (pre-request/tests), WebSocket/gRPC/GraphQL-specific UI, cloud sync, multiple themes,
HTTP/2-specific settings, cookie jar persistence beyond explicit cookies.

## Verification of the whole project
- Automated: unit, integration and UI tests per the stages; coverage floors in CI; installed-binary smoke test.
- Compatibility: export from the Python app, import into the Rust app, export again, and check the result is equivalent (golden fixtures).
- Manual: `docs/validation.md` walkthrough on macOS, Windows and Linux before each release.
