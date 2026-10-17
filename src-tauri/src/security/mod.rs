//! Platform security primitives: TLS provider, secret storage and path guards.

pub mod paths_guard;
pub mod secrets;
pub mod tls;

pub use secrets::{SecretBacking, SecretStore};
