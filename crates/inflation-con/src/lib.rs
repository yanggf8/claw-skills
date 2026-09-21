//! Library half of the inflation-con skill. Pure analysis / render / config
//! are available here so integration tests can exercise them without
//! network or clock. The fetch module faces the network via an injected
//! seam (tests never make a real call). `run` is the injectable entry used
//! by contract goldens and the binary.

pub mod analysis;
pub mod config;
pub mod fetch;
pub mod render;
pub mod run;
