//! HTTP transport for Imposter.

pub mod cookies;
pub mod destination;
pub mod digest;
pub mod oauth;
pub mod sign_in;
pub mod tls;
pub mod transport;
pub mod upload;

pub use transport::{
    ErrorKind, EventSink, Listening, Measured, Policy, SendError, Session, send, send_streaming,
};

pub mod mock;
