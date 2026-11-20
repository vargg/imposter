# Transfer & Formats Decisions

Part of the [Imposter agent manual](../../AGENTS.md). Decisions governing file formats (.imposter.json, collections folder YAML), Postman & OpenAPI import, and curl translation.

## Decisions

- **Native format: `.imposter.json`, marked `"format": "imposter"`, version 2, 3 or 4.** Version 2
  holds a collection with its folders, requests, checks, captures, variables, saved values,
  environments and colors. The marker and version are required and unknown fields are
  refused at every level, so a file from another tool is refused rather than half-read; a
  file with any other marker is reported as native, not tried as Postman. A document keeps
  its version when decoded and encoded again, and its bytes round-trip exactly
  (`imposter-transfer/tests/native_golden.rs`, with the cases in
  `tests/fixtures/native/decode-cases.json`). Bytes are base64 with the URL-safe alphabet
  and padding; both alphabets are read. Why: one strict format whose exports diff line by
  line. Earlier formats are not read: nothing using them was released (user's choice).
  Version 3 (still `"imposter"`) adds scripts, on requests and on the collection and its
  folders. An export takes the lowest version that holds it (`NativeDocument::fit_version`:
  3 only when something has scripts), so script-free exports stay readable by Imposter
  0.4; a version 2 document with scripts is refused, naming what has them. Postman exports list scripts as not exported (Postman's are
  JavaScript), and Postman's own scripts are still removed on import, with consent. The
  import review says how many requests have scripts. A script changes only its own request
  and reads only the variables that request could already use, so it adds no way out
  that a plain imported request lacks.
  Version 4 adds captures stored as secrets (`Capture::secret`); `fit_version` picks it
  only for a document with one, and an older version holding one is refused, naming the
  request, as scripts are below 3. Version 5 adds API-key and OAuth 2.0 auth
  (`RequestAuth::api_key`, `oauth2`), a request's own cookie jar setting
  (`Options::cookie_jar`) and event stream limits (`stream_seconds`, `stream_events`),
  all left out of the JSON when unset, so other documents keep their bytes, by the same rule.

- **The collections folder format** (`store::files::format`, format `imposter-project`,
  version 1): one YAML file per request, per folder (`folder.yaml` in its directory),
  per collection (`imposter.yaml` in its directory) and per environment
  (`environments/<name>.yaml`). Why: an edit to a request or check must be a diff of its
  own lines, and two people editing different requests must not touch the same file
  (the roadmap's M2 layout; one export file failed both).
  - **Names to files:** lowercase ASCII letters and digits, `-` for the rest
    (`Shop API` → `shop-api`, user's choice: portable, no quoting, no case clashes on
    macOS and Windows); `-2`, `-3` for siblings that collide; `folder`, `imposter` and
    `environments` are reserved where they would clash.
  - **Identity and order are fields:** each file carries an `id` and a `position`, so a
    reorder renames nothing and a rename moves one file.
  - **Readable values:** request files have their own serde types mirroring
    `RequestTemplate`: bodies as text (a literal block when multi-line), form fields as
    lists, a binary body as `file: <path>`, settings and auth only where set; bytes that
    are not text fall back to base64. Long single lines are never folded.
  - **An environment that does not share its values** is written with its names and
    empty values (`share_values: false`); its values stay in the profile.
  - **Strict reading, as the native format:** unknown fields, duplicate ids, a
    directory without `folder.yaml`, an empty name or another format or version are
    refused, naming the file. Only directories with `imposter.yaml` and
    `environments/` are read, so the folder can live inside a repository beside other
    files. YAML is read with a node budget, as OpenAPI import does.
  - **A half-filled auth reads; only its send is refused.** A request whose selected
    auth kind has no configuration block at all is an inconsistent file and refused;
    a block that is present but half-filled (an empty OAuth token URL, an unnamed API
    key) reads and round-trips, because the writer saves that state mid-edit and from
    an OpenAPI flow without a token URL (`RequestAuth::has_configuration` at reading,
    `RequestAuth::active` at sending, where it is missing credentials). Why: judging
    it by `active()` made one saved request leave the whole folder unreadable (the
    report from the field).
  Pinned by `imposter-store/tests/project_format.rs` and the golden folder
  `tests/fixtures/project/`.

- **Imports never change existing data and write nothing before review.** The preview computes
  fresh names (`Name (2)`) and global conflicts first; the import is one transaction; existing
  global values win, with a warning. Postman losses that change how a request runs (scripts,
  unsupported auth or bodies) need the explicit "Remove the listed unsupported execution
  features" consent, enforced by the service (`TransferError::ConsentRequired`), not only by
  the dialog. Other losses are warnings. Why: an import must be undoable by deleting the new
  collection.

- **Postman files are validated against the bundled v2.1 schema (draft 4), offline.** The file
  is pinned by SHA-256 (`imposter-transfer/schemas/README.md`, checked by `tests/schema_pin.rs`).

- **Exports are opt-in and atomic.** Environments and globals are included only when checked.
  The open request is exported as the editor shows it, unsaved edits included. Files are
  written to a temporary file and renamed; a Postman bundle with companions goes to a new
  directory and never overwrites one. Tied sibling positions are renumbered on export, so a
  trip through the database keeps content and order, not tied numbers.

- **curl in and out** (`imposter_core::curl`). Import parses POSIX
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
  empty collection variables (`token`, `username`/`password`, the API key scheme's name,
  `client_id`/`client_secret`); an OAuth scheme with a client credentials flow becomes
  OAuth 2.0 auth with the scopes the operation asks for, and other flows send `{{token}}`
  as a bearer token. Only `$ref`s inside the document are followed;
  example bodies come from the document's examples, else from the schema with recursion
  included once and size capped. Swagger 2.0 is refused rather than half-supported. Files are
  recognised by content, not extension (`transfer::detect`), so only Postman files are asked
  for companions. YAML goes through `serde-saphyr` with a node budget scaled to the file
  size, which refuses alias bombs. The expectations are in
  `imposter-transfer/tests/openapi.rs`.

