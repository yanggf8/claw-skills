//! Library half of the chipcon skill. Pure analysis is available here so
//! integration tests can exercise classification without network or clock.
//! `run` is the injectable entry used by contract goldens and the binary.

pub mod analysis;
pub mod config;
pub mod fetch;
pub mod render;
pub mod run;
