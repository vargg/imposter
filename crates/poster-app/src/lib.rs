//! Services and runtime glue for Poster. The UI reaches the lower crates through these
//! re-exports only (see the architecture test).

pub mod executor;
pub mod ops;
pub mod paths;
pub mod runtime;
pub mod saver;
pub mod transfer;

pub use poster_core as core;
pub use poster_http as http;
pub use poster_store as store;
pub use poster_transfer as formats;

pub mod mock;
