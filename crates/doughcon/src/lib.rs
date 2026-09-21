//! Library half of the doughcon skill. The binary in main.rs consumes these
//! modules, and tests/ imports them directly — a bin-only crate cannot be
//! imported by an integration test.

pub mod cli;
pub mod pizzint;
pub mod report;
