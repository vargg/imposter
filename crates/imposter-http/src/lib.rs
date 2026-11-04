//! HTTP transport for Imposter.

pub mod digest;
pub mod tls;
pub mod transport;
pub mod upload;

pub use transport::{ErrorKind, Policy, SendError, Session, send};

pub mod mock;
