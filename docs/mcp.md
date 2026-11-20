# Try the local MCP experiment

Use a collections folder from Settings or a separate checkout. Start with a development
API and definitions whose descriptions, scripts and sample data you can share with
your host. A local MCP process can still hand tool output to the host's model provider.

## Set the grant

The subprocess arguments live in your own host configuration. Use an absolute path
to the binary and to the collections folder. For example, this grants inspection,
execution against a loopback API, and at most 2 KiB of each body in a result.

```sh
/path/to/imposter mcp --project /path/to/collections --inspect \
  --env Dev --allow-origin http://127.0.0.1:8080 --max-body 2048
```

| Argument | Permission |
| --- | --- |
| `--inspect` | Find requests, inspect saved definitions, list environments and prepare a masked preview. |
| `--edit` | Update an existing request's method, URL, description or checks at the expected revision. Review the diff in git. Ignore the project's hidden `.imposter.lock` file in version control. |
| `--env Dev` | Use this one environment. Omit it to use globals and collection variables. Pass an empty environment string to tools in that case. |
| `--allow-origin http://127.0.0.1:8080` | Execute against that exact scheme, host and port. Repeat for each needed origin, including an OAuth token endpoint. For a DNS name use `https://api.example.test=192.0.2.10` with the IP you approve. |
| `--allow-production Prod` | Permit a red environment when `--env Prod` is also configured. |
| `--max-body 2048` | Disclose redacted evidence and up to this many bytes per body, from 0 through 4096. Zero discloses evidence such as checks and headers with no body text. Omit the option to disclose only the outcome and counts. |
| `--secret token=IMPOSTER_API_TOKEN` | Supply `{{secret:token}}` from the named process variable. Set the value in your environment or secret manager. |
| `--input base=IMPOSTER_DEV_URL` | Supply an ordinary variable from the named process variable. |

Keep the grant outside the checked-out definitions. MCP reads regular YAML definitions,
uses direct connections with normal TLS verification, and refuses requests that need
file uploads, proxies or custom certificates. A run accepts one request, folder or
collection at an explicit revision, up to 32 requests. At most eight results are kept
until the host stops the subprocess. The capture limit is 1 MiB per response, and
`read_result` can return a single step for a large report. Secret filtering covers known
credentials and secret values. Other sensitive content returned by your API can still
appear in evidence.

## Codex

Run this from your terminal with your paths and grant. Then start a new Codex session
and check `/mcp` for `imposter`.

```sh
codex mcp add imposter -- /path/to/imposter mcp \
  --project /path/to/collections --inspect --env Dev \
  --allow-origin http://127.0.0.1:8080 --max-body 2048
```

For named process inputs, add their names to `env_vars` under
`[mcp_servers.imposter]` in your user `config.toml`. Export the values before starting
Codex. Keep the values in your environment. [Codex MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)

## Claude Code

Register the subprocess at local scope from the directory where you use Claude Code.
Start a new session and check `/mcp`. Export any named inputs before starting the host.
[Claude Code MCP configuration](https://code.claude.com/docs/en/mcp)

```sh
claude mcp add --scope local --transport stdio imposter -- /path/to/imposter mcp \
  --project /path/to/collections --inspect --env Dev \
  --allow-origin http://127.0.0.1:8080 --max-body 2048
```

## Reproduce a failure and check it

1. Give the host the saved request's name and expected behavior. Ask it to use
   `find_requests`, `inspect_request` and `prepare`. Verify the request and destination.
2. Ask it to use `execute` for that ID, environment and revision, then read the run ID
   with `read_result`. A failed assertion appears as a failed run outcome.
3. If you grant editing, have the host add the intended check with `save_request`,
   review the YAML diff, and run the new revision. Keep the definition and the evidence
   together in your review.
4. Save the `report` object from `read_result` as a JSON file. In History use
   **Open result file…** to see the run's origin, revision, environment and outcomes.
   Choose a request in the report to see its verdicts and evidence.

For a shell agent, `imposter skill` prints the instructions for `list`, `show`, `send`
and `run`. The CLI can write a report directly with `imposter run --out run.json`.
