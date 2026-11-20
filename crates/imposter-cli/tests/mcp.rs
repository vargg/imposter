//! The MCP tool boundary against a real project folder and loopback HTTP.
#![cfg(test)]

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use imposter_cli::mcp::{Args, serve};
use imposter_cli::{InputArg, InputSource};
use imposter_testkit::{Server, read_request, response};
use serde_json::{Value, json};
use tokio::sync::mpsc::{Sender, UnboundedReceiver, UnboundedSender, channel, unbounded_channel};

const SECRET: &str = "private-token-42";
const WAIT: Duration = Duration::from_secs(5);

struct Output {
    bytes: Vec<u8>,
    messages: UnboundedSender<Value>,
}

impl Write for Output {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let value = serde_json::from_slice(&self.bytes)?;
        self.bytes.clear();
        self.messages.send(value).map_err(io::Error::other)
    }
}

struct Client {
    requests: Sender<String>,
    messages: UnboundedReceiver<Value>,
    last: usize,
}

impl Client {
    async fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.last += 1;
        let id = self.last;
        self.requests
            .send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string())
            .await
            .unwrap();
        let reply = tokio::time::timeout(WAIT, self.messages.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reply["id"], id);
        assert_eq!(reply["jsonrpc"], "2.0");
        assert!(
            !reply.to_string().contains(SECRET),
            "secret reached the host"
        );
        reply["result"].clone()
    }

    async fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.rpc("tools/call", json!({"name":name,"arguments":arguments}))
            .await
    }

    async fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let reply = self.call(name, arguments).await;
        assert_eq!(reply["isError"], false, "{reply}");
        reply["structuredContent"].clone()
    }

    async fn denied(&mut self, name: &str, arguments: Value, why: &str) {
        let reply = self.call(name, arguments).await;
        assert_eq!(reply["isError"], true, "{reply}");
        assert!(
            reply["content"][0]["text"].as_str().unwrap().contains(why),
            "{reply}"
        );
    }

    async fn revision(&mut self) -> Value {
        self.ok("find_requests", json!({})).await["revision"].clone()
    }

    async fn start(&mut self, id: &str, revision: &Value, environment: &str) -> Value {
        self.ok(
            "execute",
            json!({"id":id,"revision":revision,"environment":environment}),
        )
        .await["run_id"]
            .clone()
    }

    async fn finished(&mut self, run: &Value) -> Value {
        tokio::time::timeout(WAIT, async {
            loop {
                let answer = self.ok("read_result", json!({"run_id":run})).await;
                if answer["status"] == "finished" {
                    break answer;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
}

async fn with_client(args: Args, test: impl AsyncFnOnce(&mut Client)) {
    let (requests, input) = channel(16);
    let (messages, answers) = unbounded_channel();
    let mut writer = Output {
        bytes: Vec::new(),
        messages,
    };
    let mut client = Client {
        requests,
        messages: answers,
        last: 0,
    };
    let var = |name: &str| match name {
        "TOKEN" => Some(SECRET.to_owned()),
        "VERDICT_WORD" => Some("passed".to_owned()),
        _ => None,
    };
    let server = serve(input, &mut writer, args, &var);
    let task = async move {
        test(&mut client).await;
    };
    let (result, ()) = tokio::join!(server, task);
    result.unwrap();
}

fn args(dir: &Path, origin: Option<&str>) -> Args {
    let mut destinations = imposter_app::http::destination::Destinations::default();
    if let Some(origin) = origin {
        destinations.allow(origin).unwrap();
    }
    Args {
        project: dir.to_owned(),
        environment: None,
        production: false,
        destinations,
        inspect: true,
        edit: false,
        excerpt: origin.map(|_| 96),
        inputs: vec![InputArg {
            variable: "token".into(),
            from: InputSource::ProcessVar {
                name: "TOKEN".into(),
                secret: true,
            },
        }],
    }
}

fn project(dir: &Path) {
    std::fs::create_dir(dir.join("api")).unwrap();
    std::fs::write(
        dir.join("api/imposter.yaml"),
        "format: imposter-project\nversion: 1\nid: collection\nname: API\n",
    )
    .unwrap();
}

fn request(dir: &Path, id: &str, name: &str, url: &str, extra: &str) {
    std::fs::write(
        dir.join(format!("api/{id}.yaml")),
        format!("id: {id}\nname: {name}\nurl: '{url}'\n{extra}"),
    )
    .unwrap();
}

#[tokio::test]
async fn catalogs_use_stable_ids_for_duplicate_names_and_prepare_sends_nothing() {
    let server = Server::start(|_, _| response("200 OK", &[], b"ok")).await;
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    request(
        dir.path(),
        "first",
        "Same",
        &format!("{}/first", server.url),
        "headers:\n  - name: Authorization\n    value: 'Bearer {{secret:token}}'\n",
    );
    request(
        dir.path(),
        "second",
        "Same",
        &format!("{}/second", server.url),
        "",
    );
    with_client(args(dir.path(), None), async |client| {
        let init = client.rpc("initialize", json!({"protocolVersion":"2025-06-18"})).await;
        assert_eq!(init["protocolVersion"], "2025-06-18");
        let list = client.rpc("tools/list", json!({})).await;
        assert_eq!(list["tools"].as_array().unwrap().len(), 4);
        for tool in list["tools"].as_array().unwrap() {
            assert_eq!(tool["outputSchema"]["type"], "object");
        }
        let listing = client.ok("find_requests", json!({"filter":"method:GET Same"})).await;
        assert_eq!(listing["requests"].as_array().unwrap().len(), 2);
        assert_eq!(listing["scopes"][0]["id"], "collection");
        let first = client.ok("inspect_request", json!({"id":"first"})).await;
        assert!(first["request"]["url"].as_str().unwrap().ends_with("/first"));
        let envs = client.ok("list_environments", json!({})).await;
        assert_eq!(envs["granted_environment"], "");
        assert_eq!(envs["inputs"][0]["variable"], "token");
        let revision = listing["revision"].clone();
        let prepared = client.ok("prepare", json!({"id":"first","revision":revision,"environment":""})).await;
        assert_eq!(prepared["headers"][0], json!(["Authorization","«redacted»"]));
        assert!(prepared["url"].as_str().unwrap().ends_with("/first"));
        client.denied("execute", json!({"id":"first","revision":revision,"environment":""}), "outside").await;
        client.denied("prepare", json!({"id":"first","revision":revision,"environment":"","inputs":{"token":"@/file"}}), "invalid arguments").await;
    }).await;
    assert_eq!(server.requests().len(), 0);
}

#[tokio::test]
async fn stale_writes_are_refused_and_a_check_edit_only_changes_its_file() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    request(dir.path(), "one", "One", "http://127.0.0.1:1/one", "");
    request(dir.path(), "two", "Two", "http://127.0.0.1:1/two", "");
    let mut grant = args(dir.path(), None);
    grant.edit = true;
    with_client(grant, async |client| {
        let revision = client.revision().await;
        let one = std::fs::read(dir.path().join("api/one.yaml")).unwrap();
        client
            .denied(
                "save_request",
                json!({"id":"one","revision":revision,"changes":{"description":SECRET}}),
                "contains a credential",
            )
            .await;
        assert_eq!(std::fs::read(dir.path().join("api/one.yaml")).unwrap(), one);
        request(dir.path(), "two", "Renamed", "http://127.0.0.1:1/two", "");
        let two = std::fs::read(dir.path().join("api/two.yaml")).unwrap();
        let changes = json!({"checks":[{"selector":"status","op":"equals","expected":"201"}]});
        client
            .denied(
                "save_request",
                json!({"id":"one","revision":revision,"changes":changes}),
                "revision changed",
            )
            .await;
        assert_eq!(std::fs::read(dir.path().join("api/one.yaml")).unwrap(), one);
        let revision = client.revision().await;
        let result = client
            .ok(
                "save_request",
                json!({"id":"one","revision":revision,"changes":changes}),
            )
            .await;
        assert_eq!(result["file"], "api/one.yaml");
        assert_eq!(result["changes"]["checks"]["after"][0]["expected"], "201");
        assert_ne!(result["revision"], revision);
        assert_eq!(result["revision"], client.revision().await);
        assert_eq!(std::fs::read(dir.path().join("api/two.yaml")).unwrap(), two);
        let request = client.ok("inspect_request", json!({"id":"one"})).await;
        assert_eq!(request["request"]["checks"][0]["expected"], "201");
    })
    .await;
}

#[tokio::test]
async fn the_pinned_host_works_and_report_evidence_is_redacted_and_bounded() {
    let server = Server::start(|_, _| {
        let body = format!("{SECRET}{}", "z".repeat(16 * 1024));
        response("200 OK", &[("X-Echo", SECRET)], body.as_bytes())
    })
    .await;
    let host = server.url.replace("127.0.0.1", "imposter.invalid");
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    request(
        dir.path(),
        "request",
        "Checked",
        &format!("{host}/ok"),
        "checks:\n  - selector: status\n    op: equals\n    expected: '201'\nheaders:\n  - name: Authorization\n    value: 'Bearer {{secret:token}}'\n",
    );
    with_client(
        args(dir.path(), Some(&format!("{host}=127.0.0.1"))),
        async |client| {
            let revision = client.revision().await;
            let run = client.start("collection", &revision, "").await;
            let result = client.finished(&run).await;
            assert_eq!(result["outcome"], "failed");
            assert_eq!(result["report"]["definition"]["sha256"], revision);
            assert_eq!(result["report"]["run"]["id"], run);
            assert_eq!(result["report"]["format"], "imposter-run");
            let step = &result["report"]["steps"][0]["ran"];
            assert_eq!(step["checks"][0]["actual"], "200");
            let excerpt = &step["response"]["body"];
            assert_eq!(excerpt["cut"], true);
            assert!(excerpt["text"].as_str().unwrap().len() <= 96);
            assert!(excerpt["text"].as_str().unwrap().starts_with("«redacted»"));
            assert_eq!(
                client
                    .ok("read_result", json!({"run_id":run,"step":0}))
                    .await["step"]["ran"],
                *step
            );
        },
    )
    .await;
    assert_eq!(
        server.requests()[0].header("authorization").unwrap(),
        format!("Bearer {SECRET}")
    );
    assert!(
        server.requests()[0]
            .header("host")
            .unwrap()
            .starts_with("imposter.invalid:")
    );
}

#[tokio::test]
async fn direct_script_redirect_and_file_destinations_are_refused() {
    let denied = Server::start(|_, _| response("200 OK", &[], b"wrong")).await;
    let elsewhere = format!("{}/denied", denied.url);
    let location = elsewhere.clone();
    let accepted = Server::start(move |_, _| {
        response("307 Temporary Redirect", &[("Location", &location)], b"")
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let approved = format!("{}/start", accepted.url);
    request(dir.path(), "direct", "Direct", &elsewhere, "");
    request(
        dir.path(),
        "script",
        "Script",
        &approved,
        &format!("scripts:\n  before: |\n    request.url = \"{elsewhere}\";\n"),
    );
    request(dir.path(), "redirect", "Redirect", &approved, "");
    request(
        dir.path(),
        "file",
        "File",
        &approved,
        "method: POST\nbody:\n  file: ../outside\n",
    );
    request(
        dir.path(),
        "proxy",
        "Proxy",
        &approved,
        &format!("settings:\n  proxy_url: {elsewhere}\n"),
    );
    with_client(args(dir.path(), Some(&accepted.url)), async |client| {
        let revision = client.revision().await;
        for (id, outcome, why) in [
            ("direct", "not_sent", "outside the execution grant"),
            ("script", "not_sent", "outside the execution grant"),
            ("redirect", "transport_error", "outside the execution grant"),
            ("file", "not_sent", "does not allow file uploads"),
            ("proxy", "not_sent", "does not allow proxies"),
        ] {
            let run = client.start(id, &revision, "").await;
            let finished = client.finished(&run).await;
            assert_eq!(finished["outcome"], outcome, "{finished}");
            assert!(
                finished["report"]["steps"][0]["ran"]["error"]
                    .as_str()
                    .unwrap()
                    .contains(why),
                "{finished}"
            );
        }
    })
    .await;
    assert_eq!(denied.requests().len(), 0);
    assert_eq!(accepted.requests().len(), 1);
}

#[tokio::test]
async fn an_error_cannot_echo_a_named_secret_and_disclosure_is_independent() {
    let server = Server::start(|_, _| response("200 OK", &[], b"unreviewed body")).await;
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    request(
        dir.path(),
        "error",
        "Failure",
        &server.url,
        "scripts:\n  before: |\n    throw vars.get(\"token\");\n",
    );
    request(dir.path(), "ok", "Success", &server.url, "");
    with_client(args(dir.path(), Some(&server.url)), async |client| {
        let revision = client.revision().await;
        let run = client.start("error", &revision, "").await;
        let evidence = client.finished(&run).await;
        assert_eq!(evidence["outcome"], "script_failed");
        assert!(
            evidence["report"]["steps"][0]["ran"]["error"]
                .as_str()
                .unwrap()
                .contains("«redacted»")
        );
    })
    .await;
    let mut grant = args(dir.path(), Some(&server.url));
    grant.excerpt = None;
    with_client(grant, async |client| {
        let revision = client.revision().await;
        let run = client.start("ok", &revision, "").await;
        let outcome = client.finished(&run).await;
        assert_eq!(outcome["outcome"], "no_checks");
        assert!(outcome["report"].is_null());
        assert!(!outcome.to_string().contains("unreviewed body"));
    })
    .await;
}

#[tokio::test]
async fn cancelling_a_request_records_the_cancelled_outcome() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    request(dir.path(), "wait", "Wait", &origin, "");
    with_client(args(dir.path(), Some(&origin)), async |client| {
        let revision = client.revision().await;
        let run = client.start("wait", &revision, "").await;
        let (mut socket, _) = tokio::time::timeout(WAIT, listener.accept())
            .await
            .unwrap()
            .unwrap();
        let sent = tokio::time::timeout(WAIT, read_request(&mut socket))
            .await
            .unwrap();
        assert!(sent.starts_with(b"GET / HTTP/1.1"));
        let cancellation = client.ok("cancel", json!({"run_id":run})).await;
        assert_eq!(cancellation["status"], "cancelling");
        assert_eq!(client.finished(&run).await["outcome"], "cancelled");
        client
            .denied("cancel", json!({"run_id":run}), "No active run")
            .await;
    })
    .await;
}

#[tokio::test]
async fn production_executes_only_with_a_named_opt_in() {
    let server = Server::start(|_, _| response("200 OK", &[], b"ok")).await;
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    std::fs::create_dir(dir.path().join("environments")).unwrap();
    std::fs::write(
        dir.path().join("environments/prod.yaml"),
        "id: prod\nname: Prod\ncolor: red\n",
    )
    .unwrap();
    request(dir.path(), "check", "Check", &server.url, "");
    let mut grant = args(dir.path(), Some(&server.url));
    grant.environment = Some("Prod".to_owned());
    with_client(grant.clone(), async |client| {
        let revision = client.revision().await;
        for environment in ["Prod", "Other"] {
            client
                .denied(
                    "execute",
                    json!({"id":"check","revision":revision,"environment":environment}),
                    "outside",
                )
                .await;
        }
    })
    .await;
    assert_eq!(server.requests().len(), 0);
    grant.production = true;
    with_client(grant, async |client| {
        let revision = client.revision().await;
        let run = client.start("check", &revision, "Prod").await;
        assert_eq!(
            client.finished(&run).await["report"]["definition"]["environment"],
            "Prod"
        );
    })
    .await;
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn a_writer_rechecks_the_snapshot_after_it_acquires_the_lock() {
    use imposter_app::project::{AgentProject, ProjectError, RequestEdit};

    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    request(dir.path(), "one", "One", "http://127.0.0.1:1/one", "");
    let snapshot = AgentProject::read(dir.path()).unwrap();
    request(dir.path(), "two", "Two", "http://127.0.0.1:1/two", "");
    let edit = RequestEdit {
        method: Some("POST".into()),
        ..RequestEdit::default()
    };
    assert!(matches!(
        snapshot.save(dir.path(), "one", edit),
        Err(ProjectError::Stale)
    ));
    assert!(
        !std::fs::read_to_string(dir.path().join("api/one.yaml"))
            .unwrap()
            .contains("POST")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_cannot_add_an_external_definition_to_the_grant() {
    let dir = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    project(dir.path());
    let outside = external.path().join("private.yaml");
    std::fs::write(&outside, format!("id: private\nname: {SECRET}\n")).unwrap();
    std::os::unix::fs::symlink(outside, dir.path().join("api/linked.yaml")).unwrap();
    with_client(args(dir.path(), None), async |client| {
        client
            .denied(
                "find_requests",
                json!({}),
                "Cannot read the granted project definitions",
            )
            .await;
    })
    .await;
}

#[tokio::test]
async fn redaction_cannot_change_a_runs_check_totals_or_outcome() {
    let server = Server::start(|_, _| response("200 OK", &[], b"passed")).await;
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    request(
        dir.path(),
        "request",
        "Checked",
        &server.url,
        "checks:\n  - selector: status\n    op: equals\n    expected: '200'\n",
    );
    let mut grant = args(dir.path(), Some(&server.url));
    grant.inputs.push(InputArg {
        variable: "another".to_owned(),
        from: InputSource::ProcessVar {
            name: "VERDICT_WORD".to_owned(),
            secret: true,
        },
    });
    with_client(grant, async |client| {
        let revision = client.revision().await;
        let run = client.start("request", &revision, "").await;
        let result = client.finished(&run).await;
        assert_eq!(result["outcome"], "passed");
        assert_eq!(result["totals"]["checks_passed"], 1);
    })
    .await;
}
