#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod auth;
mod client;
mod error;
mod fixed;
pub mod margin;
pub mod models;
mod rest;
pub mod websocket;
mod websocket_transport;

pub use auth::Credentials;
pub use client::{Client, ClientBuilder, Environment};
pub use error::{ApiError, Error, Result};
pub use fixed::{FixedPointCount, FixedPointDollars};
