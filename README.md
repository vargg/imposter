# I’mposter

A fast, local desktop HTTP API client. Build requests, organize them in collections, reuse
values through variables and environments, and keep a searchable history of everything you
sent. It runs as one native binary per OS, with no account, no cloud and no telemetry.

![Requests: collection tree, request editor and response viewer](docs/images/requests.png)

I’mposter is written in Rust with [egui](https://github.com/emilk/egui). It replaces an earlier
Python/PyQt6 version, and preserves its native collection format. New exports use `.imposter.json`; existing `.poster.json` files still import.

## Features

**Requests**
- Collections with nested folders. Move requests and folders by dragging, or with Move.
- Any method, custom methods included; query parameters kept in sync with the URL.
- Headers, cookies, and bodies as JSON, plain text, URL-encoded form, multipart (text fields
  and file uploads) or a binary file. Uploads are streamed from disk.
- Authentication: Basic, Bearer and Digest (MD5, SHA-256, SHA-512, `-sess`).
- Per-request settings that override the application defaults: timeout, redirects,
  certificate verification, cookies, proxy, CA bundle, client certificate and key, and
  response size limit.
- Autosave, or explicit save. Unsaved changes are never lost silently: switching requests,
  restoring from history and closing all ask first.

**Variables**
- `{{name}}` placeholders in the URL, headers, body, auth and settings.
- Three scopes: global, per collection, and environments. The active environment overrides
  the collection, which overrides globals.
- Placeholders are highlighted: defined names in orange, unknown ones in red. Typing `{{`
  suggests names. Clicking a placeholder, or pressing Alt+Enter, shows where its value comes
  from. Values from password fields stay masked until revealed.

**Responses**
- Status, time, size and content type at a glance.
- JSON, XML and HTML highlighting, formatted JSON, find, wrap and copy. Large bodies stay
  responsive (the viewer only lays out what is visible; previews stop at 256 KiB).
- Earlier runs of the same request stay one click away.

**History**
- Every execution with its environment, the request exactly as it was sent, and the full
  response.
- Restore an execution as an unsaved draft, or copy it as a curl command.
- Retention by number of entries and total size.

![History: executions, and the request as it was sent](docs/images/history.png)

**Import and export**
- Native `.imposter.json` (version 1), compatible with `.poster.json` from the Python version.
  The serialized `"format": "poster"` marker remains unchanged for compatibility.
- Postman Collection v2.1 import and export, validated against the official schema, with
  environment and globals files. Anything Postman features Imposter cannot represent is listed
  before import; changes to how a request runs need explicit consent.
- Imports never overwrite: names that already exist get a suffix, existing globals win.
- curl: paste a command (including the "Copy as cURL" output of Chrome and Firefox, for bash
  or cmd) to open it as a draft, or copy any request as a ready-to-run curl command.

**Interface**
- Side-by-side or stacked request and response, switched from the status bar. Collections
  sidebar can be hidden. Splitter positions, window size and the last selection are restored.
- Dark theme. Every control has an accessible name for screen readers; text colors meet
  WCAG AA contrast.

## Not supported

Pre-request and test scripts; WebSocket, gRPC and GraphQL-specific tools; HTTP/2 (requests
use HTTP/1.1); cloud sync and sharing; a persistent cookie jar (cookies are sent as you list
them); themes other than the dark one.

## Installation

### Packages

Release packages are built by CI from version tags: a `.dmg` for macOS (Apple Silicon), a
`.deb` and an `.AppImage` for Linux (x86_64), and an installer for Windows (x86_64). Until
the first release is published, build from source.

The packages are not signed with an Apple or Microsoft developer certificate yet, so the first
launch needs one extra step:
- **macOS 15 and later:** open Imposter once and dismiss the warning, then go to System Settings
  → Privacy & Security and click **Open Anyway** next to the Imposter message.
- **macOS 14 and earlier:** right-click Imposter in Applications, choose Open, then Open again.
- **Windows:** in the SmartScreen prompt, click More info → Run anyway.

### From source

You need [rustup](https://rustup.rs). The pinned toolchain (`rust-toolchain.toml`) is
installed automatically on the first build. No system libraries or C compiler are needed
beyond what Rust itself requires.

```sh
git clone https://github.com/vargg/imposter.git
cd imposter
cargo run --release -p imposter
```

The binary is `target/release/imposter`.

On Linux, the app loads the usual desktop libraries at runtime: OpenGL/EGL, X11 or Wayland,
and xkbcommon. File dialogs go through the XDG desktop portal
(`xdg-desktop-portal` with a backend for your desktop).

## Usage

1. Create a collection with **+ → New Collection**, then add requests with **+ → New
   Request** (or Cmd/Ctrl+N).
2. Enter a URL, choose a method, add what the request needs in the tabs below, and **Send**.
3. Put shared values on the **Variables** screen (globals, an environment, or the
   collection) and use them as `{{name}}`. Choose the active environment at the top right.
4. Find any earlier execution on the **History** screen.
5. Application defaults (timeout, proxy, TLS, history limits, autosave) are on **Settings**.

### Keyboard shortcuts

Cmd on macOS, Ctrl elsewhere. Shortcuts do nothing while a dialog is open.

| Shortcut | Action |
|---|---|
| Cmd/Ctrl+Enter, Cmd/Ctrl+R | Send (or cancel the running request) |
| Cmd/Ctrl+S | Save the current screen: request, variables or settings |
| Cmd/Ctrl+N | New request (a new collection when there is none) |
| Cmd/Ctrl+F | Find in the viewer under the pointer |
| Alt+Enter | Inspect the variable at the cursor |
| Esc | Close the inspector, find bar or suggestions |

### Moving from the Python version

Export each collection from the Python app as `.poster.json` (with its environments and
globals if you want them), then use **+ → Import Collection…** in Imposter. The two apps never
share a database, so the Python app keeps working.

## Your data

I’mposter is the display name; the installed app is **Imposter**, the command is `imposter`,
and the compact header wordmark is **I’mp**.

On default-profile startup, if the new database is absent, the app copies the previous Rust
Poster profile at `~/.poster/poster.db` using SQLite's backup API, including committed WAL data.
Collections, variables, settings, history, mocks and UI state are preserved. The old database
is left logically unchanged. An existing new database always wins; profiles are never merged.
A busy, unreadable, corrupt or newer-schema legacy profile produces an error with recovery
instructions instead of silently starting empty. Close Poster and retry a busy copy.

Everything lives in one SQLite database: `~/.imposter/imposter.db`. Set `IMPOSTER_DATA_DIR` to use
another directory (for a second profile, or for testing):

```sh
IMPOSTER_DATA_DIR=/tmp/imposter-scratch imposter
```

Before a new version changes the database format, the old database is copied to
`imposter.db.bak-<version>` next to it. A database written by a newer version is refused
rather than changed.

An explicit `IMPOSTER_DATA_DIR` uses `imposter.db` in that directory and never looks for or
copies the personal legacy profile. An empty override is an error. Update existing scripts:
`POSTER_DATA_DIR` and `POSTER_LOG` are no longer recognized; use `IMPOSTER_DATA_DIR` and
`IMPOSTER_LOG`.

Logs go to stderr; `IMPOSTER_LOG` sets the level (for example `IMPOSTER_LOG=debug`). Imposter never
logs credentials, tokens, or request and response bodies.

## Development

The root Makefile wraps the existing Cargo commands and scripts. It requires GNU Make
and a POSIX shell (on Windows, use Git Bash with GNU Make). `make` lists all targets.

```sh
make run                                  # debug application
make release                              # optimized binary
make test                                 # all workspace tests
make test CRATE=imposter-http TEST=redirect  # one crate, optional test-name filter
make fmt                                  # format Rust code
make lint                                 # Clippy, warnings are errors
make check                                # full verification gate, including coverage
make screenshots OUT=/tmp/imposter-shots     # render the UI offscreen
make smoke                                # build and smoke-test the debug application
make package                              # local packages for the host OS
make clean                                # remove build artifacts, keep application data
```

`make check` and `make coverage` both run `scripts/check.sh`; coverage thresholds and
exclusions remain defined there. Install its prerequisites with
`cargo install cargo-llvm-cov` and `rustup component add llvm-tools`. CI also checks macOS,
Linux and Windows. Cargo and rustup must be on `PATH`; the repository pins the toolchain.

Use `TEST_ARGS="--nocapture"` to pass arguments to the test harness, and `PYTHON=python`
if that is your Python 3 executable. `make smoke BINARY="path/to/imposter"` tests an existing
executable without rebuilding; smoke tests open a real window and need a display (Xvfb
on headless Linux). Their database is temporary.

`make package` requires `cargo-packager` 0.11.8; install it with
`cargo install cargo-packager --locked --version 0.11.8`. It defaults to `app,dmg` on macOS,
`deb,appimage` on Linux, and `nsis` on Windows; override with, for example,
`make package FORMATS=app`. It uses the existing packaging configuration and
`target/packages/` output, with `CI=true` to skip interactive DMG styling. Packaging uses
the repository's default Cargo target directory. See [release.md](docs/release.md) for
signing and release instructions; the Makefile does not tag, push, or publish releases.

The workspace is split by responsibility; each layer only depends on the ones before it,
which an architecture test enforces:

| Crate | Responsibility |
|---|---|
| `imposter-core` | Domain types and rules: variables, settings, URLs, request preparation, curl. No IO. |
| `imposter-http` | HTTP transport: TLS (rustls), redirects, Digest auth, streamed uploads, cancellation. |
| `imposter-store` | SQLite persistence on its own thread, with migrations and backups. |
| `imposter-transfer` | `.poster.json` and Postman v2.1 conversion and validation. |
| `imposter-app` | Services the UI calls: execution, saving, history, import and export. |
| `imposter-ui` | The egui interface. |
| `imposter` | The binary. |
| `imposter-testkit` | Loopback servers and certificates for tests. |

Further reading:
- [AGENTS.md](AGENTS.md): architecture principles, rules and decisions, with their reasons.
  Read it before changing anything substantial.
- [docs/testing.md](docs/testing.md): where tests go and how they are written.
- [docs/validation.md](docs/validation.md): the manual checklist run before each release.
- [docs/release.md](docs/release.md): packaging, signing and cutting a release.

`cargo run -p imposter-ui --example screenshot -- <dir>` renders the app with sample data to
PNG files, without a display.

Request tabs open on **Body**, followed by Auth, Query, Headers, Cookies, Settings and Docs.
Use Docs to edit the request description; it saves and exports with the request.

Right-click a request and choose **Duplicate** to create and open a saved sibling copy.
An open request copies its current edits; navigation still resolves unsaved changes.

Settings → **Host client certificates** assigns a client certificate, optional key and password to an exact HTTPS host and port (default 443). Request overrides take priority. Redirects select credentials again for their destination. Copy as curl uses the initial identity and cannot reproduce identity switching across redirects.

Use **Mocks** to configure a local static HTTP server on `127.0.0.1` (port 8080 by default; 0 chooses an available port). Add routes by exact method and encoded path, with a response status, headers and text body. Query strings and request bodies do not affect matching; unmatched requests return 404. Save before Start; edits to a running server apply after Stop and Start. Configuration persists, but the server never starts automatically and stops when Imposter closes.
