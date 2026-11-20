# HTTP & Transport Decisions

Part of the [Imposter agent manual](../../AGENTS.md). Decisions governing TLS (rustls), HTTP/1.1 protocol handling, headers, streaming/SSE, cookies, and authentication.

## Decisions

- **TLS: rustls with the `ring` provider, configured by us** (`imposter_http::tls`) and passed to
  every reqwest client with `use_preconfigured_tls`, including plain-HTTP clients (reqwest
  panics without a provider). Why: no OpenSSL, cmake or system library on any OS; full control
  of roots, verification and client keys. A custom CA bundle replaces the platform roots.
  Clients belong to a `transport::Session`, one per set of TLS settings: a single send's
  session keeps no idle connection (as before), so each send connects afresh; a session
  shared by a load run keeps them and reuses them.

- **Client keys: PKCS#8 (plain or encrypted, PBES2), PKCS#1 and SEC1. Legacy encrypted PEM
  (`Proc-Type: 4,ENCRYPTED`) is rejected** with advice to run `openssl pkcs8 -topk8`. Why:
  rustls and RustCrypto do not implement the legacy OpenSSL scheme. PBKDF2-HMAC-SHA1 is enabled
  because LibreSSL and older OpenSSL use it by default (fixtures in `imposter-http/tests/fixtures`).
  A decryption that is not a key is a wrong password: about once in 256 tries a wrong one
  passes the padding check and yields noise, which was reported as an invalid PEM file (and
  made `encrypted_key_errors_are_specific` flaky).

- **HTTP/1.1 only.** Why: it keeps the wire format the tests pin byte for byte; HTTP/2 waits
  for a target API that needs it.

- **Server-sent events are read as they arrive and judged by how they end**
  (`core::sse`, the WHATWG rules for lines, fields and dispatch, fed chunks split
  anywhere; `imposter_http::Listening`). Why: a body that appeared only at the end hid a
  stream (M3), and a stream that never closes would never end a send.
  - **Reading:** a `text/event-stream` answer is read chunk by chunk; its text is kept
    as the body up to the capture limit, its events (at most `sse::MAX_EVENTS`, the rest
    counted) on `Response::stream` with the milliseconds since the head, and they go to
    the sink as they come (`execute_streaming`, the bridge's notifier, `Message::
    StreamEvents`). Load runs read a stream as a body, frozen by the roadmap.
  - **Ends** (`StreamEnd`): `Closed` by the server and `Limit`, the request's own
    listening limit (`Options::stream_seconds`, `stream_events`; Settings tab, "Event
    streams"; native version 5), are normal (user's choice: a limit the request sets is
    not a failure). `Cut` at the capture limit, `TimedOut` (the read timeout between
    chunks), `Failed` and `Cancelled` never pass (`RunOutcome::of`: not checked,
    transport error, cancelled), keeping what arrived; then checks are undecided ("the
    stream was cut at the size limit") and no capture or after-response script runs.
  - **Results:** a stream adds `stream` (end, received, the events whose data fits the
    excerpt limit) and makes the result version 2, which this app reads with 1; other
    results stay version 1 for older readers. History keeps the stream with the body.
  - **The app:** while events arrive the response pane lists them live in place of
    "Sending…" and Send reads Stop; the response opens on its Events tab (virtualized
    rows: time since the head, event, the data's first line; a chosen event shown whole
    under the list); the status says how it ended ("Stream closed by the server · 42
    events").
  Pinned by the parser's unit tests (every split point of a stream),
  `imposter-http/tests/sse.rs` (a hand-written server that waits for the test between
  writes: live events, both limits, the cap, cancel, the timeout), `imposter-app/tests/
  sse.rs`, `imposter-cli/tests/stream.rs` (each end's exit status),
  `imposter-ui/tests/sse.rs` and the layout lint.

- **Header order: values of one name keep their order; names go out in first-occurrence order,
  lowercased.** So `A, B, A` is sent as `a, a, b`. Why: hyper stores headers in a `HeaderMap`
  and its order-preserving extensions are private; writing our own HTTP/1.1 client is not
  worth the risk. Pinned by `imposter-http/tests/header_order.rs`.

- **Redirects are followed by `imposter-http`, not reqwest:** at most 20;
  303 becomes GET (except HEAD), 301/302 turn POST into a bodiless GET; 307/308 keep the method
  and replay the body; `Authorization` is dropped across origins (except an http→https upgrade
  of the same host); explicit `Cookie` headers are never carried to the next location. Why:
  reqwest cannot replay streamed uploads, and credentials must not follow a redirect to
  another origin.

- **The cookie jar keeps what servers set and sends it back** (`imposter_http::cookies`,
  over `cookie_store`: RFC 6265 domain, path, `Secure` and expiry rules, pure Rust, without
  the public suffix list). Why: a session cookie from a login had to be copied into the next
  request by hand (M3). Parsing and matching are the crate's, not ours to get wrong.
  - **Each hop** sends the jar's cookies for its URL in its `Cookie` header, except names
    the header already has, so the request's own cookies win; what the hop's response sets
    is kept, and listed with the hop's URL on `Response::set_cookies` (never stored) for
    the caller to keep. A request's explicit cookies still stay with the first location.
  - **Kept:** session cookies until cleared (a login's session cookie usually has no
    expiry, and is the one worth keeping); expired ones are dropped, and an expired
    `Set-Cookie` removes the cookie it names. Cookies of one path length go out by name,
    as the jar keeps no creation time. Localhost and loopback addresses count as secure,
    as browsers treat them.
  - **Logs:** `Set-Cookie` values go through `CookieStore::parse`; its
    `store_response_cookies` logs each value at debug, which `imposter-http/tests/logging.rs`
    catches.
  - **Settings:** `AppSettings::cookie_jar` ("Keep cookies", on by default, also for
    settings saved before it: user's choice) and the request's `Options::cookie_jar`,
    left out of documents when inheriting, so they keep their bytes (native version 5,
    the folder format's `settings`). The jar sends only while "Attach cookies" is on too,
    which then means every cookie, the request's rows and the jar's; it keeps what
    arrives either way.
  - **As sent:** the jar's cookies for the first location are put into the request as
    sent (`cookies::add_to`: after an enabled `Cookie` header, else as rows), so History,
    the response pane and Copy as curl show them; the transport adds later hops'.
  - **Stored** (migration V9, `store::cookies`): one jar per collections folder and
    environment (none for requests sent without one), as JSON, in the profile: cookies
    are personal, like secrets, and never in the folder's files. An environment's jar
    goes with it. A jar that does not read is taken as empty.
  - **Who uses which:** Send reads the stored jar with its inputs and keeps what the
    response set afterwards, merged into the jar as it is then (`executor::execute`), so
    two sends at once both keep theirs; a failure says so on the response. A run's
    requests share one jar chosen when they are selected: the app's Run screen a copy
    of the stored jar, the command line (`send`, `run`, `--file`, `--project`) an empty
    one (an agent must not borrow the user's sessions); neither writes back, as captures
    stay in the run. Load runs keep no cookies (frozen by the roadmap).
  - **The app:** the Variables screen lists a jar under Global (requests sent without an
    environment) and each environment, after the Secrets: name, value masked with an eye
    for the session, and where and until when it is sent ("api.test · / · HTTPS only ·
    until cleared"). It is not part of the page's edits, since a send changes it while
    the page is open: removing one cookie or clearing them all is written at once, each
    after a dialog naming what goes. Values are not edited: a server sets them, and a
    request's cookie rows send one by hand. The list reloads after a send that set
    cookies. Settings → "Keep cookies".
  Pinned by `imposter-http/tests/cookies.rs` (the bytes each hop receives), the module's
  unit tests, `imposter-store/tests/cookies.rs`, `imposter-app/tests/cookies.rs`
  (environments apart, the settings, runs and the command line writing nothing),
  `imposter-ui/tests/cookies.rs` and the layout lint's Variables state.

- **API-key auth sends one header or one query parameter** (`model::ApiKey`: name, value,
  `in: header | query`). `prepare` adds it after the request's own headers, or after its
  query, which keeps its order and encoding. A sent header (compared without case) or
  query parameter of the same name is refused (`PrepareError::ApiKeyConflict`); an
  `Authorization` header beside a key is the user's own; a key without a name is missing
  credentials. The value is a credential: results redact it with its percent-encoded form
  (so a key in the URL goes too), and `show` prints it only as placeholders. A key in a
  cookie is not offered (OpenAPI allows it, Postman does not): a cookie row does that.
  Postman `apikey` maps both ways (without `in` it is a header, as Postman sends it).
  OpenAPI's `apiKey` in a header or the query becomes the request's API-key auth when no
  other scheme of the requirement wants the auth slot; beside one, or beside another key,
  it stays a row, so every scheme required together is still sent. The folder format
  keeps version 1, writing `api_key` only where set. The Auth tab's place picker reads
  "Header" or "Query" ("Query parameter" ran past the pane at 720 points, the layout
  lint). Why: M3; a key typed as a header row was not treated as a credential. Pinned by
  `prepare` and `redact` unit tests, `imposter-app/tests/executor.rs` (the bytes received,
  History without a secret key), the native decode cases, `postman_golden.rs`,
  `openapi.rs`, the project golden folder, `imposter-ui/tests/widgets.rs` and the layout
  lint.

- **OAuth 2.0 (client credentials, authorization code with PKCE): the executor gets the
  token, the transport never sees OAuth** (`imposter_http::oauth` asks the token endpoint, `imposter_app::oauth`
  decides which token to send). Hand-written, not the `oauth2` crate, which pins its own
  reqwest: the token request goes through the request's own `Policy` (TLS, proxy,
  timeout; no cookie jar) and is a form POST of a few fields (RFC 6749 4.4, 6).
  - **Model** (`model::OAuth2`, native version 5, the folder format's `oauth2`): token
    URL, client ID and secret, scope, audience, and how the client authenticates (a
    Basic header, form-encoded first as RFC 6749 2.3.1 says, or the form). Every field
    takes `{{variables}}` and `{{secret:…}}`, so a missing secret stops the send as
    unresolved before any token request.
  - **When:** after the before-sending scripts, on resolved values: the token replaces
    the auth in the request as sent (`RequestAuth::bearer`), so History, the response
    pane and the run result show a Bearer token, which they remove with the client
    secret and refresh token (`Ran::secrets`). `prepare` refuses an OAuth request
    without a token (`PrepareError::NoToken`); Copy as curl uses a cached good token
    and never asks for one.
  - **Cache** (migration V10, `store::oauth`): per collections folder and environment,
    keyed by a SHA-256 of grant, token URL, client ID, scope and audience, never the
    secret. A token is used while it has more than 30 seconds left (`oauth::MARGIN`;
    one without a lifetime until cleared), then refreshed with its refresh token; a
    refresh that fails asks for a new token, which client credentials can always do.
    Send keeps what it fetched; the app's Run screen starts from a copy and the command
    line from nothing, and neither keeps one (the roadmap's isolated headless runs).
  - **Failures** are not sent and not recorded (`ExecuteError::Token`): "Token request
    failed: invalid_client: Unknown client", the OAuth error and description, or the
    status when the answer names none. A token type other than Bearer is refused.
  - **The Auth tab** says where the token stands under the form (`oauth::state`: "No
    token yet: Send gets one.", "Token valid for 54 min", "Token expired: the next send
    refreshes it"), read again when the OAuth settings or the active environment change
    and after a send. "Get new token" asks for one now and keeps it (`oauth::fetch`,
    without scripts: it is for the settings as they stand); "Clear token" forgets it
    without asking, since the next send gets a new one (a cache, not the user's data).
  - **Authorization code** (`imposter_http::sign_in`, RFC 8252's loopback redirect):
    signing in is an explicit act, never part of a send. "Sign in" (the Auth tab, or the
    "Not sent" card) listens on 127.0.0.1 (the redirect port, or any free one) before
    opening the system browser (`open` crate, pure Rust; eframe's `open_url` does nothing
    here) at the authorization URL with PKCE S256 and a random `state`, waits up to five
    minutes or Cancel, refuses another sign-in's `state`, reports the server's `error`,
    then exchanges the code with the verifier. The browser's page is plain text, so
    nothing the server sent runs in it. A public client (no secret) only names itself in
    the form. Send uses the cached token and refreshes it; with none, or one that cannot
    be refreshed, it is not sent ("Sign in to get a token: …", `ExecuteError::SignIn`),
    and the dead token is forgotten so the tab says to sign in. The command line never
    opens a browser and has no tokens of its own, so such a request is not sent there and
    says to send it from the app. Tests play the browser (`ImposterApp::script_browser`,
    the same seam idea as file dialogs).
  - **Import and export:** Postman's `oauth2` maps both ways for client credentials
    and the authorization code grant (`authorization_code`, `_with_pkce` or none, which
    is Postman's default; exported as `_with_pkce`): `client_authentication` header or
    body, a loopback `redirect_uri` as the redirect port. Its cached `accessToken`, token
    labels, PKCE settings and `state` are dropped without asking, since Imposter makes its
    own; another redirect URI is a warning, as sending is unchanged and only the server's
    registered redirect may need adding. Other grants (implicit, password) are removed
    with consent. OpenAPI's `clientCredentials` flow becomes client credentials, else
    an `authorizationCode` flow a sign-in, with the scopes the operation asks for.
  Pinned by the module's unit tests (the form and Basic header, answers),
  `imposter-app/tests/oauth.rs` (fetch, reuse, refresh, a failed refresh, refusal, a
  missing secret, History, the command line, Copy as curl), `imposter-http/tests/sign_in.rs`
  and `imposter-app/tests/sign_in.rs` (PKCE against the RFC 7636 example, `state`,
  timeout, cancel, a failed refresh, the command line), `imposter-store/tests/oauth.rs`,
  the native decode cases, the project golden folder, `postman_golden.rs`, `openapi.rs`,
  `imposter-ui/tests/widgets.rs`, `imposter-ui/tests/oauth.rs` and the layout lint.

- **Digest is answered once per hop** (MD5, SHA, SHA-256, SHA-512, `-sess`, qop `auth`; not
  `auth-int`), rebuilding the body for the retry. Verified against the RFC 2617/7616 vectors.

- **Uploads stream with chunked transfer encoding** (no `Content-Length`) and are reopened for
  every attempt, so a large file is never held in memory and a redirect or Digest retry can
  send it again. Multipart bytes are pinned by `golden/multipart.json`.

- **Timeouts apply to connecting and to each read**, not to the whole exchange: a slow but
  steady download is not cut off.

- **Response headers arrive grouped by name and lowercased, and the reason phrase is the
  standard one for the status.** Why: hyper exposes neither the received order across names
  nor custom reason phrases. Values of one name keep their order.

- **`User-Agent: Imposter/<version>`** unless the request sets one; environment and system proxies
  are never used, only the configured one.

- **Host client identities match an exact normalized HTTPS host and port.** A match replaces global client credentials, then explicit request fields override individually. Each redirect selects again from the execution snapshot; CA and verification settings remain unchanged. Why: one host must never inherit another host’s selected certificate. Curl copies the initial identity and reports that it cannot switch identities across redirects. Enforced by core settings, executor, and loopback TLS tests.

