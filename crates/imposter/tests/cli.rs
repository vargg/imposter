//! The installed command line, as a process: `imposter send` against a loopback server with
//! a throwaway home and data directory, and nothing but the variables it is given.
#![cfg(test)]

use std::path::Path;
use std::process::{Command, Output, Stdio};

use imposter_testkit::{Server, response};
use serde_json::{Value, json};

const SECRET: &str = "tok-SECRET-42";

fn write_document(dir: &Path, base: &str) -> std::path::PathBuf {
    let request = |name: &str, path: &str| {
        json!({
            "name": name, "method": "GET", "url": format!("{{{{base}}}}{path}"),
            "body": "", "body_type": "json",
            "auth": {"type": "bearer_token", "bearer_token": {"token": "{{token}}"}},
            "checks": [{"selector": "status", "op": "equals", "expected": "200"}],
        })
    };
    let document = json!({
        "format": "imposter",
        "version": 2,
        "collection": {
            "name": "API",
            "variables": {"base": base, "token": "none"},
            "folders": [{"name": "Users", "requests": [request("List", "/users"), request("Hold", "/hold")]}],
        },
    });
    let path = dir.join("api.imposter.json");
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    path
}

/// The binary with an empty environment apart from a temp home and data directory, so it
/// can neither read nor create personal data.
fn imposter(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_imposter"));
    command
        .env_clear()
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("IMPOSTER_DATA_DIR", home.join("data"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(root) = std::env::var_os("SYSTEMROOT") {
        // Windows needs it to load its networking libraries.
        command.env("SYSTEMROOT", root);
    }
    command
}

async fn output(mut command: Command) -> Output {
    tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap()
}

fn json_of(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
async fn send_runs_headless_and_touches_no_personal_data() {
    let server = Server::start(|_, _| response("200 OK", &[], b"ok")).await;
    let home = tempfile::tempdir().unwrap();
    let file = write_document(home.path(), &server.url);
    let mut command = imposter(home.path());
    command.args(["send", "--file", file.to_str().unwrap(), "Users/List"]);
    let output = output(command).await;

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let result = json_of(&output);
    assert_eq!(result["outcome"], "passed");
    assert_eq!(result["format"], "imposter-result");
    let received = server.requests();
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].header("user-agent"),
        Some(format!("Imposter/{}", env!("CARGO_PKG_VERSION")).as_str())
    );
    let left: Vec<_> = std::fs::read_dir(home.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        left,
        ["api.imposter.json"],
        "no profile, database or settings were created"
    );
}

#[tokio::test]
async fn secrets_stay_out_of_stdout_and_stderr_even_with_trace_logs() {
    let server = Server::start(|_, received| {
        let auth = received
            .header("authorization")
            .unwrap_or_default()
            .to_owned();
        response(
            "500 Internal Server Error",
            &[("X-Echo", &auth)],
            auth.as_bytes(),
        )
    })
    .await;
    let home = tempfile::tempdir().unwrap();
    let file = write_document(home.path(), &server.url);
    let mut command = imposter(home.path());
    command
        .env("API_TOKEN", SECRET)
        .env("IMPOSTER_LOG", "trace")
        .args([
            "send",
            "--file",
            file.to_str().unwrap(),
            "Users/List",
            "--secret",
            "token=API_TOKEN",
        ]);
    let output = output(command).await;

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        server.requests()[0].header("authorization"),
        Some(format!("Bearer {SECRET}").as_str())
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(SECRET), "{stdout}");
    assert!(!stderr.contains(SECRET), "{stderr}");
    assert!(stderr.contains("Failed: 1 of 1 checks failed"), "{stderr}");
}

#[tokio::test]
async fn usage_errors_exit_2_without_a_result() {
    let home = tempfile::tempdir().unwrap();
    let mut command = imposter(home.path());
    command.args(["send", "--file", "@@BOGUS@@"]);
    let output = output(command).await;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
}

/// Ctrl+C while the response is on its way stops the run and reports it cancelled.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_interrupt_cancels_the_run() {
    let (arrived, mut wait) = tokio::sync::mpsc::unbounded_channel();
    // The server holds its one worker until the test ends; the other runs the test.
    let (_release, hold) = std::sync::mpsc::channel::<()>();
    let hold = std::sync::Mutex::new(hold);
    let server = Server::start(move |_, _| {
        let _ = arrived.send(());
        // Answer only once the test has gone: the run must end without a response.
        let _ = hold.lock().unwrap().recv();
        response("200 OK", &[], b"")
    })
    .await;
    let home = tempfile::tempdir().unwrap();
    let file = write_document(home.path(), &server.url);
    let mut command = imposter(home.path());
    command.args(["send", "--file", file.to_str().unwrap(), "Users/Hold"]);
    let child = command.spawn().unwrap();
    wait.recv().await.unwrap();
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let output = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(130), "{output:?}");
    assert_eq!(json_of(&output)["outcome"], "cancelled");
}

/// The experimental MCP server answers on stdout over a pipe and ends with its input.
#[tokio::test]
async fn mcp_answers_over_stdio_and_ends_with_its_input() {
    use std::io::Write as _;
    let home = tempfile::tempdir().unwrap();
    let file = write_document(home.path(), "http://127.0.0.1:1");
    let mut command = imposter(home.path());
    command
        .args(["mcp", "--file", file.to_str().unwrap()])
        .stdin(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(
            concat!(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
                "\n",
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_requests","arguments":{}}}"#,
                "\n"
            )
            .as_bytes(),
        )
        .unwrap();
    drop(stdin);
    let output = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let answers: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(answers[0]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        answers[1]["result"]["structuredContent"]["requests"],
        json!(["Users/List", "Users/Hold"])
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("experimental"),
        "diagnostics go to stderr"
    );
}

/// The main use: a request saved in the app runs from the command line while the app has
/// the same database open, and the app sees the run in History.
#[tokio::test]
async fn a_saved_request_runs_beside_the_open_app() {
    use imposter_app::core::model::{RequestTemplate, Variables};
    use imposter_app::store::{Store, collections, executions};
    let server = Server::start(|_, _| response("200 OK", &[], b"ok")).await;
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("data");
    std::fs::create_dir(&data).unwrap();
    // Stands in for the running app: its own connection, open throughout.
    let app = Store::open(&data.join("imposter.db")).unwrap();
    let url = format!("{}/users", server.url);
    app.call(move |db| {
        let api = collections::create_collection(db, "API", "", &Variables::new())?;
        let users = collections::create_folder(db, api.id, None, "Users", "", 0)?;
        collections::create_request(
            db,
            api.id,
            Some(users.id),
            &RequestTemplate {
                name: "List".into(),
                method: "GET".into(),
                url,
                ..RequestTemplate::default()
            },
        )?;
        Ok(())
    })
    .await
    .unwrap();

    let mut command = imposter(home.path());
    command.args(["send", "API / Users / List"]);
    let output = output(command).await;

    assert_eq!(output.status.code(), Some(7), "no checks: {output:?}");
    assert_eq!(json_of(&output)["definition"]["source"], "profile");
    assert_eq!(server.requests().len(), 1);
    let rows = app
        .call(|db| executions::page(db, None, 0, 10))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].via, imposter_app::core::execution::Via::CommandLine);
}
