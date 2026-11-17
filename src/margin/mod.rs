//! Perps (margin) REST and WebSocket API bindings.
//!
//! Obtain a borrowed API with [`crate::Client::margin`]. It shares the client's
//! credentials, REST root, connection pool and timeout. Models retain the names
//! in Kalshi's separate Perps specifications; method names omit `Margin`.
//! Requests are sent once. Pagination, retries, market validation and trading
//! decisions remain the caller's responsibility.

pub mod models;
mod rest;
pub mod websocket;

/// A borrowed Perps API sharing an existing [`crate::Client`].
#[derive(Clone, Copy, Debug)]
pub struct Margin<'a> {
    pub(crate) client: &'a crate::Client,
}
