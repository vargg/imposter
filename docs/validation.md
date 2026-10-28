# Manual validation

Automated tests drive the app headless (see [testing.md](testing.md)). This walkthrough covers
what they cannot: the native window, OS dialogs, platform shortcuts, rendering and packaging.
Run it on macOS, Windows and Linux before each release, with the packaged build.

## Setup

- Use a throwaway data directory: `IMPOSTER_DATA_DIR=$(mktemp -d) <packaged imposter>`. Never use
  your own `~/.imposter`, and never the Python app's databases.
- Start a loopback server, for example `python3 -m http.server 8765`.
- Have a `.poster.json` exported from the Python app at hand.

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
- [ ] Import the Python app's `.poster.json` from the + menu: the review shows the new name and
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
| 2026-10-28 | macOS 26.6.2, arm64 | I’mposter 0.1.3 | Codex: local full gate passed; packaged real-window HTTP/history/shutdown smoke test passed; bundle version, ad-hoc signature and DMG integrity verified. Full manual walkthrough (including native dialogs and display checks) not performed; Linux/Windows package checks are run by the release workflow. |
