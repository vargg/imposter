//! The database actor: one thread owns the connection and the collections folder, and runs
//! commands in order. A command is one transaction: it commits if it returns `Ok` and
//! rolls back otherwise; the folder's files it changed are written just before SQLite
//! commits, and a file that cannot be written rolls the command back.

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

use rusqlite::{Connection, Transaction, TransactionBehavior};
use tokio::sync::oneshot;

use crate::StoreError;
use crate::files::workspace::State;
use crate::schema::{self, MIGRATIONS, Migrated};

type Job = Box<dyn FnOnce(&mut Connection, &mut State) + Send>;

/// The transaction a store command runs in: SQLite and the collections folder together.
/// It derefs to the SQLite connection, so SQL helpers take it as they are.
pub struct Tx<'a> {
    sql: Transaction<'a>,
    /// The folder as committed, moved in for the command and back out after it.
    committed: State,
    staged: RefCell<Option<State>>,
    /// The folder was replaced by another one: its files are read, not written.
    switched: Cell<bool>,
}

impl std::ops::Deref for Tx<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.sql
    }
}

impl Tx<'_> {
    /// The folder as this command sees it, its own changes included.
    pub(crate) fn read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        let staged = self.staged.borrow();
        match staged.as_ref() {
            Some(state) => f(state),
            None => f(&self.committed),
        }
    }

    /// Change the folder; the files are written when the command commits.
    pub(crate) fn write<R>(
        &self,
        f: impl FnOnce(&mut State, &Connection) -> Result<R, StoreError>,
    ) -> Result<R, StoreError> {
        let mut staged = self.staged.borrow_mut();
        let state = staged.get_or_insert_with(|| self.committed.clone());
        f(state, &self.sql)
    }

    /// Use `state` (another folder, just read) from now on; nothing is written to it.
    pub(crate) fn replace(&self, state: State) {
        *self.staged.borrow_mut() = Some(state);
        self.switched.set(true);
    }

    /// The folder the collections live in.
    pub fn folder(&self) -> PathBuf {
        self.read(|state| state.root.clone())
    }

    /// Why the folder could not be read, when it could not; nothing is written meanwhile.
    pub fn folder_error(&self) -> Option<String> {
        self.read(|state| state.error.clone())
    }
}

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
        let state = crate::files::open(&mut connection, path)?;
        let (commands, inbox) = mpsc::channel::<Command>();
        let thread = std::thread::Builder::new()
            .name("imposter-db".to_owned())
            .spawn(move || {
                let mut connection = connection;
                let mut state = state;
                while let Ok(Command::Run(job)) = inbox.recv() {
                    job(&mut connection, &mut state);
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
    state: &mut State,
    work: impl FnOnce(&Tx<'_>) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let committed = std::mem::replace(state, State::unavailable(Path::new(""), ""));
    let tx = Tx {
        sql: transaction,
        committed,
        staged: RefCell::new(None),
        switched: Cell::new(false),
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| work(&tx)));
    let Tx {
        sql,
        committed,
        staged,
        switched,
    } = tx;
    // Whatever happens below, the thread keeps a folder: the committed one unless a staged
    // one was written and committed.
    *state = committed;
    match outcome {
        Ok(Ok(value)) => {
            match staged.into_inner() {
                Some(mut staged) => {
                    // Files first: a refused write drops `sql`, which rolls it back.
                    if !switched.get() {
                        staged.write_changes(state)?;
                    }
                    sql.commit()?;
                    *state = staged;
                }
                None => sql.commit()?,
            }
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
        F: FnOnce(&Tx<'_>) -> Result<T, StoreError> + Send + 'static,
    {
        let (reply, receiver) = oneshot::channel();
        let job: Job = Box::new(move |connection, state| {
            // The caller may have stopped waiting; the work is done either way.
            let _ = reply.send(run(connection, state, work));
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
        F: FnOnce(&Tx<'_>) -> Result<T, StoreError> + Send + 'static,
    {
        self.submit(work)?.await.map_err(|_| StoreError::Closed)?
    }

    /// Like [`Self::call`], blocking the current thread. Never use it on an async runtime
    /// thread or the UI thread.
    pub fn call_blocking<T, F>(&self, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Tx<'_>) -> Result<T, StoreError> + Send + 'static,
    {
        self.submit(work)?
            .blocking_recv()
            .map_err(|_| StoreError::Closed)?
    }
}
