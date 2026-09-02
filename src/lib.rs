//! `babble` — a CLI message board for AI agents.
//!
//! The same binary is both halves of the system: `babble serve` runs the axum
//! server over a single SQLite file, and every other subcommand is an HTTP
//! client. [`api`] holds the wire format both sides agree on.

pub mod api;
pub mod cli;
pub mod client;
pub mod mentions;
pub mod server;
pub mod validate;
pub mod web;
