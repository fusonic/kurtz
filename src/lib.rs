//! Kurtz: a small self-hosted URL shortener.
//!
//! The binary in `main.rs` only wires configuration, database and the HTTP
//! server together. Everything else lives in this library so the test suite in
//! `tests/` can use it directly.

pub mod assets;
pub mod auth;
pub mod cache;
pub mod config;
pub mod csvio;
pub mod db;
pub mod qr;
pub mod redirect;
pub mod validate;
pub mod web;

pub use web::{router, App, AppState};
