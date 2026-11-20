# Platform & Release Decisions

Part of the [Imposter agent manual](../../AGENTS.md). Decisions governing native distribution, packaging, smoke testing, branding, toolchain pinning, and panic strategies.

## Decisions

- **Rust only, one native binary per OS; no webview, JVM or own C/C++.** Why: one file per OS
  to distribute, with no interpreter or runtime to install. Speed and memory matter, but come
  second.

- **Packaging: cargo-packager** (`.app`/`.dmg`, `.deb`/`.AppImage`, NSIS per-user installer),
  configured in `crates/imposter/Cargo.toml`. Releases are drafts built from `v*` tags. The macOS
  bundle is always signed, ad-hoc unless the Developer ID secrets exist (`docs/release.md`);
  the release job verifies it with `codesign --verify --deep --strict`. Why: one tool and one
  config for all three OSes; a draft keeps a human between CI and users; an unsealed bundle
  is "damaged" to macOS once downloaded (the v0.1.0 draft was).

- **The packaged binary is smoke-tested through the real app** (`imposter --smoke-test <url>`,
  driven by `scripts/smoke.py`): window, migrations, HTTP, history and the close sequence, with
  a throwaway `IMPOSTER_DATA_DIR`. It is the only argument that opens the window; any other
  goes to the command line. Why: packaging breaks
  things unit tests cannot see (missing GL, TLS provider, bundled SQLite, paths). The app-side
  flow is covered by `imposter-ui/tests/smoke.rs`, CI runs the script on Linux on every push. The script
  checks the User-Agent against the workspace version, so release bumps remain verified
  without maintaining a second version string. `smoke.py --headless` runs `imposter send`
  with every display variable removed and checks the outcome, the User-Agent and that no
  profile was created; CI runs it before any display or GL library is installed, and the
  release on every package (on Windows before Mesa is added). Why: the roadmap allows a
  separate headless executable only if this binary cannot run without a display; this
  shows it can.

- **Release measurements use the packaged smoke workflow, with one excluded warm-up and
  ten successful runs, each in a fresh temporary profile.** Launch-to-request is measured
  before spawning the process through receipt at the loopback server, not first-frame time;
  macOS `time -l` supplies peak resident bytes for the whole workflow, not idle memory.
  Failed or incomplete series yield no report. Why: figures need a repeatable workload
  and honest boundaries, not an unexplained startup or memory claim. Sample validation,
  warm-up exclusion and failure handling are checked by `scripts/test_smoke.py`, run by
  `scripts/check.sh` and CI; timing values are evidence, never CI performance thresholds.

- **The app icon is drawn in code** (`icons::app_icon`): the window icon at runtime and the
  packaging PNGs, which a test compares with the drawing. Why: no design files to lose, and
  the icon cannot drift from the theme.

- **Release profile keeps `panic = "unwind"`.** Why: a panic in one background task is caught by
  tokio and must not abort the app.

- **Golden fixtures pin exact behavior** (`tests/fixtures/golden`: request preparation,
  variable and settings resolution, multipart bytes, Postman conversion;
  `tests/fixtures/native`: documents and the accepted and refused cases). A change to one is
  a change of behavior: it is made in the same commit as the code, with the reason in the
  commit message. Why: they record behavior as it was verified when the cases were written;
  a test that only compares the code with itself would not catch a regression.

- **No line-ending conversion** (`.gitattributes`: `* -text`). Why: fixtures, golden exports
  and the pinned schema are compared byte for byte; a CRLF checkout on Windows broke them.
  Enforced by: the Windows CI job, which runs those tests.

- **Toolchain pinned in `rust-toolchain.toml`, `Cargo.lock` committed, CI uses `--locked`.**
  Why: every machine and CI run builds the same thing.

- **Branding: I’mposter in the UI, the app icon in narrow headers, Imposter for installed apps, and `imposter` for code and commands; bundle ID `dev.imposter.app`.** Only `’m` uses the orange accent; the icon is a geometric I/apostrophe. Why: preserve the wordplay while keeping shell and package names portable. Wordmark/layout tests and icon tests pin accessible naming, colors and generated packaging images.

