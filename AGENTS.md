# Imposter agent manual

Imposter is a local desktop HTTP API client written in Rust (egui/eframe). It rewrites the
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
- **This file is the durable record; plans and roadmaps are not kept in the repository.** When
  work settles an open question, record the outcome and its reason under
  [Decisions](#decisions) in the same commit. User-facing documentation is `README.md`.
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
  deadline. No external network, no personal data directory (`IMPOSTER_DATA_DIR` points at a temp
  dir). Every test owns and cleans up its resources.
  Why: flaky tests train people to ignore red CI.
- **Report only what was run.** A change is done when `scripts/check.sh` passes locally; say
  which checks ran and which did not (for example, CI on other OSes). Commit only after it
  passed: the script stops at the first failure, so never chain a commit after it with `;`.

```sh
scripts/check.sh     # fmt, clippy (-D warnings), all tests under the coverage floor; as CI
cargo test -p <crate>
cargo run -p imposter
```

Coverage must stay at or above 85% of lines and 70% of regions (stable Rust has no branch
coverage; regions are the closest measure). `main.rs` only opens the window; the
installed-binary smoke test covers it. `imposter-testkit` is test code and would inflate the
numbers, so both are excluded. Each logical change is its own commit.

## Architecture principles

- **Dependency direction: `core` <- `http`, `store`, `transfer` <- `app` <- `ui` <- `imposter`.**
  Never add an edge against it.
  Why: each layer stays testable without the ones above it, and `ui` can change without touching
  the domain.
  Enforced by: `crates/imposter/tests/architecture.rs` (every manifest against the allowed
  graph). `imposter-ui` reaches the store and transfer crates only through `imposter-app`.
- **`imposter-core` does no IO** (no files, network, database, clock or environment).
  Why: the domain rules (variables, settings resolution, validation) can then be tested
  exhaustively, quickly and deterministically.
  Enforced by: `crates/imposter-core/clippy.toml` (`disallowed-types` / `disallowed-methods` for
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
- **The app's data is `~/.imposter/imposter.db`, overridable with `IMPOSTER_DATA_DIR`. Never read or
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
  not. Measured by `cargo run --release -p imposter-ui --example text_perf` (M-series Mac).
- **JSON navigation keeps the virtualized rows.** Folding hides row ranges computed from an
  outline of the formatted text (`viewer::Outline`: path, depth and closing line for every
  line), so nothing is laid out that is not visible. A row with a fold marker is exactly as
  tall as a plain row, because the scroll area places rows by a fixed height. Paths are
  RFC 9535 JSONPath, written one way everywhere (`select::member`: `.name` when the shorthand
  allows it, `['name']` otherwise), so a copied path works in a filter, and later in checks
  and extraction. JSONPath comes from `serde_json_path` (pure Rust); a hand-written subset
  would differ from what users read in the RFC.
- **The response filter runs over the whole captured body in the background**, parsed once
  per response and kept for later queries; one run is in flight at a time and the newest query
  follows. A late result for a replaced response is dropped. Results are re-serialized, so
  numbers keep their value but not their spelling (`1.50` becomes `1.5`); the formatted body,
  "Copy value" and the preview keep tokens exactly as received. A truncated body is refused
  rather than reported as invalid JSON.
- **Highlighting: small hand-written tokenizers, not syntect.** Why: JSON highlighting of
  256 KiB takes about 2 ms by hand and about 100 ms with syntect, which is also a large
  dependency.
- **Body editor: egui `TextEdit`; `{{var}}`/JSON highlighting only up to 64 KiB.** Why: per
  interactive frame the editor costs about 4 ms at 64 KiB, 24 ms at 256 KiB and 130 ms at 1 MiB
  (plain), and highlighting adds about 30%. Larger bodies stay editable but slow; a custom
  virtualized editor would be needed to fix that and is out of scope.
- **TLS: rustls with the `ring` provider, configured by us** (`imposter_http::tls`) and passed to
  every reqwest client with `use_preconfigured_tls`, including plain-HTTP clients (reqwest
  panics without a provider). Why: no OpenSSL, cmake or system library on any OS; full control
  of roots, verification and client keys. A custom CA bundle replaces the platform roots.
- **Client keys: PKCS#8 (plain or encrypted, PBES2), PKCS#1 and SEC1. Legacy encrypted PEM
  (`Proc-Type: 4,ENCRYPTED`) is rejected** with advice to run `openssl pkcs8 -topk8`. Why:
  rustls and RustCrypto do not implement the legacy OpenSSL scheme. PBKDF2-HMAC-SHA1 is enabled
  because LibreSSL and older OpenSSL use it by default (fixtures in `imposter-http/tests/fixtures`).
- **HTTP/1.1 only.** Why: matches the Python app (httpx default) and keeps the wire format the
  tests pin; HTTP/2 settings are out of scope.
- **Header order: values of one name keep their order; names go out in first-occurrence order,
  lowercased.** So `A, B, A` is sent as `a, a, b`. Why: hyper stores headers in a `HeaderMap`
  and its order-preserving extensions are private; writing our own HTTP/1.1 client is not
  worth the risk. Pinned by `imposter-http/tests/header_order.rs`.
- **Migrations: our own runner over `user_version`, not `rusqlite_migration`.** Each migration
  runs in its own transaction and must pass `foreign_key_check`; an existing database is copied
  to `imposter.db.bak-<version>` with SQLite's backup API first; a database from a newer app is
  refused. Why: about 40 lines, no second rusqlite version to keep in step, and the backup
  happens exactly before the first change. Released migrations are frozen
  (`imposter-store/tests/schema.rs` pins their fingerprints); schema changes are new migrations.
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
- **Redirects are followed by `imposter-http`, not reqwest, with httpx's rules:** at most 20;
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
- **`User-Agent: Imposter/<version>`** unless the request sets one; environment and system proxies
  are never used, only the configured one.
- **An execution reads its settings, variables and collection in one database command.**
  The database thread runs commands in order, so this is the snapshot taken before the first
  await: an environment switch queued before Send applies, a later one cannot change a running
  request. The variable inspector uses the same function (`executor::inputs`).
- **Requests that cannot be sent are not history.** Unresolved variables, a bad URL, auth or
  cookie conflicts are reported and nothing is recorded (the Python app recorded URL errors as
  transport errors). Cancelled and failed sends are recorded with their outcome. A failed
  history write keeps the response and says so (`history_error`).
- **The saver is plain UI-side state** (`imposter_app::saver`): one write in flight at a time,
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
  glyph, which would otherwise render as a box. `cargo run -p imposter-ui --example screenshot --
  <dir>` renders the app offscreen (wgpu) for looking at layouts; `--example tour` renders
  every primary task step by step and the states a design review needs (first start,
  loading, errors, dialogs) at the minimum and a wide size. Design reviews follow the
  `design-ux` procedure: a reviewer that did not build the screens scores the tour.
- **Variables in the editor:** fields mark `{{name}}` tokens with the Python app's highlight
  pattern (`\{\{([^{}\n]+)\}\}`, stricter than the resolution pattern): defined names and
  generated values in the accent color, unknown ones in the error color. Typing `{{` suggests names with their scope and
  source, never values; the list opens only from typing in an unfinished token (clicking into
  a token only inspects it) and can be chosen by keyboard or pointer. Values appear only in the inspector, opened explicitly (click on a token
  or Alt+Enter); from a password field they stay masked until "Reveal". The inspector and
  execution share `executor::inputs`, so it explains exactly what a send would use.
- **The Variables screen edits one scope at a time** (Global, an environment, a collection),
  independently of the active execution environment, with its own unsaved-changes guard for
  scope changes, reloads, environment management, "Open definition" and closing. Closing
  resolves the request guard first, then the variables guard, then writes UI state.
- **Saved values: a list per variable, kept in the variable's scope beside its current value**
  (`model::SavedValues`, the `saved_values` column of migration V2, `store::scopes`). The
  current value stays the single source for execution, precedence, curl, history and exports;
  swapping (`scopes::choose_value`, from the inspector) is a stored write, not a mode. Lists
  live in the defining scope, so each environment has its own. The inspector does not offer a
  swap while the Variables screen has unsaved edits to that scope, so a later Save cannot
  undo it unseen, and a clean screen on that scope reloads. Saved values travel in native
  version 2 with their scope; an import's new global names bring theirs and existing globals
  keep their own, as their values do. Postman has no equivalent, so its exports leave them
  out. Rejected: linked rows switched together (Paw's environment domains; the
  user swaps values independently), a send-time prompt (Insomnia; keeps nothing), and one
  environment per value (Postman; multiplies environments).
- **History search uses the sidebar's filter language** (`filter.rs`, with `status:` and
  `env:` added for History: `404`, `4xx`, `failed`, `cancelled`). Words match the request
  name and the URL as sent; bodies are not searched, because that would decode every
  payload. The database thread scans the summary columns newest first, keeps matches and
  stops when the page is full (`executions::search`), so retention of up to a million entries
  never loads them all into the UI; without a filter, paging stays in SQL. Each list declares
  the fields it knows, and an unknown `name:` is text, so `status:` in the sidebar stays a
  word.
- **History shows times in the system time zone** (via `jiff`, display only); records store UTC
  milliseconds. The request of an execution is shown as sent (`viewer::sent_request`, built
  with the executor's own `prepare`): `METHOD URL`, then Body and Headers tabs like the
  response, never a dump of the stored template. Restoring an entry opens an unsaved draft through the unsaved-changes guard and
  never sends; the draft keeps the captured collection variables for a deleted collection.
- **The layout is a switch, not a mode:** side by side or stacked, changed only by the status
  bar button, never by the window width. The window can shrink to 720×480 (`MIN_WINDOW`; the
  Python app's 1000×650 took more than half a laptop screen), so narrow panes adapt instead:
  status as wrapping text, tabs and tools on separate rows, single-line fields never wrap.
  Why: an automatic choice next to a manual one made the button seem broken.
- **The status bar tells the latest result, only while it applies** (`app::StatusLine`).
  - **Request messages:** a message about the open request (a send's result, "Not sent", a
    restored draft) lasts while that request and screen stay shown.
  - **Other messages** fade after `STATUS_SECONDS`.
  - **Wording:** results read plainly ("200 OK · 12 ms", "Couldn't connect · 3 ms", "Send
    cancelled", `viewer::outcome_summary`), never internal type names.
  - **During a send** the bar shows "Sending… n s" with a spinner.
  - **The right end** shows the active environment and, while mocks serve, their port.

  Why: the review found it reporting an earlier request's failure after a blocked send, and
  "Ready" during a send. A status that outlives its context misleads more than no status.
- **Every problem in the response pane says what happened and what to do next.**
  - **Transport failure:** a plain headline per error kind and a hint (`viewer::failure_title`,
    `failure_hint`); the raw library text sits under a collapsed Details; Retry appears in the
    open request's pane, not in History.
  - **A send that never went out** (`app::NotSent`): the reason replaces any earlier response,
    which would read as this send's. Undefined variables come as names
    (`ExecuteError::Unresolved`), so the pane offers "Define {{x}} in <active environment or
    Global>". That opens Variables with a new unsaved row; nothing is written until Save.

  Why: the review found raw `os error 61` text clipped at the pane edge, and an empty-state
  invitation shown right under "Request failed".
- **Every screen saves the same way:** its state in one word and a disk icon at the top right
  (`widgets::save_controls`); Variables adds a revert icon. The header logo is the app icon
  drawing, like the window and Dock icon.
- **Save states are one word** ("Saved", "Unsaved", "Saving…", "Save failed", "Draft",
  "Invalid fields") in the request bar, the Variables screen and Settings, colored by tone,
  with the explanation in the hover text; Save is an icon button named by its action. Section
  headings and explanatory subtitles are left out where tabs and controls already say it.
  Why: user review found the longer texts noisy (the Python app's wording was kept until then).
- **Removing a row takes two clicks** (`widgets::remove_button`): the first arms the × (red,
  "Click again to remove …"), the second removes; it disarms after a few seconds or when the
  pointer goes elsewhere. Why: one stray click must not lose a header or variable; this works
  from the keyboard and screen readers, unlike a double-click.
- **The UI thread never blocks.** `imposter-ui/clippy.toml` disallows blocking database calls,
  file IO, sleeps, `block_on` and synchronous file dialogs. Only test and example crates allow
  them, at their top, and `window_state`, which runs before the window exists. Why: a frozen
  window is the most visible failure a desktop app has.
- **Settings follow the Python screen** (fields, ranges, units) and save only on Save
  Settings, against a baseline: undoing an edit is not a change. Settings the screen does not
  show are kept. Closing asks about unsaved settings after the request and variables guards.
  Saving applies history retention at once (the Python app waited for the next send), so the
  limits the History screen shows are already true.
- **Shortcuts are one table, `shortcuts::ALL`, handled in one place before widgets draw**
  (`shortcuts::handle`) and doing nothing while a dialog is open. The same table drives the
  shortcut sheet (Cmd/Ctrl+Shift+/), and a test checks that the README table equals it.
  Cmd/Ctrl+R sends (the Python app's Run); F2 renames, with Ctrl+Shift+R because F2 needs Fn
  on Mac keyboards (Yaak's choice). egui ignores extra Shift/Alt when matching, so the
  handler consumes the most specific combinations first. Find, Alt+Enter and Esc are listed
  but stay with the widget they act on (Find with the viewer under the pointer). Why: one
  handler means a body editor cannot swallow Send and a dialog cannot be bypassed; one table
  means the help and the README cannot drift from the keys.
- **The command palette (Cmd/Ctrl+K) searches actions, the requests of every collection and
  environments** (`palette.rs`, from Yaak). Requests come from `collections::request_index`,
  read when the palette opens through the generated columns (no template is decoded); actions
  and environments are captured at open. Matching is an in-order fuzzy subsequence that
  rewards word starts and runs; while typing, the group with the best match comes first so a
  strong request match is not buried under a weak action match. Choosing returns an ordinary
  `Action`, so the unsaved-changes guard applies; a request in another collection opens
  through `Navigation::OpenIn` (switch collection, then `restore`). The palette is pinned to
  the top: centred, it moved as its list shrank while typing, and clicks landed on the
  backdrop.
- **Recent requests: the last 20 opened saved requests, newest first, in `ui_state`
  (`recent_requests`)**, pruned when the request index shows one was deleted; an unreadable
  list starts empty instead of failing startup. Cmd/Ctrl+P is the palette with requests only,
  recent first. Ctrl+Tab (the real Ctrl key on every platform) is hold-and-release, as in
  editors: Tab steps while Ctrl is held, releasing Ctrl opens the selection, so a quick
  Ctrl+Tab goes back to the previous request; Shift starts from the oldest; Esc cancels. A
  release that comes before the index loads opens as soon as it arrives. Why: the
  back-and-forth between two requests is the most common jump.
- **The sidebar filter is a small query language** (`filter.rs`, after Yaak's): words combine
  with AND, plus OR, NOT/`-word`, `"phrases"`, parentheses, and `method:` (whole method),
  `url:` and `name:`. Plain words match the request's folder path, name and URL, so typing a
  folder name shows its requests and a negation never brings a whole folder back; folders
  left without matches are hidden. Text that does not parse is searched as typed. Filtered
  folders are drawn open under their own collapse ids, so clearing the filter restores the
  folders the user closed. The tree reads `url` through a virtual generated column
  (migration V3), like `name` and `method`. Cmd/Ctrl+F over the sidebar focuses the filter,
  as Find elsewhere applies to what is under the pointer.
- **A curl command pasted over the whole URL field replaces the open request** (after Yaak):
  method, URL, headers, cookies, auth, body and options come from the command; identity,
  name, description and place stay; the result is an ordinary unsaved edit (autosave or Save
  applies) and ignored options are listed afterwards. Only a paste that would replace the
  whole field (empty, or all of it selected, as after Cmd/Ctrl+L) and that parses as curl
  is taken (`widgets::take_whole_paste`, before the field draws); any other paste is text.
  Why: updating a request from a browser's "Copy as cURL" is one paste, while pasting into
  part of a URL can never replace a request by surprise. + → Paste curl still opens a draft.
- **Packaging: cargo-packager** (`.app`/`.dmg`, `.deb`/`.AppImage`, NSIS per-user installer),
  configured in `crates/imposter/Cargo.toml`. Releases are drafts built from `v*` tags. The macOS
  bundle is always signed, ad-hoc unless the Developer ID secrets exist (`docs/release.md`);
  the release job verifies it with `codesign --verify --deep --strict`. Why: one tool and one
  config for all three OSes; a draft keeps a human between CI and users; an unsealed bundle
  is "damaged" to macOS once downloaded (the v0.1.0 draft was).
- **The packaged binary is smoke-tested through the real app** (`imposter --smoke-test <url>`,
  driven by `scripts/smoke.py`): window, migrations, HTTP, history and the close sequence, with
  a throwaway `IMPOSTER_DATA_DIR`. It is the binary's only argument. Why: packaging breaks
  things unit tests cannot see (missing GL, TLS provider, bundled SQLite, paths). The app-side
  flow is covered by `imposter-ui/tests/smoke.rs`, CI runs the script on Linux on every push. The script
  checks the User-Agent against the workspace version, so release bumps remain verified
  without maintaining a second version string.
- **The app icon is drawn in code** (`icons::app_icon`): the window icon at runtime and the
  packaging PNGs, which a test compares with the drawing. Why: no design files to lose, and
  the icon cannot drift from the theme.
- **Release profile keeps `panic = "unwind"`.** Why: a panic in one background task is caught by
  tokio and must not abort the app.
- **Native format: version 1 (`"format": "poster"`) is read as the Python app's contract;
  Imposter writes version 2 (`"format": "imposter"`), which adds request checks, captures
  and saved values.** New exports
  use `.imposter.json`; imports accept both extensions and both versions without a companion
  prompt. A document keeps its version when decoded and encoded again, so Python exports still
  round-trip byte for byte (`imposter-transfer/tests/native_golden.rs`); markers and versions
  must agree, and a version 1 document with checks, captures or saved values is refused
  rather than half-read. Fields
  only version 2 has are left out when empty, so its requests otherwise look like version 1.
  Why: moving from the Python app stays supported, and Imposter-only data travels with the
  collection instead of being dropped (user's choice); the cost is that the Python app cannot
  read new exports. Bytes are base64 with the URL-safe alphabet and padding, as Pydantic
  writes them; both alphabets are read.
- **Checks are data, not scripts** (`core::check`): a selector (status, a header, a JSONPath,
  the body, duration, size), an operator and an expected value that may use `{{variables}}`,
  stored in the request template. The executor evaluates them after every response with the
  execution's own variable snapshot and keeps the results on the response, so History shows
  them. A check ends Passed, Failed (with the value read) or Not checked with the reason (a
  cut or non-JSON body, a bad path or regular expression, an unresolved variable): an
  undecidable check never passes or fails silently, and it never blocks a send. Strings
  compare as text and numbers by value (`19.50` equals `19.5`); status also accepts `2xx`.
  OpenAPI import adds a status check for the lowest documented success code and none when
  there is no documented success. Postman export lists checks as not exported. Rejected: a
  JavaScript engine (Postman's `pm.test`), a large dependency with an open-ended surface.
- **Generated values are placeholders named `$function args`** (`core::generate`): `$uuid`
  (and Postman's `$guid`, `$randomUUID`), `$timestamp`, `$timestampMs`, `$isoTimestamp`, `$date`
  with a strftime format, each with an optional offset (`-1d`, `+2h`), `$randomInt [min max]`
  and `$randomString [length]`. They are generated when a request is resolved for sending (or
  copying as curl), once per distinct expression, so `{{$uuid}}` in the URL, a header, the body,
  every redirect hop, the digest retry and a check's expected value is one value; words after
  a function's own arguments are labels that make separate values (`{{$uuid a}}`). A variable
  the user defines with the same name wins. Unknown functions and bad arguments stay unresolved
  and stop the send as any missing variable does; the inspector says why. Core reads no clock
  or randomness: `Entropy` comes from the app (`executor::SystemEntropy`, operating-system
  randomness) or a fixed one in tests. Postman import keeps the dynamic variables Imposter
  generates and still removes, with consent, the ones it does not. Rejected: a counter (a
  database write on every send; timestamps and random values cover uniqueness) and Postman's
  fake names and addresses (large tables, little testing value).
- **Captures set variables from a response, explicitly and visibly** (`core::capture`): a
  variable name and a selector, stored with the request. The value goes to the scope whose
  definition the request saw (so the value it used is the one replaced), else to the active
  environment, else to the globals; current values only, saved values untouched
  (`scopes::set_current`). Nothing is taken without a response, and a selector that names
  nothing leaves the variable alone. The executor only computes the writes; the app applies
  them in one transaction when the response arrives, even if the user has moved to another
  request, except for the scope the Variables screen has unsaved edits to: those are not
  written and the response says so, as with saved values. Statuses name variables, never
  values. Rejected: Yaak's `response.body.path(request)` template function, which hides the
  dependency in a placeholder and can send another request on its own.
- **Imports never change existing data and write nothing before review.** The preview computes
  fresh names (`Name (2)`) and global conflicts first; the import is one transaction; existing
  global values win, with a warning. Postman losses that change how a request runs (scripts,
  unsupported auth or bodies) need the explicit "Remove the listed unsupported execution
  features" consent, enforced by the service (`TransferError::ConsentRequired`), not only by
  the dialog. Other losses are warnings. Why: the Python app's contract; an import must be
  undoable by deleting the new collection.
- **Postman files are validated against the bundled v2.1 schema (draft 4), offline.** The file
  is pinned by SHA-256 (`imposter-transfer/schemas/README.md`, checked by `tests/schema_pin.rs`).
- **Exports are opt-in and atomic.** Environments and globals are included only when checked.
  The open request is exported as the editor shows it, unsaved edits included. Files are
  written to a temporary file and renamed; a Postman bundle with companions goes to a new
  directory and never overwrites one. Tied sibling positions are renumbered on export, as the
  Python app does, so a trip through the database keeps content and order, not tied numbers.
- **curl in and out** (`imposter_core::curl`; the Python app has none). Import parses POSIX
  shell quoting (including `$'…'`) and cmd's `^` escapes, maps the options browsers' "Copy as
  cURL" and hand-written commands use, reports others as warnings, and always opens the result
  as an unsaved draft. Copy as curl writes what Send would send now (variables resolved, auth
  headers built, settings as `-L`/`-k`/`--proxy`/TLS options), secrets included, only on
  explicit request; History copies what was recorded. Why: commands must run as pasted, and
  a generated command must parse back to the same prepared request
  (`imposter-core/tests/curl.rs`).
- **OpenAPI import (3.0, 3.1, 3.2; JSON or YAML) generates a new collection once; there is no
  sync.** Re-importing creates `Name (2)`, because merging into an edited collection would break
  "imports never change existing data". Decoding is lenient, not schema-validated: real specs
  are often slightly invalid, so unknown or unsupported parts become warnings and nothing needs
  consent (generating removes nothing a request did). One request per operation; the first
  tag is its folder, in declared tag order; the first server goes into a `baseUrl` collection
  variable (other servers are listed in a warning). Path parameters become variables, defined
  only when the document gives a value: an empty one would send `DELETE /pets/` instead of
  stopping on the unresolved variable. Optional parameters are imported disabled. Secrets are
  empty collection variables (`token`, `username`/`password`, the API key scheme's name);
  OAuth sends `{{token}}` as a bearer token. Only `$ref`s inside the document are followed;
  example bodies come from the document's examples, else from the schema with recursion
  included once and size capped. Swagger 2.0 is refused rather than half-supported. Files are
  recognised by content, not extension (`transfer::detect`), so only Postman files are asked
  for companions. YAML goes through `serde-saphyr` with a node budget scaled to the file
  size, which refuses alias bombs. There is no Python implementation to check parity against;
  the expectations are in `imposter-transfer/tests/openapi.rs`.
- **File dialogs go through one seam** (`ImposterApp::pick`); tests answer them with
  `script_file_dialogs`, since native dialogs cannot be driven headless.
- **Each dialog kind has its own modal id.** Why: a dialog replacing another (the import review
  followed by the unsaved-changes guard) inherited the old one's area, so a click on its buttons
  counted as a click outside and cancelled it.
- **Parity with the Python app is tested against fixtures produced by running the Python code**
  (`tests/fixtures/python/generate.py`), never against expectations written by hand. Why: a
  port written from reading code copies the reader's misreadings. Change a fixture only by
  regenerating it; a deliberate divergence is asserted in its own test with the reason.
  Current divergences: no empty `Cookie:` header when every cookie is disabled; converted
  documents are marked native version 2; Postman dynamic variables Imposter generates are
  kept rather than removed.
- **No line-ending conversion** (`.gitattributes`: `* -text`). Why: fixtures, golden exports
  and the pinned schema are compared byte for byte; a CRLF checkout on Windows broke them.
  Enforced by: the Windows CI job, which runs those tests.
- **Toolchain pinned in `rust-toolchain.toml`, `Cargo.lock` committed, CI uses `--locked`.**
  Why: every machine and CI run builds the same thing.

- **Request tabs start on Body and run Body, Auth, Query, Headers, Cookies, Settings, Docs, Tests.** Docs edits the existing plain-text description through the normal save guard and exports; Tests edits the request's checks. Why: body editing is the primary workflow; descriptions and checks belong with the request rather than in a separate document. Enforced by `imposter-ui/tests/editing.rs`.

- **Request names stay on one line with ellipsis and full text on hover; breadcrumbs reveal their full path on hover.** Why: long names must not expand a sidebar row or hide their identity. Accessible names remain complete. Enforced by `imposter-ui/tests/editing.rs`.

- **Duplicate creates a fresh saved sibling, including current editor content when open, through the navigation guard.** Name allocation and insertion are one transaction; history stays with the original. Why: copying must neither lose edits nor overwrite an existing request. Enforced by app operation tests and `imposter-ui/tests/editing.rs`.

- **Host client identities match an exact normalized HTTPS host and port.** A match replaces global client credentials, then explicit request fields override individually. Each redirect selects again from the execution snapshot; CA and verification settings remain unchanged. Why: one host must never inherit another host’s selected certificate. Curl copies the initial identity and reports that it cannot switch identities across redirects. Enforced by core settings, executor, and loopback TLS tests.

- **Mocks is one saved loopback-only static HTTP/1 server, started and stopped explicitly.** Enabled routes match exact method and encoded path, ignoring query and body; unmatched requests return 404. A running server keeps its start-time configuration until restarted. Why: predictable responses and no implicit listener after startup. Configuration has its own save/reload/close guard; shutdown cancels and drains the listener and all connections before closing. Enforced by mock domain, service, loopback, and UI tests.

## Enforcement gaps

Rules above that no tool checks yet. Close a gap together with the first code it applies to.

None at the moment.

- **Branding: I’mposter in the UI, I’mp in narrow headers, Imposter for installed apps, and `imposter` for code and commands; bundle ID `dev.imposter.app`.** Only `’m` uses the orange accent; the icon is a geometric I/apostrophe. Why: preserve the wordplay while keeping shell and package names portable. Wordmark/layout tests and icon tests pin accessible naming, colors and generated packaging images.
- **Default profiles copy `~/.poster/poster.db` once to `~/.imposter/imposter.db`, only when the new database is absent.** SQLite backup captures WAL data, validates integrity, references and supported schema, closes connections, then publishes without replacing a concurrently created destination. Busy handling is bounded; failed copies leave no partial profile and report recovery guidance. Explicit `IMPOSTER_DATA_DIR` directories never consult home. Why: preserve every saved domain without merging profiles or changing the old database. Enforced by store profile tests and app startup tests.
- **Only `IMPOSTER_DATA_DIR` and `IMPOSTER_LOG` configure the renamed app; old names are ignored and an empty directory override is an error.** Why: scripts must explicitly select the intended app and an invalid override must not fall back to personal data. Enforced by isolated subprocess environment tests and the binary smoke test.
