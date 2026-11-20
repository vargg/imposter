# CLI & MCP Decisions

Part of the [Imposter agent manual](../../AGENTS.md). Decisions governing the CLI runner (`imposter send`, `list`, `show`, `run`, `skill`), headless execution, and the local MCP server experiment.

## Decisions

- **Load runs send one request many times, apart from Send** (`imposter_app::load`,
  `imposter_core::load`), as `ab` does with JMeter's measures.
  - **Plan** (`LoadPlan`, checked when built): requests in total (1 to 1,000,000, as `ab`
    counts them, not per worker), concurrency (1 to 256, no more than the requests), an
    optional time limit (1 s to 1 h), an optional ramp-up that starts the workers evenly
    over it (under 10 min and the time limit), and keep-alive.
  - **Each request** gets fresh generated values (`{{$uuid}}` differs every time), from
    one variable and settings snapshot read at the start. The request is checked once
    first: one that cannot be sent stops the run as it stops Send, before anything goes.
  - **Kept apart:** nothing goes to History (a run would push everything else out through
    retention) and captures are never written. Checks run on every response; the body is
    kept up to the capture limit only when a check reads it, and is otherwise counted
    and dropped, but always read to its end so the connection can be reused.
  - **One session per run** (`transport::Session`): with keep-alive, up to the
    concurrency idle connections are kept and reused; the pool may open one more while a
    finished connection is on its way back, so tests check reuse, not an exact count.
    Without keep-alive every request connects afresh.
  - **Its own runtime per run**, shut down at its end, so the UI, the database thread and
    other background work do not skew the timings (JMeter's advice not to load-test from
    its GUI). Progress is two counters the UI reads each frame.
  - **Measures** (`Summary`, after JMeter's Aggregate Report and `ab`): requests and body
    bytes per second; errors are transport errors plus responses failing a check, as
    JMeter counts a failed assertion; min, mean, p50, p90, p95, p99 and max (nearest
    rank) of the whole exchange and of the time to the response's head (JMeter's
    "latency"); `ab`'s "served within" table; counts by status and error kind; a 1-2-5
    millisecond histogram. Latency counts responses only: a refused connection is an
    error, not a fast request.
  - **Scripts** run for every request: the before-sending one on each request's resolved
    values (so per-request signatures work), the after-response one on each response,
    its tests counted with the checks and an error as one failure. Each is compiled once
    per run (`imposter_script::compile`) and run with its own small engine (about 0.13 ms);
    the first request is checked with its before script, so one that cannot compile or
    run stops the run before anything is sent. Later failures are `ScriptError` samples
    with nothing sent, and the first error message of the run is kept on the report and
    shown on the result. Script time is not latency: the exchange alone is timed.
    `vars.set` and printed lines are dropped, as captures are never written. The screen
    says so for an open request with scripts.
  - **Stopping** (the user or the time limit) drops the requests in flight and sums up
    what finished.
  - **The Load screen** (`load_screen.rs`) opens from Load in the left rail, reached also from a
    request's right-click "Load test…" and the palette ("Load Test This Request", and
    "Change…" opens the palette to choose any saved request). Opened with nothing chosen
    it takes the open request. The request bar does not change: a Send menu whose one
    item only switched screens would add a click and cost the URL width. A run sends what
    Send would send now when the request is open, edits included, else the stored one.
    Target and plan come first so Start and Stop never move; why a plan cannot run is
    written beside Start. With a red environment active, Start asks first ("Load test
    Prod?"), since red marks production. The status bar opens its screen while a run goes on, and the
    last ten runs of the session are kept in memory, nothing stored but the plan's values.
    Plan fields are spoken with their unit ("Time limit in seconds"); "Requests" alone was
    also the header's tab.
  - **Closed loop:** each worker waits for its response before the next request, as `ab`
    does, so when the server slows down fewer requests go out and tail latency reads
    lower than users would see. Expansion is frozen:
    fixed-rate mode, CSV test data, multi-request scenarios, pass/fail thresholds and
    an over-time chart are out of scope, as are distributed load, other protocols and
    JMeter's test-plan trees. Why: capacity testing belongs in specialist tools; this
    app keeps a one-request probe and fixes to its current behavior.

- **One engine runs every request: `executor::run`, which reads and writes nothing.** Send
  is `inputs` + `run` + `executor::record`; the command line is `profile::select` (a saved
  request, from the personal profile) or `document::select` (a native file) + `run`, then
  `record` for the profile. `Inputs::writes` says where captures and script variables go
  (`capture::Destination`): back to the stored scopes (Send), or only into the run, named
  "this run" (files, and the command line against the profile: an agent must not change
  the user's variables behind their back; writing a value back is a separate edit). Scopes
  read from a file have no database id (`None`) rather than invented ones, and a value given
  to the run is a `Source::Input` above every scope.
  - **Naming a request** (`document::RequestPath`): the breadcrumb, collection first
    (`Shop API / Users / Create user`); spaces around `/` are ignored so a copied breadcrumb
    works, `\/` is a slash in a name, and in a file the collection's name may be left out.
    A path or environment naming nothing or several (collection names need not be unique)
    is refused with the candidates.
  - **The profile run** (`profile::select`) reads the request, settings, variables and
    scripts in one database command, like Send, with the environment named by `--env` and
    none otherwise, never the app's active one (user's choice: an agent's run must not
    depend on what the app shows). A red environment is refused unless Settings → Command
    line → "Allow production environments" is on (`AppSettings::allow_production_from_cli`,
    off by default): a flag on the command line would be one an agent can add (user's
    choice). It records the run in History as the command line's (`Via::CommandLine`). It
    opens only a profile that exists (`paths::open_existing_profile`): nothing is created,
    copied or chosen implicitly.
  - **A file run** uses the application defaults, never the profile's settings.

  Why: the roadmap's "one contract": a headless run must not grow a second variable,
  script or check implementation. Pinned by `imposter-app/tests/document.rs`, which sends
  one file through both paths and compares the bytes received and the verdicts, and
  `imposter-app/tests/profile.rs`.

- **A headless run reports one result schema** (`core::run_result::RunResult`, format
  `imposter-result`, version 1, or 2 with an event stream), for the command line's JSON, result files and the
  desktop's view. It leads with a summary line and an outcome, then what ran (file SHA-256,
  request path, environment, input names, never values), checks, script tests, captures,
  the request as sent, the response head and body excerpts, and the limits it was made
  under. The script console is never part of it, as it is never stored. `definition.source`
  says whether it ran from the profile or a file; only a file run has a SHA-256, since
  database rows have no portable revision.
  - **Outcomes** each have an exit status: passed 0, failed 1 (a check or a script test),
    not sent 3, transport error 4, script failed 5 (before sending or after the
    response), not checked 6, no checks 7, cancelled 130; 2 is for usage errors, which
    have no result. When several apply, the one that says least was verified wins
    (`RunOutcome::of`). A response with no checks is not success: it is an execution,
    not verified behavior (the roadmap's "What a run means").
  - **Redaction** (`core::redact::Redactor`) runs over every string of the result, keys
    included: the run's secret inputs and the credentials it sent (Basic and Digest
    passwords, the Basic `user:password` pair, Bearer tokens, API keys, OAuth client
    secrets, access and refresh tokens, the client key password),
    each also percent-encoded and base64 in both alphabets; `user:password@` in any URL;
    and the whole value of `Authorization`, `Proxy-Authorization`, `Cookie` and
    `Set-Cookie`. Bodies are redacted before the excerpt is cut, so a secret across the
    cut leaves nothing behind (found by the golden test). Filtering by known values cannot
    find every sensitive value a server returns, and the README says so. Pinned by
    `imposter-core/tests/run_result.rs` and its golden file
    `tests/fixtures/results/sent.imposter-result.json`.

- **The command line is `imposter send <REQUEST>` in the same binary** (`imposter-cli`,
  reached from `main.rs` before logging or the window start). By default it runs a saved
  request from the personal profile (`profile::select` + `executor::run` +
  `executor::record`): the user edits requests in the app and runs them from a terminal or
  an agent with nothing to export (user's choice; exporting after every edit only added
  manual steps). `--file FILE` runs from a native file instead, for CI or a machine without
  the profile (`document::select` + `run`), reading the file once and recording its
  SHA-256; it writes nothing. Either way it prints the redacted `RunResult` on stdout, its
  summary and notes (a failed History write) on stderr, and with `--out` the same JSON to
  a file (atomically). It reads only the process variables named by `--input`/`--secret`
  (`Io::var`) and `IMPOSTER_DATA_DIR`, never the rest of the environment; `--var` takes its
  value in the command line (or a file after `@`) and reads none. A missing or
  empty input, or a secret under 4 characters (`redact::MIN_SECRET`, which would redact
  ordinary text), stops the run before anything is sent. Ctrl+C cancels (exit 130). Logs
  default to warnings, since stderr carries the summary. Arguments are parsed with
  `lexopt`; usage, the README's tables and the help come from `SEND_FLAGS` and
  `exit_statuses()`, and a test keeps the README equal to them. Why: agents and CI cannot
  depend on the window, and a second executable would need its own packaging. Pinned by
  `imposter-cli/tests/send.rs` (every outcome, inputs, the freeze, the profile and its
  History, production and a missing profile refused) and `imposter/tests/cli.rs` (the
  process: no profile created for a file run, no secret on stdout or stderr even at trace
  level, the interrupt).

- **Literal per-run inputs: `--var NAME=VALUE`, variables only, never secrets** (user's
  choice). An agent driving the command line must give values that change every run
  without touching the saved request — an OpenSearch body changes completely per call, and
  predefining every value was impractical enough that one agent rewrote the collections
  folder's files to work around it. A `--var` literal fills a plain `{{variable}}` only,
  never a `{{secret:name}}` (`InputSource::Literal` has no secret flag, so a literal secret
  is unrepresentable): a literal bypasses the redactor and sits in the shell history, so
  secrets stay with `--secret` and the profile. A leading `@` reads the value's whole file
  relative to the working directory (`@@` is a literal `@`; `@-` is refused); a file that cannot be read, is not UTF-8 or is empty stops the run as
  not_sent, as an unset process variable does. A name given twice across
  `--input`/`--secret`/`--var` is a usage error, so a `--var` can never silently shadow the
  user's `--secret`. The result schema is unchanged: the input records
  `from: "--var"`, never the value. Pinned by `imposter-cli/tests/send.rs`,
  `run.rs` and `imposter/tests/cli.rs`.

- **`imposter list`, `show` and `envs` let an agent find and read requests before running
  them** (`imposter_app::catalog`, `imposter_cli::catalog`), from the profile or a `--file`,
  writing nothing. `list` gives each request's path, method, URL template and how many
  checks and captures it has, and whether scripts run around it, filtered with the
  sidebar's language (`imposter_app::filter`, moved out of the UI for this); its own parser
  takes `-word` as a negation, not a flag. `show` gives one request as stored and every
  `{{variable}}` it uses (`variables::used`) with the scope that defines it for `--env`
  (`generated`, `undefined`), names only. `envs` gives environments with their colour,
  globals and collections, by variable name. Values never appear (user's choice:
  placeholders, not values), and a stored credential (Basic and Digest passwords, Bearer
  tokens, API key values, OAuth client secrets, the key password, `Authorization`, `Proxy-Authorization` and `Cookie` headers,
  cookies) shows only when it is nothing but placeholders, optionally after a scheme word
  (`redact::shown`), else `«redacted»`; a secret pasted into a body cannot be told from
  data, and the README says so. Inspecting with a production environment is allowed
  (`profile::Purpose::Inspect`): it reveals only names. Output is JSON, or text with
  `--text` (user's choice). The commands, help, README table and skill come from
  `COMMANDS`, which a test keeps equal to the README. Why: an agent could run a request
  but not find one, nor learn what it does without sending it. Pinned by
  `imposter-app/tests/catalog.rs` (profile and file agree; no value or literal credential
  in any output), `imposter-cli/tests/catalog.rs` and a process test in
  `imposter/tests/cli.rs`.

- **A run executes a folder or a collection as one run** (`imposter_app::runner`, selected
  by `profile::select_all` or `document::select_all`). The target is a breadcrumb naming a
  collection, a folder or one request; requests run depth first in the tree's order
  (siblings by position, folders and requests mixed, as the sidebar shows them). Every
  request's template, settings, variables and scripts are read at the start, in one
  database command, so an edit during the run changes nothing. Values a request captures
  or sets (`Writes::Run`) go into a layer above the inputs and every scope
  (`Source::Run`, "this run"), so a login's token reaches the requests after it; nothing is
  written back to the user's variables. The run stops at the first request that did not
  pass and reports the rest as skipped, unless told to keep going (user's choice: later
  requests usually depend on what failed). A run from the profile records each request in
  History, marked with who started it (`runner::Options::record`: `Via::CommandLine` or
  `Via::App`); on the command line a production environment is refused once, up front. The
  report (`core::run_report::RunReport`, `imposter-run` version 1) has each request's
  redacted `RunResult` or the reason it was skipped, totals, one outcome and JUnit XML.
  The run passes when no request failed, errored or had an undecidable check and at least
  one check ran (user's choice: a login that only captures needs no check of its own); a
  run with no checks is `no_checks`, an empty selection `not_sent`; otherwise the outcome
  that says least was verified wins (`run_report::outcome`, which the app's Run screen
  uses too, so both give the same verdict). On the command line it is
  `imposter run <TARGET>` (`imposter_cli::run`): `send`'s options plus `--keep-going` and
  `--junit FILE` (`RUN_FLAGS`, kept equal to the README by a test); stderr gets one line per
  request as it ends, stdout the redacted report, and `--out`/`--junit` are written
  atomically. `docs/ci.md` shows a CI job running it from an exported file. Pinned by
  `imposter-app/tests/runner.rs`, `imposter-core/tests/run_report.rs`, the JUnit golden
  file `tests/fixtures/results/run.junit.xml`, `imposter-cli/tests/run.rs` and a process
  test of an interrupted run in `imposter/tests/cli.rs`.

- **The command line reads the collections folder** the profile names, and `--project DIR`
  names another one (a repository's checkout in CI), read straight from disk with no
  profile at all (`imposter_app::project`): each collection becomes a native document
  carrying the folder's environments, so `document::select`, the runner and the catalog
  work on it as on an exported file. A path names its collection by its first part, or
  may leave it out when the folder holds one. The result says `source: "project"`, with
  the folder in `file` and, as `sha256`, a hash over every file's path, length and bytes
  (`send::folder_revision`), so any edit is a new revision. `--file` and `--project` are
  exclusive. Why: a teammate or CI runs a checkout without the author's database or an
  export step (M2's exit). Pinned by `imposter-cli/tests/project.rs` and a process test
  in `imposter/tests/cli.rs`.

- **`imposter skill` prints the instructions a coding agent needs** (`skill.md` in
  `imposter-cli`): how to choose a request, pass secrets by process variable name, read
  the result and propose a check as a reviewable diff. Its option, exit status and
  operator lists are filled from `SEND_FLAGS`, `exit_statuses()` and `check::Op::ALL`
  when printed, so it cannot drift from the parser. It only prints: the user decides
  where their agent reads it, and the app writes no agent configuration (the roadmap's
  rule). It teaches the order `list`, `show`, `send`.

- **MCP authority lives in the stdio subprocess configuration.** The experiment uses
  one explicitly named collections folder and environment. Inspection, editing, execution
  and evidence disclosure are separate grants, and red environments need a named
  production opt-in. The host selects by stable file ID and revision. It can supply
  neither inputs nor paths to file operations. Why: project files and tool arguments are
  editable by the agent, and the initial spike let an input name a local file. Stdio
  carries JSON-RPC alone on stdout, and the server ends with its input.
  - A network grant pins each allowed origin to a literal IP. The shared transport
    checks the URL after scripts and checks redirects and OAuth token requests before
    connecting. Proxies, custom Host headers, HTTP tunnels, uploads, custom
    certificates and disabled TLS verification are refused in this experiment. Why:
    an origin is only enforceable if the transport controls the connection, and the
    initial grant has no separate authority to read arbitrary files. Tests pin direct
    denial, a script changing destinations, redirects, host resolution and file denial.
  - A granted edit replaces the method, URL, description or checks in the existing
    request file and returns a redacted before-and-after diff. App and MCP writers lock
    the same hidden project lock file before comparing bytes and replacing definitions.
    MCP compares the entire expected revision; the desktop's existing outside-edit
    guard preserves an unsaved draft. Why: two writers could otherwise both compare
    against the same bytes before either replaces them.
  - Execute uses the shared sequential runner and returns a run ID immediately. A
    separate call cancels the active run and the outcome remains readable. Evidence is
    redacted by the same result code as the CLI. A session keeps eight completed runs,
    accepts up to 32 requests per run, captures up to 1 MiB per response and discloses
    at most 4 KiB per body. Oversized reports can be read a step at a time. Why:
    a host should remain responsive and responses should fit the disclosure grant.
  Pinned by the stdio process test and the MCP integration tests using a project on
  disk and loopback servers. Host usefulness remains an experiment.
