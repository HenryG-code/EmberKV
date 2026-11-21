//! The EmberKV server as a library, so the `emberkv` program and the integration tests share it.
//!
//! [`resp`] parses client requests, [`command`] runs them against the engine, [`reply`] encodes the
//! answers, and [`server`] handles the network.

pub mod command;
pub mod reply;
pub mod resp;
pub mod server;
