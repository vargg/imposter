# Scripts & Mocks Decisions

Part of the [Imposter agent manual](../../AGENTS.md). Decisions governing Rhai request scripting, checks, captures, generated values, and the loopback mock server.

## Decisions

- **Checks are data, not scripts** (`core::check`): a selector (status, a header, a JSONPath,
  the body, duration, size), an operator and an expected value that may use `{{variables}}`,
  stored in the request template. The executor evaluates them after every response with the
  execution's own variable snapshot and keeps the results on the response, so History shows
  them. A check ends Passed, Failed (with the value read) or Not checked with the reason (a
  cut or non-JSON body, a bad path or regular expression, an unresolved variable): an
  undecidable check never passes or fails silently, and it never blocks a send. Strings
  compare as text and numbers by value (`19.50` equals `19.5`); status also accepts `2xx`.
  OpenAPI import adds a status check for the lowest documented success code and none when
  there is no documented success. Postman export lists checks as not exported. Logic that
  data cannot say goes in a script (below); checks stay the plain way to test a value.

- **Scripts are Rhai** (`imposter-script`, the only crate that embeds it), for requests
  (before sending, after the response) and for mock routes, pure Rust and sandboxed by
  design: nothing reaches files, the network, the clock or the environment
  unless the host registers it, and it limits operations, string, array and map sizes and
  call depth. Its features are pinned in the root manifest: `sync` (a compiled script can be
  shared by threads), `no_module` (no `import`, so no file module resolver), `no_time` (the
  host gives time), `no_custom_syntax`, `serde`; never `unchecked`, which removes every limit,
  or `internals`. The crate repeats core's no-IO clippy lists.
  - **Engines must not compile C of their own** (stricter than the rule for other
    dependencies, which lets rusqlite build its vendored SQLite) and must not ship prebuilt
    binaries: that rules out QuickJS (rquickjs, or as WASM run by wasmi), Lua (mlua, Luau)
    and V8. Boa, pure-Rust JavaScript, was the alternative; it has no memory cap and cannot be
    stopped from another thread. JavaScript would have been familiar and kept Postman's
    `pm.*` scripts, which therefore are still removed on import, with consent.
  - **The sandbox** (`imposter_script::{before, after}`), one engine per run: at most
    1,000,000 operations, strings of 8 MiB (bodies are strings in scripts), arrays and
    maps of 100,000 items, 64 call levels, 64 KiB of source, 500 tests; `eval` is disabled.
    Every 1,024 operations the host is asked whether to stop (its deadline, or Cancel), so
    the crate reads no clock. A limit or a stop ends the script with a plain "Stopped: …"
    and is never taken for a failed test. `print` and `debug` go to a console capped at 200
    lines and 64 KiB, never to stdout or `tracing`. Errors carry their line (and column when
    compiling) without Rhai's own position suffix. Digests and HMACs give bytes, so
    signatures can be chained (AWS Signature 4); `hex` and `base64_encode` make text. The API
    is one table (`imposter_script::API`), and a test runs every example in its phase.
  - **Measured** (Phase 0, M-series Mac, release): the binary grows 1.9 MiB (19.3 to 21.3
    MB); Rhai compiles in about 23 s; an engine is made in about 0.13 ms; parsing and walking
    a 10,000-item JSON array takes about 12.5 ms.

- **Request scripts run around the send** (`executor::execute`), in this order: resolve
  variables and generated values; the before-sending script on the resolved template;
  prepare (auth headers are added here, after the script); send; checks; captures; the
  after-response script; record. So the before script sees real values and what it
  changes is what goes out, what "Request as sent", History and Copy as curl show
  (user's choice over Postman's order, where scripts see `{{placeholders}}`).
  - **The chain** (`imposter_app::scripts`): the collection's scripts, then each enclosing
    folder's from the outermost, then the request's, in that order in both phases
    (Postman's order, user's choice over unwinding the after scripts). Each before script
    runs on what the one before left and sees its variables; after scripts all run, one
    failing does not stop the others, their tests are pooled and each failure is one line
    of the report's error naming its script ("Folder \"Users\" after-response script, line
    2: …"). The chain is read with the variables in `executor::inputs`, so it is the same
    snapshot; a folder that no longer exists, as for a History draft, adds nothing, and
    its collection's scripts still run. Send, Copy as curl and load runs all compile and
    run it through this one module.
  - **Failures:** a before-script error or stop is the request's "not sent" reason
    ("Before-sending script, line 3: …"), and nothing is recorded. An after-script error
    is reported with the response, its tests so far kept, and counts as a failure. The
    after script runs only when a response arrived, as checks do.
  - **Time:** each script may run 2 s (`SCRIPT_TIME`); Cancel stops it too. Scripts run on
    `spawn_blocking`, so a slow one never holds an async worker.
  - **Variables:** the after script sees the before script's sets. Every `vars.set` becomes
    a capture-style write (`capture::target`: the defining scope, else the active
    environment, else Global), after the captures, so a scope with unsaved edits on the
    Variables screen is protected exactly as for captures.
  - **The Scripts tab** (`scripts_tab.rs`, after Captures, a dot when either is set) holds
    both scripts in one view (two levels of tabs were removed from History, the review's #46).
    Each editor is highlighted by a small hand-written Rhai
    tokenizer (`highlight::rhai`) and gets the body editor's typing aids (`widgets::Typing`).
    The [shared script header](ui.md#request-bar--editors) holds compile status and
    phase-specific examples. Script reference opens a sheet drawn from
    `imposter_script::API`, as is the README's table (a test keeps them equal). In a narrow
    pane a header's title is cut with its full text on hover rather than widening the pane
    (found by the layout lint at 720 points).
  - **Collection and folder scripts are edited in a dialog** (`Dialog::Scripts`, from
    "Collection Scripts…" in the collection's "⋯" menu and "Scripts…" in a folder's), with
    the same two editors and Save; the request's Scripts tab lists the levels that also run
    ("Also runs, before this request's:", outer first), each a button opening its dialog,
    and a folder with scripts is described "Has scripts" to screen readers. The caption and
    buttons use shorter labels in narrow panes with their full meaning available to screen
    readers and on hover. Why: the inherited script controls compete with editing space at
    720 points, and the dialog saves collection or folder scripts in one command.
    Enforced by `script_editors_fit_and_reveal_the_last_line_in_every_layout`.
  - **Results:** the response's Tests tab lists the script's error, failed tests, checks,
    passed tests, captures, then the console; script failures count with failed checks in
    the tab's badge, the status line's "✗n" and "n checks failed".
  - **Load runs run them too**, for every request (see the load decision); only their
    variable writes and console are dropped there.
  - **Console:** shown with the live response only, never stored (`Response.console` is
    `serde(skip)`: printed values often hold tokens; user's choice) and never logged
    (`imposter-app/tests/scripts.rs` captures trace-level logs). Script tests are stored
    with the run, like check results.

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
  The response pane's Request tab ("Request as sent") shows the request that brought the
  shown response, generated values filled in, from `Executed::sent` or the run's record;
  before it they could be read only in History (the user's note).

- **Captures set variables from a response, explicitly and visibly** (`core::capture`): a
  variable name and a selector, stored with the request. The value goes to the scope whose
  definition the request saw (so the value it used is the one replaced), else to the active
  environment, else to the globals; as local values only, never a file, saved values
  untouched (`scopes::set_current`). Nothing is taken without a response, and a selector that names
  nothing leaves the variable alone. The executor only computes the writes; the app applies
  them in one transaction when the response arrives, even if the user has moved to another
  request, except for the scope the Variables screen has unsaved edits to: those are not
  written and the response says so, as with saved values. Statuses name variables, never
  values. Rejected: Yaak's `response.body.path(request)` template function, which hides the
  dependency in a placeholder and can send another request on its own.

- **A mock route may compute its answer with a script** (`MockRoute::script`, Rhai through
  `imposter_script::mock`), starting from its static status, headers and body; static routes
  are unchanged and never read bodies. `imposter-http` serves scripted routes through a
  `Responder` callback the app passes in, so the server crate needs no script engine.
  - **Inputs:** the request (method, path, query, headers, body up to 1 MiB, else 413) and
    `state`, one map per run, empty on every Start. Scripts run one at a time per server
    under one lock, so counters never lose an update, each for at most 1 s
    (`MOCK_SCRIPT_TIME`); the delay a script asks for (up to 10 s) is waited after the lock.
    `state` changes only when the script succeeds.
  - **Failures:** a script that fails, is stopped, or answers what the server will not send
    (`MockReply::problem`: status, framing headers, line breaks) is answered 500 with
    "Mock script, line 3: …" as plain text. A script that does not compile keeps the server
    from starting or autosaving (`mock::script_problem`, "Route 2: …"). Explicit Save
    can keep a broken script as work in progress.
  - **The log:** every answered request reaches the UI through a callback (`MockLogEntry`:
    method, path, status, time, error, console), never a body, and never `tracing`
    (`imposter-http/tests/logging.rs`).
  - The route's **Script** tab uses the request script editor (`scripts_tab::script_editor`)
    with mock examples. An enabled route whose script does not compile is a
    `RouteField::Script` problem (`mock_screen::script_problems`):
    ⚠ in the list, Start disabled naming it. Scripted routes show the `</>` script mark
    after their status, the same mark as a collection or folder with scripts in the request
    tree (a "· script" word was one more way to say it), and screen readers hear
    "Status 200, answered by its script". "Answered
    requests" under the routes (newest first, the last 100, cleared on Start) shows each
    request with its time, status in its tone, a script's error and printed lines; it is
    not called "Requests", which names the header's tab. Why: without it a script's errors
    and prints would only show as a 500 in the client.

- **Mocks is one saved loopback-only static HTTP/1 server, started and stopped explicitly.** Enabled routes match exact method and encoded path, ignoring query and body; unmatched requests return 404. A running server keeps its start-time configuration until restarted. Why: predictable responses and no implicit listener after startup. Configuration uses the shared autosave setting. Pending changes save on leaving the screen or closing, and the existing guard asks when validation or saving fails. Revert still offers the guard before reloading. Shutdown cancels and drains the listener and all connections before closing. Enforced by mock domain, service, loopback, and UI tests. Route problems are all found at once (`MockConfig::problems`, each tied to a field) and shown under that field while typing; saving and the server refuse with the first (`validate`), and a disabled Start says which route to fix. Why: a Start button that only greyed out left the user guessing. The screen is a list and an editor: the header puts Port ("auto" for 0) and Start or Stop first, so they never move, then the state in words ("● Stopped", "● Running at http://127.0.0.1:62543" with a copy button), with Revert and Save at the right; the list shows each route's enabled box, method in its color, path, and status or a ⚠ when it has a problem. The editor beside it has the request bar's method picker, path, status, and a two-click "Delete route"; the list's box is the route's only on/off control, because a second Enabled switch in the editor repeated it. The response fields have their own tabs so a growing header table leaves the body and script editors in place. The first route is open until another is chosen. Why: the server's state was not shown, its URL pushed the buttons aside, routes were plain text, and the method was free text (the review's #47–#49).

