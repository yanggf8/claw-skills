//! Library half of the traffic skill. The binary in main.rs consumes these
//! modules, and tests/ imports them directly — a bin-only crate cannot be
//! imported by an integration test.

pub mod cli;
pub mod locations;
pub mod render;
pub mod route;
