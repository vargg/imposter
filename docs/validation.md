# Manual validation

Automated tests drive the app headless (see [testing.md](testing.md)). This walkthrough covers
what they cannot: the native window, OS dialogs, platform shortcuts, rendering and packaging.
Run it on macOS, Windows and Linux before each release, with the packaged build.

## Setup

- Use a throwaway data directory: `IMPOSTER_DATA_DIR=$(mktemp -d) <packaged imposter>`. Never use
  your own `~/.imposter`.
- Start a loopback server, for example `python3 -m http.server 8765`.
- Have an `.imposter.json` export at hand (`tests/fixtures/native/full.imposter.json` will do).

## Checklist

**Start and layout**
- [ ] The window opens quickly at a sensible size; the empty state offers to create or import
      a collection.
- [ ] At the minimum window size (720×480) every control is reachable in both layouts
      (multipart editor included); the layout switch in the status bar changes side by side
      and stacked, and resizing the window never changes it.
- [ ] The header logo, the window icon and the Dock / taskbar icon are the same drawing.
- [ ] Text is crisp on HiDPI and standard displays; the dark theme is readable.

**Requests**
- [ ] Create a collection, a folder and a request; send `GET http://127.0.0.1:8765/`; status,
      time, size and body show.
- [ ] Edit the URL, wait for "Saved automatically", restart: the edit and selection are back.
- [ ] Turn autosave off in Settings; edit a request and switch to another: Save / Discard /
      Cancel each behave as labelled.
- [ ] "Choose file…" for a binary body and a multipart file opens the native picker;
      cancelling it changes nothing.
- [ ] A long request can be cancelled; the previous response stays visible while one runs.

**Variables and history**
- [ ] `{{name}}` in the URL is highlighted; Alt+Enter on it opens the inspector; values used
      in a password field stay masked until Reveal.
- [ ] Switching the environment changes what the next send uses.
- [ ] History lists the send; "Restore as draft" opens it without sending.

**Import and export**
- [ ] Import the `.imposter.json` from the + menu: the review shows the new name and
      environments; after Import the collection opens.
- [ ] Export it again as Imposter and as Postman (with an environment: a new folder with
      companion files appears). The native save dialog opens and names the file sensibly.
- [ ] Import a Postman collection with scripts: Import stays disabled until the removal
      consent is checked.

**Settings and shortcuts**
- [ ] Settings: an edit shows "Unsaved application settings"; Save Settings saves; closing with
      unsaved settings asks first.
- [ ] Cmd+Enter / Ctrl+Enter and Cmd+R / Ctrl+R send, Cmd+S / Ctrl+S saves the current screen,
      Cmd+N / Ctrl+N opens New Request, Cmd+F / Ctrl+F opens Find in the viewer under the
      pointer. None of them fire while a dialog is open.
- [ ] The Send and Save hover texts show their shortcuts in the platform's notation.

**Closing**
- [ ] Closing during a running request cancels it and closes; window size and position, layout
      and selection come back on the next start.

## Results

Record each run: date, OS and version, build, who ran it, and anything that failed (with the
issue or commit that fixed it).

| Date | OS | Build | Result |
|---|---|---|---|
| 2026-11-06 | macOS 26.6.2, arm64 | I’mposter 0.6.0 | Codex: `scripts/check.sh` passed (95.45% line coverage, 94.98% region coverage); the packaged real-window HTTP/history/shutdown smoke test passed; bundle version 0.6.0, signature (`codesign --verify --deep --strict`) and DMG checksum verified. Full manual walkthrough not performed; Linux/Windows checks have not run locally and are left to CI and the release workflow. |
| 2026-11-04 | macOS 26.6.2, arm64 | I’mposter 0.5.0 | Claude: `scripts/check.sh` passed (coverage 94.9% of lines); the packaged real-window HTTP/history/shutdown smoke test passed; bundle version 0.5.0, identifier `dev.imposter.app`, ad-hoc signature (`codesign --verify --deep --strict`) and DMG checksum verified; DMG 9.9 MB (9.0 MB for 0.4.0; Rhai adds about 1.9 MiB to the binary). New in this version and checked only by automated tests and tour screenshots: request scripts, mock route scripts, native format version 3. CI on Linux and Windows not yet run for this version. Full manual walkthrough (including native dialogs and display checks) not performed; Linux/Windows package checks are run by the release workflow. |
| 2026-11-04 | macOS 26.6.2, arm64 | I’mposter 0.4.0 | Claude: `scripts/check.sh` passed (coverage 95.0% of lines); the packaged real-window HTTP/history/shutdown smoke test passed; bundle version 0.4.0, identifier `dev.imposter.app`, ad-hoc signature (`codesign --verify --deep --strict`) and DMG checksum verified; DMG 8.6 MB (8.5 MB for 0.3.0). New in this version and checked only by automated UI tests and screenshots: tree reordering, JSON typing aids, the Load screen. CI on Linux and Windows not yet run for this version. Full manual walkthrough (including native dialogs and display checks) not performed; Linux/Windows package checks are run by the release workflow. |
| 2026-11-04 | macOS 26.6.2, arm64 | I’mposter 0.3.0 | Claude: `scripts/check.sh` passed (coverage 94.9% of lines); the packaged real-window HTTP/history/shutdown smoke test passed; bundle version 0.3.0, identifier `dev.imposter.app`, ad-hoc signature (`codesign --verify --deep --strict`) and DMG checksum verified; DMG 8.5 MB, as 0.2.0. CI on Linux and Windows not yet run for this version. Full manual walkthrough (including native dialogs and display checks) not performed; Linux/Windows package checks are run by the release workflow. |
| 2026-11-01 | macOS 26.6.2, arm64 | I’mposter 0.2.0 | Claude: `scripts/check.sh` passed (coverage 94.9% of lines); CI passed on macOS, Linux and Windows (run 41310778075); the packaged real-window HTTP/history/shutdown smoke test passed; bundle version 0.2.0, identifier `dev.imposter.app`, ad-hoc signature (`codesign --verify --deep --strict`) and DMG checksum verified. Full manual walkthrough (including native dialogs and display checks) not performed; Linux/Windows package checks are run by the release workflow. |
| 2026-10-28 | macOS 26.6.2, arm64 | I’mposter 0.1.3 | Codex: local full gate passed; packaged real-window HTTP/history/shutdown smoke test passed; bundle version, ad-hoc signature and DMG integrity verified. Full manual walkthrough (including native dialogs and display checks) not performed; Linux/Windows package checks are run by the release workflow. |

## Packaged 0.6.0 measurements

Measured on 6 November 2026 by Codex, on a Mac model **Mac15,7**, **Apple M3 Pro**,
**36 GiB RAM**, running **macOS 26.6.2 (25G83), arm64**. These are local observations,
not performance guarantees for other machines or operating systems.

- App: the existing packaged **0.6.0**, source commit
  `a11af1acb2aae87c9c65edc39c0e89342a03042a` (`v0.6.0`).
- Build command: `CI=true cargo packager -p imposter --release --formats app,dmg`;
  its configured build step is `cargo build --release --locked -p imposter`.
- Executable: `target/packages/Imposter.app/Contents/MacOS/imposter`.
- Packaged executable SHA-256:
  `08ac3c6c1e958d8fded0c819799c3ccee1e8f03e64134ab7443bcac3fcc980f8`.
  The bundle reports version 0.6.0 and passes `codesign --verify --deep --strict`.
- Harness: Python 3.9, `scripts/smoke.py --measure`. One excluded warm-up, then ten
  sequential samples. Every launch creates and removes its own temporary profile and
  uses a server on an ephemeral loopback port. No personal data is read.

**Launch-to-request** starts immediately before spawning the `/usr/bin/time` wrapper and
ends when the loopback server enters its GET handler. It includes wrapper/process launch,
fresh-profile database initialization, window/app startup, request preparation and the
loopback request. It is **not first-frame or visible-window time**. The server is already
started before the timer begins; the clock is monotonic (`time.perf_counter`).

**Peak resident memory** comes from macOS `/usr/bin/time -l` for the app process over its
entire smoke workflow: startup, HTTP response, history write and shutdown. The raw unit
is bytes; MiB below means 1,048,576 bytes. It is **not idle memory**, combined memory for
the harness and app, or GPU memory usage.

Each sample must also pass the ordinary smoke assertions: exactly one `GET /smoke`,
`User-Agent: Imposter/0.6.0`, HTTP 200 reported, exactly one history entry in the temporary
database, ignored legacy directory/log overrides, suppressed app logs, and a successful
exit. Each process has a 120-second deadline. A failed warm-up or sample invalidates the
series; the harness emits no partial report.

| Sample | Launch-to-request (ms) | Peak resident bytes | Peak resident MiB |
|---|---:|---:|---:|
| 1 | 224.972 | 107855872 | 102.859 |
| 2 | 217.279 | 111427584 | 106.266 |
| 3 | 211.401 | 111460352 | 106.297 |
| 4 | 218.939 | 111247360 | 106.094 |
| 5 | 209.708 | 111181824 | 106.031 |
| 6 | 224.411 | 111394816 | 106.234 |
| 7 | 183.092 | 107773952 | 102.781 |
| 8 | 234.908 | 111329280 | 106.172 |
| 9 | 218.164 | 111099904 | 105.953 |
| 10 | 230.510 | 111755264 | 106.578 |

| Metric | Median | Minimum | Maximum |
|---|---:|---:|---:|
| Launch-to-request (ms) | 218.551 | 183.092 | 234.908 |
| Peak resident bytes | 111288320 | 107773952 | 111755264 |
| Peak resident MiB | 106.133 | 102.781 | 106.578 |

The ordinary smoke invocation and all eleven measurement launches passed. OS caches
were warm, application profiles were fresh, and no repository build ran during the
series. Caches were not flushed and the machine was not rebooted between samples;
these are not cold-boot or first-install measurements. A populated profile, large
responses, other system load and different displays may change the results. No
Linux/Windows performance measurements or full manual walkthrough were performed.

To reproduce on macOS after building the package:

```sh
python3 scripts/smoke.py target/packages/Imposter.app/Contents/MacOS/imposter
python3 scripts/smoke.py --measure target/packages/Imposter.app/Contents/MacOS/imposter > target/measurements.json
```

Measurement mode writes progress to stderr and a JSON report containing all ten samples
and their summaries to stdout. It requires macOS; the ordinary smoke invocation remains
cross-platform. The measurement-tool tests run in `scripts/check.sh` and CI, without
timing thresholds.

The README's Requests and History screenshots were regenerated from the same 0.6.0 UI
using `cargo run --locked -p imposter-ui --example screenshot -- target/m0-screenshots`.
Both wide renders were visually inspected before copying them to `docs/images`; they
use synthetic data and a loopback server. The example also rendered its narrow states.

M0 implementation validation completed on 7 November 2026: `scripts/check.sh` passed
on macOS (95.45% line coverage, 94.97% region coverage), including the existing README
script/shortcut reference and native-format tests. The smoke tool's own tests also passed
all 13 tests, including fault injection for every original smoke assertion. Published
sample values were checked against the JSON report and relative documentation links
were checked for existing targets. Linux/Windows CI and the full manual walkthrough
were not run for this change.
