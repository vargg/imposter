# Storage & State Decisions

Part of the [Imposter agent manual](../../AGENTS.md). Decisions governing SQLite database ownership, file-backed collections and environments, secrets, history, and profile migrations.

## Decisions

- **Migrations: our own runner over `user_version`, not `rusqlite_migration`.** Each migration
  runs in its own transaction and must pass `foreign_key_check`; an existing database is copied
  to `imposter.db.bak-<version>` with SQLite's backup API first; a database from a newer app is
  refused. Why: about 40 lines, no second rusqlite version to keep in step, and the backup
  happens exactly before the first change. Released migrations are frozen
  (`imposter-store/tests/schema.rs` pins their fingerprints); schema changes are new migrations.
  V5 adds a `scripts` column (JSON `model::Scripts`) to collections and folders; requests
  needed none, as their scripts live in the template JSON.

- **Collections and environments are files; there is no database copy** (user's choice,
  after weighing an opt-in link that kept SQLite as the source: two copies and two kinds of
  collection to keep in step). The folder (`store::files`, format above) is loaded into
  memory when the store opens and is what every store function reads and changes, behind
  the same API as the tables it replaced, so the UI, executor, runner, catalog and transfer
  did not change. SQLite keeps personal data only: History, settings, UI state, secrets
  and local values.
  - **Writes:** each command stages its changes on a copy; at commit only the files whose
    rendering changed are written, each atomically, at the path their name gives, and the
    old file of a renamed or moved item goes. A file kept under another name by hand stays
    where it is until it changes.
  - **Outside changes:** every file written or removed must still hold what Imposter read
    or wrote, else the command fails naming the file and nothing is saved (a request edit
    stays in the editor). The folder is read at every start; reloading while running is
    the next stage. A folder that cannot be read leaves the store empty, says why
    (`Tx::folder_error`) and is never written to.
  - **Reloading** (`files::reload`): reads the folder again and adopts what changed on
    disk since it was read or written, reporting it by id (`Reloaded`: requests changed
    and removed, collections, environments, scopes). The store's own writes match what it
    remembers, so they are no change. A folder that does not read (a half-done merge)
    keeps the last good model with `error` set, refusing writes until a reload reads it.
    A collection or folder whose directory was renamed outside the app keeps that
    directory: unchanged parents map the canonical path of what is written under them
    (`kept_directories`), so a changed request never lands away from its `folder.yaml`.
  - **Watching** (`imposter_app::watch`, the `notify` crate): the folder is watched
    recursively with the OS's own notifications (FSEvents, inotify,
    ReadDirectoryChangesW; no other system library). Bursts are coalesced until the
    folder is quiet for `watch::QUIET` (250 ms), then one callback says that something
    may have changed; `files::reload` finds out what. Events under hidden directories
    (`.git`) and reads are ignored; a watcher error counts as a change, since events may
    have been lost. Only the part of a path inside the folder is judged hidden, found
    under the folder as given or as canonical (Windows reports the first, macOS the
    second); a path under neither is a change. Judged whole, every event under the default
    `~/.imposter/collections` read as hidden on Windows. Pinned by
    `imposter-app/tests/watch.rs`, which probes until the watch reports instead of
    sleeping.
  - **Live reload in the app:** the app watches its collections folder (replaced with
    the folder) and checks it again when the window regains focus, since a watcher can
    miss events; each signal is `Message::FolderChanged`, then a reload task.
    - A clean open request is reloaded in place, on the same tab.
    - One with unsaved edits gets a banner and its writes are held (`Saver::hold`): after
      a reload the store's cache equals the disk, so a stale save would otherwise pass
      the conflict check and silently overwrite the outside change.
    - Its choices (user's choice: a banner, not a dialog that pops up while typing):
      "Take theirs" shows the file's version and keeps the edit beside it as
      "… (your edits)" (`ops::copy_beside`), so nothing is lost; "Keep mine" lets the
      next save write over the file on purpose. A request deleted on disk closes when
      clean, or offers "Keep as draft" and "Close".
    - The Variables screen reloads a changed scope when clean. With pending edits it
      shows a banner (Reload, Keep mine) and holds autosave while the user decides.
    - A folder that does not read keeps the tree and says why on the status line, and
      says so again once it reads.
    Pinned by `imposter-ui/tests/project.rs` (driven by `reload_folder`, not the
    watcher's timing) and the layout lint's banners.
  - **Relative paths** in a request (a binary body, multipart files) and in the TLS
    settings it resolves (CA bundle, certificate, key, host identities) are found under
    `Inputs::base_dir`: the collections folder for the app and profile runs, the folder for
    `--project`, the file's directory for `--file`. They are joined after variables
    resolve and again after the before-sending scripts (`executor::root_template`,
    `root_settings`, `IdentityRouting::base_dir`), so History and Copy as curl show the
    joined path. It is joined part by part, so it has the OS's separators throughout
    (`fixtures/data.bin` gave `C:\api\fixtures/data.bin` on Windows). Browse keeps a file
    inside the folder relative, with `/` on every OS.
    Why: a checkout must run on any machine (M2's exit); paths relative to the working
    directory only worked from the right one. Pinned by `imposter-app/tests/paths.rs` and
    `imposter-cli/tests/project.rs`.
  - **Ids:** files carry UUIDs; `ids` (migration V8) gives each one a stable number, so
    `RequestId`s survive restarts and History's `request_id` still opens its request.
  - **Order and saves:** sibling order is `position`; saving a template keeps the stored
    position, since the editor's copy goes stale when the tree is reordered (an autosave
    put a moved request back).
  - **Migration:** the first open after V8 moves the old tables' collections and
    environments into an empty folder, keeping their ids (and so History links, the active
    environment and secrets); a folder that already holds collections is left alone. The
    old tables stay, unused.
  - **The app:** Settings → Collections folder shows the folder (in red with why, when
    it could not be read) and "Change…" picks another (`FileDialog::Folder`), switched
    at once through the unsaved-changes guard (`Navigation::SwitchFolder`,
    `files::switch`); the editor starts empty, since the open request was the old
    folder's. It is not part of Save: it replaces every collection shown. A refused save
    says why on the status line and keeps the edit for Retry. The environment dialog has
    "Share values".
  Pinned by `imposter-store/tests/files.rs`, `imposter-ui/tests/project.rs` and the
  existing store, app and UI suites, which run on files.

- **Variables are ordered maps** (in collection, environment and globals files). No
  environment may be named `Global` in any case, which names the globals scope ("global"
  beside it read as the same thing). Why: order is kept for display and export, and saving
  a scope replaces it whole, so a save never merges.

- **Globals are a file too** (`globals.yaml` at the top of the collections folder): the
  variables every collection of the folder shares, with the same "Share values" switch
  and local values as environments (local ones keyed by the folder, since the globals
  are the folder's). Why: an exception for globals left a checkout and CI without values
  requests used (they failed as unresolved there and worked here), and secrets and local
  values already cover what is personal (user's choice, after weighing a personal layer
  across folders). The old `globals` row moves to the file once, values shared (user's
  choice over keeping them local), and stays unused. A `--project` run gets the shared
  globals only, so a name whose value stays on someone's computer is unresolved, not sent
  empty. In the app, the Global page's ⋯ menu has "Share values" (through the Variables guard)
  and its explanation says where the values are. Pinned by `imposter-store/tests/files.rs`,
  `imposter-ui/tests/project.rs` and the layout lint's Global menu.

- **Shared values in files, local values on this computer** (user's choice: a send must
  never dirty git). A collection's and an environment's file hold the shared values;
  `local_scopes` (V8) holds what captures, `vars.set` and the inspector's value swap set
  (`scopes::set_current`, `choose_value`), and all of an environment's or the globals'
  values when they do not share them (`share_values: false`, the file then lists names
  only). The values in
  effect are local over shared. Saving on the Variables screen (`scopes::write`) makes what
  it shows the shared values and clears the local ones.

- **An environment may have a color** (`model::EnvColor`: red, yellow, green, blue, purple,
  or none), stored by name in the environment's file, which refuses other names
  and exported in native version 2; Postman has no equivalent and leaves it out. A new
  environment is offered red when a whole word of its name is `prod`, `production` or `live`
  (`EnvColor::suggested_for`), so "delivery" is not. Why: the environment in use, above all
  production, must be recognisable before Send (user's choice); a name pattern alone guessed
  wrong, so the user picks and the pattern only suggests. Colors are names, not RGB, so each
  theme draws them with checked contrast. Orange is not offered: it is the accent, and an
  orange environment read as focus (Phase 5 review, Q19; dropped before V4 was released).
  - **Choosing:** the New and Edit environment dialogs have swatches (No color, then the
    five; radio buttons to screen readers). A new environment's color follows its name until
    the user picks one, and says "Suggested for production environments" meanwhile; editing
    never changes it by itself. On the Variables screen, "⋯ → Edit name and color…".
  - **Showing:** the header's picker names the active environment on screen. A
    colored one fills it with its color's tint, its name semibold in that color after a
    "●" and a 3-point stripe of the full color on its left edge. Without a color the
    picker is plain. The idle button always reads "Send". Its hover text ("Send with Prod"
    with the shortcut hint) and accessible description ("Sends with the Prod environment")
    name the active environment with or without a color. The picker's options and the
    palette's environment rows have the dot. Every shade is ≥4.5:1 on every background
    and on its own tint (`theme.rs` tests). Why: few people read the button's words, and a
    long name cut the label or squeezed the URL field. A pill above Send repeated the name
    already shown in the picker and status bar, and a small dot still did not read as a sure
    sign (user's note). Send keeps the accent because coloring it by environment would
    break "orange means primary", and a red Send read as an error. A colored rim around
    the request bar was rejected because the shades also serve as status tones. A green
    rim read as success and a red one as an error. Hue alone failed colour-blind users
    (Q17, Q18). Not done: an imported or migrated "Production" still arrives without a
    color.
  - **Managing:** the picker ends with "Manage environments…", which opens Variables on the
    active environment (Global without one), through the Variables guard.

- **History rows carry summary columns** (name, method, URL, status, outcome, elapsed, and
  since V6 `via`: Send or the command line) next to the JSON payload. A run from the
  command line shows a muted "CLI" before its name in the list (before it, so a cut name
  cannot hide it) and is described "from the command line" to screen readers; its record's
  `via` is left out of the payload for Send, so older records read the same. Why: the list must not decode multi-megabyte payloads; details load one.
  Retention deletes oldest first by count and total payload size; a record larger than the
  whole budget is refused.

- **An execution reads its settings, variables and collection in one database command.**
  The database thread runs commands in order, so this is the snapshot taken before the first
  await: an environment switch queued before Send applies, a later one cannot change a running
  request. The variable inspector uses the same function (`executor::inputs`).

- **Secrets are references, `{{secret:name}}`, to values kept apart from variables**
  (user's choice, after Postman's `{{vault:name}}`; `{{$name}}` stays for generated
  values). Why: a token typed into an environment, or captured from a login, was an
  ordinary variable value, bound for exports, History and the project files M2 will
  write; the roadmap's rule is that secrets are references resolved from sources the
  user named.
  - **Scope:** each environment has its own secrets, falling back to the global ones,
    as variables do (user's choice over one flat vault). Collections have none.
  - **Storage:** the `secrets` table (migration V7), keyed by environment (NULL for
    global) and name, read and written by `store::secrets`. Not encrypted (user's choice):
    pure Rust on every OS, and encryption can come later without changing the syntax.
    Yaak's encrypted values inside synced files were rejected: secrets would still be in
    the files, and a key would have to travel to every teammate. Backups copy the table
    like the rest of the profile.
  - **Resolution:** the variable context carries the secrets that apply
    (`VariableContext::global_secrets`, `ActiveEnvironment::secrets`, read with the
    variables in one transaction), and `Snapshot::build` adds them as definitions keyed
    `secret:<name>` from `Source::Secret`. Resolution, checks, the inspector and the
    catalog therefore treat them as one more name; a missing one is unresolved like any
    variable. Resolution and redaction walk the same fields (`variables::visit_texts`).
  - **Captures** marked secret write to the secrets where the name is defined, else the
    active environment's, else the global ones (`capture::secret_target`), never to a
    variable; in a run they stay in the run, under `secret:<name>`.
  - **Scripts** read a secret with `secrets.get(name)` and store one with
    `secrets.set(name, value)` (`imposter_script::values::Secrets`), which goes where a
    secret capture goes; a later script in the chain sees it at once. Kept apart from
    `vars` so a set token never becomes a variable. A secret written into a script's
    source is code, stored with the request as written.
  - **History keeps no secret value.** `executor::record` removes every value given for
    a `secret:` name (stored, given to the run or captured by it, at least
    `redact::MIN_SECRET` long) from the stored copy: the request as sent, checks,
    captures, the response and the URL column (`Redactor::record`, which changes nothing
    else, so a `user:password@` the user typed stays theirs to see). A secret capture's
    own value is removed whatever its length. The live response pane keeps the values:
    it is on screen only. Why: History is kept for weeks and searched; the request as
    sent there shows `«redacted»` instead.
  - **The command line:** `--secret VAR=PROCESS_VAR` fills `{{secret:VAR}}` as well as
    `{{VAR}}`, so commands written before secrets keep working. `--var` never fills one
    (user's choice: "variables only, never secrets"): a literal stays in the shell
    history and the result, so secrets come only from `--secret` or the profile. A profile
    run reads the
    named environment's stored secrets with its variables; a `--file` run has none. Every
    secret a run used, captured or set goes into its redactor (`Ran::secrets`, carried on
    `runner::Step::Ran`), and a secret capture's value is replaced whatever its length
    (`send::without_secrets`). `imposter envs` lists secret names per environment and
    the global ones; `show` reports a `secret:` name as "Dev secrets", "Global secrets"
    or undefined. Pinned by `imposter-cli/tests/secrets.rs`.
  - **The app:** the Variables screen has a Secrets section under the variables of Global
    and each environment (none for a collection), saved with the page and guarded with
    it; values are masked with an eye per row (shown for the session only), have no
    saved values, and are refused under `redact::MIN_SECRET` characters so they can be
    removed reliably. The inspector always masks a secret until Reveal. A capture row
    has a lock (`Icon::Lock`) that stores it as a secret, its target then "→ Dev secrets";
    the response's Tests tab says "Stored … secret in Dev secrets" without the value. A
    blocked send offers "Define {{secret:x}} in Dev secrets", which adds the secret row.
    Pinned by `imposter-ui/tests/secrets.rs` and the layout lint.
  - **Exports never carry one:** exports read the variables, never the `secrets` table;
    references are kept as written.
  Pinned by `imposter-core` unit tests, `imposter-store/tests/secrets.rs` and
  `imposter-app/tests/secrets.rs` (the bytes received carry the secret; History,
  its URL column and both exports do not).

- **Requests that cannot be sent are not history.** Unresolved variables, a bad URL, auth or
  cookie conflicts, a failed before-sending script are reported and nothing is recorded: a
  URL error is not a transport error, since nothing was sent. Cancelled and failed sends are recorded with their outcome. A failed
  history write keeps the response and says so (`history_error`).

- **Saved values: a list per variable, kept in the variable's scope beside its current value**
  (`model::SavedValues`, in the scope's file, or locally for an environment that does not
  share its values; `store::scopes`). The
  current value stays the single source for execution, precedence, curl, history and exports;
  swapping (`scopes::choose_value`, from the inspector) is a local value, not a mode. Lists
  live in the defining scope, so each environment has its own. The inspector does not offer a
  swap while the Variables screen has unsaved edits to that scope, so a later Save cannot
  undo it unseen, and a clean screen on that scope reloads. Saved values travel in native
  version 2 with their scope; an import's new global names bring theirs and existing globals
  keep their own, as their values do. Postman has no equivalent, so its exports leave them
  out. Rejected: linked rows switched together (Paw's environment domains; the
  user swaps values independently), a send-time prompt (Insomnia; keeps nothing), and one
  environment per value (Postman; multiplies environments).
  Variables renders one wrapping list per row, including a single legacy scalar, with a
  filled selected chip and a short native Add value field after the chips. Enter adds and
  selects the draft. Save accepts pending drafts too. Typing alone leaves the selection
  in place. Why: the editable current field created a separate path to an active value,
  and the far-right add control and reserved remove spaces broke up the rows. Right-click,
  Shift+F10 and the accessibility action open the same chip menu. It copies or removes
  a value, with the last value removed through whole-row deletion.
  Capture and `vars.set` replace one SQLite slot per variable and activate it for request
  chaining. It stays available after selecting another chip or restarting. Keep in list
  adds it to the ordinary list and preserves the effective selection. A value already in
  the list appears once. Saving the scope keeps the latest slot local, and refuses a stale
  edit if a concurrent capture changed it. Why: rapidly changing request IDs should
  remain available without growing a history or being written into shared files by Save.

- **History search uses the sidebar's filter language** (`imposter_app::filter`, with `status:`,
  `env:` and `via:` added for History: `404`, `4xx`, `failed`, `cancelled`; `via:cli` or
  `via:app`). Words match the request
  name and the URL as sent; bodies are not searched, because that would decode every
  payload. The database thread scans the summary columns newest first, keeps matches and
  stops when the page is full (`executions::search`), so retention of up to a million entries
  never loads them all into the UI; without a filter, paging stays in SQL. Each list declares
  the fields it knows, and an unknown `name:` is text, so `status:` in the sidebar stays a
  word.

- **History shows times in the system time zone** (via `jiff`, display only); records store UTC
  milliseconds. The request of an execution is shown as sent (`viewer::sent_request`, built
  with the executor's own `prepare`), never a dump of the stored template. Restoring an
  entry opens an unsaved draft through the unsaved-changes guard and never sends; the draft
  keeps the captured collection variables for a deleted collection, and the response pane
  shows the recorded response, so the run it came from stays in view. Every draft (restored,
  pasted curl, or left open last time) has a banner above the request bar: "This draft is
  not in a collection yet." with "Save to <collection>", which is Save. Why: the draft's
  instructions lived only in the status bar, and the restored run's response was gone (the
  review's #42).
  - **The list is a table** (`history_screen::columns`): When (relative, "5 min ago", from
    `viewer::relative_time`; the exact time is the row's hover text), Name, Method in its
    color, URL with the repeating origin muted, a status pill ("201", "Failed",
    "Cancelled"), Time, and Environment from 900 points on. Text is cut with "…", never
    clipped mid-glyph. Rows are painted, not laid out, so the list stays virtualized.
  - **Tools:** the filter, "Current request only" and Refresh; "Clear all history…" is in
    the "⋯" menu. Newer and Older are a pager under the list, with which executions show
    and what history holds against what it keeps ("312 of 1000 executions · 41 MB of 250
    MiB", `executions::usage`, the same sum retention compares); Settings says it under the
    limits. Why: the limits alone left no way to judge them.
  - **Details:** one line ("GET url · 201 Created · 3 ms") with Restore as draft and Copy as
    curl beside it, then one row of tabs: Response (first, as runs are looked up for it),
    Request body, Request headers. An empty body or header list says so in muted text.

  Why: the list had no name or duration and spent its width on full dates, URLs were
  clipped without an ellipsis, "Clear all history…" sat beside "Copy as curl", the actions
  were far from what they acted on, and the details had two levels of tabs with "No body"
  printed as code (the review's #43–#46).

- **Default profiles copy `~/.poster/poster.db` once to `~/.imposter/imposter.db`, only when the new database is absent.** SQLite backup captures WAL data, validates integrity, references and supported schema, closes connections, then publishes without replacing a concurrently created destination. Busy handling is bounded; failed copies leave no partial profile and report recovery guidance. Explicit `IMPOSTER_DATA_DIR` directories never consult home. Why: preserve every saved domain without merging profiles or changing the old database. Enforced by store profile tests and app startup tests.

- **Only `IMPOSTER_DATA_DIR` and `IMPOSTER_LOG` configure the renamed app; old names are ignored and an empty directory override is an error.** Why: scripts must explicitly select the intended app and an invalid override must not fall back to personal data. Enforced by isolated subprocess environment tests and the binary smoke test.

