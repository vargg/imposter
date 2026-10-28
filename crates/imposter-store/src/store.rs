//! The database actor: one thread owns the connection and runs commands in order.
//! A command is one transaction: it commits if it returns `Ok` and rolls back otherwise.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

use rusqlite::{Connection, Transaction, TransactionBehavior};
use tokio::sync::oneshot;

use crate::StoreError;
use crate::schema::{self, MIGRATIONS, Migrated};

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

enum Command {
    Run(Job),
    Close,
}

/// Sends commands to the database thread. Cheap to clone; every clone talks to the same thread.
#[derive(Clone)]
pub struct StoreHandle {
    commands: mpsc::Sender<Command>,
}

/// Owns the database thread. Dropping it closes the database after queued commands finish.
pub struct Store {
    handle: StoreHandle,
    thread: Option<JoinHandle<Result<(), StoreError>>>,
    path: PathBuf,
    migrated: Migrated,
}

impl Store {
    /// Open (creating if needed) and migrate the database at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::open_with(path, MIGRATIONS)
    }

    #[doc(hidden)]
    pub fn open_with(path: &Path, migrations: &[&str]) -> Result<Self, StoreError> {
        let mut connection = schema::connect(path)?;
        let migrated = schema::migrate(&mut connection, path, migrations)?;
        let (commands, inbox) = mpsc::channel::<Command>();
        let thread = std::thread::Builder::new()
            .name("imposter-db".to_owned())
            .spawn(move || {
                let mut connection = connection;
                while let Ok(Command::Run(job)) = inbox.recv() {
                    job(&mut connection);
                }
                connection
                    .close()
                    .map_err(|(_, error)| StoreError::Sqlite(error))
            })
            .map_err(StoreError::Thread)?;
        Ok(Self {
            handle: StoreHandle { commands },
            thread: Some(thread),
            path: path.to_owned(),
            migrated,
        })
    }

    pub fn handle(&self) -> StoreHandle {
        self.handle.clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn migrated(&self) -> &Migrated {
        &self.migrated
    }

    /// Finish queued commands, then close the connection. Later calls fail with `Closed`.
    pub fn close(mut self) -> Result<(), StoreError> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<(), StoreError> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        // The thread may already be gone if a previous close happened; either way it stops.
        let _ = self.handle.commands.send(Command::Close);
        thread.join().map_err(|_| StoreError::Panicked)?
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            tracing::error!(%error, "closing the database failed");
        }
    }
}

impl std::ops::Deref for Store {
    type Target = StoreHandle;

    fn deref(&self) -> &StoreHandle {
        &self.handle
    }
}

fn run<T>(
    connection: &mut Connection,
    work: impl FnOnce(&Transaction<'_>) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    match catch_unwind(AssertUnwindSafe(|| work(&transaction))) {
        Ok(Ok(value)) => {
            transaction.commit()?;
            Ok(value)
        }
        // Dropping the transaction rolls it back.
        Ok(Err(error)) => Err(error),
        Err(_) => {
            tracing::error!("a database command panicked; rolled back");
            Err(StoreError::Panicked)
        }
    }
}

impl StoreHandle {
    fn submit<T, F>(&self, work: F) -> Result<oneshot::Receiver<Result<T, StoreError>>, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, StoreError> + Send + 'static,
    {
        let (reply, receiver) = oneshot::channel();
        let job: Job = Box::new(move |connection| {
            // The caller may have stopped waiting; the work is done either way.
            let _ = reply.send(run(connection, work));
        });
        self.commands
            .send(Command::Run(job))
            .map_err(|_| StoreError::Closed)?;
        Ok(receiver)
    }

    /// Run `work` in one transaction on the database thread.
    pub async fn call<T, F>(&self, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, StoreError> + Send + 'static,
    {
        self.submit(work)?.await.map_err(|_| StoreError::Closed)?
    }

    /// Like [`Self::call`], blocking the current thread. Never use it on an async runtime
    /// thread or the UI thread.
    pub fn call_blocking<T, F>(&self, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, StoreError> + Send + 'static,
    {
        self.submit(work)?
            .blocking_recv()
            .map_err(|_| StoreError::Closed)?
    }
}
