//! SQLite persistence for Imposter.

pub mod collections;
pub mod environments;
mod error;
pub mod executions;
pub mod kv;
pub mod profile;
pub mod schema;
mod store;

pub use error::{Conflict, StoreError};
pub use store::{Store, StoreHandle};

/// The transaction a store command runs in.
pub type Tx<'a> = rusqlite::Transaction<'a>;
