//! HTTP transport for Imposter.

pub mod cookies;
pub mod digest;
pub mod oauth;
pub mod tls;
pub mod transport;
pub mod upload;

pub use transport::{ErrorKind, Measured, Policy, SendError, Session, send};

pub mod mock;
