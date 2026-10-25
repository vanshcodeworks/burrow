//! Library surface for fuzz targets.
//!
//! The `burrow` binary declares its modules in `main.rs`. This target exposes
//! the state-file parser so a fuzz target can feed it arbitrary bytes without
//! spawning the CLI.

pub mod state;
