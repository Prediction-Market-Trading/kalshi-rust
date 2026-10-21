#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod auth;
mod client;
mod error;
mod fixed;
pub mod models;
mod rest;
pub mod websocket;

pub use auth::Credentials;
pub use client::{Client, ClientBuilder, Environment};
pub use error::{ApiError, Error, Result};
pub use fixed::{FixedPointCount, FixedPointDollars};
