//! Persistence and lifecycle of the user's one local mock server, whose routes answer
//! statically or through their scripts.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use imposter_core::mock::{MockConfig, MockConfigError, MockLogEntry, MockReply, Responded};
use imposter_http::mock::{MockError, Responder};
use imposter_store::{StoreError, StoreHandle, kv};
use tokio_util::sync::CancellationToken;

use crate::executor::script_host;

/// How long a mock route's script may run for one request: a mock must answer fast.
pub const MOCK_SCRIPT_TIME: Duration = Duration::from_secs(1);

/// The words naming a route's script in its errors.
pub const MOCK_SCRIPT: &str = "Mock script";

const KEY: &str = "mock_server";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] MockConfigError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Server(#[from] MockError),
    /// A route's script does not compile; the server is not started.
    #[error("Route {route}: {message}")]
    Script { route: usize, message: String },
}

pub async fn load(store: &StoreHandle) -> Result<MockConfig, Error> {
    let config: MockConfig = store
        .call(|db| Ok(kv::get(db, kv::Table::Settings, KEY)?.unwrap_or_default()))
        .await?;
    config.validate()?;
    Ok(config)
}

pub async fn save(store: &StoreHandle, config: MockConfig) -> Result<(), Error> {
    config.validate()?;
    store
        .call(move |db| kv::set(db, kv::Table::Settings, KEY, &config))
        .await?;
    Ok(())
}

/// The first enabled route whose script does not compile, as Start must refuse it.
pub fn script_problem(config: &MockConfig) -> Option<Error> {
    config
        .routes
        .iter()
        .enumerate()
        .filter(|(_, route)| route.enabled && route.is_scripted())
        .find_map(|(index, route)| {
            imposter_script::check(&route.script)
                .err()
                .map(|error| Error::Script {
                    route: index + 1,
                    message: error.describe(MOCK_SCRIPT),
                })
        })
}

/// Answers scripted routes: one script at a time, so the run's state never loses an update;
/// a failed script answers 500 naming the error.
fn responder(config: &MockConfig, cancel: CancellationToken) -> Option<Responder> {
    if !config.routes.iter().any(|r| r.enabled && r.is_scripted()) {
        return None;
    }
    let routes = config.routes.clone();
    let state = Mutex::new(imposter_script::MockState::default());
    Some(Arc::new(move |index, request| {
        let route = &routes[index];
        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
        let run = imposter_script::mock(
            &route.script,
            &request,
            MockReply::from_route(route),
            &mut state,
            script_host(cancel.clone(), MOCK_SCRIPT_TIME),
        );
        match run.reply {
            Ok(reply) => Responded {
                reply,
                error: None,
                console: run.console,
            },
            Err(error) => Responded::failed(error.describe(MOCK_SCRIPT), run.console),
        }
    }))
}

/// Serve `config` until `cancel`; scripts must compile first. Every answered request is
/// passed to `log`.
pub async fn run(
    config: MockConfig,
    listening: impl FnOnce(std::net::SocketAddr) + Send,
    log: impl Fn(MockLogEntry) + Send + Sync + 'static,
    cancel: CancellationToken,
) -> Result<(), Error> {
    config.validate()?;
    if let Some(error) = script_problem(&config) {
        return Err(error);
    }
    let responder = responder(&config, cancel.clone());
    imposter_http::mock::serve(config, responder, listening, Arc::new(log), cancel).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use imposter_core::mock::MockRoute;
    use imposter_store::Store;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    fn counter() -> MockConfig {
        MockConfig {
            port: 0,
            routes: vec![MockRoute {
                path: "/count".into(),
                script: "state.n = (state.n ?? 0) + 1; print(`call ${state.n}`); response.body = `${state.n}`;".into(),
                ..MockRoute::default()
            }],
        }
    }

    /// Run `config`, send `requests` GETs to `/count`, stop, and give the bodies and log.
    async fn serve_and_count(
        config: MockConfig,
        requests: usize,
    ) -> (Vec<String>, Vec<MockLogEntry>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let cancel = CancellationToken::new();
        let logged = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&logged);
        let task = tokio::spawn(run(
            config,
            move |address| {
                tx.send(address).unwrap();
            },
            move |entry| log.lock().unwrap().push(entry),
            cancel.clone(),
        ));
        let address = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .unwrap()
            .unwrap();
        let mut bodies = Vec::new();
        for _ in 0..requests {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream
                .write_all(b"GET /count HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let mut answer = String::new();
            stream.read_to_string(&mut answer).await.unwrap();
            bodies.push(answer.rsplit("\r\n\r\n").next().unwrap().to_owned());
        }
        cancel.cancel();
        task.await.unwrap().unwrap();
        let log = logged.lock().unwrap().clone();
        (bodies, log)
    }

    #[tokio::test]
    async fn script_state_lasts_for_a_run_and_starts_fresh_on_the_next() {
        let (bodies, log) = serve_and_count(counter(), 3).await;
        assert_eq!(bodies, ["1", "2", "3"]);
        assert_eq!(log[2].console, ["call 3"]);
        let (again, _) = serve_and_count(counter(), 1).await;
        assert_eq!(again, ["1"], "a restart empties the state");
    }

    #[tokio::test]
    async fn a_script_error_answers_500_naming_its_line() {
        let mut config = counter();
        config.routes[0].script = "let a = 1;\nthrow \"no stock\";".into();
        let (bodies, log) = serve_and_count(config, 1).await;
        assert_eq!(bodies, ["Mock script, line 2: no stock"]);
        assert_eq!(log[0].status, 500);
        assert_eq!(
            log[0].error.as_deref(),
            Some("Mock script, line 2: no stock")
        );
    }

    #[tokio::test]
    async fn a_script_that_does_not_compile_keeps_the_server_from_starting() {
        let mut config = counter();
        config.routes.push(MockRoute {
            path: "/broken".into(),
            script: "let x = ;".into(),
            ..MockRoute::default()
        });
        let result = run(
            config,
            |_| panic!("must not listen"),
            |_| {},
            CancellationToken::new(),
        )
        .await;
        let error = result.unwrap_err().to_string();
        assert!(error.starts_with("Route 2: Mock script, line 1"), "{error}");
        let mut disabled = counter();
        disabled.routes[0].script = "let x = ;".into();
        disabled.routes[0].enabled = false;
        assert!(
            script_problem(&disabled).is_none(),
            "a disabled route is not checked"
        );
    }

    #[tokio::test]
    async fn saved_configuration_survives_reopening_and_invalid_saves_do_not_replace_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("imposter.db");
        let store = Store::open(&path).unwrap();
        assert_eq!(load(&store.handle()).await.unwrap(), MockConfig::default());
        let config = MockConfig {
            port: 0,
            routes: vec![MockRoute {
                body: "persistent".into(),
                ..Default::default()
            }],
        };
        save(&store.handle(), config.clone()).await.unwrap();
        let mut invalid = config.clone();
        invalid.routes[0].status = 100;
        assert!(save(&store.handle(), invalid).await.is_err());
        assert_eq!(load(&store.handle()).await.unwrap(), config);
        store.close().unwrap();
        let reopened = Store::open(&path).unwrap();
        assert_eq!(load(&reopened.handle()).await.unwrap(), config);
    }
}
