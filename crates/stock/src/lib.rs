//! Library half of the stock skill. main.rs consumes these; tests/ imports
//! them directly, which a bin-only crate cannot offer.

pub mod cli;
pub mod quote;
pub mod render;
pub mod sources;
