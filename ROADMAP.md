# I’mposter roadmap

The product plan: who it is for, which work is core, which is frozen or out, and the
order it lands. Shipped behavior and engineering rules live in `AGENTS.md`. A milestone
describes work that is not built yet. When a milestone lands, its decision moves into
`AGENTS.md` with the tests that pin it.

Adopted on 6 November 2026.

## Vision

**I’mposter is a local, private HTTP workbench. Developers and small teams who build
HTTP APIs, by hand or with a coding agent, turn a failure into a saved request, a check
and evidence that can be reviewed, rerun and kept with the project, from the app, the
terminal and CI. No account, no cloud, no telemetry.**

The collection is the product. A feature that does not author, run, check or inspect a
collection's requests is out of scope.

The same collection is sent, checked, answered locally by the mock while the API is not
there yet, or repeated as a closed-loop probe. The mock and the probe are supporting
tools.

People already have curl and other clients. I’mposter earns a place when the saved
request, the check and the evidence outlive the session and run the same way in the app,
in CI and for an agent, without an account. The desktop is where a person inspects and
corrects that work. The terminal is enough to run it. The license stays MIT, with no
paid gate on this workflow.

## Who it is for

- Developers and small teams building and testing their own HTTP APIs, sharing
  definitions through the code repository, working in the app or with a coding agent.
- The same workflow by hand is in scope. A person who never runs an agent can use the
  app, the runner and CI.
- Outside this plan: performance engineering, enterprise API governance, and QA
  platforms that need test-case management.

The switching test: if an agent plus curl, or an existing client, produces the same
evidence with less setup, the milestone has not earned its place. Users with large
Postman JavaScript suites are not asked to rewrite them in order to adopt I’mposter.
A small working subset has to be useful on its own.

## Landscape

Checked against first-party docs in November 2026. An advertised feature is not a
quality, adoption or pricing claim.

| Tool | What it changes in this plan |
|---|---|
| [Postman](https://learning.postman.com/docs/reference/postman-api/postman-mcp-server/overview) | Documents a CLI for repository agents and an MCP server for chat tools. This plan follows that split: the CLI is core, MCP is a later experiment. |
| [Bruno](https://docs.usebruno.com/introduction/getting-started) | Collections as files in git, plus a CLI. The reason project files are core. |
| [Yaak](https://yaak.app/) | A local app with directory sync, a [CLI](https://yaak.app/docs/getting-started/cli-usage) and an [MCP plugin](https://yaak.app/docs/getting-started/mcp-server). The closest existing shape. |
| [Hurl](https://hurl.dev/) | Text-defined sequences with captures, checks and a failing exit. The CLI example this plan follows. |
| [Insomnia](https://developer.konghq.com/insomnia/), [Hoppscotch](https://hoppscotch.com/) | Broader clients, with local and self-hosted options. This plan does not claim that every alternative requires a cloud account. |
| [Mockoon](https://mockoon.com/docs/latest/about/), WireMock | Where full mocking belongs. |
| [k6](https://grafana.com/docs/k6/latest/using-k6/scenarios/concepts/open-vs-closed/), JMeter | Where capacity testing belongs. Our probe is a closed loop: a slow server receives fewer requests, and the result says so. |

## Core, already built

Protect these. The milestones run on them.

- Requests: any method, headers, cookies, and bodies as JSON, text (including XML and a
  GraphQL document sent as an ordinary POST), form, multipart or a file. Uploads stream.
- Basic, Bearer and Digest auth. Per-request settings. TLS, proxies, and client
  certificates matched to an exact host.
- Variables in global, collection and environment scopes, with saved values, generated
  values and environment colours.
- History, search, restore, and the request as it was sent.
- Response viewer: virtualized rows, highlighting, JSONPath filter and fold, a bounded
  preview, and saving the full body.
- Declarative checks, captures, and sandboxed Rhai at collection, folder and request
  level, including mock routes. Scripts have no file, network or process-environment
  access.
- Import and export: native format (version 3 when scripts are present), Postman v2.1
  and curl, both ways. OpenAPI 3.x is import only.
- One loopback mock server, started explicitly, exact method and path, static or
  scripted answers, and a request log.
- A closed-loop load probe of one request.
- Command palette, themes, density, and named controls for assistive technology.
- One native binary per OS. Without arguments it opens the window. `imposter send` runs a
  saved request headless from the personal profile (recorded in History as the command
  line's), or from a native file with `--file` for CI. It takes named secret inputs and
  gives a redacted JSON result, an exit status per outcome and a result file History
  opens. `imposter skill` prints the agent instructions. `imposter mcp` is the
  experimental spike below.

## Core, still to build

In milestone order. MCP is not in this table; it is an experiment below.

| Work | Why it is core | When |
|---|---|---|
| Project folder: YAML, one file per request and per folder | Teams review definitions next to the code. | M2 |
| A runner for a folder or the collection, in the app and the CLI | A check pays off when the set runs together. | M2 |
| OAuth 2.0, API-key auth, a cookie jar | A pasted bearer token goes stale, and a login's cookies are not kept. | M3 |
| SSE responses | A body that appears only at the end hides a stream. | M3 |

## Frozen: keep, and stop growing

Mocks and load stay, with their current behavior, data and tests.

- **Mocks.** One loopback server, explicit start and stop, exact method and path, static
  or scripted answers, the request log. Fixes are in scope. Hosting, tunnels, recording,
  proxying, rich matching and response sequences are not.
- **Load.** One request, many times, closed loop, the measures the app already computes.
  Fixes are in scope. Fixed-rate mode, CSV workloads, multi-request scenarios, pass/fail
  thresholds, charts and distributed workers are not. A later proposal has to name the
  task, why handing the job to a specialist tool is not enough, and which committed item
  it displaces.

## Experiment

**A thin local MCP server**, after the CLI exists. Coding agents already have a shell,
which is why this is not on the core path. Chat tools without a shell are why it is
worth trying.

- M1 includes a narrow stdio spike over the single-request path, to find contract
  problems early. The spike is not a general HTTP client and not the tool list below.
- M4 ships that small list, then keeps or drops it. Usefulness on two real hosts, and
  the security tests in the secrets decision, are the gate. A drop is recorded in this
  file.
- The desktop, the CLI and CI are a complete release without M4.

**M1 spike findings (7 November 2026).** `imposter mcp <FILE>` (experimental) serves two
tools, `list_requests` and `send_request {path}`, over stdio on the same `send` code as the
CLI. The file, environment and inputs come only from the command line the user wrote.
Covered by protocol, cancellation and secret tests; not yet tried in a real host. What it
showed for M4's contract:

- Paths of names are a weak identity. A rename breaks every reference an agent kept, and
  duplicate names are refused at call time. The tool contract needs M2's stable request
  ids, with paths for display only.
- MCP asks for structured content to be repeated as text, so a result with a 64 KiB
  excerpt costs twice that in context. M4 needs a smaller default excerpt and the
  planned "Read result" by run id, not the whole result in every call.
- A failed check is a successful tool call (`isError: false`, the outcome is data). A
  host that only shows `isError` presents a failing run as fine. The summary leads the
  text for that reason; M4 should also declare an `outputSchema`.
- Starting the server with fixed inputs works as a coarse execution grant, but it grants
  the whole file. It has no per-destination limit, and redirects are not checked against
  one. These are M4's grant and security tests.
- One send runs at a time. Other calls wait, and a long send can reach a host's timeout,
  since no progress notifications are sent. `notifications/cancelled` stops a send. A
  client that closes its input cancels the send and the server exits without answering
  queued calls.

## Not now

Revisit when a real task is blocked. A competitor's feature list is not that task.

- HTTP/2. HTTP/1.1 stays the wire format the tests pin until a target API needs
  negotiation. A later change keeps those tests and reports the protocol that was used.
- A GraphQL editor (query, variables, schema completion). Sending GraphQL over HTTP
  already works as a normal body.
- WebSocket. It needs a connection model the HTTP client does not have.
- Response comparison, and a bounded JSON Schema check. Neither is contract testing.
- Image preview, custom shortcuts, and adding a saved value from the variable inspector:
  small accepted work, free to land after M0 when it does not delay the core sequence.
- OpenCollection import and export, and code snippets other than curl.
- A small functional input table (CSV or JSON) for the runner. This is separate from a
  load-test dataset, which stays frozen.
- Turning a reviewed, redacted response into a mock fixture, and letting a run own that
  mock's lifetime. Only after M3, and only for a task people repeat.

## Rejected

| Never | Why |
|---|---|
| Accounts, cloud sync, telemetry, team servers | The repository is how work is shared. The app stores no account. |
| An assistant or a model key inside the app | The person's own agent is the agent. The app takes no model bill and no prompt. |
| A git client | The user already has git. The app reloads what changed on disk. |
| gRPC, SOAP, MQTT, and a general multi-protocol client | HTTP collections are the product. An XML or GraphQL body on HTTP stays a normal request. |
| API design, documentation sites, governance, monitors, SDK generation | They need a platform. Descriptions and OpenAPI import stay. |
| A plugin system, and script access to files, the network or the process environment | Scripts are useful because the sandbox is closed. |
| Postman `pm.*` compatibility | That is a JavaScript runtime. The Rhai decision stands; import keeps saying what it dropped. |
| One MCP tool per endpoint, a hosted MCP service, or an MCP client for testing other servers | A stable small tool list is the point of the experiment. Testing MCP servers is a different product. |
| A security scanner or a fuzzer | A check is not a scan. |

## Decisions

These bind the milestones. Field names, the OAuth library and byte budgets are settled
inside the milestone with tests. They are not a reason to reopen the direction.

Planned guarantees below are not descriptions of the current binary.

### One contract

The desktop, the CLI and any MCP adapter call the same preparation, execution and
checks. Adapters do not grow a second variable, auth, redirect, script or assertion
implementation. A headless run does not open a window and does not need a display. The
same binary is the starting point. A separate headless executable needs a platform test
that shows this binary cannot run without a display.

### Files and the private store

Two modes.

- The personal SQLite workspace stays as it is. Opening a project does not convert it.
  The command line runs its saved requests directly (decided 7 November 2026, after M1
  shipped file-only): editing in the app and exporting after every change only added
  manual steps. Only the environment named runs, production needs a switch in the app,
  captures stay in the run, and each run is recorded in History as the command line's.
- An opted-in project links a collection to a folder. For that collection the files are
  the shared definitions. SQLite holds history, UI state, secrets and a rebuildable
  cache. It is not a second copy that can win a conflict.

Layout, from the start:

- Our own YAML. One file per request, one per folder, and a collection file. A small
  manifest names the collection, stable request ids and the environments.
- Fidelity matches the native format: checks, captures, scripts, saved values,
  environment colours.
- Field order is stable. A multi-line body or script is a block scalar. An edit changes
  its own lines. Order is a field in the file, so a reorder does not rename every file.
- A spike may tune that layout. It does not collapse the folder back to one file per
  collection.
- Database row ids are not portable identity.
- Native `.imposter.json` export remains the interchange format.

Sync:

- Linking writes the folder. Edits in the app write the files. A clean editor reloads
  changes from disk.
- A conflict with unsaved edits goes through the unsaved-changes guard. The app keeps
  both versions.
- One writer at a time, with an expected revision. Saves are atomic. A stale write fails
  and stays recoverable. There is no merge UI and no git client.
- Environment files carry no secret values. Secrets stay in the local store, a
  gitignored `.env`, or named process-environment inputs.

### What a run means

A run is one request, an ordered folder, or the collection, with an explicit
environment, executed sequentially.

- Captures and script variable writes affect later requests in that run only. Writing a
  captured value back to an environment is a separate edit. Send keeps its current
  behavior until a change is specified and tested on its own.
- The run freezes its definitions and starting inputs. An environment switch during the
  run does not affect it.
- These outcomes stay distinct: preparation error, transport error, failed check, script
  failure, cancelled, not checked.
- A run succeeds only when it finished and every required check was evaluated and
  passed. A run with no checks is an execution result, not evidence that behavior was
  verified. Missing secrets, truncated inputs a check needs, and an empty selection are
  not success.
- Each run has an id. The result records the definition revision, the environment name
  and the non-secret inputs. Result files sit outside the versioned definitions. The
  desktop can open a result the CLI wrote.
- The observed exchange and the check outcomes are stored apart from any explanation
  an agent writes. Capture and truncation limits are part of the result.
- An agent may propose a check. Changing an expected value is a diff in the request
  file. A result produced by a mock says so.

### Secrets and authority

This is in place before an agent can execute a request. It is a planned guarantee, with
tests, not a claim about today's logs alone.

- Secrets are references, resolved from sources the user named. The first headless form
  is named process-environment inputs, not the whole environment. A missing input fails
  the run.
- Shared project files, agent output, reports and the default result exclude or redact
  secret values. Redaction covers URLs, headers, bodies, variable values, check
  failures, script errors and script console output, not only `Authorization`.
  Known-secret filtering does not detect every sensitive value a server returns.
  Results lead with a summary. Selected body text is bounded and stays inside the
  disclosure scope the user set.
- Raw history and saved response bodies need their own retention policy. Redacting an
  agent result does not change captures already stored.
- A local process can still hand a result to the person's model provider. Connecting an
  agent means data can leave the machine.
- Copy as curl stays an explicit local action and may contain credentials. A shared
  export is a different command.
- The user sets execution authority outside files an agent can edit: which project,
  which environment, which destinations, which operations. An agent-written flag does
  not grant it. Red environments require an explicit named opt-in inside the user's
  existing execution grant. A CLI flag cannot widen that grant. Environment colour and
  HTTP method are not authority.
- Destinations are checked after scripts and on redirects, including resolved addresses.
  Uploads, certificate paths and output paths stay under approved roots.

### MCP, if the experiment ships

Stdio only. No listening port. Diagnostics go to stderr. Response text is data, not
instructions.

| Operation | Contract |
|---|---|
| Find requests | Search metadata. No full-collection dump. |
| Inspect request | Definition, inherited scripts, variable names, revision. Secrets masked. |
| List environments | Names and required inputs. No credential values. |
| Prepare | A redacted preview. Preparing does not authorize a send. |
| Save a request or check change | Inside a write grant, with the expected revision, as a reviewable diff. A dirty desktop draft is left alone. No deletion. |
| Execute | An explicit request, folder or collection, revision and environment, inside an execution grant. Returns a run id. |
| Read result | Structured outcome and bounded evidence. Limits are visible. |
| Cancel | Stops the run and records cancelled. |

Inspection, editing, execution and disclosure are separate grants. Load runs and general
file access are outside this list. CLI JSON and MCP results share one schema.

### Streaming

SSE is core, in M3. Events appear as they arrive, with timestamps, inside a bounded view
and a byte cap. The user can cancel. The milestone defines what "the stream ended"
means. Checks run on the completed stream. A timeout, a cancellation, or a cut at the
cap is not a passed run.

### Mocks and load, stated as limits

A mock result does not verify the live API. The load probe does not estimate
independent arrivals or production capacity. Both limits are written in the UI and in
any exported result.

## Milestones

Each one finishes a slice before the next direction opens. M0 completed documentation and
emphasis. M1 and M3 cross existing boundaries. M2 and M4 change persistence or
permissions and should land as several commits. No calendar dates.

### M0 — Complete: say what the app actually is

Completed 7 November 2026: navigation, accurate README and refreshed screenshots,
explicit mock/load limits, and [packaged 0.6.0 launch-to-request and peak-memory evidence](docs/validation.md#packaged-060-measurements).

### M1 — Complete: one request, headless

Completed 7 November 2026: `imposter send` through the one engine (`executor::run`), from
the personal profile by default and from a native file with `--file`; named secret inputs,
redacted results with an exit status per outcome, result files opened in History,
`imposter skill`, and the MCP spike with its findings above. The exit tests are
`imposter-app/tests/document.rs` and `profile.rs`, `imposter-ui/tests/results.rs`,
`imposter-cli/tests/send.rs`, `imposter/tests/cli.rs` and `smoke.py --headless`, which CI
runs on Linux before any display library is installed. M2's project folder becomes a third
source; the file stays for CI until then.

### M2 — The project folder and the runner

- The YAML layout, the manifest, stable ids, relative fixture paths and secret
  references. Explicit Open project. Personal data stays put.
- Revision checks, the single writer, conflict handling, and reload of clean editors.
- A sequential runner in the app and `imposter run`, for a folder or the collection:
  run-local captures, JSON and JUnit output, and an exit status that fails when a
  required check failed, was not checked, or the run did not finish.
- A minimal CI example. The skill text gains `run`, still printed by `imposter skill`.

Not in this milestone: a git UI, merging, parallel runs, retries, a scheduler.

Exit: a fresh checkout plus supplied secrets runs with no personal database. A request
or check edit is a reviewable diff. Create, capture, a later request and a check work
as one run. Competing writes, an external edit, an interrupted save and an upgrade from
a native export are tested. The same definitions produce the same check outcomes in the
app and the CLI. A live server that changes during a run is not promised to match.

This is the first release of the shared workflow for the app, the terminal and CI.

### M3 — Auth, cookies, SSE

Pull the smallest auth and cookie slice ahead of M2 as soon as a real user cannot
proceed with a supplied token. That pull does not drag SSE with it.

- OAuth 2.0: client credentials, and authorization code with PKCE through the system
  browser and a loopback redirect. Tokens are cached per project and environment,
  refreshed before expiry, and stored as secrets.
- API-key auth, in a header or the query.
- A cookie jar scoped by project and environment, so two projects do not share one.
  Visible, editable and clearable, with an explicit on/off choice, and used by Send,
  the runner and the CLI.
- Headless runs use isolated session state unless an explicit session input is
  supplied. They do not borrow the personal profile's cookies or tokens.
- SSE, with the streaming decision above.

Exit: the task that needed a second client for tokens, cookies or a stream no longer
does. Tests cover refresh failure, a missing secret, jar isolation between
environments, cancellation, and a stream cut at the cap that must not pass.

### M4 — MCP experiment

- The tool list above, on the same services as the CLI, inside the grants from the
  secrets decision.
- The desktop shows origin, revision, environment and outcomes for a headless result.
- Written setup for two MCP hosts and for shell agents. The app installs nothing into
  an agent.

Exit: either a person completes a real reproduce-and-check task through two hosts, and
the security tests pass (cancellation, a stale write, a denied execution, a redirect
outside the grant, a secret in an error, an oversized body), or the experiment is
dropped and this file says so. No other milestone waits for this exit.

## How we know

No telemetry. Evidence is voluntary sessions and the automated tests. Stars and the
number of MCP tools are not the measure. The figures below are gates for a pilot, not
results already collected. Pilot recruitment does not block M1, and it does not gate the three accepted
conveniences under "Not now". It is the gate for calling the shared workflow a
success and for promoting anything else listed there.

Every implementation milestone runs `scripts/check.sh` and preserves the 85% line and
70% region coverage floors, with regression tests at the real boundary. Cross-platform
CI and installed or headless smoke tests are separate release evidence. A milestone
reports the checks that actually ran.

| Question | Gate |
|---|---|
| Can someone start? | At least 4 of 5 pilot users go from a fresh install to a checked result with an agent, using the guide, within 10 minutes, without the maintainer driving. |
| Does it remove reconstruction? | At least 3 of 5 prefer it on one real debugging task and can name the step it removed. Otherwise the workflow changes before any item under "Not now" is added. |
| Are the checks real? | Each accepted regression fails on the bug and passes on the fix. Expected values were not weakened to get there. |
| Can a teammate rerun it? | A checkout, their own secrets, and the CLI reproduce the intended outcomes without the author's database. |
| Can a person explain a failure? | At least 4 of 5 can say why a check failed from the desktop result, without application logs and without asking the agent to reconstruct the run. |
| Does it stick? | After two weeks, at least 3 of 5 used it on three different days. Directional only. |
| Can it be trusted? | No known silent data loss, out-of-scope execution, or false success at the release. |

## Keeping this file

- A tier or a milestone changes only with a reason written next to the change.
- A proposal this file rejects needs the file changed first, with the reason.
- When work lands, mark it done here and record the decision in `AGENTS.md`.
- When a milestone is finished, shorten it to a line.
- New work names the task, how often it shows up or what it blocks, the smallest scope,
  the alternative that already exists, the acceptance test, and what it displaces.
  "A competitor has it" does not qualify, and neither does "an agent could use it".

## Sources

- [Postman MCP overview](https://learning.postman.com/docs/reference/postman-api/postman-mcp-server/overview)
- [Yaak CLI](https://yaak.app/docs/getting-started/cli-usage)
- [Yaak MCP](https://yaak.app/docs/getting-started/mcp-server)
- [Bruno](https://docs.usebruno.com/introduction/getting-started)
- [Hurl](https://hurl.dev/)
- [k6 open and closed workloads](https://grafana.com/docs/k6/latest/using-k6/scenarios/concepts/open-vs-closed/)
- [MCP transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)
- [MCP tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
