//! Persistence and lifecycle of the user's one local static mock server.

use imposter_core::mock::{MockConfig, MockConfigError};
use imposter_http::mock::MockError;
use imposter_store::{StoreError, StoreHandle, kv};
use tokio_util::sync::CancellationToken;

const KEY: &str = "mock_server";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] MockConfigError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Server(#[from] MockError),
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

pub async fn run(
    config: MockConfig,
    listening: impl FnOnce(std::net::SocketAddr) + Send,
    cancel: CancellationToken,
) -> Result<(), Error> {
    imposter_http::mock::serve(config, listening, cancel).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use imposter_core::mock::MockRoute;
    use imposter_store::Store;

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
